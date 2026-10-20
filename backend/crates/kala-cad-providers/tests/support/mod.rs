//! Shared test support: a fake `FreeCADCmd` and an in-process mock bridge.
//!
//! The fake FreeCAD executable is a *test fixture*, not FreeCAD. The mock
//! bridge is a real loopback HTTP server that implements the bridge's health
//! contract only; it is not evidence about Solid Edge.

#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use kala_cad_providers::{FreeCadProvider, FreeCadProviderOptions};
use kala_contracts::ConstructionScript;
use tempfile::TempDir;

/// Install the fake `FreeCADCmd` into a fresh directory and return it.
pub fn fake_freecad() -> (TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let target = directory.path().join("FreeCADCmd");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_freecad.py");
    std::fs::copy(&fixture, &target).unwrap_or_else(|error| {
        panic!("copy {}: {error}", fixture.display());
    });
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
            .expect("mark the fixture executable");
    }
    (directory, target)
}

/// The base child environment for the fake executable.
pub fn fake_environment(mode: &str) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    environment.insert(
        "PATH".to_owned(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_owned()),
    );
    environment.insert(
        "HOME".to_owned(),
        std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_owned()),
    );
    environment.insert("LANG".to_owned(), "C.UTF-8".to_owned());
    environment.insert("FAKE_FREECAD_MODE".to_owned(), mode.to_owned());
    // Credential-shaped keys and the interpreter path the Python provider
    // removes must never reach the child.
    environment.insert(
        "OPENAI_API_KEY".to_owned(),
        "sk-should-not-reach-freecad".to_owned(),
    );
    environment.insert(
        "KALA_WORKER_TOKEN".to_owned(),
        "worker-token-should-not-reach-freecad".to_owned(),
    );
    environment.insert("DATABASE_PASSWORD".to_owned(), "hunter2".to_owned());
    environment.insert("MY_SECRET".to_owned(), "secret-value".to_owned());
    environment.insert("AWS_CREDENTIAL".to_owned(), "credential-value".to_owned());
    environment.insert(
        "PYTHONPATH".to_owned(),
        "/should/not/reach/the/freecad/child".to_owned(),
    );
    environment
}

/// Build a fake-backed provider.
pub fn fake_provider(executable: &Path, mode: &str) -> FreeCadProvider {
    fake_provider_with(executable, mode, BTreeMap::new())
}

/// Build a fake-backed provider with extra child environment entries.
pub fn fake_provider_with(
    executable: &Path,
    mode: &str,
    extra: BTreeMap<String, String>,
) -> FreeCadProvider {
    let mut environment = fake_environment(mode);
    environment.extend(extra);
    FreeCadProvider::new(FreeCadProviderOptions {
        executable: Some(executable.to_path_buf()),
        timeout_seconds: Some(60.0),
        probe_cache_seconds: Some(0.0),
        environ: Some(environment),
        sys_path: Some(Vec::new()),
        ..FreeCadProviderOptions::default()
    })
    .expect("a valid fake provider")
}

/// One rectangle pad.
pub fn pad_script() -> ConstructionScript {
    kala_contracts::load_construction_json_str(
        r#"{"summary":"fake pad","components":[{"name":"body","steps":[
            {"op":"sketch","plane":"XY","profile":"rectangle","width":4,"height":5},
            {"op":"extrude","length":6}]}]}"#,
    )
    .expect("a valid script")
}

/// Two components with a fuse step, so a generic refusal names the op.
pub fn fuse_script() -> ConstructionScript {
    kala_contracts::load_construction_json_str(
        r#"{"summary":"fake fuse","components":[
            {"name":"a","steps":[
                {"op":"sketch","plane":"XY","profile":"rectangle","width":4,"height":5},
                {"op":"extrude","length":6}]},
            {"name":"b","steps":[
                {"op":"sketch","plane":"XY","profile":"rectangle","width":2,"height":2},
                {"op":"extrude","length":2}]}],
            "assembly":[{"op":"place","component":"a"},{"op":"place","component":"b","x":20},
                        {"op":"fuse","components":["a","b"]}]}"#,
    )
    .expect("a valid script")
}

/// Two components with only place steps.
pub fn place_only_script() -> ConstructionScript {
    kala_contracts::load_construction_json_str(
        r#"{"summary":"fake placed pair","components":[
            {"name":"a","steps":[
                {"op":"sketch","plane":"XY","profile":"rectangle","width":4,"height":5},
                {"op":"extrude","length":6}]},
            {"name":"b","steps":[
                {"op":"sketch","plane":"XY","profile":"rectangle","width":2,"height":2},
                {"op":"extrude","length":2}]}],
            "assembly":[{"op":"place","component":"a"},{"op":"place","component":"b","x":20}]}"#,
    )
    .expect("a valid script")
}

