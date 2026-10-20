//! The live differential of the local Kala HTTP API.
//!
//! The test drives the **live** Python FastAPI application of
//! `kala_agent.api` and the **native** server of this crate with the very
//! same request script, and compares the answers: the status, the headers
//! that matter and the exact body bytes.
//!
//! Two normalizations are applied to the bodies, and only two:
//!
//! * the wall clock, because the two runs happen at different times;
//! * the identity of the domain packages, because the accepted `kala-domains`
//!   crate publishes native Rust digests and not the historical Python source
//!   digests. Every digest that is computed over a package digest therefore
//!   differs, and so does the commit identity that carries two of them.
//!
//! Everything else must be equal, byte for byte.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The interpreter of the repository.
fn python() -> PathBuf {
    std::env::var("KALA_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/home/v9i/Projects/Kala-Agent/.venv/bin/python"))
}

/// The repository root.
fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .expect("the repository root exists")
}

/// One case of the request script.
struct Case {
    /// The name of the case.
    name: &'static str,
    /// The method of the request.
    method: &'static str,
    /// The path of the request.
    path: String,
    /// The headers of the request.
    headers: Vec<(String, String)>,
    /// The body of the request.
    body: Vec<u8>,
    /// Whether the request carries the launch token.
    token: bool,
}

impl Case {
    /// One case that reads.
    fn get(name: &'static str, path: &str) -> Self {
        Self {
            name,
            method: "GET",
            path: path.to_owned(),
            headers: Vec::new(),
            body: Vec::new(),
            token: false,
        }
    }

    /// One case that writes.
    ///
    /// The body of a write is a JSON document, so the request carries the
    /// content type that names it.
    fn post(name: &'static str, path: &str, body: &str) -> Self {
        Self {
            name,
            method: "POST",
            path: path.to_owned(),
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: body.as_bytes().to_vec(),
            token: true,
        }
    }

    /// One case that writes a body of bytes.
    fn post_bytes(name: &'static str, path: &str, body: &[u8]) -> Self {
        Self {
            name,
            method: "POST",
            path: path.to_owned(),
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: body.to_vec(),
            token: true,
        }
    }

    /// One case with a header of its own.
    ///
    /// A header of the same name replaces the one the case already carries,
    /// so a case can name another content type.
    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers
            .retain(|(key, _)| !key.eq_ignore_ascii_case(name));
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// One case with a header of its own, beside the ones it already carries.
    fn extra_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// One answer of a server.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Reply {
    /// The status of the answer.
    status: u16,
    /// The headers that matter.
    headers: BTreeMap<String, String>,
    /// The body of the answer.
    body: Vec<u8>,
}

