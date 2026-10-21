//! `GET /api/studio/providers/{id}/models`: auth gates, consent, mapping,
//! caching, error envelopes and the OpenAI-compatible wire path.
//!
//! The remote calls run through the transport seam with mock responses; one
//! test uses a real loopback HTTP server so the production `ReqwestTransport`
//! GET path is exercised as well. No live credential or provider is used.

mod common;

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kala_studio::providers::remote::{
    GetRequest, HttpTransport, RemoteRequest, RemoteResponse, TransportError,
};

use common::{RequestSpec, TOKEN, TestResponse, TestServer};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Default)]
struct MockCatalogTransport {
    requests: Mutex<Vec<GetRequest>>,
    responses: Mutex<VecDeque<Result<RemoteResponse, TransportError>>>,
}

impl MockCatalogTransport {
    fn push(&self, response: Result<RemoteResponse, TransportError>) {
        self.responses.lock().unwrap().push_back(response);
    }

    fn push_json(&self, body: &[u8]) {
        self.push(Ok(RemoteResponse {
            status: 200,
            reason: Some("OK".to_owned()),
            content_length: Some(body.len() as u64),
            body: body.to_vec(),
        }));
    }

    fn requests(&self) -> Vec<GetRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl HttpTransport for MockCatalogTransport {
    fn post<'a>(
        &'a self,
        _request: RemoteRequest,
    ) -> BoxFuture<'a, Result<RemoteResponse, TransportError>> {
        Box::pin(async {
            Err(TransportError::Network(
                "the catalog route must not POST".to_owned(),
            ))
        })
    }

    fn get<'a>(
        &'a self,
        request: GetRequest,
    ) -> BoxFuture<'a, Result<RemoteResponse, TransportError>> {
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

fn catalog_environment() -> HashMap<String, String> {
    let mut environ = HashMap::new();
    environ.insert(
        "OPENROUTER_API_KEY".to_owned(),
        "sk-or-synthetic-not-a-live-credential-0001".to_owned(),
    );
    environ.insert(
        "OPENAI_API_KEY".to_owned(),
        "sk-synthetic-not-a-live-credential-0002".to_owned(),
    );
    environ
}

async fn catalog_server(
    transport: Arc<dyn HttpTransport>,
    environ: HashMap<String, String>,
) -> (TestServer, tempfile::TempDir) {
    let work = tempfile::tempdir().unwrap();
    let server = TestServer::start_with(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        environ,
        Arc::new(kala_studio::geometry::UnavailableStepExporter),
        Some(transport),
    )
    .await;
    (server, work)
}

async fn get_models_with(
    addr: std::net::SocketAddr,
    provider: &str,
    query: &str,
    bearer: Option<&str>,
    origin: Option<&str>,
) -> TestResponse {
    let mut spec = RequestSpec::default();
    if let Some(token) = bearer {
        spec.headers
            .push(("Authorization".to_owned(), format!("Bearer {token}")));
    }
    if let Some(origin) = origin {
        spec.headers.push(("Origin".to_owned(), origin.to_owned()));
    }
    let path = format!("/api/studio/providers/{provider}/models{query}");
    common::request(addr, "GET", &path, spec).await
}

async fn get_models(addr: std::net::SocketAddr, provider: &str, query: &str) -> TestResponse {
    get_models_with(addr, provider, query, Some(TOKEN), None).await
}

