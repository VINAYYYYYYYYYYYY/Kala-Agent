//! The live Python differential for the model gateway.
//!
//! The test drives the repository's own Python gateway and the native Rust
//! gateway against the same scenario and the same mock provider server, and
//! compares the decisions, the budget ledger bytes, the requests that were
//! sent (headers minus the credential) and the results.
//!
//! The Python interpreter is a test oracle only; the crate never invokes it
//! at runtime.

mod support;

use std::sync::Arc;

use kala_budget::{BudgetLedger, RunBudget};
use kala_json::{CanonicalValue, JsonText};
use kala_kernel::policy::PolicyEngine;
use kala_kernel::time::KernelTimestamp;
use kala_runtime::gateway::decimal::DecimalArithmeticError;
use kala_runtime::gateway::{
    ClientError, GatewayError, GatewayTimestamp, LlmClient, LocalModelClient, ModelCapability,
    ModelError, ModelGateway, ModelRequest, ModelResponse, OpenAICompatibleClient,
    ProviderTransport, ReqwestTransport,
};
use num_bigint::BigUint;
use num_traits::ToPrimitive;

use support::{
    array, bool_value, build_request, get, int, mock_log, object, parse, reset_mock, run_oracle,
    serde_to_canonical, start_mock_provider, text, text_field,
};

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn recorded_at() -> GatewayTimestamp {
    GatewayTimestamp::Aware(
        KernelTimestamp::from_civil(2026, 1, 2, 3, 4, 5, 0, 0).expect("a valid instant"),
    )
}

fn message(role: &str, content: &str) -> CanonicalValue {
    object(vec![("role", text(role)), ("content", text(content))])
}

fn artifact(
    artifact_id: &str,
    classification: &str,
    preview_text: Option<&str>,
    metadata: Option<CanonicalValue>,
) -> CanonicalValue {
    let mut entries = vec![
        ("artifact_id", text(artifact_id)),
        ("digest", text(DIGEST)),
        ("classification", text(classification)),
        ("media_type", text("application/json")),
        ("size_bytes", int(4096)),
    ];
    if let Some(preview_text) = preview_text {
        entries.push(("preview_text", text(preview_text)));
    }
    if let Some(metadata) = metadata {
        entries.push(("metadata", metadata));
    }
    object(entries)
}

fn schema_object() -> CanonicalValue {
    object(vec![
        ("type", text("object")),
        ("additionalProperties", bool_value(false)),
        (
            "properties",
            object(vec![("ok", object(vec![("type", text("boolean"))]))]),
        ),
        ("required", array(vec![text("ok")])),
    ])
}

