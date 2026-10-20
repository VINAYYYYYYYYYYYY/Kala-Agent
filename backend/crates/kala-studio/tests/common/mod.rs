//! Shared real-socket HTTP harness for the Studio integration tests.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use kala_studio::geometry::StepExporter;
use kala_studio::geometry_backend::{
    BackendFuture, BackendHealth, GeometryBackend, WorkerResponse,
};
use kala_studio::providers::remote::HttpTransport;
use kala_studio::server::{StudioConfig, StudioService};

/// A deterministic stand-in for the geometry worker.
///
/// The front door proxies the geometry routes; the offline tests need an
/// answer that does not depend on a Python process. The stub answers the
/// primitive preview with a fixed document and refuses a body that is not a
/// JSON object, exactly like the worker's validation branch.
pub struct StubBackend;

impl GeometryBackend for StubBackend {
    fn health(&self) -> BackendFuture<'_, BackendHealth> {
        Box::pin(async {
            BackendHealth {
                state: "ready".to_owned(),
                version: Some("0.1.0".to_owned()),
                design_available: false,
                step_available: false,
            }
        })
    }

    fn preview<'a>(
        &'a self,
        body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, kala_studio::geometry_backend::BackendError>>
    {
        Box::pin(async move {
            let parsed: Result<serde_json::Value, _> = serde_json::from_slice(body);
            Ok(match parsed {
                Ok(serde_json::Value::Object(_)) => WorkerResponse {
                    status: 200,
                    headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                    body: b"{\"positions\":[],\"normals\":[],\"indices\":[],\"bounds\":{\"min\":[0.0,0.0,0.0],\"max\":[0.0,0.0,0.0]},\"engine\":\"stub\",\"warnings\":[]}".to_vec(),
                },
                Ok(_) => WorkerResponse {
                    status: 422,
                    headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                    body: b"{\"code\":\"invalid_request\",\"message\":\"the request body or parameters are invalid\",\"details\":[\"body: model_attributes_type: Input should be a valid dictionary or object to extract fields from\"]}".to_vec(),
                },
                Err(_) => WorkerResponse {
                    status: 422,
                    headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                    body: b"{\"code\":\"invalid_request\",\"message\":\"the request body or parameters are invalid\",\"details\":[\"body.1: json_invalid: JSON decode error\"]}".to_vec(),
                },
            })
        })
    }

    fn journal_append<'a>(
        &'a self,
        _body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, kala_studio::geometry_backend::BackendError>>
    {
        Box::pin(async {
            Ok(WorkerResponse {
                status: 201,
                headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                body: b"{\"sequence\":1}".to_vec(),
            })
        })
    }

    /// The view-only STEP import answers with the number of bytes it was
    /// given, so a test can prove that a body reached the worker instead of
    /// being refused by the front door's own body gate.
    fn import_step<'a>(
        &'a self,
        body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, kala_studio::geometry_backend::BackendError>>
    {
        Box::pin(async move {
            Ok(WorkerResponse {
                status: 200,
                headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                body: format!("{{\"bytes\":{}}}", body.len()).into_bytes(),
            })
        })
    }
}

