//! Live differential: the native `kala-agent serve` against the Python CLI's
//! `serve`, driven with the request script of the accepted `api-routes-r1`
//! cell.
//!
//! Both command lines are started as real processes, each on its own temporary
//! data directory and HOME, and both are stopped by a signal. The corpus is the
//! one of `backend/crates/kala-api/server/tests/routes_differential.rs`: every
//! route of the local API, its validation failures, its conflicts, its
//! pagination and its request boundary. The answers are compared by status,
//! by the headers that matter, and by the exact body bytes after the declared
//! normalizations only (the data directory, the port, the wall clock, and the
//! native identity digests).
//!
//! The launch and shutdown contract is compared as well: the token file, its
//! owner-only mode, the pre-bind refusals and their data-directory side
//! effects, a failed bind, an unstorable token, the summary of an interrupt,
//! and the signal termination of a terminate request.
//!
//! Python is a test oracle only. The binary never invokes it.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// The interpreter of the repository.
fn python() -> PathBuf {
    std::env::var("KALA_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/home/v9i/Projects/Kala-Agent/.venv/bin/python"))
}

/// The repository root.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("crate lives under backend/crates/kala-cli")
        .to_path_buf()
}

/// The native binary.
fn native_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kala-agent"))
}

/// Whether the repository interpreter can import the live command line.
fn python_available() -> bool {
    Command::new(python())
        .args(["-I", "-c", "import kala_agent.cli.main, uvicorn"])
        .current_dir(repository())
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// One side of the differential: its data directory and its HOME.
struct Side {
    data: PathBuf,
    home: PathBuf,
}

impl Side {
    fn new(root: &Path, name: &str) -> Self {
        let home = root.join(format!("{name}-home"));
        fs::create_dir_all(&home).expect("home");
        Self {
            data: root.join(format!("{name}-data")),
            home,
        }
    }

    fn token_path(&self) -> PathBuf {
        self.data.join("api-token")
    }
}

/// Build the command line of one side.
///
/// `arguments` is the complete argument list after the program name; the
/// root options therefore come before the subcommand, as argparse requires.
fn command_argv(python_side: bool, side: &Side, arguments: &[&str]) -> Command {
    let mut command = if python_side {
        let mut command = Command::new(python());
        command.args(["-I", "-m", "kala_agent.cli"]);
        command
    } else {
        Command::new(native_binary())
    };
    command
        .args(arguments)
        .current_dir(repository())
        .env_clear()
        .env("HOME", &side.home)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LC_ALL", "C.UTF-8")
        .env("LANG", "C.UTF-8")
        .env("TZ", "UTC")
        .env("KALA_AGENT_DATA_DIR", &side.data)
        .env("KALA_AGENT_MODEL_PROVIDER", "local")
        .env("KALA_AGENT_MODEL_ID", "local");
    command
}

/// Start one command line with piped output.
fn start_argv(python_side: bool, side: &Side, arguments: &[&str]) -> Child {
    command_argv(python_side, side, arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the command line starts")
}

/// Start one serve process.
fn start(python_side: bool, side: &Side, extra: &[&str]) -> Child {
    let mut arguments = vec!["serve"];
    arguments.extend_from_slice(extra);
    start_argv(python_side, side, &arguments)
}

/// Wait until the launch accepts a connection.
fn wait_ready(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("the launch on {port} did not start");
}

/// Wait until the launch accepts a connection on `::1`.
fn wait_ready_v6(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if TcpStream::connect(("::1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("the launch on [::1]:{port} did not start");
}

/// Ask one process to stop.
fn signal(child: &Child, signal: i32) {
    let result = unsafe { libc::kill(child.id() as i32, signal) };
    assert_eq!(result, 0, "the process is signalled");
}

/// A launched server that is killed if a test fails before it stops it.
struct Running {
    child: Option<Child>,
}

impl Running {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn child(&self) -> &Child {
        self.child.as_ref().expect("a running server")
    }

    fn finish(mut self, seconds: u64) -> Output {
        wait_with_timeout(self.child.take().expect("a running server"), seconds)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Wait for a child, with a bound so a broken launch cannot hang the suite.
fn wait_with_timeout(mut child: Child, seconds: u64) -> Output {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        match child.try_wait().expect("the child is waited for") {
            Some(_) => return child.wait_with_output().expect("the output is collected"),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let output = child.wait_with_output().expect("the killed output");
                panic!(
                    "the launch did not stop in time; stdout: {}; stderr: {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// Run one short serve command and collect its output.
fn run(python_side: bool, side: &Side, argv: &[&str]) -> Output {
    let mut arguments = vec!["serve"];
    arguments.extend_from_slice(argv);
    run_argv(python_side, side, &arguments)
}

/// Run one complete command line and collect its output.
fn run_argv(python_side: bool, side: &Side, arguments: &[&str]) -> Output {
    command_argv(python_side, side, arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("the command line runs")
}

/// A port nothing listens on.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port is free");
    let port = listener.local_addr().expect("the address is known").port();
    drop(listener);
    port
}

/// A port nothing listens on, on the IPv6 loopback interface.
fn free_port_v6() -> u16 {
    let listener = TcpListener::bind("[::1]:0").expect("a port is free");
    let port = listener.local_addr().expect("the address is known").port();
    drop(listener);
    port
}

/// The stored token of one launch.
fn read_token(data_dir: &Path) -> String {
    fs::read_to_string(data_dir.join("api-token"))
        .expect("the launch wrote its token")
        .trim()
        .to_owned()
}

/// Assert the token file of one launch has the Python shape and mode.
fn assert_token_file(side: &Side, token: &str) {
    assert_eq!(token.len(), 43, "a 32-byte urlsafe token has 43 characters");
    assert!(
        token
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')),
        "the token uses the urlsafe alphabet"
    );
    let raw = fs::read_to_string(side.token_path()).expect("the token file is text");
    assert_eq!(raw, format!("{token}\n"), "the token file carries one line");
    let mode = fs::metadata(side.token_path())
        .expect("the token file is known")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "the token file is owner only");
}

/// One case of the request script.
struct Case {
    name: &'static str,
    method: &'static str,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    token: bool,
}

impl Case {
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

    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers
            .retain(|(key, _)| !key.eq_ignore_ascii_case(name));
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    fn extra_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// One answer of a server.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Reply {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

/// The request script of the accepted API differential.
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
    cases.push(Case::post(
        "revision-with-a-large-integer",
        "/api/v1/revisions",
        "{\"design_id\":\"probe\",\"domain\":\"building\",\"parameters\":{\"a\":123456789012345678901234567890}}",
    ));
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
struct Transcript {
    cases: Vec<Reply>,
}

/// Send one request and read its answer.
fn send(
    name: &str,
    port: u16,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Reply {
    send_on(name, "127.0.0.1", port, method, path, headers, body)
}

/// Send one request to one interface and read its answer.
fn send_on(
    name: &str,
    host: &str,
    port: u16,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Reply {
    let mut stream = TcpStream::connect((host, port)).expect("the server accepts");
    let mut request = format!("{method} {path} HTTP/1.1\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!("content-length: {}\r\n", body.len()));
    request.push_str("connection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .expect("the request is written");
    let _ = stream.write_all(body);
    stream.flush().expect("the request is flushed");
    let mut raw = Vec::new();
    loop {
        let mut chunk = [0_u8; 8192];
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => raw.extend_from_slice(&chunk[..count]),
            // A server that refuses a request from its declared length may
            // close the connection while the client is still writing, which
            // reaches the reader as a reset. The bytes that did arrive are
            // still the answer.
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(error) => panic!("the answer of {name} is read: {error}"),
        }
    }
    if raw.is_empty() {
        return Reply {
            status: 0,
            headers: BTreeMap::new(),
            body: Vec::new(),
        };
    }
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
        let reply = send(case.name, port, case.method, &path, &headers, &case.body);
        if let Ok(document) = serde_json::from_slice::<Value>(&reply.body) {
            if let Some(found) = document.get("request_id").and_then(Value::as_str) {
                request_id = found.to_owned();
            }
        }
        answers.push(reply);
    }
    Transcript { cases: answers }
}

/// The launch facts that differ between the two runs.
struct Launch {
    data_dir: String,
    port: u16,
}

/// Normalize the body of one answer.
fn normalize(body: &[u8], launch: &Launch) -> String {
    normalize_text(&String::from_utf8_lossy(body), launch)
}

/// Normalize one text: the launch facts, the wall clock, the native digests.
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
const PARSER_REFUSALS: [&str; 1] = ["host-repeated"];

/// Compare the answers of the two launches.
fn compare_answers(
    cases: &[Case],
    python: &Transcript,
    native: &Transcript,
    launches: [&Launch; 2],
) {
    let [python_launch, native_launch] = launches;
    assert_eq!(
        python.cases.len(),
        native.cases.len(),
        "the two runs answered the same number of cases"
    );
    let mut mismatches = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        if PARSER_REFUSALS.contains(&case.name) {
            let expected = python.cases[index].status;
            let actual = native.cases[index].status;
            if expected != 400 || actual != 421 {
                mismatches.push(format!(
                    "{}: the two parsers did not refuse the request (python {expected}, native {actual})",
                    case.name
                ));
            }
            continue;
        }
        let expected = python.cases[index].status;
        let actual = native.cases[index].status;
        if expected != actual {
            mismatches.push(format!(
                "{}: the status differs (python {expected}, native {actual})",
                case.name
            ));
            continue;
        }
        for (name, value) in &python.cases[index].headers {
            let value = normalize_text(value, python_launch);
            match native.cases[index].headers.get(name) {
                Some(actual) if normalize_text(actual, native_launch) == value => {}
                Some(actual) => mismatches.push(format!(
                    "{}: the header {name} differs (python {value}, native {})",
                    case.name,
                    normalize_text(actual, native_launch)
                )),
                None => mismatches.push(format!(
                    "{}: the header {name} is missing (python {value})",
                    case.name
                )),
            }
        }
        let expected_text = normalize(&python.cases[index].body, python_launch);
        let actual_text = normalize(&native.cases[index].body, native_launch);
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
    assert!(
        mismatches.is_empty(),
        "{} mismatches out of {} cases:\n{}",
        mismatches.len(),
        cases.len(),
        mismatches.join("\n")
    );
}

/// Normalize one command's stdout/stderr for a byte comparison.
fn normalize_output(output: &Output, launch: &Launch) -> (String, String) {
    (
        normalize_text(&String::from_utf8_lossy(&output.stdout), launch),
        normalize_text(&String::from_utf8_lossy(&output.stderr), launch),
    )
}

/// Normalize the summary document: the data directory and the port are the
/// facts of the run.
fn normalize_summary(text: &str, launch: &Launch) -> String {
    let mut text = text.replace(&launch.data_dir, "<data-dir>");
    text = text.replace(
        &format!("http://127.0.0.1:{}", launch.port),
        "http://127.0.0.1:<port>",
    );
    text = text.replace(&format!("\"port\": {}", launch.port), "\"port\": <port>");
    text
}

/// Every file under a directory whose bytes contain a needle.
fn files_containing(root: &Path, needle: &[u8]) -> Vec<PathBuf> {
    let mut matches = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(bytes) = fs::read(&path) {
                if bytes.windows(needle.len()).any(|window| window == needle) {
                    matches.push(path);
                }
            }
        }
    }
    matches.sort();
    matches
}

/// Read the tables of a data directory and their row counts.
fn table_counts(data_dir: &Path) -> Vec<(String, i64)> {
    let connection = rusqlite::Connection::open(data_dir.join("kala.sqlite3"))
        .expect("the launch wrote its database");
    let names: Vec<String> = {
        let mut statement = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .expect("the schema is readable");
        statement
            .query_map([], |row| row.get(0))
            .expect("the tables are listed")
            .collect::<Result<_, _>>()
            .expect("the tables are read")
    };
    names
        .into_iter()
        .map(|name| {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM \"{name}\""), [], |row| {
                    row.get(0)
                })
                .expect("the rows are counted");
            (name, count)
        })
        .collect()
}

/// The request-level log lines the Python ASGI server writes and the native
/// transport does not.
///
/// Uvicorn's warning log and Starlette's deprecation warning describe the
/// requests a server received, not the launch contract of the command. The
/// native transport has no request logger, so its stderr stays empty; the
/// Python server's request log is removed before the launch stream is
/// compared. Anything other than these exact lines is still a mismatch.
fn without_python_request_logs(stderr: &str) -> String {
    let mut kept = Vec::new();
    let mut skip_source_line = false;
    for line in stderr.lines() {
        if line == "WARNING:  Invalid HTTP request received." {
            continue;
        }
        if line.contains("StarletteDeprecationWarning") {
            skip_source_line = true;
            continue;
        }
        if skip_source_line && line.starts_with("  ") {
            skip_source_line = false;
            continue;
        }
        skip_source_line = false;
        kept.push(line);
    }
    kept.join("\n").trim().to_owned()
}

#[test]
fn the_native_serve_answers_the_live_python_serve() {
    if !python_available() {
        eprintln!("skipping: the repository interpreter cannot import the live command line");
        return;
    }
    let root = tempfile::tempdir().expect("a temporary directory is free");
    let python_side = Side::new(root.path(), "python");
    let native_side = Side::new(root.path(), "native");
    let python_port = free_port();
    let native_port = free_port();

    // Both launches carry an explicit data directory and an explicit loopback
    // host, which is the shape the operator uses. The root `--data-dir`
    // precedes the subcommand; the port is a string of the launch.
    let python_port_text = python_port.to_string();
    let native_port_text = native_port.to_string();
    let python_child = Running::new(start_argv(
        true,
        &python_side,
        &[
            "--data-dir",
            python_side.data.to_str().expect("text"),
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            &python_port_text,
        ],
    ));
    let native_child = Running::new(start_argv(
        false,
        &native_side,
        &[
            "--data-dir",
            native_side.data.to_str().expect("text"),
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            &native_port_text,
        ],
    ));
    wait_ready(python_port);
    wait_ready(native_port);

    let python_token = read_token(&python_side.data);
    let native_token = read_token(&native_side.data);
    assert_token_file(&python_side, &python_token);
    assert_token_file(&native_side, &native_token);
    assert_ne!(
        python_token, native_token,
        "the two launches mint independently"
    );

    // The operator can read the launch token while the server runs, from both
    // command lines. The token file survives the stop as well (§ below).
    for (python_cli, side, token) in [
        (true, &python_side, &python_token),
        (false, &native_side, &native_token),
    ] {
        let output = run_argv(python_cli, side, &["token"]);
        assert!(
            output.status.success(),
            "the running token command succeeds: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let document: Value = serde_json::from_slice(&output.stdout).expect("the token is JSON");
        assert_eq!(document["data"]["token"], token.as_str());
        assert_eq!(
            document["data"]["path"],
            side.token_path().to_string_lossy().as_ref()
        );
    }

    let cases = script();
    assert_eq!(cases.len(), 96, "the accepted api-routes corpus");
    println!(
        "the live serve differential compared {} cases ({} of them strictly, {} declared parser refusals)",
        cases.len(),
        cases.len() - PARSER_REFUSALS.len(),
        PARSER_REFUSALS.len()
    );
    let python_answers = replay(python_port, &python_token, &cases);
    let native_answers = replay(native_port, &native_token, &cases);
    let python_launch = Launch {
        data_dir: python_side.data.to_string_lossy().into_owned(),
        port: python_port,
    };
    let native_launch = Launch {
        data_dir: native_side.data.to_string_lossy().into_owned(),
        port: native_port,
    };
    compare_answers(
        &cases,
        &python_answers,
        &native_answers,
        [&python_launch, &native_launch],
    );

    // A terminate-free, normal stop is an interrupt: both print the summary.
    signal(python_child.child(), libc::SIGINT);
    signal(native_child.child(), libc::SIGINT);
    let python_output = python_child.finish(60);
    let native_output = native_child.finish(60);

    assert_eq!(python_output.status.code(), Some(0), "{python_output:?}");
    assert_eq!(native_output.status.code(), Some(0), "{native_output:?}");
    assert!(
        without_python_request_logs(&String::from_utf8_lossy(&python_output.stderr)).is_empty(),
        "python stderr: {}",
        String::from_utf8_lossy(&python_output.stderr)
    );
    assert!(
        native_output.stderr.is_empty(),
        "native stderr: {}",
        String::from_utf8_lossy(&native_output.stderr)
    );
    let python_summary = normalize_summary(
        &String::from_utf8_lossy(&python_output.stdout),
        &python_launch,
    );
    let native_summary = normalize_summary(
        &String::from_utf8_lossy(&native_output.stdout),
        &native_launch,
    );
    assert_eq!(python_summary, native_summary, "the shutdown summary");

    // The summary is the Python document, not just an equal one.
    let summary: Value =
        serde_json::from_slice(&native_output.stdout).expect("the summary is JSON");
    assert_eq!(summary["command"], "serve");
    assert_eq!(summary["status"], "stopped");
    assert_eq!(summary["data"]["authentication"], "bearer");
    assert_eq!(summary["data"]["loopback_only"], true);
    assert_eq!(
        summary["data"]["api_token_path"],
        native_side.token_path().to_string_lossy().as_ref()
    );
    assert_eq!(
        summary["data"]["url"],
        format!("http://127.0.0.1:{native_port}")
    );
    assert_eq!(summary["data"]["port"], native_port);
    assert!(native_summary.contains("\"api_token_path\": \"<data-dir>/api-token\""));

    // The launch token is still readable after the launch, by both CLIs.
    for (python_cli, side, token) in [
        (true, &python_side, &python_token),
        (false, &native_side, &native_token),
    ] {
        let output = run_argv(python_cli, side, &["token"]);
        assert!(
            output.status.success(),
            "the token command succeeds: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let document: Value = serde_json::from_slice(&output.stdout).expect("the token is JSON");
        assert_eq!(document["command"], "token");
        assert_eq!(document["status"], "issued");
        assert_eq!(document["data"]["token"], token.as_str());
        assert_eq!(
            document["data"]["path"],
            side.token_path().to_string_lossy().as_ref()
        );
        assert!(
            document["data"]["path"]
                .as_str()
                .expect("the path is text")
                .ends_with("api-token")
        );
    }

    // The two launches left the same shape of local state.
    assert_eq!(
        table_counts(&python_side.data),
        table_counts(&native_side.data),
        "the two launches wrote the same tables and row counts"
    );

    // The launch token is stored only in the token file, on both sides.
    for (side, token) in [(&python_side, &python_token), (&native_side, &native_token)] {
        assert_eq!(
            files_containing(&side.data, token.as_bytes()),
            vec![side.token_path()],
            "the token appears only in the token file"
        );
    }
}

#[test]
fn the_native_serve_stops_on_sigterm_the_way_python_does() {
    if !python_available() {
        eprintln!("skipping: the repository interpreter cannot import the live command line");
        return;
    }
    let root = tempfile::tempdir().expect("a temporary directory is free");
    let python_side = Side::new(root.path(), "python");
    let native_side = Side::new(root.path(), "native");
    let python_port = free_port();
    let native_port = free_port();
    let python_child = Running::new(start(
        true,
        &python_side,
        &["--port", &python_port.to_string()],
    ));
    let native_child = Running::new(start(
        false,
        &native_side,
        &["--port", &native_port.to_string()],
    ));
    wait_ready(python_port);
    wait_ready(native_port);

    signal(python_child.child(), libc::SIGTERM);
    signal(native_child.child(), libc::SIGTERM);
    let python_output = python_child.finish(60);
    let native_output = native_child.finish(60);

    assert_eq!(
        python_output.status.signal(),
        Some(libc::SIGTERM),
        "the Python launch is terminated by SIGTERM"
    );
    assert_eq!(
        native_output.status.signal(),
        Some(libc::SIGTERM),
        "the native launch is terminated by SIGTERM"
    );
    assert_eq!(python_output.status.code(), None);
    assert_eq!(native_output.status.code(), None);
    assert!(
        python_output.stdout.is_empty(),
        "no summary before termination"
    );
    assert!(
        native_output.stdout.is_empty(),
        "no summary before termination"
    );
    assert!(
        python_output.stderr.is_empty(),
        "the Python launch is quiet"
    );
    assert!(
        native_output.stderr.is_empty(),
        "the native launch is quiet"
    );
    assert_token_file(&python_side, &read_token(&python_side.data));
    assert_token_file(&native_side, &read_token(&native_side.data));
}

/// The launches that fail before a socket exists.
#[test]
fn the_native_serve_refuses_the_same_launches_as_python() {
    if !python_available() {
        eprintln!("skipping: the repository interpreter cannot import the live command line");
        return;
    }
    let cases: [&[&str]; 18] = [
        &["--host", "0.0.0.0"],
        &["--host", "::"],
        &["--host", "192.168.1.10"],
        &["--host", "example.com"],
        &["--host", "  "],
        &["--host", "0.0.0.0", "--port", "8931"],
        &["--port", "0"],
        &["--port", "-1"],
        &["--port", "65536"],
        &["--port", "1000000"],
        &["--port", "999999999999999999999999"],
        &["--port=0"],
        &["--host=0.0.0.0"],
        &["--port", "x"],
        &["--port"],
        &["--host"],
        &["--nonsense"],
        &["--h", "127.0.0.1"],
    ];
    let root = tempfile::tempdir().expect("a temporary directory is free");
    let mut mismatches = Vec::new();
    for (index, arguments) in cases.iter().enumerate() {
        let python_side = Side::new(&root.path().join(format!("case-{index}")), "python");
        let native_side = Side::new(&root.path().join(format!("case-{index}")), "native");
        let python_output = run(true, &python_side, arguments);
        let native_output = run(false, &native_side, arguments);
        let python_launch = Launch {
            data_dir: python_side.data.to_string_lossy().into_owned(),
            port: 0,
        };
        let native_launch = Launch {
            data_dir: native_side.data.to_string_lossy().into_owned(),
            port: 0,
        };
        let (python_stdout, python_stderr) = normalize_output(&python_output, &python_launch);
        let (native_stdout, native_stderr) = normalize_output(&native_output, &native_launch);
        if python_output.status.code() != native_output.status.code()
            || python_stdout != native_stdout
            || python_stderr != native_stderr
        {
            mismatches.push(format!(
                "{arguments:?}: python exit {:?}, native exit {:?}\n--- python stdout ---\n{python_stdout}\n--- native stdout ---\n{native_stdout}\n--- python stderr ---\n{python_stderr}\n--- native stderr ---\n{native_stderr}",
                python_output.status.code(),
                native_output.status.code(),
            ));
        }
        if python_side.data.exists() != native_side.data.exists() {
            mismatches.push(format!(
                "{arguments:?}: the data-directory side effect differs"
            ));
        }
        if python_side.token_path().exists() != native_side.token_path().exists() {
            mismatches.push(format!("{arguments:?}: the token side effect differs"));
        }
        if python_side.token_path().exists() || native_side.token_path().exists() {
            mismatches.push(format!("{arguments:?}: a refused launch minted a token"));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// A occupied port: the Python server logs the failure and exits `3`.
#[test]
fn the_native_serve_reports_a_failed_bind_like_python() {
    if !python_available() {
        eprintln!("skipping: the repository interpreter cannot import the live command line");
        return;
    }
    let root = tempfile::tempdir().expect("a temporary directory is free");
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port is held");
    let port = listener.local_addr().expect("the address is known").port();
    let python_side = Side::new(root.path(), "python");
    let native_side = Side::new(root.path(), "native");
    let python_output = run(true, &python_side, &["--port", &port.to_string()]);
    let native_output = run(false, &native_side, &["--port", &port.to_string()]);
    drop(listener);

    assert_eq!(python_output.status.code(), Some(3), "{python_output:?}");
    assert_eq!(native_output.status.code(), Some(3), "{native_output:?}");
    assert!(python_output.stdout.is_empty());
    assert!(native_output.stdout.is_empty());
    let python_line = String::from_utf8_lossy(&python_output.stderr);
    let native_line = String::from_utf8_lossy(&native_output.stderr);
    assert_eq!(native_line, python_line, "the uvicorn failure line");
    assert!(python_line.contains("ERROR:    [Errno 98]"));
    assert!(python_line.contains(&format!("('127.0.0.1', {port})")));
    assert!(python_line.contains("[errno 98] address already in use"));
    assert!(
        python_side.token_path().exists(),
        "the token is minted first"
    );
    assert!(
        native_side.token_path().exists(),
        "the token is minted first"
    );

    // The same failure on the IPv6 loopback, when it is available.
    if let Ok(held) = TcpListener::bind("[::1]:0") {
        let port = held.local_addr().expect("the address is known").port();
        let python_side = Side::new(root.path(), "python-v6");
        let native_side = Side::new(root.path(), "native-v6");
        let python_output = run(
            true,
            &python_side,
            &["--host", "::1", "--port", &port.to_string()],
        );
        let native_output = run(
            false,
            &native_side,
            &["--host", "::1", "--port", &port.to_string()],
        );
        drop(held);
        assert_eq!(native_output.status.code(), python_output.status.code());
        assert_eq!(
            String::from_utf8_lossy(&native_output.stderr),
            String::from_utf8_lossy(&python_output.stderr)
        );
    }

    // A privileged port that this user cannot bind. If the user is root the
    // bind would succeed and the launches would serve, so the case is skipped.
    if let Err(error) = TcpListener::bind("127.0.0.1:80") {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            let python_side = Side::new(root.path(), "python-privileged");
            let native_side = Side::new(root.path(), "native-privileged");
            let python_output = run(true, &python_side, &["--port", "80"]);
            let native_output = run(false, &native_side, &["--port", "80"]);
            assert_eq!(native_output.status.code(), python_output.status.code());
            assert_eq!(
                String::from_utf8_lossy(&native_output.stderr),
                String::from_utf8_lossy(&python_output.stderr)
            );
            assert!(String::from_utf8_lossy(&native_output.stderr).contains("[Errno 13]"));
        }
    }
}

/// A token that cannot be stored still guards the process, on both sides.
#[test]
fn the_native_serve_keeps_an_unstorable_token_like_python() {
    if !python_available() {
        eprintln!("skipping: the repository interpreter cannot import the live command line");
        return;
    }
    let root = tempfile::tempdir().expect("a temporary directory is free");
    let python_side = Side::new(root.path(), "python");
    let native_side = Side::new(root.path(), "native");
    for side in [&python_side, &native_side] {
        fs::create_dir_all(&side.data).expect("the data directory is created");
        fs::create_dir(side.data.join("api-token")).expect("the token path is a directory");
    }
    let python_port = free_port();
    let native_port = free_port();
    let python_child = Running::new(start(
        true,
        &python_side,
        &["--port", &python_port.to_string()],
    ));
    let native_child = Running::new(start(
        false,
        &native_side,
        &["--port", &native_port.to_string()],
    ));
    wait_ready(python_port);
    wait_ready(native_port);

    // The token is held in memory only, so a write is refused anonymously.
    for port in [python_port, native_port] {
        let response = send(
            "unstored-token-write",
            port,
            "POST",
            "/api/v1/revisions",
            &[
                ("Host".to_owned(), format!("127.0.0.1:{port}")),
                ("Content-Type".to_owned(), "application/json".to_owned()),
            ],
            b"{}",
        );
        assert_eq!(response.status, 401);
    }

    signal(python_child.child(), libc::SIGINT);
    signal(native_child.child(), libc::SIGINT);
    let python_output = python_child.finish(60);
    let native_output = native_child.finish(60);
    assert_eq!(python_output.status.code(), Some(0));
    assert_eq!(native_output.status.code(), Some(0));
    let warning =
        "the launch token was not stored in the local data directory: IsADirectoryError\n";
    assert_eq!(String::from_utf8_lossy(&python_output.stderr), warning);
    assert_eq!(String::from_utf8_lossy(&native_output.stderr), warning);
    let python_launch = Launch {
        data_dir: python_side.data.to_string_lossy().into_owned(),
        port: python_port,
    };
    let native_launch = Launch {
        data_dir: native_side.data.to_string_lossy().into_owned(),
        port: native_port,
    };
    assert_eq!(
        normalize_summary(
            &String::from_utf8_lossy(&python_output.stdout),
            &python_launch
        ),
        normalize_summary(
            &String::from_utf8_lossy(&native_output.stdout),
            &native_launch
        )
    );
}

/// The IPv6 loopback, when this machine has it.
#[test]
fn the_native_serve_binds_the_ipv6_loopback_like_python() {
    if !python_available() {
        eprintln!("skipping: the repository interpreter cannot import the live command line");
        return;
    }
    if TcpListener::bind("[::1]:0").is_err() {
        eprintln!("skipping: the IPv6 loopback interface is not available");
        return;
    }
    let root = tempfile::tempdir().expect("a temporary directory is free");
    let python_side = Side::new(root.path(), "python");
    let native_side = Side::new(root.path(), "native");
    let python_port = free_port_v6();
    let native_port = free_port_v6();
    let python_child = Running::new(start(
        true,
        &python_side,
        &["--host", "::1", "--port", &python_port.to_string()],
    ));
    let native_child = Running::new(start(
        false,
        &native_side,
        &["--host", "::1", "--port", &native_port.to_string()],
    ));
    wait_ready_v6(python_port);
    wait_ready_v6(native_port);

    for port in [python_port, native_port] {
        let response = send_on(
            "ipv6-health",
            "::1",
            port,
            "GET",
            "/health",
            &[("Host".to_owned(), format!("[::1]:{port}"))],
            b"",
        );
        assert_eq!(response.status, 200);
    }

    signal(python_child.child(), libc::SIGINT);
    signal(native_child.child(), libc::SIGINT);
    let python_output = python_child.finish(60);
    let native_output = native_child.finish(60);
    assert_eq!(python_output.status.code(), Some(0));
    assert_eq!(native_output.status.code(), Some(0));
    let python_summary = String::from_utf8_lossy(&python_output.stdout).replace(
        &format!("http://[::1]:{python_port}"),
        "http://[::1]:<port>",
    );
    let native_summary = String::from_utf8_lossy(&native_output.stdout).replace(
        &format!("http://[::1]:{native_port}"),
        "http://[::1]:<port>",
    );
    let python_summary = python_summary
        .replace(
            &python_side.data.to_string_lossy().into_owned(),
            "<data-dir>",
        )
        .replace(&format!("\"port\": {python_port}"), "\"port\": <port>");
    let native_summary = native_summary
        .replace(
            &native_side.data.to_string_lossy().into_owned(),
            "<data-dir>",
        )
        .replace(&format!("\"port\": {native_port}"), "\"port\": <port>");
    assert_eq!(python_summary, native_summary);
    assert!(native_summary.contains("\"host\": \"::1\""));
}
