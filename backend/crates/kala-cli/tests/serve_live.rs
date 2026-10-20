//! The live launch contract of `kala-agent serve`, without Python.
//!
//! These cases start the real binary on temporary data directories: the token
//! file and its owner-only mode, the loopback health answer, the token gate on
//! a write, the summary of an interrupt, the signal termination of a terminate
//! request, a failed bind, a refused address and an unstorable token. The live
//! differential against the Python command line is in `serve_differential.rs`.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// The native binary.
fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kala-agent"))
}

/// The environment of a launch: nothing of the caller's, and temporary paths.
fn command(data_dir: &Path, home: &Path, extra: &[&str]) -> Command {
    let mut command = Command::new(binary());
    command
        .args(extra)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LC_ALL", "C.UTF-8")
        .env("LANG", "C.UTF-8")
        .env("TZ", "UTC")
        .env("KALA_AGENT_DATA_DIR", data_dir)
        .env("KALA_AGENT_MODEL_PROVIDER", "local")
        .env("KALA_AGENT_MODEL_ID", "local");
    command
}

/// Start one serve process.
fn start(data_dir: &Path, home: &Path, port: u16) -> Child {
    command(
        data_dir,
        home,
        &[
            "--data-dir",
            data_dir.to_str().expect("text"),
            "serve",
            "--port",
            &port.to_string(),
        ],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("the launch starts")
}

/// Run one command line and collect its output.
fn run(data_dir: &Path, home: &Path, arguments: &[&str]) -> Output {
    command(data_dir, home, arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("the command line runs")
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

/// A port nothing listens on.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port is free");
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

/// Send one request and read the status and body.
fn request(
    port: u16,
    method: &str,
    path: &str,
    body: &[u8],
    token: Option<&str>,
) -> (u16, Vec<u8>) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("the server accepts");
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\ncontent-length: {}\r\nconnection: close\r\n",
        body.len()
    );
    if let Some(token) = token {
        request.push_str(&format!("authorization: Bearer {token}\r\n"));
    }
    if method == "POST" {
        request.push_str("content-type: application/json\r\n");
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .expect("the request is written");
    stream.write_all(body).expect("the body is written");
    stream.flush().expect("the request is flushed");
    let mut raw = Vec::new();
    loop {
        let mut chunk = [0_u8; 8192];
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => raw.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(error) => panic!("the answer is read: {error}"),
        }
    }
    let separator = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("the answer carries a header block");
    let head = String::from_utf8_lossy(&raw[..separator]);
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .expect("the status line carries a status");
    (status, raw[separator + 4..].to_vec())
}

#[test]
fn serve_starts_the_native_api_and_prints_the_summary_on_interrupt() {
    let temporary = tempfile::tempdir().expect("a temporary directory is free");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let port = free_port();
    let child = Running::new(start(&data, &home, port));
    wait_ready(port);

    let token = read_token(&data);
    assert_eq!(token.len(), 43);
    let mode = fs::metadata(data.join("api-token"))
        .expect("the token file is known")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "the token file is owner only");

    let (status, body) = request(port, "GET", "/health", b"", None);
    assert_eq!(status, 200);
    assert!(
        String::from_utf8_lossy(&body).starts_with("{\"status\":\"ok\""),
        "{}",
        String::from_utf8_lossy(&body)
    );

    let (status, _) = request(port, "POST", "/api/v1/revisions", b"{}", None);
    assert_eq!(status, 401, "a write without the launch token is refused");
    let (status, _) = request(port, "POST", "/api/v1/revisions", b"{}", Some(&token));
    assert_eq!(status, 422, "the token reaches the route validation");

    // The operator reads the launch token with the `token` command.
    let output = run(
        &data,
        &home,
        &["--data-dir", data.to_str().expect("text"), "token"],
    );
    assert_eq!(output.status.code(), Some(0));
    let document: Value = serde_json::from_slice(&output.stdout).expect("the token is JSON");
    assert_eq!(document["data"]["token"], token.as_str());

    let result = unsafe { libc::kill(child.child().id() as i32, libc::SIGINT) };
    assert_eq!(result, 0);
    let output = child.finish(60);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let summary: Value = serde_json::from_slice(&output.stdout).expect("the summary is JSON");
    assert_eq!(summary["command"], "serve");
    assert_eq!(summary["status"], "stopped");
    assert_eq!(summary["data"]["host"], "127.0.0.1");
    assert_eq!(summary["data"]["port"], port);
    assert_eq!(summary["data"]["loopback_only"], true);
    assert_eq!(summary["data"]["authentication"], "bearer");
    assert_eq!(
        summary["data"]["api_token_path"],
        data.join("api-token").to_string_lossy().as_ref()
    );
    assert_eq!(summary["data"]["url"], format!("http://127.0.0.1:{port}"));
}

