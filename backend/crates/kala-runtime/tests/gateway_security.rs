//! The security properties of the native model gateway.
//!
//! The cases mirror `tests/security/test_model_residency.py`: egress is
//! denied by default, every call needs a per-request approval, the manifest
//! is recorded before the dispatch, the transcripts are bounded and
//! byte-safe, and no credential ever reaches a log or a transcript.

mod support;

use std::sync::{Arc, Mutex};

use kala_budget::{BudgetLedger, RunBudget};
use kala_contracts::ArtifactClassification;
use kala_json::CanonicalValue;
use kala_kernel::policy::PolicyEngine;
use kala_kernel::time::KernelTimestamp;
use kala_runtime::gateway::{
    CallbackError, GatewayTimestamp, LlmClient, ModelArtifact, ModelCapability, ModelError,
    ModelGateway, ModelMessage, ModelRequest, ModelResponse, ModelRole,
    OPENAI_COMPATIBLE_PROVIDER_ID, is_secret_name, redact_secrets, truncate_encoded,
};

use support::mock_transport;

const AWS_SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const VALID_RESPONSE: &[u8] =
    b"{\"choices\":[{\"finish_reason\":\"stop\",\"message\":{\"content\":\"{}\"}}]}";
const LOOPBACK_ENDPOINT: &str = "http://127.0.0.1:11434/v1/chat/completions";
const HOSTED_ENDPOINT: &str = "https://api.example.com/v1/chat/completions";

fn recorded_at() -> GatewayTimestamp {
    GatewayTimestamp::Aware(
        KernelTimestamp::from_civil(2026, 5, 6, 7, 8, 9, 0, 0).expect("a valid instant"),
    )
}

fn request(request_id: &str) -> ModelRequest {
    ModelRequest::new(
        request_id,
        vec![ModelMessage::new(ModelRole::User, "summarize the run").expect("a valid message")],
    )
    .expect("a valid request")
}

fn egress_policy() -> Arc<PolicyEngine> {
    Arc::new(
        PolicyEngine::from_capabilities([kala_contracts::Text::new("model.egress")])
            .expect("a policy engine"),
    )
}

fn remote_client(
    transport: Arc<dyn kala_runtime::gateway::ProviderTransport>,
) -> Arc<dyn LlmClient> {
    Arc::new(
        kala_runtime::gateway::OpenAICompatibleClient::new(
            LOOPBACK_ENDPOINT,
            "local-model",
            vec![LOOPBACK_ENDPOINT.to_owned()],
            None,
            true,
            OPENAI_COMPATIBLE_PROVIDER_ID,
            30.0,
            1_048_576,
            transport,
            kala_budget::Decimal::zero(),
            kala_budget::Decimal::zero(),
        )
        .expect("a client"),
    )
}

#[tokio::test]
async fn egress_without_a_policy_engine_fails_closed() {
    let client = remote_client(mock_transport(200, b"{\"choices\":[]}"));
    let gateway = ModelGateway::new(client)
        .with_egress_manifest(Arc::new(|_manifest| Ok(())))
        .with_clock(Arc::new(recorded_at));
    assert!(gateway.requires_egress());
    let error = gateway
        .complete(&request("model-missing-policy"))
        .await
        .unwrap_err();
    assert_eq!(
        error.message(),
        "provider openai-compatible requires egress and no policy engine is attached"
    );
    assert!(gateway.transcripts().is_empty());
    assert!(gateway.manifests().is_empty());
}

