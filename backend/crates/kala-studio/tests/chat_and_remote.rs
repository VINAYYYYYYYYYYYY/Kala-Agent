//! Chat routes: local determinism plus remote provider wire/error behaviour
//! through the isolated transport seam (no live credentials or model calls).

mod common;

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use kala_studio::providers::local::DEFAULT_LOCAL_MODEL;
use kala_studio::providers::remote::{
    HttpTransport, OPENAI_DEFAULT_ENDPOINT, OPENROUTER_DEFAULT_ENDPOINT, RemoteRequest,
    RemoteResponse, TransportError,
};

use common::{RequestSpec, TOKEN, TestServer};

#[derive(Default)]
struct MockTransport {
    requests: Mutex<Vec<RemoteRequest>>,
    responses: Mutex<VecDeque<Result<RemoteResponse, TransportError>>>,
}

impl MockTransport {
    fn push(&self, response: Result<RemoteResponse, TransportError>) {
        self.responses.lock().unwrap().push_back(response);
    }

    fn requests(&self) -> Vec<RemoteRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl HttpTransport for MockTransport {
    fn post<'a>(
        &'a self,
        request: RemoteRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteResponse, TransportError>> + Send + 'a>> {
        self.requests.lock().unwrap().push(request);
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(TransportError::Network("no mock response".to_owned())));
        Box::pin(async move { response })
    }
}

fn chat_spec(body: &serde_json::Value) -> RequestSpec {
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    spec.headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    spec.body = Some(serde_json::to_vec(body).unwrap());
    spec
}

async fn chat(addr: std::net::SocketAddr, body: serde_json::Value) -> common::TestResponse {
    common::request(addr, "POST", "/api/studio/chat", chat_spec(&body)).await
}

fn provider_environment() -> HashMap<String, String> {
    let mut environ = HashMap::new();
    environ.insert(
        "OPENAI_API_KEY".to_owned(),
        "sk-corpus-synthetic-not-a-live-credential-0001".to_owned(),
    );
    environ.insert(
        "OPENROUTER_API_KEY".to_owned(),
        "sk-or-corpus-synthetic-not-a-live-credential-0002".to_owned(),
    );
    environ
}

