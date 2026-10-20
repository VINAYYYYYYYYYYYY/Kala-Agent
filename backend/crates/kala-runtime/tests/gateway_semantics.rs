//! The semantics of the native model gateway.
//!
//! The cases mirror the repository's Python tests for
//! `kala_agent.runtime.model_gateway` (`tests/unit/test_runtime.py` and
//! `tests/unit/test_model_gateway_branches.py`).

mod support;

use std::sync::Arc;

use kala_budget::{BudgetLedger, RunBudget};
use kala_contracts::ArtifactClassification;
use kala_json::CanonicalValue;
use kala_kernel::policy::PolicyEngine;
use kala_kernel::time::KernelTimestamp;
use kala_runtime::gateway::{
    CallbackError, DecimalInput, GatewayTimestamp, LlmClient, LocalModelClient, ModelArtifact,
    ModelCapability, ModelError, ModelFinishReason, ModelGateway, ModelMessage, ModelRedaction,
    ModelRequest, ModelResponse, ModelRole, ModelTranscript, OPENAI_COMPATIBLE_PROVIDER_ID,
    bounded_float, bounded_identifier, bounded_text, estimate_tokens, freeze_json,
    is_loopback_host, is_secret_name, json_depth, nonnegative_decimal, nonnegative_int,
    normalize_endpoint, parse_destination, redact_metadata, redact_secrets, synthesize_value,
    thaw_json, truncate_encoded,
};

use support::mock_transport;

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn recorded_at() -> GatewayTimestamp {
    GatewayTimestamp::Aware(
        KernelTimestamp::from_civil(2026, 1, 2, 3, 4, 5, 0, 0).expect("a valid instant"),
    )
}

fn message(role: ModelRole, content: &str) -> ModelMessage {
    ModelMessage::new(role, content).expect("a valid message")
}

fn artifact() -> ModelArtifact {
    ModelArtifact::new(
        "artifact-1",
        DIGEST,
        ArtifactClassification::DerivedData,
        "application/json",
        7,
    )
    .expect("a valid artifact")
}

fn request() -> ModelRequest {
    ModelRequest::new("request-1", vec![message(ModelRole::User, "hello")])
        .expect("a valid request")
}

#[test]
fn bounded_identifier_rejects_bad_values() {
    let error = bounded_identifier("has space", "request_id").unwrap_err();
    assert_eq!(error.message(), "request_id must be a safe identifier");
    assert_eq!(
        bounded_identifier("  request-1  ", "request_id").expect("a safe identifier"),
        "request-1"
    );
    assert_eq!(
        bounded_identifier("a".repeat(129).as_str(), "request_id")
            .unwrap_err()
            .message(),
        "request_id must be a safe identifier"
    );
}

#[test]
fn bounded_text_rejects_null_bytes_and_length() {
    assert_eq!(
        bounded_text("a\u{0}b", "content", 32_768)
            .unwrap_err()
            .message(),
        "content cannot contain null bytes"
    );
    assert_eq!(
        bounded_text(&"x".repeat(11), "content", 10)
            .unwrap_err()
            .message(),
        "content exceeds the bounded length"
    );
    assert_eq!(bounded_text("ok", "content", 32_768).expect("ok"), "ok");
}

#[test]
fn nonnegative_int_rejects_negative_values() {
    assert_eq!(
        nonnegative_int(-1, "seed").unwrap_err().message(),
        "seed must be nonnegative"
    );
    assert_eq!(nonnegative_int(0, "seed").expect("zero"), 0);
}

#[test]
fn bounded_float_rejects_out_of_range_values() {
    assert_eq!(
        bounded_float(0.0, "timeout_seconds", 0.1, 10.0)
            .unwrap_err()
            .message(),
        "timeout_seconds must be finite and within its bounds"
    );
    assert_eq!(
        bounded_float(f64::INFINITY, "timeout_seconds", 0.1, 10.0)
            .unwrap_err()
            .message(),
        "timeout_seconds must be finite and within its bounds"
    );
    assert_eq!(
        bounded_float(1.0, "timeout_seconds", 0.1, 10.0).expect("one"),
        1.0
    );
}