#[tokio::test]
async fn egress_requires_a_per_request_approval() {
    let policy = egress_policy();
    let manifests = Arc::new(Mutex::new(Vec::new()));
    let sink = manifests.clone();
    let client = remote_client(mock_transport(200, VALID_RESPONSE));
    let gateway = ModelGateway::new(client)
        .with_policy(policy.clone())
        .with_egress_manifest(Arc::new(move |manifest| {
            sink.lock()
                .expect("the manifest sink")
                .push(manifest.clone());
            Ok(())
        }))
        .with_clock(Arc::new(recorded_at));

    let approved = request("model-approved");
    let other = request("model-other");
    let error = gateway.complete(&approved).await.unwrap_err();
    assert_eq!(
        error.message(),
        "egress policy denied the provider call: approval_required"
    );
    let policy_request = gateway
        .egress_policy_request(&other)
        .expect("a policy request");
    policy
        .approve(
            &policy_request,
            &kala_kernel::policy::DecisionContext::new(),
        )
        .expect("an approval");
    let error = gateway.complete(&approved).await.unwrap_err();
    assert_eq!(
        error.message(),
        "egress policy denied the provider call: approval_required"
    );
    assert!(manifests.lock().expect("the manifest sink").is_empty());

    let policy_request = gateway
        .egress_policy_request(&approved)
        .expect("a policy request");
    policy
        .approve(
            &policy_request,
            &kala_kernel::policy::DecisionContext::new(),
        )
        .expect("an approval");
    assert_eq!(
        gateway
            .complete(&approved)
            .await
            .expect("a response")
            .provider_id(),
        OPENAI_COMPATIBLE_PROVIDER_ID
    );
    assert_eq!(manifests.lock().expect("the manifest sink").len(), 1);
}

#[tokio::test]
async fn a_stale_egress_approval_is_refused() {
    let policy = egress_policy();
    let client = remote_client(mock_transport(200, b"{\"choices\":[]}"));
    let gateway = ModelGateway::new(client)
        .with_policy(policy.clone())
        .with_egress_manifest(Arc::new(|_manifest| Ok(())))
        .with_clock(Arc::new(recorded_at));

    let approved = request("model-stale");
    let policy_request = gateway
        .egress_policy_request(&approved)
        .expect("a policy request");
    policy
        .approve(
            &policy_request,
            &kala_kernel::policy::DecisionContext::new(),
        )
        .expect("an approval");
    let mut mutated = request("model-stale");
    mutated = mutated
        .with_messages(vec![
            ModelMessage::new(ModelRole::User, "mutated text").expect("a valid message"),
        ])
        .expect("a valid request");
    let error = gateway.complete(&mutated).await.unwrap_err();
    assert_eq!(
        error.message(),
        "egress policy denied the provider call: stale_approval"
    );
}

#[tokio::test]
async fn the_egress_action_digest_pins_every_facet() {
    let client = remote_client(mock_transport(200, b"{\"choices\":[]}"));
    let gateway = ModelGateway::new(client)
        .with_policy(egress_policy())
        .with_egress_manifest(Arc::new(|_manifest| Ok(())))
        .with_clock(Arc::new(recorded_at));
    let baseline = gateway
        .egress_action_digest(&request("model-digest"))
        .expect("a digest");
    let state = gateway
        .egress_state_digest(&request("model-digest"))
        .expect("a digest");
    assert_eq!(baseline.len(), 64);
    assert_eq!(state.len(), 64);
    assert_ne!(baseline, state);
    assert_ne!(
        gateway
            .egress_action_digest(&request("model-other"))
            .expect("a digest"),
        baseline
    );
    let mut secret = request("model-digest");
    secret = secret
        .with_messages(vec![
            ModelMessage::new(
                ModelRole::User,
                "use AWS_SECRET_ACCESS_KEY=abcdefghijklmnop",
            )
            .expect("a valid message"),
        ])
        .expect("a valid request");
    assert_ne!(
        gateway.egress_action_digest(&secret).expect("a digest"),
        baseline
    );
    let policy_request = gateway
        .egress_policy_request(&request("model-digest"))
        .expect("a request");
    assert_eq!(
        policy_request.request_id().to_string_lossy(),
        "egress-model-digest"
    );
    assert_eq!(policy_request.action_digest().to_string_lossy(), baseline);
    assert_eq!(policy_request.state_digest().to_string_lossy(), state);
    assert!(policy_request.egress());
    assert!(!policy_request.destructive());
    assert!(policy_request.requires_approval());
}

#[tokio::test]
async fn the_egress_manifest_is_recorded_before_the_dispatch() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let transport = Arc::new(RecordingTransport {
        order: order.clone(),
    });
    let client = remote_client(transport);
    let policy = egress_policy();
    let manifest_order = order.clone();
    let gateway = ModelGateway::new(client)
        .with_policy(policy.clone())
        .with_egress_manifest(Arc::new(move |_manifest| {
            manifest_order
                .lock()
                .expect("the order sink")
                .push("manifest".to_owned());
            Ok(())
        }))
        .with_clock(Arc::new(recorded_at));
    let request = request("model-manifest");
    let policy_request = gateway
        .egress_policy_request(&request)
        .expect("a policy request");
    policy
        .approve(
            &policy_request,
            &kala_kernel::policy::DecisionContext::new(),
        )
        .expect("an approval");
    let _ = gateway.complete(&request).await;
    assert_eq!(
        order.lock().expect("the order sink").clone(),
        vec!["manifest".to_owned(), "dispatch".to_owned()]
    );
}