/// The request script of the differential.
fn script() -> Vec<Case> {
    let mut cases = Vec::new();

    // The health route and the root router.
    cases.push(Case::get("health", "/health"));
    cases.push(Case::get("health-redirect", "/health/"));
    cases.push(Case::get("health-twice-redirect", "/health//"));

    // The domain packages.
    cases.push(Case::get("domains", "/api/v1/domains"));
    cases.push(Case::get("domain", "/api/v1/domains/building"));
    cases.push(Case::get("domain-unknown", "/api/v1/domains/nope"));
    cases.push(Case::get("domain-invalid", "/api/v1/domains/not%20valid"));
    cases.push(Case::get(
        "domain-too-long",
        &format!("/api/v1/domains/{}", "x".repeat(200)),
    ));
    cases.push(Case::get("domain-redirect", "/api/v1/domains/building/"));
    cases.push(Case::get(
        "domain-extra-segment",
        "/api/v1/domains/building/extra",
    ));
    cases.push(Case::get(
        "domain-encoded-slash",
        "/api/v1/domains/building%2Fx",
    ));

    // The evaluations of one package.
    cases.push(Case::post(
        "evaluation",
        "/api/v1/domains/building/evaluations",
        "{}",
    ));
    cases.push(Case::post(
        "evaluation-with-parameters",
        "/api/v1/domains/mechanical/evaluations",
        "{\"design_id\":\"probe\",\"parameters\":{\"material\":\"steel\"}}",
    ));
    cases.push(Case::post(
        "evaluation-without-a-body",
        "/api/v1/domains/building/evaluations",
        "",
    ));
    cases.push(Case::post(
        "evaluation-unknown-domain",
        "/api/v1/domains/nope/evaluations",
        "{}",
    ));
    cases.push(Case::post(
        "evaluation-invalid-body",
        "/api/v1/domains/building/evaluations",
        "{\"design_id\": \"d\", ",
    ));
    cases.push(Case::post(
        "evaluation-body-not-an-object",
        "/api/v1/domains/building/evaluations",
        "[1, 2]",
    ));
    cases.push(Case::post(
        "evaluation-extra-field",
        "/api/v1/domains/building/evaluations",
        "{\"unknown\": 1}",
    ));

    // The revisions.
    cases.push(Case::post(
        "revision-created",
        "/api/v1/revisions",
        "{\"design_id\":\"probe\",\"domain\":\"building\",\"summary\":\"the first revision\"}",
    ));
    cases.push(Case::post(
        "revision-missing-field",
        "/api/v1/revisions",
        "{\"domain\":\"building\"}",
    ));
    cases.push(Case::post(
        "revision-extra-field",
        "/api/v1/revisions",
        "{\"design_id\":\"probe\",\"domain\":\"building\",\"unknown\":1}",
    ));
    cases.push(Case::post(
        "revision-unknown-domain",
        "/api/v1/revisions",
        "{\"design_id\":\"probe\",\"domain\":\"nope\"}",
    ));
    cases.push(Case::post(
        "revision-invalid-design",
        "/api/v1/revisions",
        "{\"design_id\":\"not valid\",\"domain\":\"building\"}",
    ));
    cases.push(Case::post(
        "revision-stale-digest",
        "/api/v1/revisions",
        "{\"design_id\":\"probe\",\"domain\":\"building\",\"state_digest\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}",
    ));
    cases.push(Case::get("revisions", "/api/v1/revisions?design_id=probe"));
    cases.push(Case::get(
        "revisions-empty",
        "/api/v1/revisions?design_id=other",
    ));
    cases.push(Case::get("revisions-without-design", "/api/v1/revisions"));
    cases.push(Case::get(
        "revisions-limit-too-small",
        "/api/v1/revisions?design_id=probe&limit=0",
    ));
    cases.push(Case::get(
        "revisions-limit-too-large",
        "/api/v1/revisions?design_id=probe&limit=201",
    ));
    cases.push(Case::get(
        "revisions-limit-not-a-number",
        "/api/v1/revisions?design_id=probe&limit=abc",
    ));
    cases.push(Case::get(
        "revisions-limit-blank",
        "/api/v1/revisions?design_id=probe&limit=",
    ));
    cases.push(Case::get(
        "revisions-limit-repeated",
        "/api/v1/revisions?design_id=probe&limit=1&limit=2",
    ));
    cases.push(Case::get("revision", "/api/v1/revisions/rev-probe-r0"));
    cases.push(Case::get(
        "revision-unknown",
        "/api/v1/revisions/rev-probe-r9",
    ));

    // The prepared revision commits and their decisions.
    cases.push(Case::post(
        "commit-prepared",
        "/api/v1/revision-commits",
        "{\"design_id\":\"committed\",\"domain\":\"building\"}",
    ));
    cases.push(Case::get(
        "commit-read",
        "/api/v1/revision-commits/{request_id}",
    ));
    cases.push(Case::get(
        "commit-unknown",
        "/api/v1/revision-commits/missing",
    ));
    cases.push(Case::post(
        "commit-approved",
        "/api/v1/revision-commits/{request_id}/approve",
        "{}",
    ));
    cases.push(Case::post(
        "commit-approved-again",
        "/api/v1/revision-commits/{request_id}/approve",
        "{}",
    ));
    cases.push(Case::post(
        "commit-rejected-after-approval",
        "/api/v1/revision-commits/{request_id}/reject",
        "{}",
    ));
    cases.push(Case::post(
        "commit-prepared-for-rejection",
        "/api/v1/revision-commits",
        "{\"design_id\":\"rejected\",\"domain\":\"building\"}",
    ));
    cases.push(Case::post(
        "commit-rejected",
        "/api/v1/revision-commits/{request_id}/reject",
        "{\"actor_id\":\"local-user\",\"reason\":\"not now\"}",
    ));
    cases.push(Case::post(
        "commit-rejected-again",
        "/api/v1/revision-commits/{request_id}/reject",
        "{}",
    ));
    cases.push(Case::post(
        "commit-with-a-digest",
        "/api/v1/revision-commits",
        "{\"design_id\":\"committed\",\"domain\":\"building\",\"state_digest\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}",
    ));

    // The mechanical reference workflow.
    cases.push(Case::post(
        "workflow-started",
        "/api/v1/workflows/mechanical/reference",
        "{}",
    ));
    cases.push(Case::post(
        "workflow-resumed",
        "/api/v1/workflows/mechanical/reference",
        "{}",
    ));
    cases.push(Case::get(
        "workflow-state",
        "/api/v1/workflows/workflow-mechanical-reference",
    ));
    cases.push(Case::get("workflow-unknown", "/api/v1/workflows/nope"));
    cases.push(Case::get(
        "workflow-checkpoints",
        "/api/v1/workflows/workflow-mechanical-reference/checkpoints",
    ));
    cases.push(Case::get(
        "workflow-checkpoints-unknown",
        "/api/v1/workflows/nope/checkpoints",
    ));
    cases.push(Case::post(
        "workflow-approved",
        "/api/v1/workflows/workflow-mechanical-reference/checkpoints/approve",
        "{\"actor_id\":\"local-user\"}",
    ));
    cases.push(Case::post(
        "workflow-approved-again",
        "/api/v1/workflows/workflow-mechanical-reference/checkpoints/approve",
        "{}",
    ));
    cases.push(Case::post(
        "workflow-rejected-after-approval",
        "/api/v1/workflows/workflow-mechanical-reference/checkpoints/reject",
        "{}",
    ));

    // The journal.
    cases.push(Case::get("events", "/api/v1/events"));
    cases.push(Case::get("events-page", "/api/v1/events?limit=1"));
    cases.push(Case::get(
        "events-after-five",
        "/api/v1/events?after_sequence=5",
    ));
    cases.push(Case::get(
        "events-after-the-last",
        "/api/v1/events?after_sequence=1000",
    ));
    cases.push(Case::get(
        "events-after-the-end",
        "/api/v1/events?after_sequence=1000000000",
    ));
    cases.push(Case::get(
        "events-limit-too-small",
        "/api/v1/events?limit=0",
    ));
    cases.push(Case::get(
        "events-limit-too-large",
        "/api/v1/events?limit=201",
    ));
    cases.push(Case::get(
        "events-after-negative",
        "/api/v1/events?after_sequence=-1",
    ));
    cases.push(Case::get(
        "events-after-not-a-number",
        "/api/v1/events?after_sequence=abc",
    ));
    cases.push(Case::get(
        "events-type-invalid",
        "/api/v1/events?event_type=not%20valid",
    ));
    cases.push(Case::get(
        "events-workflow",
        "/api/v1/events?workflow_id=workflow-mechanical-reference",
    ));
    cases.push(Case::get(
        "events-unknown-parameter",
        "/api/v1/events?unknown=1",
    ));

    // The router.
    cases.push(Case::get("unknown-route", "/nope"));
    cases.push(Case::get("unknown-route-under-the-prefix", "/api/v1/nope"));
    cases.push(Case::post("method-not-allowed", "/health", ""));
    cases.push(Case {
        name: "method-not-allowed-head",
        method: "HEAD",
        path: "/health".to_owned(),
        headers: Vec::new(),
        body: Vec::new(),
        token: false,
    });
    cases.push(Case {
        name: "method-not-allowed-options",
        method: "OPTIONS",
        path: "/health".to_owned(),
        headers: Vec::new(),
        body: Vec::new(),
        token: false,
    });
    cases.push(Case::get(
        "method-not-allowed-read",
        "/api/v1/revision-commits",
    ));
    cases.push(Case::get(
        "method-not-allowed-workflow",
        "/api/v1/workflows/mechanical/reference",
    ));
    cases.push(Case::get("double-slash", "/api/v1//domains"));
    cases.push(
        Case::get("host-repeated", "/api/v1/domains")
            .header("Host", "127.0.0.1:8000")
            .extra_header("Host", "127.0.0.1:8000"),
    );
    cases.push(Case::post("read-with-a-body", "/api/v1/domains", "{}"));

    // A body that is not named as JSON is not read as JSON.
    cases.push(
        Case::post(
            "revision-without-a-content-type",
            "/api/v1/revisions",
            "{\"design_id\":\"probe\",\"domain\":\"building\"}",
        )
        .header("Content-Type", ""),
    );
    cases.push(
        Case::post(
            "revision-with-a-text-content-type",
            "/api/v1/revisions",
            "{\"design_id\":\"probe\",\"domain\":\"building\"}",
        )
        .header("Content-Type", "text/plain"),
    );

    // The bodies that are read the way the framework reads them.
    cases.push(Case::post(
        "revision-with-a-repeated-key",
        "/api/v1/revisions",
        "{\"design_id\":\"probe\",\"design_id\":\"repeated\",\"domain\":\"building\"}",
    ));
    cases.push(Case::post(
        "revision-with-a-not-finite-number",
        "/api/v1/revisions",
        "{\"design_id\":\"probe\",\"domain\":\"building\",\"parameters\":{\"a\":NaN}}",
    ));
    cases.push(
        Case::post(
            "revision-with-a-large-integer",
            "/api/v1/revisions",
            "{\"design_id\":\"probe\",\"domain\":\"building\",\"parameters\":{\"a\":123456789012345678901234567890}}",
        ),
    );
    cases.push(
        Case::post(
            "revision-with-a-charset",
            "/api/v1/revisions",
            "{\"design_id\":\"probe\",\"domain\":\"building\"}",
        )
        .header("Content-Type", "application/json; charset=utf-8"),
    );
    cases.push(
        Case::post(
            "revision-with-a-vendor-content-type",
            "/api/v1/revisions",
            "{\"design_id\":\"probe\",\"domain\":\"building\"}",
        )
        .header("Content-Type", "application/vnd.kala+json"),
    );
    cases.push(Case::post(
        "revision-with-a-stale-parent",
        "/api/v1/revisions",
        "{\"design_id\":\"probe\",\"domain\":\"building\",\"parent_revision_id\":\"rev-probe-r0\"}",
    ));
    cases.push(Case::post(
        "evaluation-with-an-out-of-range-parameter",
        "/api/v1/domains/building/evaluations",
        "{\"parameters\":{\"storey_count\":1000}}",
    ));
    cases.push(Case::post(
        "workflow-with-a-design",
        "/api/v1/workflows/mechanical/reference",
        "{\"design_id\":\"probe-workflow\",\"actor_id\":\"local-user\"}",
    ));

    // A body that is not UTF-8 is a failure of the reader.
    cases.push(Case::post_bytes(
        "revision-with-a-body-that-is-not-utf-8",
        "/api/v1/revisions",
        b"{\"design_id\":\"probe\",\"domain\":\"\xff\"}",
    ));

    // The requests that carry more than one failure.
    cases.push(Case::post(
        "evaluation-with-an-invalid-path-and-an-invalid-body",
        "/api/v1/domains/not%20valid/evaluations",
        "{\"domain\":\"nope\"}",
    ));
    cases.push(Case::get(
        "events-with-two-invalid-parameters",
        "/api/v1/events?limit=0&after_sequence=-1",
    ));

    // The request boundary.
    cases.push(
        Case::post("token-missing", "/api/v1/revisions", "{}").header("Authorization", "Bearer"),
    );
    cases.push(
        Case::post("token-wrong", "/api/v1/revisions", "{}")
            .header("Authorization", "Bearer wrong-token-value"),
    );
    cases.push(
        Case::post("token-not-bearer", "/api/v1/revisions", "{}")
            .header("Authorization", "Basic dXNlcjpwYXNz"),
    );
    cases.push(Case::get("host-refused", "/api/v1/domains").header("Host", "example.com"));
    cases.push(Case::get("host-rebound", "/api/v1/domains").header("Host", "kala-agent.local"));
    cases.push(
        Case::get("origin-refused", "/api/v1/domains").header("Origin", "http://127.0.0.1:8000"),
    );
    cases.push(Case::get("origin-refused-on-a-read", "/health").header("Origin", "null"));
    cases.push(Case::post(
        "body-too-large",
        "/api/v1/revisions",
        &format!("{{\"a\":\"{}\"}}", "x".repeat(300_000)),
    ));

    cases
}