#[test]
fn nonnegative_decimal_reports_every_python_message() {
    assert_eq!(
        nonnegative_decimal(&DecimalInput::Text("not-a-number".to_owned()), "cost")
            .unwrap_err()
            .message(),
        "cost must be a nonnegative decimal"
    );
    assert_eq!(
        nonnegative_decimal(
            &DecimalInput::Decimal(kala_budget::Decimal::parse("-1").unwrap()),
            "cost"
        )
        .unwrap_err()
        .message(),
        "cost must be a finite nonnegative decimal"
    );
    assert_eq!(
        nonnegative_decimal(&DecimalInput::Float(f64::NAN), "cost")
            .unwrap_err()
            .message(),
        "cost must be a finite nonnegative decimal"
    );
    assert_eq!(
        nonnegative_decimal(&DecimalInput::Text("1.5".to_owned()), "cost")
            .expect("a cost")
            .to_python_string(),
        "1.5"
    );
}

#[test]
fn freeze_json_bounds_the_nesting() {
    let mut deep = CanonicalValue::integer(1);
    for _ in 0..40 {
        deep = CanonicalValue::object([("a", deep)]);
    }
    assert_eq!(
        freeze_json(&deep).unwrap_err().message(),
        "structured output nesting exceeds the supported depth"
    );
    assert!(json_depth(&deep) > 32);
    let shallow = CanonicalValue::object([("a", CanonicalValue::integer(1))]);
    assert_eq!(freeze_json(&shallow).expect("a shallow value"), shallow);
    assert_eq!(thaw_json(&shallow), shallow);
}

#[test]
fn redact_secrets_removes_credential_shaped_text() {
    let (redacted, findings) = redact_secrets("token sk-abcdefghijklmnopqrstuvwxyz012345");
    assert!(findings >= 1);
    assert!(!redacted.contains("sk-abcdefghijklmnopqrstuvwxyz012345"));
    assert_eq!(
        redact_secrets("nothing secret here"),
        ("nothing secret here".to_owned(), 0)
    );
}