#[tokio::test]
async fn a_missing_or_failing_egress_manifest_denies_the_call() {
    let policy = egress_policy();
    let client = remote_client(mock_transport(200, b"{\"choices\":[]}"));
    let without_manifest = ModelGateway::new(client.clone())
        .with_policy(policy.clone())
        .with_clock(Arc::new(recorded_at));
    let without_manifest_request = request("model-no-manifest");
    let policy_request = without_manifest
        .egress_policy_request(&without_manifest_request)
        .expect("a policy request");
    policy
        .approve(
            &policy_request,
            &kala_kernel::policy::DecisionContext::new(),
        )
        .expect("an approval");
    let error = without_manifest
        .complete(&without_manifest_request)
        .await
        .unwrap_err();
    assert_eq!(
        error.message(),
        "provider openai-compatible requires egress and no egress manifest is attached"
    );

    let failing = ModelGateway::new(client)
        .with_policy(policy.clone())
        .with_egress_manifest(Arc::new(|_manifest| Err(CallbackError::new("OSError"))))
        .with_clock(Arc::new(recorded_at));
    let failing_request = request("model-failed-manifest");
    let policy_request = failing
        .egress_policy_request(&failing_request)
        .expect("a policy request");
    policy
        .approve(
            &policy_request,
            &kala_kernel::policy::DecisionContext::new(),
        )
        .expect("an approval");
    let error = failing.complete(&failing_request).await.unwrap_err();
    assert_eq!(
        error.message(),
        "the egress manifest could not be recorded: OSError"
    );
    assert!(failing.transcripts().is_empty());
}

#[tokio::test]
async fn the_transcript_truncation_is_byte_safe() {
    let unit = "κόσμε-πληροφορία-";
    let request_text = unit.repeat(1_000);
    let response_text = unit.repeat(3_800);
    let client = Arc::new(ChattyClient {
        content: response_text.clone(),
    });
    let gateway = ModelGateway::new(client).with_clock(Arc::new(recorded_at));
    let request = ModelRequest::new(
        "model-bytes",
        vec![ModelMessage::new(ModelRole::User, &request_text).expect("a valid message")],
    )
    .expect("a valid request");
    gateway.complete(&request).await.expect("a response");
    let transcripts = gateway.transcripts();
    let transcript = transcripts.first().expect("a transcript");
    assert_eq!(
        transcript.request_body().len(),
        kala_runtime::gateway::model::MAX_TRANSCRIPT_BODY_BYTES
    );
    assert!(
        transcript.response_body().len() <= kala_runtime::gateway::model::MAX_TRANSCRIPT_BODY_BYTES
    );
    assert_eq!(
        transcript.request_body(),
        String::from_utf8(transcript.request_body().as_bytes().to_vec()).expect("valid UTF-8")
    );
    let kinds: Vec<&str> = transcript
        .redactions()
        .iter()
        .map(|redaction| redaction.kind())
        .collect();
    assert!(kinds.contains(&"truncated"));
    assert_eq!(truncate_encoded("κόσμε".repeat(10).as_str(), 0), "");
    assert_eq!(truncate_encoded("short", 64), "short");
    assert!(truncate_encoded(&"😀".repeat(100), 10).len() <= 10);
}