/// The transcript of one run.
#[derive(Clone, Debug)]
struct Transcript {
    /// The answers, in the order of the script.
    cases: Vec<Reply>,
}

/// Send one request and read its answer.
fn send(port: u16, method: &str, path: &str, headers: &[(String, String)], body: &[u8]) -> Reply {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("the server accepts");
    let mut request = format!("{method} {path} HTTP/1.1\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!("content-length: {}\r\n", body.len()));
    request.push_str("connection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .expect("the request is written");
    stream.write_all(body).expect("the body is written");
    stream.flush().expect("the request is flushed");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("the answer is read");
    parse_reply(&raw)
}

/// Read one answer of a server.
fn parse_reply(raw: &[u8]) -> Reply {
    let separator = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("the answer carries a header block");
    let head = String::from_utf8_lossy(&raw[..separator]).into_owned();
    let body = raw[separator + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .expect("the status line carries a status");
    let mut headers = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "content-type" | "content-length" | "location"
        ) {
            headers.insert(name, value.trim().to_owned());
        }
    }
    Reply {
        status,
        headers,
        body,
    }
}

/// Read the token of a launch.
fn read_token(data_dir: &Path) -> String {
    std::fs::read_to_string(data_dir.join("api-token"))
        .expect("the launch wrote its token")
        .trim()
        .to_owned()
}

/// Run the script against one server.
fn replay(port: u16, token: &str, cases: &[Case]) -> Transcript {
    let mut answers = Vec::new();
    let mut request_id = String::new();
    for case in cases {
        let mut headers = Vec::new();
        let mut host_seen = false;
        for (name, value) in &case.headers {
            if name.eq_ignore_ascii_case("host") {
                host_seen = true;
            }
            headers.push((name.clone(), value.replace("${TOKEN}", token)));
        }
        if !host_seen {
            headers.push(("Host".to_owned(), format!("127.0.0.1:{port}")));
        }
        if case.token {
            headers.push(("Authorization".to_owned(), format!("Bearer {token}")));
        }
        let path = case.path.replace("{request_id}", &request_id);
        let reply = send(port, case.method, &path, &headers, &case.body);
        if let Ok(document) = serde_json::from_slice::<Value>(&reply.body) {
            if let Some(found) = document.get("request_id").and_then(Value::as_str) {
                request_id = found.to_owned();
            }
        }
        answers.push(reply);
    }
    Transcript { cases: answers }
}

/// Write the request script for the Python oracle.
fn write_script(path: &Path, cases: &[Case]) {
    let mut written = Vec::new();
    for case in cases {
        let mut headers = Vec::new();
        for (name, value) in &case.headers {
            headers.push(json!([name, value]));
        }
        if case.token {
            headers.push(json!(["Authorization", "Bearer ${TOKEN}"]));
        }
        written.push(json!({
            "name": case.name,
            "method": case.method,
            "path": case.path,
            "headers": headers,
            "body": base64_encode(&case.body),
        }));
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(&json!({ "cases": written })).expect("the script is JSON"),
    )
    .expect("the script is written");
}

/// One base64 encoded byte string.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or_default();
        let third = chunk.get(2).copied().unwrap_or_default();
        out.push(ALPHABET[(first >> 2) as usize] as char);
        out.push(ALPHABET[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(((second & 0x0F) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(third & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// One decoded base64 byte string.
fn base64_decode(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer = 0_u32;
    let mut bits = 0_u32;
    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        let Some(position) = ALPHABET.iter().position(|value| *value == byte) else {
            continue;
        };
        buffer = (buffer << 6) | u32::from(position as u8);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}

/// The server of the native launch.
struct Server {
    /// The process of the launch.
    child: Child,
    /// The port the launch serves.
    port: u16,
}

impl Server {
    /// Start the native server on a free port.
    fn start(data_dir: &Path) -> Self {
        let port = free_port();
        let child = Command::new(env!("CARGO_BIN_EXE_kala-api-server"))
            .arg("--data-dir")
            .arg(data_dir)
            .arg("--port")
            .arg(port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the native server starts");
        let server = Self { child, port };
        server.wait_until_ready();
        server
    }

    /// Wait until the server accepts connections.
    fn wait_until_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            if TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the native server did not start");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A port nothing listens on.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port is free");
    let port = listener.local_addr().expect("the address is known").port();
    drop(listener);
    port
}

/// The launch facts that differ between the two runs.
struct Launch {
    /// The data directory of the run.
    data_dir: String,
    /// The port the run serves.
    port: u16,
}

/// Normalize the body of one answer.
///
/// The normalizations are the wall clock, the native identity of the domain
/// packages, and the two facts of the launch that differ between the runs:
/// the data directory and the port.
fn normalize(body: &[u8], launch: &Launch) -> String {
    normalize_text(&String::from_utf8_lossy(body), launch)
}

/// Normalize one text.
fn normalize_text(text: &str, launch: &Launch) -> String {
    let mut text = text.replace(&launch.data_dir, "<data-dir>");
    text = text.replace(&format!("127.0.0.1:{}", launch.port), "127.0.0.1:<port>");
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character.is_ascii_hexdigit() {
            let mut digits = String::new();
            digits.push(character);
            while let Some(next) = characters.peek() {
                if next.is_ascii_hexdigit() && digits.len() < 64 {
                    digits.push(*next);
                    characters.next();
                } else {
                    break;
                }
            }
            if digits.len() >= 16 {
                out.push_str("<digest>");
            } else {
                out.push_str(&digits);
            }
            continue;
        }
        out.push(character);
    }
    // The wall clock: an aware timestamp of the ISO 8601 form.
    let mut normalized = String::with_capacity(out.len());
    let mut rest: &str = &out;
    loop {
        let mut found = None;
        for (index, _) in rest.char_indices() {
            if let Some(length) = timestamp_length(&rest[index..]) {
                found = Some((index, length));
                break;
            }
        }
        let Some((index, length)) = found else {
            break;
        };
        normalized.push_str(&rest[..index]);
        normalized.push_str("<timestamp>");
        rest = &rest[index + length..];
    }
    normalized.push_str(rest);
    normalized
}

/// The length of the timestamp that starts a text.
fn timestamp_length(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    let digits = |start: usize, count: usize| -> bool {
        bytes
            .get(start..start + count)
            .is_some_and(|window| window.iter().all(u8::is_ascii_digit))
    };
    if !digits(0, 4) || bytes[4] != b'-' || !digits(5, 2) || bytes[7] != b'-' || !digits(8, 2) {
        return None;
    }
    if bytes[10] != b'T'
        || !digits(11, 2)
        || bytes[13] != b':'
        || !digits(14, 2)
        || bytes[16] != b':'
    {
        return None;
    }
    if !digits(17, 2) {
        return None;
    }
    let mut length = 19;
    if bytes.get(19) == Some(&b'.') {
        let mut index = 20;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == 20 {
            return None;
        }
        length = index;
    }
    match bytes.get(length) {
        Some(b'Z') => Some(length + 1),
        Some(b'+') | Some(b'-') => {
            if bytes.get(length + 3) == Some(&b':')
                && digits(length + 1, 2)
                && digits(length + 4, 2)
            {
                Some(length + 6)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The place of the first difference of two texts.
fn first_difference(left: &str, right: &str) -> usize {
    left.char_indices()
        .zip(right.chars())
        .find(|((_, left), right)| left != right)
        .map(|((index, _), _)| index)
        .unwrap_or_else(|| left.len().min(right.len()))
}

/// The window of a text around a place.
fn window(text: &str, place: usize) -> String {
    let start = text[..place.min(text.len())]
        .char_indices()
        .rev()
        .nth(120)
        .map(|(index, _)| index)
        .unwrap_or(0);
    let end = text[place.min(text.len())..]
        .char_indices()
        .nth(120)
        .map(|(index, _)| place + index)
        .unwrap_or(text.len());
    format!("...{}...", &text[start..end])
}

/// The cases the two HTTP parsers answer differently.
///
/// A request that carries a duplicated `Host` header is refused by the parser
/// of the Python server before the application sees it, and the parser of the
/// native server hands it to the application, which refuses it with its own
/// answer. The difference belongs to the parsers and not to the routes, and
/// the `api-core-r1` cell already declares it.
const PARSER_REFUSALS: [&str; 1] = ["host-repeated"];

/// The live differential of the API.
#[test]
fn the_native_api_answers_the_live_python_api() {
    let repository = repository();
    let python = python();
    if !python.exists() {
        panic!("the interpreter of the repository is missing: {python:?}");
    }
    let temporary = tempfile::tempdir().expect("a temporary directory is free");
    let python_data = temporary.path().join("python");
    let native_data = temporary.path().join("native");
    std::fs::create_dir_all(&python_data).expect("the data directory is created");
    std::fs::create_dir_all(&native_data).expect("the data directory is created");

    let cases = script();
    let script_path = temporary.path().join("script.json");
    let transcript_path = temporary.path().join("python.json");
    write_script(&script_path, &cases);

    // The live Python application.
    let oracle = Command::new(&python)
        .arg("-I")
        .arg(repository.join("backend/crates/kala-api/server/tests/oracle_routes.py"))
        .arg(&script_path)
        .arg(&transcript_path)
        .arg(&python_data)
        .current_dir(&repository)
        .output()
        .expect("the oracle runs");
    assert!(
        oracle.status.success(),
        "the oracle failed: {}",
        String::from_utf8_lossy(&oracle.stderr)
    );

    let document: Value = serde_json::from_str(
        &std::fs::read_to_string(&transcript_path).expect("the transcript is read"),
    )
    .expect("the transcript is JSON");
    let python_answers = document["cases"]
        .as_array()
        .expect("the transcript carries the answers");
    let python_port = document["port"]
        .as_u64()
        .expect("the transcript carries the port") as u16;

    // The native server.
    let server = Server::start(&native_data);
    let native_token = read_token(&native_data);
    let native = replay(server.port, &native_token, &cases);
    let python_launch = Launch {
        data_dir: python_data.to_string_lossy().into_owned(),
        port: python_port,
    };
    let native_launch = Launch {
        data_dir: native_data.to_string_lossy().into_owned(),
        port: server.port,
    };

    assert_eq!(
        python_answers.len(),
        native.cases.len(),
        "the two runs answered the same number of cases"
    );

    let mut mismatches = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        if PARSER_REFUSALS.contains(&case.name) {
            let expected = python_answers[index]["status"]
                .as_u64()
                .expect("the transcript carries a status");
            let actual = native.cases[index].status;
            if expected != 400 || actual != 421 {
                mismatches.push(format!(
                    "{}: the two parsers did not refuse the request (python {expected}, native {actual})",
                    case.name
                ));
            }
            continue;
        }
        let expected = python_answers[index]
            .get("status")
            .and_then(Value::as_u64)
            .expect("the transcript carries a status") as u16;
        let actual = native.cases[index].status;
        if expected != actual {
            mismatches.push(format!(
                "{}: the status differs (python {expected}, native {actual})",
                case.name
            ));
            continue;
        }
        let expected_headers = python_answers[index]["headers"]
            .as_object()
            .expect("the transcript carries the headers");
        for (name, value) in expected_headers {
            let value = normalize_text(value.as_str().expect("a header is text"), &python_launch);
            let actual = native.cases[index]
                .headers
                .get(name)
                .map(|actual| normalize_text(actual, &native_launch));
            match actual {
                Some(actual) if actual == value => {}
                Some(actual) => mismatches.push(format!(
                    "{}: the header {name} differs (python {value}, native {actual})",
                    case.name
                )),
                None => mismatches.push(format!(
                    "{}: the header {name} is missing (python {value})",
                    case.name
                )),
            }
        }
        let expected_body = base64_decode(
            python_answers[index]["body"]
                .as_str()
                .expect("the transcript carries the body"),
        );
        let actual_body = native.cases[index].body.clone();
        let expected_text = normalize(&expected_body, &python_launch);
        let actual_text = normalize(&actual_body, &native_launch);
        if expected_text != actual_text {
            let difference = first_difference(&expected_text, &actual_text);
            mismatches.push(format!(
                "{}: the body differs at {difference}\n  python: {}\n  native: {}",
                case.name,
                window(&expected_text, difference),
                window(&actual_text, difference)
            ));
        }
    }

    println!(
        "the live differential compared {} cases ({} of them strictly, {} declared parser refusals)",
        cases.len(),
        cases.len() - PARSER_REFUSALS.len(),
        PARSER_REFUSALS.len()
    );
    assert!(
        mismatches.is_empty(),
        "{} mismatches out of {} cases:\n{}",
        mismatches.len(),
        cases.len(),
        mismatches.join("\n")
    );
}