pub struct TestServer {
    pub addr: std::net::SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    join: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl TestServer {
    pub async fn start(
        data_dir: PathBuf,
        token: &str,
        max_body_bytes: usize,
        static_root: Option<PathBuf>,
        environ: HashMap<String, String>,
    ) -> Self {
        Self::start_with(
            data_dir,
            token,
            max_body_bytes,
            static_root,
            environ,
            kala_studio::geometry::default_step_exporter(),
            None,
        )
        .await
    }

    /// Start with the truthful unavailable STEP seam instead of the linked
    /// native exporter. This is the explicit alternate build/testing path.
    pub async fn start_unavailable_step(
        data_dir: PathBuf,
        token: &str,
        max_body_bytes: usize,
        static_root: Option<PathBuf>,
        environ: HashMap<String, String>,
    ) -> Self {
        Self::start_with(
            data_dir,
            token,
            max_body_bytes,
            static_root,
            environ,
            Arc::new(kala_studio::geometry::UnavailableStepExporter),
            None,
        )
        .await
    }

    pub async fn start_with(
        data_dir: PathBuf,
        token: &str,
        max_body_bytes: usize,
        static_root: Option<PathBuf>,
        environ: HashMap<String, String>,
        step_exporter: Arc<dyn StepExporter>,
        transport: Option<Arc<dyn HttpTransport>>,
    ) -> Self {
        Self::start_with_backend(
            data_dir,
            token,
            max_body_bytes,
            static_root,
            environ,
            step_exporter,
            transport,
            Arc::new(StubBackend),
        )
        .await
    }

    /// Start with an explicit geometry backend.
    #[allow(clippy::too_many_arguments)]
    pub async fn start_with_backend(
        data_dir: PathBuf,
        token: &str,
        max_body_bytes: usize,
        static_root: Option<PathBuf>,
        environ: HashMap<String, String>,
        step_exporter: Arc<dyn StepExporter>,
        transport: Option<Arc<dyn HttpTransport>>,
        geometry_backend: Arc<dyn GeometryBackend>,
    ) -> Self {
        Self::start_config(StudioConfig {
            data_dir: Some(data_dir),
            api_token: Some(token.to_owned()),
            max_body_bytes,
            static_root,
            environ: Some(environ),
            import_max_bytes: None,
            step_exporter,
            transport,
            geometry_backend: Some(geometry_backend),
        })
        .await
    }

    pub async fn start_config(config: StudioConfig) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let service = StudioService::new(config).unwrap();
        let (shutdown, receiver) = oneshot::channel();
        let join = tokio::spawn(service.serve(listener, async move {
            let _ = receiver.await;
        }));
        Self {
            addr,
            shutdown: Some(shutdown),
            join,
        }
    }

    pub async fn stop(mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        let _ = tokio::time::timeout(Duration::from_secs(6), self.join).await;
    }
}

#[derive(Debug, Clone)]
pub struct TestResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl TestResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("response JSON")
    }
}

#[derive(Clone, Debug, Default)]
pub struct RequestSpec {
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub chunked: Option<Vec<Vec<u8>>>,
}

pub async fn request(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    spec: RequestSpec,
) -> TestResponse {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut has_host = false;
    let mut has_close = false;
    let mut has_content_length = false;
    let mut request = format!("{method} {path} HTTP/1.1\r\n");
    for (name, value) in &spec.headers {
        if name.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        if name.eq_ignore_ascii_case("connection") {
            has_close = true;
        }
        if name.eq_ignore_ascii_case("content-length") {
            has_content_length = true;
        }
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    if !has_host {
        request.push_str("Host: 127.0.0.1\r\n");
    }
    if !has_close {
        request.push_str("Connection: close\r\n");
    }
    if spec.chunked.is_some() {
        request.push_str("Transfer-Encoding: chunked\r\n");
    } else if let Some(body) = &spec.body {
        if !has_content_length {
            request.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    if let Some(chunks) = spec.chunked {
        for chunk in chunks {
            stream
                .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                .await
                .unwrap();
            stream.write_all(&chunk).await.unwrap();
            stream.write_all(b"\r\n").await.unwrap();
        }
        stream.write_all(b"0\r\n\r\n").await.unwrap();
    } else if let Some(body) = spec.body {
        stream.write_all(&body).await.unwrap();
    }
    stream.flush().await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    parse_response(&raw)
}

fn parse_response(raw: &[u8]) -> TestResponse {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("response header terminator");
    let head = std::str::from_utf8(&raw[..split]).expect("response header text");
    let body = raw[split + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect::<Vec<_>>();
    let declared = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok());
    let body = match declared {
        Some(length) if body.len() >= length => body[..length].to_vec(),
        _ => body,
    };
    TestResponse {
        status,
        headers,
        body,
    }
}

pub const TOKEN: &str = "corpus-studio-token-0123456789abcdef";

pub fn empty_environ() -> HashMap<String, String> {
    HashMap::new()
}