#[tokio::test]
async fn the_artifact_preview_and_metadata_text_is_redacted() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let client = Arc::new(RecordingClient { seen: sink });
    let gateway = ModelGateway::new(client).with_clock(Arc::new(recorded_at));
    let binary = ModelArtifact::new(
        "artifact-binary",
        &"a".repeat(64),
        ArtifactClassification::DesignBinary,
        "application/step",
        4096,
    )
    .expect("an artifact")
    .with_preview_text(Some("SECRET-GEOMETRY"))
    .expect("a preview");
    let text = ModelArtifact::new(
        "artifact-text",
        &"a".repeat(64),
        ArtifactClassification::Text,
        "application/step",
        4096,
    )
    .expect("an artifact")
    .with_preview_text(Some(&format!(
        "preview with AWS_SECRET_ACCESS_KEY={AWS_SECRET}"
    )))
    .expect("a preview")
    .with_metadata(&CanonicalValue::object([
        ("source", CanonicalValue::text("inbox")),
        (
            "nested",
            CanonicalValue::object([("api_key", CanonicalValue::text("abcdefghijklmnop"))]),
        ),
        (
            "tags",
            CanonicalValue::array([
                CanonicalValue::text("safe"),
                CanonicalValue::text("Bearer abcdefghijklmnop"),
            ]),
        ),
        ("size", CanonicalValue::integer(12)),
    ]))
    .expect("metadata");
    let request = request("model-artifacts")
        .with_artifacts(vec![binary, text])
        .expect("artifacts");
    gateway.complete(&request).await.expect("a response");
    let sent = seen.lock().expect("the request sink").clone();
    let sent = sent.first().expect("a request");
    assert!(sent.request_body().is_ok());
    let transcripts = gateway.transcripts();
    let transcript = transcripts.first().expect("a transcript");
    assert!(!transcript.request_body().contains(AWS_SECRET));
    assert!(!transcript.request_body().contains("abcdefghijklmnop"));
    let subjects: Vec<&str> = transcript
        .redactions()
        .iter()
        .map(|redaction| redaction.subject())
        .collect();
    assert!(subjects.contains(&"artifact-text.preview_text"));
    assert!(subjects.contains(&"artifact-text.metadata"));
    assert!(subjects.contains(&"artifact-binary"));
}

#[test]
fn the_aws_style_secrets_are_redacted() {
    let cases = [
        format!("AWS_SECRET_ACCESS_KEY={AWS_SECRET}"),
        format!("aws-secret-access-key: '{AWS_SECRET}'"),
        format!("{{\"AWS_SECRET_ACCESS_KEY\": \"{AWS_SECRET}\", \"region\": \"us-east-1\"}}"),
        "export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE".to_owned(),
        "AWS_SESSION_TOKEN=FwoGZXIvYXdzEBYaDF".to_owned(),
        "ghp_abcdefghijklmnopqrstuvwxyz0123".to_owned(),
        "client_secret=abcdefghijklmnop".to_owned(),
        "private_key=-----BEGIN RSA PRIVATE KEY-----".to_owned(),
    ];
    for candidate in cases {
        let (redacted, findings) = redact_secrets(&candidate);
        assert!(findings >= 1, "{candidate}");
        assert!(redacted.contains("<redacted>"), "{candidate}");
        assert!(!redacted.contains(AWS_SECRET));
        assert!(!redacted.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!redacted.contains("abcdefghijklmnop"));
        assert!(!redacted.contains("FwoGZXIvYXdzEBYaDF"));
        assert!(!redacted.contains("ghp_abcdefghijklmnopqrstuvwxyz0123"));
        assert!(!redacted.contains("BEGIN RSA PRIVATE KEY"));
    }
    assert_eq!(
        redact_secrets("nothing sensitive"),
        ("nothing sensitive".to_owned(), 0)
    );
    assert_eq!(redact_secrets(AWS_SECRET), (AWS_SECRET.to_owned(), 0));
}

#[test]
fn the_hosted_endpoints_require_an_allowlist_and_consent() {
    assert_eq!(
        kala_runtime::gateway::normalize_endpoint(LOOPBACK_ENDPOINT, &[], false)
            .expect("a destination"),
        LOOPBACK_ENDPOINT
    );
    assert_eq!(
        kala_runtime::gateway::normalize_endpoint(HOSTED_ENDPOINT, &[], false)
            .unwrap_err()
            .message(),
        "hosted provider endpoints require explicit egress consent"
    );
    assert_eq!(
        kala_runtime::gateway::normalize_endpoint(
            HOSTED_ENDPOINT,
            &[HOSTED_ENDPOINT.to_owned()],
            true
        )
        .expect("a destination"),
        "https://api.example.com:443/v1/chat/completions"
    );
    assert_eq!(
        kala_runtime::gateway::normalize_endpoint(
            "http://api.example.com/v1/chat/completions",
            &[],
            true
        )
        .unwrap_err()
        .message(),
        "plaintext provider endpoints are limited to loopback hosts"
    );
}