#[test]
fn redact_secrets_covers_prefixed_names_headers_and_key_bodies() {
    let key_body = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASC";
    let pem = format!("-----BEGIN RSA PRIVATE KEY-----\n{key_body}\n-----END RSA PRIVATE KEY-----");
    let cases = [
        ("OPENAI_API_KEY=abc123xyz", "abc123xyz"),
        ("export DB_PASSWORD='hunter22'", "hunter22"),
        (r#"{"OPENAI_API_KEY": "abc123xyz"}"#, "abc123xyz"),
        (r#"{"db_password": "a\"quoted\"secret"}"#, "secret"),
        ("Authorization: Basic dXNlcjpwYXNz", "dXNlcjpwYXNz"),
        ("proxy-authorization: Digest username=x", "username"),
    ];
    for (text, secret) in cases {
        let (redacted, findings) = redact_secrets(text);
        assert!(findings >= 1, "{text}");
        assert!(!redacted.contains(secret), "{text}");
    }
    assert_eq!(
        redact_secrets(&format!("{pem}\ntrailer")).0,
        "<redacted>\ntrailer"
    );
    assert_eq!(
        redact_secrets("max_tokens=512 token_count=3"),
        ("max_tokens=512 token_count=3".to_owned(), 0)
    );
    let once = redact_secrets(r#"{"api_key": "abc"} Authorization: Basic abc"#).0;
    assert_eq!(redact_secrets(&once), (once, 0));
}

#[test]
fn is_secret_name_detects_credential_keys() {
    for name in [
        "api_key",
        "AWS_SECRET_ACCESS_KEY",
        "passphrase",
        "credentials",
        "Authorization",
        "OPENAI_API_KEY",
        "db-password",
        "github_token",
    ] {
        assert!(is_secret_name(name), "{name}");
    }
    for name in ["units", "max_tokens", "secretary"] {
        assert!(!is_secret_name(name), "{name}");
    }
}

#[test]
fn redact_metadata_covers_every_shape() {
    let metadata = CanonicalValue::object([
        ("api_key", CanonicalValue::text("value")),
        (
            "note",
            CanonicalValue::text("token sk-abcdefghijklmnopqrstuvwxyz012345"),
        ),
        (
            "nested",
            CanonicalValue::object([
                ("password", CanonicalValue::text("value")),
                ("count", CanonicalValue::integer(1)),
            ]),
        ),
        (
            "items",
            CanonicalValue::array([
                CanonicalValue::text("sk-abcdefghijklmnopqrstuvwxyz012345"),
                CanonicalValue::object([("secret_key", CanonicalValue::text("v"))]),
            ]),
        ),
        ("plain", CanonicalValue::integer(7)),
    ]);
    let (redacted, findings) = redact_metadata(&metadata);
    assert!(findings >= 4);
    let text = kala_json::canonical_json_dumps(&redacted).expect("canonical metadata");
    assert!(!text.contains("sk-abcdefghijklmnopqrstuvwxyz012345"));
    assert!(text.contains("\"api_key\":\"<redacted>\""));
    assert!(text.contains("\"plain\":7"));
}

#[test]
fn truncate_encoded_cuts_at_the_byte_bound() {
    assert_eq!(truncate_encoded("hello", 10), "hello");
    assert_eq!(truncate_encoded("hello", 3), "hel");
    assert_eq!(truncate_encoded("καλός", 3), "κ");
    assert_eq!(truncate_encoded(&"x".repeat(100), 0), "");
    assert_eq!(truncate_encoded("é", 1), "");
}

#[test]
fn estimate_tokens_rounds_up() {
    assert_eq!(estimate_tokens(""), 0);
    assert_eq!(estimate_tokens("a"), 1);
    assert_eq!(estimate_tokens("abcd"), 1);
    assert_eq!(estimate_tokens("abcde"), 2);
}

#[test]
fn synthesize_value_honours_const_enum_and_default() {
    let const_schema = CanonicalValue::object([("const", CanonicalValue::integer(7))]);
    assert_eq!(
        synthesize_value(&const_schema, 0).expect("a value"),
        CanonicalValue::integer(7)
    );
    let enum_schema = CanonicalValue::object([(
        "enum",
        CanonicalValue::array([CanonicalValue::text("a"), CanonicalValue::text("b")]),
    )]);
    assert_eq!(
        synthesize_value(&enum_schema, 0).expect("a value"),
        CanonicalValue::text("a")
    );
    let object_schema = CanonicalValue::object([
        ("type", CanonicalValue::text("object")),
        (
            "properties",
            CanonicalValue::object([
                (
                    "a",
                    CanonicalValue::object([("type", CanonicalValue::text("string"))]),
                ),
                (
                    "b",
                    CanonicalValue::object([("type", CanonicalValue::text("integer"))]),
                ),
            ]),
        ),
        (
            "required",
            CanonicalValue::array([
                CanonicalValue::text("a"),
                CanonicalValue::text("b"),
                CanonicalValue::text("c"),
            ]),
        ),
    ]);
    assert_eq!(
        synthesize_value(&object_schema, 0).expect("a value"),
        CanonicalValue::object([
            ("a", CanonicalValue::text("local-offline-deterministic")),
            ("b", CanonicalValue::integer(0)),
            ("c", CanonicalValue::Null),
        ])
    );
    let array_schema = CanonicalValue::object([
        ("type", CanonicalValue::text("array")),
        ("minItems", CanonicalValue::integer(2)),
        (
            "items",
            CanonicalValue::object([("type", CanonicalValue::text("boolean"))]),
        ),
    ]);
    assert_eq!(
        synthesize_value(&array_schema, 0).expect("a value"),
        CanonicalValue::array([CanonicalValue::Bool(false), CanonicalValue::Bool(false)])
    );
}

#[test]
fn model_message_validates_its_role_and_content() {
    assert_eq!(
        message(ModelRole::User, "hello").to_payload(),
        CanonicalValue::object([
            ("content", CanonicalValue::text("hello")),
            ("role", CanonicalValue::text("user")),
        ])
    );
    assert_eq!(
        ModelMessage::from_text("wizard", "hello")
            .unwrap_err()
            .message(),
        "role is not a model message role"
    );
    assert_eq!(
        ModelMessage::new(ModelRole::User, "a\u{0}b")
            .unwrap_err()
            .message(),
        "content cannot contain null bytes"
    );
}

#[test]
fn model_artifact_validates_every_field() {
    let artifact = artifact()
        .with_preview_text(Some("preview"))
        .expect("a preview")
        .with_path(Some("/tmp/a.json"))
        .expect("a path")
        .with_metadata(&CanonicalValue::object([("a", CanonicalValue::integer(1))]))
        .expect("metadata");
    let payload = artifact.to_payload();
    assert!(matches!(payload, CanonicalValue::Object(_)));
    assert_eq!(
        artifact
            .clone()
            .with_preview_text(Some(&"x".repeat(9_000)))
            .unwrap_err()
            .message(),
        "preview_text exceeds the bounded length"
    );
    assert_eq!(
        artifact
            .clone()
            .with_path(Some(&"x".repeat(2_000)))
            .unwrap_err()
            .message(),
        "path exceeds the bounded length"
    );
    assert_eq!(
        artifact
            .clone()
            .with_metadata(&CanonicalValue::array(Vec::new()))
            .unwrap_err()
            .message(),
        "metadata must be a JSON object"
    );
    assert_eq!(
        ModelArtifact::new(
            "bad id",
            DIGEST,
            ArtifactClassification::Text,
            "text/plain",
            1
        )
        .unwrap_err()
        .message(),
        "artifact_id must be a safe identifier"
    );
    assert_eq!(
        ModelArtifact::new(
            "artifact-1",
            "not-a-digest",
            ArtifactClassification::Text,
            "text/plain",
            1
        )
        .unwrap_err()
        .message(),
        "digest must be a sha256 hexadecimal digest"
    );
    assert_eq!(
        ModelArtifact::new(
            "artifact-1",
            DIGEST,
            ArtifactClassification::Text,
            "text/plain",
            -1
        )
        .unwrap_err()
        .message(),
        "size_bytes must be nonnegative"
    );
}

#[test]
fn model_artifact_redaction_strips_content() {
    let artifact = artifact()
        .with_preview_text(Some("preview"))
        .expect("a preview")
        .with_path(Some("/tmp/a.json"))
        .expect("a path")
        .with_metadata(&CanonicalValue::object([("a", CanonicalValue::integer(1))]))
        .expect("metadata");
    let redacted = artifact.redacted();
    assert!(redacted.preview_text().is_none());
    assert!(redacted.path().is_none());
    assert_eq!(redacted.digest(), artifact.digest());
}

#[test]
fn model_request_validates_every_field() {
    assert_eq!(
        ModelRequest::new("request-1", Vec::new())
            .unwrap_err()
            .message(),
        "a model request requires at least one message"
    );
    assert_eq!(
        request()
            .with_capabilities(vec![ModelCapability::StructuredOutput])
            .unwrap_err()
            .message(),
        "structured output requires a response schema"
    );
    assert_eq!(
        request()
            .with_capabilities(vec![ModelCapability::Seed])
            .unwrap_err()
            .message(),
        "a seeded request requires a seed value"
    );
    assert_eq!(
        request()
            .with_seed(Some(kala_runtime::gateway::model::MAX_SEED + 1))
            .unwrap_err()
            .message(),
        "seed exceeds the supported bound"
    );
    assert_eq!(
        request()
            .with_max_output_tokens(kala_runtime::gateway::model::MAX_OUTPUT_TOKENS + 1)
            .unwrap_err()
            .message(),
        "max_output_tokens exceeds the supported bound"
    );
    assert_eq!(
        request().with_timeout_seconds(0.0).unwrap_err().message(),
        "timeout_seconds must be finite and within its bounds"
    );
    assert_eq!(
        request()
            .with_artifacts(vec![artifact(), artifact()])
            .unwrap_err()
            .message(),
        "artifact identifiers must be unique"
    );
}

#[test]
fn model_request_body_binds_every_field() {
    // The cross-field checks run on every mutation, exactly like the Python
    // `dataclasses.replace`, so a seeded request is built in one step.
    let request = ModelRequest::from_parts(
        "request-1",
        vec![message(ModelRole::User, "hello")],
        vec![ModelCapability::Seed],
        None,
        vec![artifact()],
        Some(7),
        1_024,
        60.0,
    )
    .expect("a valid request");
    let body = request.request_body().expect("a request body");
    assert!(body.contains("\"seed\":7"));
    assert!(body.contains("\"capabilities\":[\"seed\"]"));
    assert!(body.contains("\"response_schema\":null"));
    assert_eq!(
        request.request_digest().expect("a digest"),
        kala_tools::protocol::sha256_hex_text(&body)
    );
}

#[test]
fn model_response_validates_every_field() {
    let response = ModelResponse::new("request-1", "local", "local-deterministic", "{\"ok\":true}")
        .expect("a response");
    assert_eq!(response.total_tokens(), 0);
    assert_eq!(
        response.response_digest(),
        kala_tools::protocol::sha256_hex_text("{\"ok\":true}")
    );
    assert_eq!(
        response.canonical_content().expect("content"),
        "{\"ok\":true}"
    );
    assert_eq!(
        ModelResponse::new("bad id", "local", "local-deterministic", "{}")
            .unwrap_err()
            .message(),
        "request_id must be a safe identifier"
    );
    assert_eq!(
        ModelResponse::new("request-1", "", "local-deterministic", "{}")
            .unwrap_err()
            .message(),
        "provider_id must be a safe identifier"
    );
    assert_eq!(
        response.clone().with_tokens(-1, 0).unwrap_err().message(),
        "input_tokens must be nonnegative"
    );
    assert_eq!(
        response
            .clone()
            .with_cost(&DecimalInput::Text("-1".to_owned()))
            .unwrap_err()
            .message(),
        "cost must be a finite nonnegative decimal"
    );
    assert_eq!(
        response
            .clone()
            .with_provider_request_id(Some(&"x".repeat(300)))
            .unwrap_err()
            .message(),
        "provider_request_id exceeds the bounded length"
    );
    assert_eq!(
        response
            .with_finish_reason(ModelFinishReason::Length)
            .finish_reason(),
        ModelFinishReason::Length
    );
}

#[test]
fn model_transcript_validates_its_bodies() {
    let transcript = ModelTranscript::new(
        "request-1",
        "local",
        "local-deterministic",
        DIGEST,
        DIGEST,
        "{}",
        "{}",
        1,
        1,
        kala_budget::Decimal::zero(),
        vec![ModelRedaction::new("secret", "user", "removed")],
        vec![DIGEST.to_owned()],
        recorded_at(),
    )
    .expect("a transcript");
    let payload = transcript.to_payload();
    let CanonicalValue::Object(entries) = &payload else {
        panic!("the transcript payload is an object");
    };
    assert_eq!(
        entries
            .get(&kala_json::JsonText::new("cost"))
            .and_then(|value| value.as_text())
            .map(|value| value.to_string_lossy()),
        Some("0".to_owned())
    );
    assert_eq!(
        ModelTranscript::new(
            "request-1",
            "local",
            "local-deterministic",
            DIGEST,
            DIGEST,
            "{}",
            "{}",
            1,
            1,
            kala_budget::Decimal::zero(),
            Vec::new(),
            Vec::new(),
            GatewayTimestamp::Naive(
                chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
                    .expect("a date")
                    .and_hms_opt(0, 0, 0)
                    .expect("a time")
            ),
        )
        .unwrap_err()
        .message(),
        "recorded_at must include a timezone"
    );
    assert_eq!(
        ModelTranscript::new(
            "request-1",
            "local",
            "local-deterministic",
            DIGEST,
            DIGEST,
            &"x".repeat(kala_runtime::gateway::model::MAX_TRANSCRIPT_BODY_BYTES + 1),
            "{}",
            1,
            1,
            kala_budget::Decimal::zero(),
            Vec::new(),
            Vec::new(),
            recorded_at(),
        )
        .unwrap_err()
        .message(),
        "the transcript request body exceeds the bounded size"
    );
}

#[test]
fn parse_destination_accepts_https_endpoints() {
    let destination =
        parse_destination("https://API.Example.com:8443/v1/../v2").expect("a destination");
    assert_eq!(destination.scheme(), "https");
    assert_eq!(destination.host(), "api.example.com");
    assert_eq!(destination.port(), 8443);
    assert_eq!(destination.path(), "/v2");
    assert!(destination.is_tls());
    assert!(!destination.is_loopback());
    assert_eq!(
        parse_destination("http://localhost")
            .expect("a destination")
            .port(),
        80
    );
    assert_eq!(
        parse_destination("https://example.com")
            .expect("a destination")
            .path(),
        "/"
    );
}

#[test]
fn parse_destination_rejects_malformed_endpoints() {
    let cases = [
        ("   ", "the provider endpoint must be nonempty and bounded"),
        (
            "https://[unclosed",
            "the provider endpoint is not a valid URL",
        ),
        (
            "ftp://example.com",
            "the provider endpoint must use http or https",
        ),
        (
            "https://user:secret@example.com",
            "the provider endpoint cannot contain credentials",
        ),
        (
            "https://example.com?a=1",
            "the provider endpoint cannot contain a query or fragment",
        ),
        ("https:///path", "the provider endpoint must contain a host"),
        (
            "https://example.com:70000",
            "the provider endpoint is not a valid URL",
        ),
    ];
    for (value, message) in cases {
        assert_eq!(
            parse_destination(value).unwrap_err().message(),
            message,
            "{value}"
        );
    }
}

#[test]
fn normalize_endpoint_requires_tls_and_consent() {
    assert_eq!(
        normalize_endpoint("http://127.0.0.1:8080/v1", &[], false).expect("a destination"),
        "http://127.0.0.1:8080/v1"
    );
    assert_eq!(
        normalize_endpoint("http://localhost/v1", &[], false).expect("a destination"),
        "http://localhost:80/v1"
    );
    assert_eq!(
        normalize_endpoint("http://example.com/v1", &[], false)
            .unwrap_err()
            .message(),
        "plaintext provider endpoints are limited to loopback hosts"
    );
    assert_eq!(
        normalize_endpoint("https://example.com/v1", &[], false)
            .unwrap_err()
            .message(),
        "hosted provider endpoints require explicit egress consent"
    );
    assert_eq!(
        normalize_endpoint(
            "https://example.com/v1",
            &["https://other.example.com/v1".to_owned()],
            true
        )
        .unwrap_err()
        .message(),
        "the hosted provider endpoint is not in the egress allowlist"
    );
    assert_eq!(
        normalize_endpoint(
            "https://example.com/v1",
            &["https://example.com:443/v1".to_owned()],
            true
        )
        .expect("a destination"),
        "https://example.com:443/v1"
    );
}

#[test]
fn is_loopback_host_accepts_names_and_addresses() {
    assert!(is_loopback_host("localhost"));
    assert!(is_loopback_host("127.0.0.1"));
    assert!(is_loopback_host("::1"));
    assert!(!is_loopback_host("example.com"));
    assert!(!is_loopback_host("not an address"));
}

#[tokio::test]
async fn local_model_client_answers_a_digest_only_request() {
    let client = LocalModelClient::new(
        "local-deterministic",
        "local",
        vec![ModelCapability::StructuredOutput, ModelCapability::Seed],
    )
    .expect("a client");
    assert!(!client.requires_egress());
    assert_eq!(client.calls(), 0);
    assert!(client.supports(ModelCapability::Seed));
    assert!(!client.supports(ModelCapability::ToolCalling));
    let response = client.complete(&request()).await.expect("a response");
    assert_eq!(response.provider_id(), "local");
    assert_eq!(response.finish_reason(), ModelFinishReason::Stop);
    assert!(response.input_tokens() >= 1);
    assert_eq!(client.calls(), 1);
    let expected = kala_tools::protocol::canonical_json_dumps(
        &CanonicalValue::object([(
            "request_digest",
            CanonicalValue::text(request().request_digest().expect("a digest")),
        )]),
        kala_runtime::gateway::model::MAX_RESPONSE_BYTES,
    )
    .expect("canonical json");
    assert_eq!(response.content(), expected);
}

#[tokio::test]
async fn local_model_client_refuses_unsupported_requests() {
    let client =
        LocalModelClient::new("local-deterministic", "local", Vec::new()).expect("a client");
    let error = client
        .complete(
            &request()
                .with_capabilities(vec![ModelCapability::ToolCalling])
                .expect("capabilities"),
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.as_model().map(|error| error.message()),
        Some("provider local does not declare tool_calling")
    );
}

#[tokio::test]
async fn gateway_completes_and_records_a_transcript() {
    let client = Arc::new(
        LocalModelClient::new("local-deterministic", "local", Vec::new()).expect("a client"),
    );
    let gateway = ModelGateway::new(client).with_clock(Arc::new(recorded_at));
    assert_eq!(gateway.provider_id(), "local");
    assert_eq!(gateway.model_id(), "local-deterministic");
    assert!(!gateway.requires_egress());
    assert_eq!(gateway.destination(), "provider:local");
    assert!(gateway.transcripts().is_empty());
    let response = gateway.complete(&request()).await.expect("a response");
    assert_eq!(response.provider_id(), "local");
    assert_eq!(gateway.transcripts().len(), 1);
    assert!(gateway.manifests().is_empty());
}

#[tokio::test]
async fn gateway_accounts_budget_before_and_after_dispatch() {
    let ledger = Arc::new(BudgetLedger::new(
        RunBudget::new()
            .with_max_model_calls(1u32)
            .with_max_total_tokens(4_000u32),
    ));
    let client = Arc::new(
        LocalModelClient::new("local-deterministic", "local", Vec::new()).expect("a client"),
    );
    let gateway = ModelGateway::new(client)
        .with_ledger(ledger.clone())
        .with_clock(Arc::new(recorded_at));
    let response = gateway.complete(&request()).await.expect("a response");
    assert!(response.total_tokens() > 0);
    assert_eq!(ledger.usage().model_calls().to_string(), "1");
    assert_eq!(
        ledger.usage().total_tokens().to_string(),
        response.total_tokens().to_string()
    );
    assert!(ledger.reservations().is_empty());
    let error = gateway.complete(&request()).await.unwrap_err();
    assert_eq!(error.python_class_name(), "ModelCallBudgetExhaustedError");
}

#[tokio::test]
async fn gateway_transcript_failure_fails_closed() {
    let client = Arc::new(
        LocalModelClient::new("local-deterministic", "local", Vec::new()).expect("a client"),
    );
    let gateway = ModelGateway::new(client)
        .with_transcript(Arc::new(|_transcript| Err(CallbackError::new("OSError"))))
        .with_clock(Arc::new(recorded_at));
    let error = gateway.complete(&request()).await.unwrap_err();
    assert_eq!(error.python_class_name(), "TranscriptError");
    assert_eq!(
        error.message(),
        "the model transcript could not be persisted: OSError"
    );
}

#[tokio::test]
async fn gateway_detects_provider_substitution() {
    let client = Arc::new(SubstitutingClient);
    let gateway = ModelGateway::new(client).with_clock(Arc::new(recorded_at));
    let error = gateway.complete(&request()).await.unwrap_err();
    assert_eq!(error.python_class_name(), "ProviderSubstitutionError");
}

#[tokio::test]
async fn gateway_degrades_an_unavailable_provider() {
    let client = Arc::new(FailingClient);
    let gateway = ModelGateway::new(client).with_clock(Arc::new(recorded_at));
    let error = gateway.complete(&request()).await.unwrap_err();
    assert_eq!(error.python_class_name(), "ProviderDegradedError");
    assert!(error.message().contains("no substitute provider is used"));
    assert!(gateway.transcripts().is_empty());
}

#[tokio::test]
async fn the_openai_client_never_leaks_its_api_key() {
    let transport = mock_transport(200, b"{\"choices\":[]}");
    let client = kala_runtime::gateway::OpenAICompatibleClient::new(
        "http://127.0.0.1:8080",
        "gpt",
        Vec::new(),
        Some("secret-key".to_owned()),
        false,
        OPENAI_COMPATIBLE_PROVIDER_ID,
        30.0,
        1_048_576,
        transport,
        kala_budget::Decimal::zero(),
        kala_budget::Decimal::zero(),
    )
    .expect("a client");
    let rendered = format!("{client}");
    assert!(!rendered.contains("secret-key"));
    assert!(rendered.contains("<redacted>"));
    assert!(rendered.contains("api_key=None") || rendered.contains("api_key='<redacted>'"));
    let headers = client.headers();
    assert_eq!(headers.len(), 2);
    assert_eq!(headers[1].0, "authorization");
    assert_eq!(headers[1].1, "Bearer secret-key");
}

#[tokio::test]
async fn the_openai_client_parses_a_provider_response() {
    let payload = br#"{"choices":[{"finish_reason":"stop","message":{"content":"{\"ok\":true}"}}],"id":"provider-request-1","model":"gpt-2024","usage":{"completion_tokens":3,"prompt_tokens":2}}"#;
    let transport = mock_transport(200, payload);
    let client = kala_runtime::gateway::OpenAICompatibleClient::new(
        "http://127.0.0.1:8080",
        "gpt",
        Vec::new(),
        None,
        false,
        OPENAI_COMPATIBLE_PROVIDER_ID,
        30.0,
        1_048_576,
        transport,
        kala_budget::Decimal::parse("1").expect("a cost"),
        kala_budget::Decimal::parse("2").expect("a cost"),
    )
    .expect("a client");
    let response = client
        .parse_response(&request(), payload)
        .expect("a response");
    assert_eq!(response.content(), "{\"ok\":true}");
    assert_eq!(response.model_id(), "gpt-2024");
    assert_eq!(response.provider_request_id(), Some("provider-request-1"));
    assert_eq!(response.input_tokens(), 2);
    assert_eq!(response.output_tokens(), 3);
    assert_eq!(response.cost().to_python_string(), "0.000008");
    assert_eq!(response.finish_reason(), ModelFinishReason::Stop);
}

#[tokio::test]
async fn the_openai_client_rejects_malformed_provider_responses() {
    let transport = mock_transport(200, b"{}");
    let client = kala_runtime::gateway::OpenAICompatibleClient::new(
        "http://127.0.0.1:8080",
        "gpt",
        Vec::new(),
        None,
        false,
        OPENAI_COMPATIBLE_PROVIDER_ID,
        30.0,
        1_048_576,
        transport,
        kala_budget::Decimal::zero(),
        kala_budget::Decimal::zero(),
    )
    .expect("a client");
    assert_eq!(
        client
            .parse_response(&request(), b"{not json")
            .unwrap_err()
            .message(),
        "the provider response is not strict JSON"
    );
    assert_eq!(
        client
            .parse_response(&request(), b"[]")
            .unwrap_err()
            .message(),
        "the provider response must be a JSON object"
    );
    assert_eq!(
        client
            .parse_response(&request(), b"{\"choices\":[]}")
            .unwrap_err()
            .message(),
        "the provider response contains no choices"
    );
    assert_eq!(
        client
            .parse_response(&request(), b"{\"choices\":[1]}")
            .unwrap_err()
            .message(),
        "the provider choice is malformed"
    );
    assert_eq!(
        client
            .parse_response(&request(), b"{\"choices\":[{}]}")
            .unwrap_err()
            .message(),
        "the provider choice has no message"
    );
    assert_eq!(
        client
            .parse_response(
                &request(),
                b"{\"choices\":[{\"message\":{\"content\":\"  \"}}]}"
            )
            .unwrap_err()
            .message(),
        "the provider returned no textual content"
    );
}

#[tokio::test]
async fn the_remote_provider_never_receives_artifact_payloads() {
    let transport = mock_transport(200, b"{\"choices\":[]}");
    let client = kala_runtime::gateway::OpenAICompatibleClient::new(
        "http://127.0.0.1:8080",
        "gpt",
        Vec::new(),
        None,
        false,
        OPENAI_COMPATIBLE_PROVIDER_ID,
        30.0,
        1_048_576,
        transport,
        kala_budget::Decimal::zero(),
        kala_budget::Decimal::zero(),
    )
    .expect("a client");
    let request = request()
        .with_artifacts(vec![artifact()])
        .expect("artifacts");
    assert_eq!(
        client.payload(&request).unwrap_err().message(),
        "artifact payloads are never sent to a remote provider"
    );
}

#[tokio::test]
async fn the_gateway_refuses_egress_without_a_policy() {
    let transport = mock_transport(200, b"{\"choices\":[]}");
    let client = Arc::new(
        kala_runtime::gateway::OpenAICompatibleClient::new(
            "http://127.0.0.1:8080",
            "gpt",
            vec!["http://127.0.0.1:8080".to_owned()],
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
    );
    let gateway = ModelGateway::new(client).with_clock(Arc::new(recorded_at));
    assert!(gateway.requires_egress());
    let error = gateway.complete(&request()).await.unwrap_err();
    assert_eq!(
        error.message(),
        "provider openai-compatible requires egress and no policy engine is attached"
    );
    assert!(gateway.transcripts().is_empty());
}

#[tokio::test]
async fn the_gateway_binds_every_egress_facet() {
    let transport = mock_transport(200, b"{\"choices\":[]}");
    let client = Arc::new(
        kala_runtime::gateway::OpenAICompatibleClient::new(
            "http://127.0.0.1:8080",
            "gpt",
            vec!["http://127.0.0.1:8080".to_owned()],
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
    );
    let gateway = ModelGateway::new(client).with_clock(Arc::new(recorded_at));
    let policy_request = gateway
        .egress_policy_request(&request())
        .expect("a policy request");
    assert_eq!(
        policy_request.request_id().to_string_lossy(),
        "egress-request-1"
    );
    assert!(policy_request.egress());
    assert_eq!(policy_request.side_effect_level().as_str(), "egress");
    assert_eq!(policy_request.requested_capabilities().len(), 1);
    assert_eq!(
        gateway
            .egress_state_digest(&request())
            .expect("a digest")
            .len(),
        64
    );
    assert_eq!(
        gateway
            .egress_action_digest(&request())
            .expect("a digest")
            .len(),
        64
    );
    assert_ne!(
        gateway.egress_state_digest(&request()).expect("a digest"),
        gateway.egress_action_digest(&request()).expect("a digest")
    );
}

struct SubstitutingClient;

#[async_trait::async_trait]
impl LlmClient for SubstitutingClient {
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
            "somewhere-else",
            "local-deterministic",
            "{}",
        )
        .map_err(kala_runtime::gateway::ClientError::model)
    }
}

struct FailingClient;

#[async_trait::async_trait]
impl LlmClient for FailingClient {
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
        _request: &ModelRequest,
    ) -> Result<ModelResponse, kala_runtime::gateway::ClientError> {
        Err(kala_runtime::gateway::ClientError::model(
            ModelError::unavailable("connection refused"),
        ))
    }
}

#[allow(dead_code)]
fn _policy_engine_is_used(_: &PolicyEngine) {}