/// The scenario that both implementations run.
fn scenario(port: u16) -> CanonicalValue {
    let endpoint = |name: &str| format!("http://127.0.0.1:{port}/case/{name}");
    let cases = vec![
        object(vec![
            ("name", text("local-deterministic")),
            ("kind", text("local")),
            (
                "request",
                object(vec![
                    ("request_id", text("local-1")),
                    (
                        "messages",
                        array(vec![message("user", "summarize the run")]),
                    ),
                ]),
            ),
            (
                "ledger",
                object(vec![
                    ("max_model_calls", int(4)),
                    ("max_total_tokens", int(4000)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("local-schema")),
            ("kind", text("local")),
            (
                "request",
                object(vec![
                    ("request_id", text("local-2")),
                    ("messages", array(vec![message("user", "answer with json")])),
                    ("capabilities", array(vec![text("structured_output")])),
                    ("response_schema", schema_object()),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("local-seed")),
            ("kind", text("local")),
            (
                "request",
                object(vec![
                    ("request_id", text("local-3")),
                    ("messages", array(vec![message("system", "be terse")])),
                    ("capabilities", array(vec![text("seed")])),
                    ("seed", int(7)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("local-secret")),
            ("kind", text("local")),
            (
                "request",
                object(vec![
                    ("request_id", text("local-4")),
                    (
                        "messages",
                        array(vec![message(
                            "user",
                            "use token sk-abcdef0123456789abcdef and AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI",
                        )]),
                    ),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("local-artifacts")),
            ("kind", text("local")),
            (
                "request",
                object(vec![
                    ("request_id", text("local-5")),
                    ("messages", array(vec![message("user", "summarize")])),
                    (
                        "artifacts",
                        array(vec![
                            artifact(
                                "artifact-binary",
                                "design_binary",
                                Some("SECRET-GEOMETRY"),
                                None,
                            ),
                            artifact(
                                "artifact-text",
                                "text",
                                Some("preview with AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI"),
                                Some(object(vec![
                                    ("source", text("inbox")),
                                    ("nested", object(vec![("api_key", text("value"))])),
                                    (
                                        "tags",
                                        array(vec![text("safe"), text("Bearer abcdefghijklmnop")]),
                                    ),
                                    ("size", int(12)),
                                ])),
                            ),
                        ]),
                    ),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("local-unsupported-capability")),
            ("kind", text("local")),
            (
                "request",
                object(vec![
                    ("request_id", text("local-6")),
                    ("messages", array(vec![message("user", "call a tool")])),
                    ("capabilities", array(vec![text("tool_calling")])),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("local-budget-exhausted")),
            ("kind", text("local")),
            (
                "request",
                object(vec![
                    ("request_id", text("local-7")),
                    ("messages", array(vec![message("user", "summarize")])),
                ]),
            ),
            (
                "ledger",
                object(vec![
                    ("max_model_calls", int(4)),
                    ("max_total_tokens", int(1)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("local-invalid-request")),
            ("kind", text("local")),
            (
                "request",
                object(vec![
                    ("request_id", text("local-8")),
                    ("messages", array(vec![])),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("remote-success")),
            ("kind", text("remote")),
            ("endpoint", text(&endpoint("success"))),
            ("consent", bool_value(true)),
            ("api_key", text("sk-provider-secret-value-123456")),
            ("model_id", text("local-model")),
            ("input_cost", text("3")),
            ("output_cost", text("15")),
            (
                "request",
                object(vec![
                    ("request_id", text("remote-1")),
                    (
                        "messages",
                        array(vec![message("user", "summarize the run")]),
                    ),
                    ("response_schema", schema_object()),
                    ("capabilities", array(vec![text("structured_output")])),
                ]),
            ),
            (
                "policy",
                object(vec![
                    ("grants", array(vec![text("model.egress")])),
                    ("approve", bool_value(true)),
                ]),
            ),
            (
                "ledger",
                object(vec![
                    ("max_model_calls", int(4)),
                    ("max_total_tokens", int(4000)),
                    ("max_cost", text("10")),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("remote-malformed")),
            ("kind", text("remote")),
            ("endpoint", text(&endpoint("malformed"))),
            ("consent", bool_value(true)),
            (
                "request",
                object(vec![
                    ("request_id", text("remote-2")),
                    ("messages", array(vec![message("user", "summarize")])),
                ]),
            ),
            (
                "policy",
                object(vec![
                    ("grants", array(vec![text("model.egress")])),
                    ("approve", bool_value(true)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("remote-oversize")),
            ("kind", text("remote")),
            ("endpoint", text(&endpoint("oversize"))),
            ("consent", bool_value(true)),
            ("max_response_bytes", int(256)),
            (
                "request",
                object(vec![
                    ("request_id", text("remote-3")),
                    ("messages", array(vec![message("user", "summarize")])),
                ]),
            ),
            (
                "policy",
                object(vec![
                    ("grants", array(vec![text("model.egress")])),
                    ("approve", bool_value(true)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("remote-slow")),
            ("kind", text("remote")),
            ("endpoint", text(&endpoint("slow"))),
            ("consent", bool_value(true)),
            ("client_timeout_seconds", int(1)),
            (
                "request",
                object(vec![
                    ("request_id", text("remote-4")),
                    ("messages", array(vec![message("user", "summarize")])),
                    ("timeout_seconds", int(1)),
                ]),
            ),
            (
                "policy",
                object(vec![
                    ("grants", array(vec![text("model.egress")])),
                    ("approve", bool_value(true)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("remote-status-503")),
            ("kind", text("remote")),
            ("endpoint", text(&endpoint("status503"))),
            ("consent", bool_value(true)),
            (
                "request",
                object(vec![
                    ("request_id", text("remote-5")),
                    ("messages", array(vec![message("user", "summarize")])),
                ]),
            ),
            (
                "policy",
                object(vec![
                    ("grants", array(vec![text("model.egress")])),
                    ("approve", bool_value(true)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("remote-redirect")),
            ("kind", text("remote")),
            ("endpoint", text(&endpoint("redirect"))),
            ("consent", bool_value(true)),
            (
                "request",
                object(vec![
                    ("request_id", text("remote-6")),
                    ("messages", array(vec![message("user", "summarize")])),
                ]),
            ),
            (
                "policy",
                object(vec![
                    ("grants", array(vec![text("model.egress")])),
                    ("approve", bool_value(true)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("remote-no-choices")),
            ("kind", text("remote")),
            ("endpoint", text(&endpoint("nochoices"))),
            ("consent", bool_value(true)),
            (
                "request",
                object(vec![
                    ("request_id", text("remote-7")),
                    ("messages", array(vec![message("user", "summarize")])),
                ]),
            ),
            (
                "policy",
                object(vec![
                    ("grants", array(vec![text("model.egress")])),
                    ("approve", bool_value(true)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("remote-empty-content")),
            ("kind", text("remote")),
            ("endpoint", text(&endpoint("empty"))),
            ("consent", bool_value(true)),
            (
                "request",
                object(vec![
                    ("request_id", text("remote-8")),
                    ("messages", array(vec![message("user", "summarize")])),
                ]),
            ),
            (
                "policy",
                object(vec![
                    ("grants", array(vec![text("model.egress")])),
                    ("approve", bool_value(true)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("remote-artifacts-refused")),
            ("kind", text("remote")),
            ("endpoint", text(&endpoint("success"))),
            ("consent", bool_value(true)),
            (
                "request",
                object(vec![
                    ("request_id", text("remote-9")),
                    ("messages", array(vec![message("user", "summarize")])),
                    (
                        "artifacts",
                        array(vec![artifact("artifact-text", "text", None, None)]),
                    ),
                ]),
            ),
            (
                "policy",
                object(vec![
                    ("grants", array(vec![text("model.egress")])),
                    ("approve", bool_value(true)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("remote-no-consent")),
            ("kind", text("remote")),
            ("endpoint", text(&endpoint("success"))),
            ("consent", bool_value(false)),
            (
                "request",
                object(vec![
                    ("request_id", text("remote-10")),
                    ("messages", array(vec![message("user", "summarize")])),
                ]),
            ),
            (
                "policy",
                object(vec![
                    ("grants", array(vec![text("model.egress")])),
                    ("approve", bool_value(true)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("remote-policy-denied")),
            ("kind", text("remote")),
            ("endpoint", text(&endpoint("success"))),
            ("consent", bool_value(true)),
            (
                "request",
                object(vec![
                    ("request_id", text("remote-11")),
                    ("messages", array(vec![message("user", "summarize")])),
                ]),
            ),
            (
                "policy",
                object(vec![
                    ("grants", array(vec![])),
                    ("approve", bool_value(false)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("local-multiple-messages")),
            ("kind", text("local")),
            (
                "request",
                object(vec![
                    ("request_id", text("local-9")),
                    (
                        "messages",
                        array(vec![
                            message("system", "you are terse"),
                            message("user", "summarize"),
                            message("assistant", "sure"),
                        ]),
                    ),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("local-invalid-schema")),
            ("kind", text("local")),
            (
                "request",
                object(vec![
                    ("request_id", text("local-10")),
                    ("messages", array(vec![message("user", "summarize")])),
                    ("response_schema", object(vec![("type", text("string"))])),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("local-duplicate-artifacts")),
            ("kind", text("local")),
            (
                "request",
                object(vec![
                    ("request_id", text("local-11")),
                    ("messages", array(vec![message("user", "summarize")])),
                    (
                        "artifacts",
                        array(vec![
                            artifact("artifact-text", "text", None, None),
                            artifact("artifact-text", "text", None, None),
                        ]),
                    ),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("local-oversized-message")),
            ("kind", text("local")),
            (
                "request",
                object(vec![
                    ("request_id", text("local-12")),
                    (
                        "messages",
                        array(vec![message("user", &"x".repeat(40_000))]),
                    ),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("remote-schema-violation")),
            ("kind", text("remote")),
            ("endpoint", text(&endpoint("violation"))),
            ("consent", bool_value(true)),
            (
                "request",
                object(vec![
                    ("request_id", text("remote-12")),
                    ("messages", array(vec![message("user", "summarize")])),
                    ("response_schema", schema_object()),
                    ("capabilities", array(vec![text("structured_output")])),
                ]),
            ),
            (
                "policy",
                object(vec![
                    ("grants", array(vec![text("model.egress")])),
                    ("approve", bool_value(true)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("remote-seed")),
            ("kind", text("remote")),
            ("endpoint", text(&endpoint("success"))),
            ("consent", bool_value(true)),
            (
                "request",
                object(vec![
                    ("request_id", text("remote-13")),
                    ("messages", array(vec![message("user", "summarize")])),
                    ("capabilities", array(vec![text("seed")])),
                    ("seed", int(7)),
                ]),
            ),
            (
                "policy",
                object(vec![
                    ("grants", array(vec![text("model.egress")])),
                    ("approve", bool_value(true)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("remote-oversized-string")),
            ("kind", text("remote")),
            ("endpoint", text(&endpoint("large"))),
            ("consent", bool_value(true)),
            (
                "request",
                object(vec![
                    ("request_id", text("remote-14")),
                    ("messages", array(vec![message("user", "summarize")])),
                ]),
            ),
            (
                "policy",
                object(vec![
                    ("grants", array(vec![text("model.egress")])),
                    ("approve", bool_value(true)),
                ]),
            ),
        ]),
        object(vec![
            ("name", text("local-large-request")),
            ("kind", text("local")),
            (
                "request",
                object(vec![
                    ("request_id", text("local-13")),
                    (
                        "messages",
                        array(vec![
                            message("user", &"a".repeat(30_000)),
                            message("user", &"b".repeat(30_000)),
                            message("assistant", &"c".repeat(30_000)),
                        ]),
                    ),
                ]),
            ),
        ]),
    ];
    object(vec![("cases", array(cases))])
}

fn build_client(case: &CanonicalValue) -> Result<Arc<dyn LlmClient>, ModelError> {
    let kind = text_field(case, "kind");
    if kind == "local" {
        let mut capabilities = Vec::new();
        if let Some(CanonicalValue::Array(items)) = get(case, "capabilities") {
            for item in items {
                let name = item
                    .as_text()
                    .map(|text| text.to_string_lossy())
                    .unwrap_or_default();
                let capability = ModelCapability::parse(&name)
                    .ok_or_else(|| ModelError::capability("unknown model capability"))?;
                capabilities.push(capability);
            }
        }
        let model_id = match get(case, "model_id") {
            Some(_) => text_field(case, "model_id"),
            None => "local-deterministic".to_owned(),
        };
        let provider_id = match get(case, "provider_id") {
            Some(_) => text_field(case, "provider_id"),
            None => "local".to_owned(),
        };
        let client = LocalModelClient::new(&model_id, &provider_id, capabilities)?;
        return Ok(Arc::new(client));
    }
    let endpoint = text_field(case, "endpoint");
    let timeout_seconds = get(case, "client_timeout_seconds")
        .and_then(|value| value.as_integer())
        .and_then(|value| value.to_string().parse::<f64>().ok())
        .unwrap_or(30.0);
    let max_response_bytes = get(case, "max_response_bytes")
        .and_then(|value| value.as_integer())
        .and_then(|value| value.to_string().parse::<usize>().ok())
        .unwrap_or(1_048_576);
    let consent = get(case, "consent")
        .and_then(|value| match value {
            CanonicalValue::Bool(value) => Some(*value),
            _ => None,
        })
        .unwrap_or(false);
    let api_key = get(case, "api_key")
        .and_then(|value| value.as_text())
        .map(|value| value.to_string_lossy());
    let model_id = match get(case, "model_id") {
        Some(_) => text_field(case, "model_id"),
        None => "local-model".to_owned(),
    };
    let client = OpenAICompatibleClient::new(
        &endpoint,
        &model_id,
        vec![endpoint.clone()],
        api_key,
        consent,
        "openai-compatible",
        timeout_seconds,
        max_response_bytes,
        Arc::new(ReqwestTransport::new().expect("the provider transport")),
        kala_budget::Decimal::parse(&text_field(case, "input_cost")).unwrap_or_default(),
        kala_budget::Decimal::parse(&text_field(case, "output_cost")).unwrap_or_default(),
    )?;
    Ok(Arc::new(client))
}

fn error_value(class: &str, message: &str) -> CanonicalValue {
    object(vec![("class", text(class)), ("message", text(message))])
}

fn response_value(response: &ModelResponse) -> CanonicalValue {
    object(vec![
        ("content", text(response.content())),
        ("cost", text(&response.cost().to_python_string())),
        ("finish_reason", text(response.finish_reason().as_str())),
        ("input_tokens", int(response.input_tokens() as i64)),
        ("model_id", text(response.model_id())),
        ("output_tokens", int(response.output_tokens() as i64)),
        ("provider_id", text(response.provider_id())),
        (
            "provider_request_id",
            match response.provider_request_id() {
                Some(value) => text(value),
                None => CanonicalValue::Null,
            },
        ),
        ("request_id", text(response.request_id())),
        (
            "structured",
            match response.structured() {
                Some(value) => value.clone(),
                None => CanonicalValue::Null,
            },
        ),
        ("total_tokens", int(response.total_tokens() as i64)),
    ])
}

async fn run_native(scenario: &CanonicalValue) -> CanonicalValue {
    let Some(CanonicalValue::Array(cases)) = get(scenario, "cases") else {
        return object(vec![("results", array(Vec::new()))]);
    };
    let mut results = Vec::new();
    for case in cases {
        let mut entries: Vec<(&str, CanonicalValue)> = vec![(
            "name",
            get(case, "name").cloned().unwrap_or(CanonicalValue::Null),
        )];
        let client = build_client(case);
        let request = match get(case, "request") {
            Some(value) => build_request(value),
            None => Err(kala_runtime::gateway::ModelError::request(
                "a model request requires at least one message",
            )),
        };
        let ledger_state = get(case, "ledger").cloned();
        let ledger = ledger_state.as_ref().map(|value| {
            let max_model_calls = get(value, "max_model_calls")
                .and_then(|item| item.as_integer())
                .and_then(|item| item.to_string().parse::<u32>().ok())
                .unwrap_or(64);
            let max_total_tokens = get(value, "max_total_tokens")
                .and_then(|item| item.as_integer())
                .and_then(|item| item.to_string().parse::<u32>().ok())
                .unwrap_or(1_000_000);
            let max_cost = get(value, "max_cost")
                .and_then(|item| item.as_text())
                .map(|item| item.to_string_lossy())
                .unwrap_or_else(|| "0".to_owned());
            let budget = RunBudget::new()
                .with_max_model_calls(max_model_calls)
                .with_max_total_tokens(max_total_tokens)
                .try_with_max_cost(kala_budget::Decimal::parse(&max_cost).expect("a cost"))
                .expect("a cost budget");
            Arc::new(BudgetLedger::new(budget))
        });
        let policy = get(case, "policy").map(|value| {
            let grants = match get(value, "grants") {
                Some(CanonicalValue::Array(items)) => items
                    .iter()
                    .filter_map(|item| item.as_text())
                    .map(|item| item.to_string_lossy())
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            Arc::new(
                PolicyEngine::from_capabilities(
                    grants
                        .into_iter()
                        .map(|grant| kala_contracts::Text::new(&grant)),
                )
                .expect("the policy grants"),
            )
        });
        let approve = get(case, "policy")
            .and_then(|value| get(value, "approve"))
            .and_then(|value| match value {
                CanonicalValue::Bool(value) => Some(*value),
                _ => None,
            })
            .unwrap_or(false);
        let mut gateway = ModelGateway::new(match client {
            Ok(client) => client,
            Err(error) => {
                entries.push(("outcome", text("error")));
                entries.push((
                    "error",
                    error_value(error.python_class_name(), error.message()),
                ));
                entries.push(("transcripts", array(Vec::new())));
                entries.push(("manifests", array(Vec::new())));
                entries.push(("ledger", CanonicalValue::Null));
                entries.push((
                    "digests",
                    object(vec![
                        ("action", CanonicalValue::Null),
                        ("state", CanonicalValue::Null),
                    ]),
                ));
                results.push(object(entries));
                continue;
            }
        });
        if let Some(ledger) = ledger.clone() {
            gateway = gateway.with_ledger(ledger);
        }
        if let Some(policy) = policy {
            gateway = gateway.with_policy(policy);
        }
        gateway = gateway.with_clock(Arc::new(recorded_at));
        gateway = gateway.with_transcript(Arc::new(|_transcript| Ok(())));
        gateway = gateway.with_egress_manifest(Arc::new(|_manifest| Ok(())));
        let request = match request {
            Ok(request) => request,
            Err(error) => {
                entries.push(("outcome", text("error")));
                entries.push((
                    "error",
                    error_value(error.python_class_name(), error.message()),
                ));
                entries.push(("transcripts", array(Vec::new())));
                entries.push(("manifests", array(Vec::new())));
                entries.push(("ledger", CanonicalValue::Null));
                entries.push((
                    "digests",
                    object(vec![
                        ("action", CanonicalValue::Null),
                        ("state", CanonicalValue::Null),
                    ]),
                ));
                results.push(object(entries));
                continue;
            }
        };
        if approve {
            if let Some(policy) = gateway.policy() {
                let policy_request = gateway.egress_policy_request(&request);
                if let Ok(policy_request) = policy_request {
                    let _ = policy.approve(
                        &policy_request,
                        &kala_kernel::policy::DecisionContext::new(),
                    );
                }
            }
        }
        match gateway.complete(&request).await {
            Ok(response) => {
                entries.push(("outcome", text("ok")));
                entries.push(("response", response_value(&response)));
            }
            Err(error) => {
                entries.push(("outcome", text("error")));
                entries.push((
                    "error",
                    error_value(error.python_class_name(), &error.message()),
                ));
            }
        }
        entries.push((
            "transcripts",
            array(
                gateway
                    .transcripts()
                    .iter()
                    .map(|transcript| transcript.to_payload())
                    .collect(),
            ),
        ));
        entries.push((
            "manifests",
            array(
                gateway
                    .manifests()
                    .iter()
                    .map(|manifest| manifest.to_payload())
                    .collect(),
            ),
        ));
        entries.push((
            "ledger",
            match &ledger {
                Some(ledger) => serde_to_canonical(&ledger.export_state()),
                None => CanonicalValue::Null,
            },
        ));
        entries.push((
            "digests",
            object(vec![
                (
                    "action",
                    match gateway.egress_action_digest(&request) {
                        Ok(digest) => text(&digest),
                        Err(_) => CanonicalValue::Null,
                    },
                ),
                (
                    "state",
                    match gateway.egress_state_digest(&request) {
                        Ok(digest) => text(&digest),
                        Err(_) => CanonicalValue::Null,
                    },
                ),
            ]),
        ));
        results.push(object(entries));
    }
    object(vec![("results", array(results))])
}

/// The canonical encoding of a result document.
///
/// The transcript bodies are bounded at 64 KiB, which is above the protocol's
/// 32 KiB string limit, so the comparison uses relaxed limits.
fn canonical(value: &CanonicalValue) -> String {
    let limits = kala_json::Limits {
        max_string_length: 1_048_576,
        max_json_bytes: 16_777_216,
        ..kala_json::Limits::protocol()
    };
    kala_json::canonical_dumps(value, &limits).expect("canonical value")
}

#[tokio::test(flavor = "multi_thread")]
async fn python_differential_matches_the_native_gateway() {
    let (mut mock, port) = start_mock_provider();
    let scenario = scenario(port);

    let oracle = run_oracle(&scenario);
    let python_requests = mock_log(port);
    reset_mock(port);

    let native = run_native(&scenario).await;
    let native_requests = mock_log(port);

    let oracle_bytes = canonical(&oracle);
    let native_bytes = canonical(&native);
    if std::env::var("KALA_RUNTIME_DIFF_DUMP").is_ok() {
        std::fs::write("/tmp/kala-runtime-native.json", &native_bytes).expect("dump");
        std::fs::write("/tmp/kala-runtime-oracle.json", &oracle_bytes).expect("dump");
    }
    if oracle_bytes != native_bytes {
        let parsed = parse(&oracle_bytes);
        let _ = parsed;
        panic!(
            "the native gateway and the Python gateway differ\nnative:\n{native_bytes}\noracle:\n{oracle_bytes}"
        );
    }

    let python_requests_bytes = canonical(&python_requests);
    let native_requests_bytes = canonical(&native_requests);
    // The wire comparison is only meaningful when the provider was actually
    // contacted.
    let CanonicalValue::Array(recorded) = &python_requests else {
        panic!("the recorded requests are an array");
    };
    assert!(
        recorded.len() >= 5,
        "the scenario must send requests to the mock provider"
    );
    if std::env::var("KALA_RUNTIME_DIFF_DUMP").is_ok() {
        eprintln!("recorded requests: {}", recorded.len());
    }
    if python_requests_bytes != native_requests_bytes {
        panic!(
            "the requests on the wire differ\nnative:\n{native_requests_bytes}\noracle:\n{python_requests_bytes}"
        );
    }

    let _ = mock.kill();
    let _ = mock.wait();
}

/// The oracle must exercise the redaction, the budget and the egress paths.
#[test]
fn the_scenario_covers_every_gateway_path() {
    let (mut mock, port) = start_mock_provider();
    let scenario = scenario(port);
    let oracle = run_oracle(&scenario);
    let Some(CanonicalValue::Array(results)) = get(&oracle, "results") else {
        panic!("the oracle returned no results");
    };
    assert!(results.len() >= 20, "the scenario must be broad");
    let mut classes: Vec<String> = Vec::new();
    for result in results {
        if let Some(error) = get(result, "error") {
            classes.push(text_field(error, "class"));
        }
    }
    for expected in [
        "ModelRequestError",
        "ModelCapabilityError",
        "ModelProviderError",
        "ProviderDegradedError",
        "EgressDeniedError",
        "StructuredOutputError",
        "TokenBudgetExhaustedError",
    ] {
        assert!(
            classes.iter().any(|class| class == expected),
            "the scenario must produce {expected}, got {classes:?}"
        );
    }
    let _ = mock.kill();
    let _ = mock.wait();
}

/// The unused imports are part of the public surface the oracle compares.
#[allow(dead_code)]
fn _unused(
    _: ClientError,
    _: GatewayError,
    _: DecimalArithmeticError,
    _: ModelRequest,
    _: JsonText,
    _: BigUint,
    _: &dyn ProviderTransport,
) {
    let _ = ToPrimitive::to_i128(&BigUint::from(0u32));
}