#[tokio::test]
async fn the_policy_failures_propagate_without_degradation() {
    let policy = Arc::new(PolicyEngine::new());
    let client = remote_client(mock_transport(200, b"{\"choices\":[]}"));
    let gateway = ModelGateway::new(client)
        .with_policy(policy)
        .with_egress_manifest(Arc::new(|_manifest| Ok(())))
        .with_clock(Arc::new(recorded_at));
    let error = gateway
        .complete(&request("model-policy-denied"))
        .await
        .unwrap_err();
    assert_eq!(error.python_class_name(), "EgressDeniedError");
    assert_eq!(
        error.message(),
        "egress policy denied the provider call: capability_not_granted"
    );
    assert!(gateway.transcripts().is_empty());
    assert!(gateway.manifests().is_empty());
}

#[tokio::test]
async fn the_model_requests_reject_oversized_content() {
    let error = ModelMessage::new(ModelRole::User, &"x".repeat(40_000)).unwrap_err();
    assert_eq!(error.message(), "content exceeds the bounded length");
    assert_eq!(error.python_class_name(), "ModelRequestError");
}

#[test]
fn the_secret_name_detection_is_conservative() {
    assert!(is_secret_name("OPENAI_API_KEY"));
    assert!(!is_secret_name("max_tokens"));
    assert!(!is_secret_name("tokenizer"));
}

/// A client that records the requests it received.
struct RecordingClient {
    seen: Arc<Mutex<Vec<ModelRequest>>>,
}

#[async_trait::async_trait]
impl LlmClient for RecordingClient {
    fn provider_id(&self) -> &str {
        "local"
    }

    fn model_id(&self) -> &str {
        "local-deterministic"
    }

    fn requires_egress(&self) -> bool {
        false
    }

    fn supports(&self, _capability: ModelCapability) -> bool {
        true
    }

    async fn complete(
        &self,
        request: &ModelRequest,
    ) -> Result<ModelResponse, kala_runtime::gateway::ClientError> {
        self.seen
            .lock()
            .expect("the request sink")
            .push(request.clone());
        ModelResponse::new(request.request_id(), "local", "local-deterministic", "{}")
            .map_err(kala_runtime::gateway::ClientError::model)
    }
}

/// A client that answers with a fixed content.
struct ChattyClient {
    content: String,
}

#[async_trait::async_trait]
impl LlmClient for ChattyClient {
    fn provider_id(&self) -> &str {
        "local"
    }

    fn model_id(&self) -> &str {
        "local-deterministic"
    }

    fn requires_egress(&self) -> bool {
        false
    }

    fn supports(&self, _capability: ModelCapability) -> bool {
        true
    }

    async fn complete(
        &self,
        request: &ModelRequest,
    ) -> Result<ModelResponse, kala_runtime::gateway::ClientError> {
        ModelResponse::new(
            request.request_id(),
            "local",
            "local-deterministic",
            &self.content,
        )
        .map_err(kala_runtime::gateway::ClientError::model)
    }
}

/// A transport that records the dispatch order.
struct RecordingTransport {
    order: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl kala_runtime::gateway::ProviderTransport for RecordingTransport {
    async fn post(
        &self,
        _request: kala_runtime::gateway::ProviderRequest,
    ) -> Result<kala_runtime::gateway::ProviderResponse, kala_runtime::gateway::TransportFailure>
    {
        self.order
            .lock()
            .expect("the order sink")
            .push("dispatch".to_owned());
        Ok(kala_runtime::gateway::ProviderResponse {
            status: 200,
            body: b"{\"choices\":[]}".to_vec(),
        })
    }
}

#[allow(dead_code)]
fn _budget_is_used(_: BudgetLedger, _: RunBudget, _: ModelError) {}

#[test]
fn the_provider_request_never_prints_the_credential() {
    let request = kala_runtime::gateway::ProviderRequest {
        url: "http://127.0.0.1:8080/v1".to_owned(),
        headers: vec![
            ("content-type".to_owned(), "application/json".to_owned()),
            ("authorization".to_owned(), "Bearer secret-key".to_owned()),
        ],
        body: b"{}".to_vec(),
        timeout: std::time::Duration::from_secs(30),
        max_response_bytes: 1_048_576,
    };
    let rendered = format!("{request:?}");
    assert!(!rendered.contains("secret-key"));
    assert!(rendered.contains("<redacted>"));
}