/// The full protocol-v1 health document the real bridge serves.
pub fn health_body(available: bool, supported: bool, se_version: &str, detail: &str) -> String {
    format!(
        r#"{{"protocol_version":1,"bridge_version":"0.1.0","platform":"win32","supported":{supported},"available":{available},"busy":false,"se_version":"{se_version}","calibration_id":"cal-1","capabilities":["protocol.v1"],"detail":"{detail}"}}"#
    )
}

/// One request the mock bridge received.
#[derive(Clone, Debug)]
pub struct RecordedRequest {
    /// The HTTP method.
    pub method: String,
    /// The request target.
    pub path: String,
    /// The headers, with lowercased names.
    pub headers: Vec<(String, String)>,
    /// The request body.
    pub body: Vec<u8>,
}

impl RecordedRequest {
    /// The first value of a header.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// One canned answer.
#[derive(Clone, Debug)]
pub enum CannedResponse {
    /// A normal response.
    Body {
        /// The status code.
        status: u16,
        /// The body.
        body: Vec<u8>,
        /// Extra headers.
        headers: Vec<(String, String)>,
    },
    /// A response that arrives only after a delay.
    Delayed {
        /// The status code.
        status: u16,
        /// The body.
        body: Vec<u8>,
        /// How long the answer is held back.
        delay: Duration,
    },
}

impl CannedResponse {
    /// A JSON response.
    pub fn json(status: u16, body: &str) -> Self {
        Self::Body {
            status,
            body: body.as_bytes().to_vec(),
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
        }
    }

    /// A response with extra headers.
    pub fn with_headers(mut self, extra: Vec<(String, String)>) -> Self {
        if let Self::Body { headers, .. } = &mut self {
            headers.extend(extra);
        }
        self
    }
}

/// A loopback HTTP server that answers with canned responses.
pub struct MockBridge {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    responses: Arc<Mutex<VecDeque<CannedResponse>>>,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockBridge {
    /// Start the server on an ephemeral loopback port.
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let address = listener.local_addr().expect("the local address");
        let requests: Arc<Mutex<Vec<RecordedRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let responses: Arc<Mutex<VecDeque<CannedResponse>>> = Arc::new(Mutex::new(VecDeque::new()));
        let running = Arc::new(AtomicBool::new(true));
        let handle = {
            let requests = Arc::clone(&requests);
            let responses = Arc::clone(&responses);
            let running = Arc::clone(&running);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if !running.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(mut stream) = stream else {
                        continue;
                    };
                    let requests = Arc::clone(&requests);
                    let responses = Arc::clone(&responses);
                    std::thread::spawn(move || {
                        let recorded = read_request(&mut stream);
                        requests.lock().expect("the request log").push(recorded);
                        let response = responses
                            .lock()
                            .expect("the response queue")
                            .pop_front()
                            .unwrap_or_else(|| {
                                CannedResponse::json(
                                    500,
                                    r#"{"code":"internal_error","message":"no response"}"#,
                                )
                            });
                        write_response(&mut stream, response);
                    });
                }
            })
        };
        Self {
            address,
            requests,
            responses,
            running,
            handle: Some(handle),
        }
    }

    /// The base URL of the mock.
    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Queue one response.
    pub fn queue(&self, response: CannedResponse) {
        self.responses
            .lock()
            .expect("the response queue")
            .push_back(response);
    }

    /// The requests the mock received.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().expect("the request log").clone()
    }
}

impl Drop for MockBridge {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0_u8; 1024];
    let end = loop {
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        match stream.read(&mut chunk) {
            Ok(0) => break buffer.len(),
            Ok(count) => buffer.extend_from_slice(&chunk[..count]),
            Err(_) => break buffer.len(),
        }
    };
    let mut leftover = buffer[end..].to_vec();
    let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = std::mem::take(&mut leftover);
    if body.len() > length {
        body.truncate(length);
    }
    while body.len() < length {
        let mut chunk = vec![0_u8; length - body.len()];
        match stream.read_exact(&mut chunk) {
            Ok(()) => body.extend_from_slice(&chunk),
            Err(_) => {
                body.clear();
                break;
            }
        }
    }
    RecordedRequest {
        method,
        path,
        headers,
        body,
    }
}

fn write_response(stream: &mut TcpStream, response: CannedResponse) {
    let (status, body, headers, delay) = match response {
        CannedResponse::Body {
            status,
            body,
            headers,
        } => (status, body, headers, Duration::ZERO),
        CannedResponse::Delayed {
            status,
            body,
            delay,
        } => (status, body, Vec::new(), delay),
    };
    if !delay.is_zero() {
        std::thread::sleep(delay);
    }
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    };
    let declared = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| body.len().to_string());
    let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
    head.push_str("Content-Type: application/json\r\n");
    head.push_str(&format!("Content-Length: {declared}\r\n"));
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}