async fn remote_server() -> (TestServer, Arc<MockTransport>, tempfile::TempDir) {
    let work = tempfile::tempdir().unwrap();
    let transport = Arc::new(MockTransport::default());
    let server = TestServer::start_with(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        provider_environment(),
        Arc::new(kala_studio::geometry::UnavailableStepExporter),
        Some(transport.clone()),
    )
    .await;
    (server, transport, work)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_route_matches_the_oracle_text() {
    let work = tempfile::tempdir().unwrap();
    let server = TestServer::start(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        HashMap::new(),
    )
    .await;
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("data/local_chat_vectors.json")).unwrap();
    let mut checked = 0;
    for vector in vectors.as_array().unwrap().iter().take(20) {
        let prompt = vector["prompt"].as_str().unwrap();
        if prompt.trim_matches(char::is_whitespace) != prompt || prompt.is_empty() {
            continue;
        }
        let request_model = vector["request_model"].as_str();
        let mut body = serde_json::json!({"provider": "local", "prompt": prompt});
        if let Some(model) = request_model {
            body["model"] = serde_json::Value::String(model.to_owned());
        }
        let response = chat(server.addr, body).await;
        assert_eq!(response.status, 200, "prompt {prompt:?}");
        let parsed = response.json();
        assert_eq!(parsed["text"], vector["text"]);
        assert_eq!(
            parsed["model"],
            vector["model"].as_str().unwrap(),
            "prompt {prompt:?}"
        );
        assert_eq!(parsed["provider"], "local");
        assert_eq!(parsed["offline"], true);
        checked += 1;
    }
    assert!(checked >= 15);

    let response = chat(
        server.addr,
        serde_json::json!({"provider": "local", "prompt": " x "}),
    )
    .await;
    let expected = vectors
        .as_array()
        .unwrap()
        .iter()
        .find(|vector| vector["prompt"] == "x" && vector["request_model"].is_null())
        .unwrap()["text"]
        .as_str()
        .unwrap();
    assert_eq!(response.json()["text"], expected);
    let response = chat(
        server.addr,
        serde_json::json!({"provider": "local", "prompt": "x", "model": "custom-model-1"}),
    )
    .await;
    assert_eq!(response.json()["model"], "custom-model-1");

    let response = chat(
        server.addr,
        serde_json::json!({"provider": "local", "prompt": ""}),
    )
    .await;
    assert_eq!(response.status, 422);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_success_captures_the_exact_wire_request() {
    let (server, transport, _work) = remote_server().await;
    transport.push(Ok(RemoteResponse {
        status: 200,
        reason: Some("OK".to_owned()),
        content_length: Some(75),
        body: br#"{"model":"served-model","choices":[{"message":{"content":"remote reply"}}]}"#
            .to_vec(),
    }));
    let response = chat(
        server.addr,
        serde_json::json!({
            "provider": "openai",
            "model": "gpt-4o-mini",
            "prompt": "bøks",
            "allow_remote": true
        }),
    )
    .await;
    assert_eq!(response.status, 200);
    assert_eq!(
        response.body_text(),
        "{\"text\":\"remote reply\",\"provider\":\"openai\",\"model\":\"served-model\",\"offline\":false,\"design\":null,\"usage\":null}"
    );
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, OPENAI_DEFAULT_ENDPOINT);
    assert_eq!(
        String::from_utf8(requests[0].body.clone()).unwrap(),
        "{\"model\": \"gpt-4o-mini\", \"messages\": [{\"role\": \"user\", \"content\": \"b\\u00f8ks\"}], \"temperature\": 0.2, \"stream\": false}"
    );
    assert_eq!(
        requests[0].headers[0].1,
        "Bearer sk-corpus-synthetic-not-a-live-credential-0001"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_failures_map_to_sanitized_statuses() {
    let (server, transport, _work) = remote_server().await;

    // Model required before any transport call.
    let response = chat(
        server.addr,
        serde_json::json!({"provider":"openai","prompt":"hello","allow_remote":true}),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["code"], "provider_not_configured");
    assert!(transport.requests().is_empty());

    // Remote consent is required before credentials.
    let response = chat(
        server.addr,
        serde_json::json!({"provider":"openai","model":"m","prompt":"hello"}),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(
        response.json()["details"][0],
        "remote calls require explicit allow_remote=true"
    );

    // HTTP error -> 502 protocol.
    transport.push(Ok(RemoteResponse {
        status: 401,
        reason: Some("Unauthorized".to_owned()),
        content_length: None,
        body: b"{}".to_vec(),
    }));
    let response = chat(
        server.addr,
        serde_json::json!({"provider":"openai","model":"m","prompt":"hello","allow_remote":true}),
    )
    .await;
    assert_eq!(response.status, 502);
    assert_eq!(response.json()["code"], "provider_protocol_error");
    assert_eq!(
        response.json()["details"][0],
        "the provider returned HTTP 401: Unauthorized"
    );

    // Timeout -> 504.
    transport.push(Err(TransportError::Timeout));
    let response = chat(
        server.addr,
        serde_json::json!({"provider":"openai","model":"m","prompt":"hello","allow_remote":true}),
    )
    .await;
    assert_eq!(response.status, 504);
    assert_eq!(response.json()["code"], "provider_timeout");

    // Network -> 502.
    transport.push(Err(TransportError::Network("gaierror".to_owned())));
    let response = chat(
        server.addr,
        serde_json::json!({"provider":"openai","model":"m","prompt":"hello","allow_remote":true}),
    )
    .await;
    assert_eq!(response.status, 502);
    assert_eq!(response.json()["code"], "provider_network_error");
    assert_eq!(
        response.json()["details"][0],
        "the provider network call failed: gaierror"
    );

    // Redirect -> 502 with the Python redirect message.
    transport.push(Ok(RemoteResponse {
        status: 302,
        reason: Some("Found".to_owned()),
        content_length: None,
        body: Vec::new(),
    }));
    let response = chat(
        server.addr,
        serde_json::json!({"provider":"openai","model":"m","prompt":"hello","allow_remote":true}),
    )
    .await;
    assert_eq!(response.status, 502);
    assert_eq!(
        response.json()["details"][0],
        "redirects to a different origin are refused"
    );

    // Malformed provider JSON -> 502 protocol.
    transport.push(Ok(RemoteResponse {
        status: 200,
        reason: Some("OK".to_owned()),
        content_length: Some(2),
        body: b"[]".to_vec(),
    }));
    let response = chat(
        server.addr,
        serde_json::json!({"provider":"openai","model":"m","prompt":"hello","allow_remote":true}),
    )
    .await;
    assert_eq!(response.status, 502);
    assert_eq!(
        response.json()["details"][0],
        "the provider response was not a JSON object"
    );

    // Oversized declared body is an unmapped failure -> sanitized 500.
    transport.push(Ok(RemoteResponse {
        status: 200,
        reason: Some("OK".to_owned()),
        content_length: Some(2_000_000),
        body: Vec::new(),
    }));
    let response = chat(
        server.addr,
        serde_json::json!({"provider":"openai","model":"m","prompt":"hello","allow_remote":true}),
    )
    .await;
    assert_eq!(response.status, 500);
    assert_eq!(response.json()["code"], "internal_error");
    server.stop().await;
}

fn utf16_response(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

fn push_ok(transport: &MockTransport, body: Vec<u8>) {
    let length = body.len();
    transport.push(Ok(RemoteResponse {
        status: 200,
        reason: Some("OK".to_owned()),
        content_length: Some(length as u64),
        body,
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_response_dto_validation_matches_the_source() {
    let (server, transport, _work) = remote_server().await;
    let body = serde_json::json!({
        "provider": "openai",
        "model": "gpt-4o-mini",
        "prompt": "hello",
        "allow_remote": true
    });

    // `ChatResponse.model` is bounded to 256 characters: a provider that
    // names a longer model is a Pydantic ValidationError and the source
    // returns its sanitized 500 envelope, never a truncated model.
    let long = serde_json::json!({
        "model": "m".repeat(257),
        "choices": [{"message": {"content": "hi"}}]
    });
    push_ok(&transport, serde_json::to_vec(&long).unwrap());
    let response = chat(server.addr, body.clone()).await;
    assert_eq!(response.status, 500);
    assert_eq!(
        response.body_text(),
        "{\"code\":\"internal_error\",\"message\":\"the studio request could not be completed\",\"details\":[]}"
    );
    assert_eq!(response.header("content-type"), Some("application/json"));
    assert_eq!(response.header("cache-control"), Some("no-store"));

    // Exactly 256 characters is accepted.
    let accepted = serde_json::json!({
        "model": "m".repeat(256),
        "choices": [{"message": {"content": "hi"}}]
    });
    push_ok(&transport, serde_json::to_vec(&accepted).unwrap());
    let response = chat(server.addr, body.clone()).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["model"].as_str().unwrap().len(), 256);

    // `json.loads(bytes)` encoding detection applies to provider replies
    // too: UTF-16LE and a UTF-8 BOM are accepted.
    let payload = r#"{"model":"served","choices":[{"message":{"content":"bøks"}}]}"#;
    push_ok(&transport, utf16_response(payload));
    let response = chat(server.addr, body.clone()).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["text"], "bøks");

    push_ok(
        &transport,
        [b"\xef\xbb\xbf".as_slice(), payload.as_bytes()].concat(),
    );
    let response = chat(server.addr, body.clone()).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["model"], "served");

    // A non-finite constant in an unrelated field is accepted by CPython's
    // parser; NaN as the model falls back to the requested model.
    push_ok(
        &transport,
        br#"{"model":NaN,"choices":[{"message":{"content":"hi"}}],"usage":Infinity}"#.to_vec(),
    );
    let response = chat(server.addr, body.clone()).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["model"], "gpt-4o-mini");

    // A lone surrogate in an unused field is parsed by CPython and ignored.
    push_ok(
        &transport,
        br#"{"model":"served","choices":[{"message":{"content":"hi"}}],"extra":"\ud800"}"#.to_vec(),
    );
    let response = chat(server.addr, body.clone()).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["model"], "served");

    // A lone surrogate in the message or model reaches ChatResponse and is a
    // sanitized 500 in the source.
    push_ok(
        &transport,
        br#"{"model":"served","choices":[{"message":{"content":"\ud800"}}]}"#.to_vec(),
    );
    let response = chat(server.addr, body.clone()).await;
    assert_eq!(response.status, 500);
    assert_eq!(response.json()["code"], "internal_error");

    // CPython's integer digit limit raises a plain ValueError, which the
    // source route returns as a sanitized 500.
    let mut huge = br#"{"model":"served","choices":[{"message":{"content":"hi"}}],"n":"#.to_vec();
    huge.extend_from_slice("9".repeat(4_301).as_bytes());
    huge.push(b'}');
    push_ok(&transport, huge);
    let response = chat(server.addr, body.clone()).await;
    assert_eq!(response.status, 500);
    assert_eq!(response.json()["code"], "internal_error");

    // Undecodable bytes raise UnicodeDecodeError, which the source route
    // does not map: sanitized 500.
    push_ok(&transport, vec![0xFF, 0xFE, 0x00]);
    let response = chat(server.addr, body.clone()).await;
    assert_eq!(response.status, 500);
    assert_eq!(response.json()["code"], "internal_error");

    // Malformed JSON after a successful decode stays a protocol 502.
    push_ok(&transport, b"{nope".to_vec());
    let response = chat(server.addr, body.clone()).await;
    assert_eq!(response.status, 502);
    assert_eq!(response.json()["code"], "provider_protocol_error");
    assert_eq!(
        response.json()["details"][0],
        "the provider response was not valid JSON"
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_needs_consent_and_openrouter_uses_the_fixed_endpoint() {
    let (server, transport, _work) = remote_server().await;
    // The Codex CLI route is a subscription route now. The test environment
    // publishes no PATH, so no CLI is installed and the route reports the
    // provider as unavailable instead of sending anything.
    let response = chat(
        server.addr,
        serde_json::json!({"provider":"codex","prompt":"hello","allow_remote":true}),
    )
    .await;
    assert_eq!(response.status, 503);
    assert_eq!(response.json()["code"], "provider_unavailable");
    assert!(transport.requests().is_empty());

    let response = chat(
        server.addr,
        serde_json::json!({"provider":"codex","prompt":"hello"}),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(
        response.json()["details"][0],
        "provider=codex requires allow_remote=true at request time"
    );

    transport.push(Ok(RemoteResponse {
        status: 200,
        reason: Some("OK".to_owned()),
        content_length: Some(65),
        body: br#"{"choices":[{"message":{"content":"ok"}}],"model":"or-model"}"#.to_vec(),
    }));
    let response = chat(
        server.addr,
        serde_json::json!({"provider":"openrouter","model":"or/model","prompt":"hello","allow_remote":true}),
    )
    .await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["model"], "or-model");
    assert_eq!(transport.requests()[0].url, OPENROUTER_DEFAULT_ENDPOINT);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_and_overlong_credentials_are_configuration_errors() {
    let work = tempfile::tempdir().unwrap();
    let transport = Arc::new(MockTransport::default());
    let server = TestServer::start_with(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        HashMap::new(),
        Arc::new(kala_studio::geometry::UnavailableStepExporter),
        Some(transport.clone()),
    )
    .await;
    let response = chat(
        server.addr,
        serde_json::json!({"provider":"openai","model":"m","prompt":"hello","allow_remote":true}),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["code"], "missing_api_key");
    assert!(transport.requests().is_empty());
    server.stop().await;

    let work = tempfile::tempdir().unwrap();
    let transport = Arc::new(MockTransport::default());
    let mut environ = HashMap::new();
    environ.insert("OPENAI_API_KEY".to_owned(), "k".repeat(300));
    let server = TestServer::start_with(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        environ,
        Arc::new(kala_studio::geometry::UnavailableStepExporter),
        Some(transport.clone()),
    )
    .await;
    let response = chat(
        server.addr,
        serde_json::json!({"provider":"openai","model":"m","prompt":"hello","allow_remote":true}),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(
        response.json()["details"][0],
        "OPENAI_API_KEY is longer than the bounded length"
    );
    assert!(!response.body_text().contains(&"k".repeat(300)));
    assert!(transport.requests().is_empty());
    server.stop().await;
}

#[test]
fn default_model_constant_matches_source() {
    assert_eq!(DEFAULT_LOCAL_MODEL, "deterministic-paraphrase-v1");
}