const OPENROUTER_BODY: &[u8] = br#"{
    "data": [
        {"id":"minimax/minimax-m3.1-flash:free","name":"MiniMax M3.1 Flash (free)",
         "context_length":262144,"pricing":{"prompt":"0","completion":"0"},
         "supported_parameters":["tools"]},
        {"id":"anthropic/claude-sonnet-4.5","name":"Claude Sonnet 4.5",
         "context_length":200000,"pricing":{"prompt":"0.000003","completion":"0.000015"},
         "supported_parameters":["max_tokens"]}
    ]
}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn openrouter_route_maps_the_catalog_and_caches_it() {
    let mock = Arc::new(MockCatalogTransport::default());
    mock.push_json(OPENROUTER_BODY);
    mock.push_json(OPENROUTER_BODY);
    let (server, _work) = catalog_server(mock.clone(), catalog_environment()).await;

    let response = get_models(server.addr, "openrouter", "?allow_remote=true").await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let body = response.json();
    assert_eq!(body["provider"], "openrouter");
    assert_eq!(body["source"], "openrouter");
    assert!(body["fetched_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(body["models"].as_array().unwrap().len(), 2);
    assert_eq!(body["models"][0]["id"], "minimax/minimax-m3.1-flash:free");
    assert_eq!(body["models"][0]["free"], true);
    assert_eq!(body["models"][0]["context_length"], 262_144);
    assert_eq!(body["models"][0]["pricing"]["prompt"], 0.0);
    assert_eq!(body["models"][0]["supports_tools"], true);
    assert_eq!(body["models"][1]["free"], false);
    assert!(body.get("error").is_none());
    assert!(
        !response
            .body_text()
            .contains("sk-or-synthetic-not-a-live-credential-0001")
    );

    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "https://openrouter.ai/api/v1/models");
    assert_eq!(requests[0].timeout, Duration::from_secs(15));
    assert!(
        requests[0]
            .headers
            .iter()
            .any(|(name, value)| name == "Authorization"
                && value == "Bearer sk-or-synthetic-not-a-live-credential-0001")
    );

    // The second request is served from the ten-minute cache: no new call,
    // and the original fetch timestamp is kept.
    let first_fetched = body["fetched_at"].clone();
    let response = get_models(server.addr, "openrouter", "?allow_remote=true").await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["fetched_at"], first_fetched);
    assert_eq!(mock.requests().len(), 1);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stored_key_is_used_and_never_returned() {
    let work = tempfile::tempdir().unwrap();
    let data_dir = work.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let stored_key = "sk-or-stored-synthetic-not-a-live-credential-0003";
    kala_studio::settings::write_stored_key(&data_dir, "openrouter", Some(stored_key)).unwrap();
    let mock = Arc::new(MockCatalogTransport::default());
    mock.push_json(OPENROUTER_BODY);
    let server = TestServer::start_with(
        data_dir,
        TOKEN,
        64 * 1024,
        None,
        HashMap::new(),
        Arc::new(kala_studio::geometry::UnavailableStepExporter),
        Some(mock.clone()),
    )
    .await;

    let response = get_models(server.addr, "openrouter", "?allow_remote=true").await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    assert_eq!(response.json()["models"].as_array().unwrap().len(), 2);
    assert!(!response.body_text().contains(stored_key));
    assert!(
        mock.requests()[0].headers.iter().any(
            |(name, value)| name == "Authorization" && value == &format!("Bearer {stored_key}")
        )
    );
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_host_origin_method_and_consent_gates() {
    let mock = Arc::new(MockCatalogTransport::default());
    let (server, _work) = catalog_server(mock.clone(), catalog_environment()).await;

    // No bearer: the route-level gate answers 401 with the plain challenge.
    let response =
        get_models_with(server.addr, "openrouter", "?allow_remote=true", None, None).await;
    assert_eq!(response.status, 401);
    assert_eq!(response.header("www-authenticate"), Some("Bearer"));
    assert_eq!(response.json()["code"], "unauthorized");

    // A foreign origin is refused before the router.
    let response = get_models_with(
        server.addr,
        "openrouter",
        "?allow_remote=true",
        Some(TOKEN),
        Some("http://evil.example"),
    )
    .await;
    assert_eq!(response.status, 403);
    assert_eq!(response.json()["code"], "cross_origin_denied");

    // The consent flag is required for a remote listing.
    let response = get_models(server.addr, "openrouter", "").await;
    assert_eq!(response.status, 403);
    assert_eq!(response.json()["code"], "consent_required");
    assert_eq!(
        response.json()["message"],
        "the remote model listing requires explicit consent"
    );
    let response = get_models(server.addr, "openrouter", "?allow_remote=false").await;
    assert_eq!(response.status, 403);

    // An unknown catalog provider is a 404, never an empty 200.
    let response = get_models(server.addr, "freecad", "?allow_remote=true").await;
    assert_eq!(response.status, 404);
    assert_eq!(response.json()["code"], "not_found");

    // Only GET is served.
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    let response = common::request(
        server.addr,
        "POST",
        "/api/studio/providers/openrouter/models?allow_remote=true",
        spec,
    )
    .await;
    assert_eq!(response.status, 405);
    assert_eq!(response.json()["code"], "method_not_allowed");

    // The Host gate still wraps the new route.
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Host".to_owned(), "evil.example".to_owned()));
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    let response = common::request(
        server.addr,
        "GET",
        "/api/studio/providers/openrouter/models?allow_remote=true",
        spec,
    )
    .await;
    assert_eq!(response.status, 421);
    assert_eq!(response.json()["code"], "invalid_host");

    assert!(mock.requests().is_empty());
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_failures_stay_inside_the_envelope() {
    let mock = Arc::new(MockCatalogTransport::default());
    mock.push(Ok(RemoteResponse {
        status: 401,
        reason: Some("Unauthorized".to_owned()),
        content_length: Some(2),
        body: b"{}".to_vec(),
    }));
    let (server, _work) = catalog_server(mock.clone(), catalog_environment()).await;

    let response = get_models(server.addr, "openrouter", "?allow_remote=true").await;
    assert_eq!(response.status, 200);
    let body = response.json();
    assert_eq!(body["source"], "none");
    assert_eq!(body["models"].as_array().unwrap().len(), 0);
    let error = body["error"].as_str().unwrap();
    assert!(error.contains("rejected the stored credential"), "{error}");
    assert!(
        !response
            .body_text()
            .contains("sk-or-synthetic-not-a-live-credential-0001")
    );

    mock.push(Err(TransportError::Timeout));
    let response = get_models(server.addr, "openrouter", "?allow_remote=true").await;
    assert_eq!(response.status, 200);
    assert!(
        response.json()["error"]
            .as_str()
            .unwrap()
            .contains("bounded timeout")
    );
    server.stop().await;

    // A provider without a stored key reports the configuration failure in
    // the same envelope and opens no connection.
    let mock = Arc::new(MockCatalogTransport::default());
    let (server, _work) = catalog_server(mock.clone(), HashMap::new()).await;
    let response = get_models(server.addr, "openai", "?allow_remote=true").await;
    assert_eq!(response.status, 200);
    assert!(
        response.json()["error"]
            .as_str()
            .unwrap()
            .contains("not configured")
    );
    assert!(mock.requests().is_empty());
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_and_static_cli_lists_need_no_consent() {
    let mock = Arc::new(MockCatalogTransport::default());
    let (server, _work) = catalog_server(mock.clone(), HashMap::new()).await;

    let response = get_models(server.addr, "local", "").await;
    assert_eq!(response.status, 200);
    let body = response.json();
    assert_eq!(body["provider"], "local");
    assert_eq!(body["source"], "static");
    assert_eq!(body["models"].as_array().unwrap().len(), 1);
    assert_eq!(body["models"][0]["id"], "deterministic-paraphrase-v1");
    assert_eq!(body["models"][0]["free"], true);

    let response = get_models(server.addr, "claude_code", "").await;
    assert_eq!(response.status, 200);
    let body = response.json();
    assert_eq!(body["source"], "static");
    assert!(
        body["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["id"] == "sonnet")
    );

    let response = get_models(server.addr, "codex_cli", "").await;
    assert_eq!(response.status, 200);
    assert!(
        response.json()["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["id"] == "gpt-5-codex")
    );

    // `codex` is the historical alias and answers under the canonical id.
    let response = get_models(server.addr, "codex", "").await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["provider"], "codex_cli");

    assert!(mock.requests().is_empty());
    server.stop().await;
}

async fn start_openai_mock() -> (std::net::SocketAddr, tokio::task::JoinHandle<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
        let text = String::from_utf8_lossy(&request).into_owned();
        let body = br#"{"object":"list","data":[{"id":"mock-model","object":"model","name":"Mock Model"}]}"#;
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();
        text
    });
    (addr, handle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn openai_compatible_endpoint_uses_the_production_get_path() {
    let (mock_addr, request) = start_openai_mock().await;
    let mut environ = HashMap::new();
    environ.insert(
        "OPENAI_API_KEY".to_owned(),
        "sk-synthetic-loopback-key".to_owned(),
    );
    environ.insert("OPENAI_BASE_URL".to_owned(), format!("http://{mock_addr}"));
    // No transport injection: the service builds the production reqwest
    // transport and this test proves its bounded GET path.
    let (server, _work) = catalog_server(
        Arc::new(kala_studio::providers::remote::ReqwestTransport::new().unwrap()),
        environ,
    )
    .await;

    let response = get_models(server.addr, "openai", "?allow_remote=true").await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let body = response.json();
    assert_eq!(body["source"], "openai");
    assert_eq!(body["models"][0]["id"], "mock-model");
    assert_eq!(body["models"][0]["name"], "Mock Model");

    let request = request.await.unwrap();
    let mut lines = request.lines();
    assert_eq!(lines.next(), Some("GET /v1/models HTTP/1.1"));
    assert!(
        request.lines().any(
            |line| line.eq_ignore_ascii_case("authorization: Bearer sk-synthetic-loopback-key")
        )
    );
    server.stop().await;
}