#[test]
fn serve_stops_on_a_terminate_signal_by_that_signal() {
    let temporary = tempfile::tempdir().expect("a temporary directory is free");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let port = free_port();
    let child = Running::new(start(&data, &home, port));
    wait_ready(port);
    let result = unsafe { libc::kill(child.child().id() as i32, libc::SIGTERM) };
    assert_eq!(result, 0);
    let output = child.finish(60);
    assert_eq!(
        output.status.signal(),
        Some(libc::SIGTERM),
        "the process is terminated by the signal"
    );
    assert_eq!(output.status.code(), None);
    assert!(output.stdout.is_empty(), "no summary before termination");
    assert!(output.stderr.is_empty(), "the launch is quiet");
}

#[test]
fn serve_reports_a_failed_bind_in_the_uvicorn_layout() {
    let temporary = tempfile::tempdir().expect("a temporary directory is free");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port is held");
    let port = listener.local_addr().expect("the address is known").port();
    let output = run(
        &data,
        &home,
        &[
            "--data-dir",
            data.to_str().expect("text"),
            "serve",
            "--port",
            &port.to_string(),
        ],
    );
    drop(listener);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        format!(
            "ERROR:    [Errno 98] error while attempting to bind on address ('127.0.0.1', {port}): [errno 98] address already in use\n"
        )
    );
    // The token was minted before the socket was attempted.
    assert_eq!(read_token(&data).len(), 43);
}

#[test]
fn serve_refuses_a_non_loopback_address_before_the_token() {
    let temporary = tempfile::tempdir().expect("a temporary directory is free");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&home).expect("home");
    for (arguments, message) in [
        (["serve", "--host", "0.0.0.0"], "loopback interface only"),
        (["serve", "--host", "::"], "loopback interface only"),
        (
            ["serve", "--host", "192.168.1.10"],
            "loopback interface only",
        ),
        (
            ["serve", "--host", "example.com"],
            "loopback interface only",
        ),
        (
            ["serve", "--port", "0"],
            "the port must be between 1 and 65535",
        ),
        (
            ["serve", "--port", "65536"],
            "the port must be between 1 and 65535",
        ),
    ] {
        let mut argv = vec!["--data-dir", data.to_str().expect("text")];
        argv.extend_from_slice(&arguments);
        let output = run(&data, &home, &argv);
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !data.join("api-token").exists(),
            "{arguments:?}: the refusal happened before the token"
        );
        // The command line opened the composition root before the command
        // validated the target, exactly as the Python command line does.
        assert!(data.exists(), "{arguments:?}: the root was opened first");
    }
}

#[test]
fn serve_keeps_an_unstorable_token_and_reports_it_by_class() {
    let temporary = tempfile::tempdir().expect("a temporary directory is free");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&data).expect("the data directory is created");
    fs::create_dir(data.join("api-token")).expect("the token path is a directory");
    let port = free_port();
    let child = Running::new(start(&data, &home, port));
    wait_ready(port);

    let (status, _) = request(port, "GET", "/health", b"", None);
    assert_eq!(status, 200);
    let (status, _) = request(port, "POST", "/api/v1/revisions", b"{}", None);
    assert_eq!(status, 401, "the in-memory token still guards the write");

    let result = unsafe { libc::kill(child.child().id() as i32, libc::SIGINT) };
    assert_eq!(result, 0);
    let output = child.finish(60);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "the launch token was not stored in the local data directory: IsADirectoryError\n"
    );
    let summary: Value = serde_json::from_slice(&output.stdout).expect("the summary is JSON");
    assert_eq!(summary["status"], "stopped");
    assert_eq!(
        summary["data"]["api_token_path"],
        data.join("api-token").to_string_lossy().as_ref()
    );
}

#[test]
fn serve_binds_the_ipv6_loopback_when_it_is_available() {
    if TcpListener::bind("[::1]:0").is_err() {
        eprintln!("skipping: the IPv6 loopback interface is not available");
        return;
    }
    let temporary = tempfile::tempdir().expect("a temporary directory is free");
    let data = temporary.path().join("data");
    let home = temporary.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let port = {
        let listener = TcpListener::bind("[::1]:0").expect("a port is free");
        listener.local_addr().expect("the address is known").port()
    };
    let child = Running::new(
        command(
            &data,
            &home,
            &[
                "--data-dir",
                data.to_str().expect("text"),
                "serve",
                "--host",
                "::1",
                "--port",
                &port.to_string(),
            ],
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the launch starts"),
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut ready = false;
    while Instant::now() < deadline && !ready {
        ready = TcpStream::connect(("::1", port)).is_ok();
        if !ready {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    assert!(ready, "the launch on [::1]:{port} did not start");
    let result = unsafe { libc::kill(child.child().id() as i32, libc::SIGINT) };
    assert_eq!(result, 0);
    let output = child.finish(60);
    assert_eq!(output.status.code(), Some(0));
    let summary: Value = serde_json::from_slice(&output.stdout).expect("the summary is JSON");
    assert_eq!(summary["data"]["host"], "::1");
    assert_eq!(summary["data"]["url"], format!("http://[::1]:{port}"));
}
