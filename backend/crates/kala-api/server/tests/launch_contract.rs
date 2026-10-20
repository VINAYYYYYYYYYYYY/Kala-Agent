//! The launch contract of the native server.
//!
//! The binary is the native counterpart of the `serve` command of the Python
//! command line adapter. These cases are the contract: the flags, the
//! environment, the exit codes, the token that is written to the local data
//! directory and the summary the launch prints when it stops.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The binary of the launch.
fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_kala-api-server")
}

/// Run the binary with the given arguments and return its exit code and its
/// two streams.
fn run(arguments: &[&str]) -> (i32, String, String) {
    let output = Command::new(binary())
        .args(arguments)
        .output()
        .expect("the binary runs");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn the_usage_is_printed_on_request() {
    let (code, stdout, stderr) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("usage: kala-api-server"));
    assert!(stderr.is_empty());
}

#[test]
fn an_unknown_argument_is_a_usage_error() {
    let (code, _stdout, stderr) = run(&["--nonsense"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("usage: kala-api-server"));
    assert!(stderr.contains("unrecognized arguments: --nonsense"));
}

#[test]
fn an_argument_without_its_value_is_a_usage_error() {
    let (code, _stdout, stderr) = run(&["--port"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("expected one argument"));
}

#[test]
fn a_port_that_is_not_a_number_is_a_usage_error() {
    let (code, _stdout, stderr) = run(&["--port", "eight"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("invalid int value: 'eight'"));
}

#[test]
fn an_address_that_is_not_loopback_is_refused_before_a_socket_exists() {
    let temporary = tempfile::tempdir().expect("a temporary directory is free");
    let data_dir = temporary.path().join("data");
    let (code, _stdout, stderr) = run(&[
        "--data-dir",
        data_dir.to_str().expect("the path is text"),
        "--host",
        "0.0.0.0",
    ]);
    assert_eq!(code, 1);
    assert!(stderr.contains("invalid_address"));
    assert!(stderr.contains("serve binds the loopback interface only"));
    assert!(
        !data_dir.exists(),
        "the launch refused the address before it wrote"
    );
}

#[test]
fn a_port_outside_the_accepted_range_is_refused() {
    let temporary = tempfile::tempdir().expect("a temporary directory is free");
    let data_dir = temporary.path().join("data");
    let (code, _stdout, stderr) = run(&[
        "--data-dir",
        data_dir.to_str().expect("the path is text"),
        "--port",
        "70000",
    ]);
    assert_eq!(code, 1);
    assert!(stderr.contains("the port must be between 1 and 65535"));
}

#[test]
fn a_launch_writes_its_token_and_prints_its_summary() {
    let temporary = tempfile::tempdir().expect("a temporary directory is free");
    let data_dir = temporary.path().join("data");
    fs::create_dir_all(&data_dir).expect("the data directory is created");
    let port = free_port();
    let child = Command::new(binary())
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--port")
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the launch starts");

    // The token is written before the socket exists.
    let token_path = data_dir.join("api-token");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !token_path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(token_path.exists(), "the launch wrote its token");
    let token = fs::read_to_string(&token_path).expect("the token is read");
    assert!(token.len() >= 16, "the token carries its entropy");

    // The token file is owner only.
    let mode = fs::metadata(&token_path)
        .expect("the token file is known")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "the token file is owner only");

    // The launch serves the loopback interface.
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut served = false;
    while Instant::now() < deadline && !served {
        served = std::net::TcpStream::connect(("127.0.0.1", port)).is_ok();
        if !served {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    assert!(served, "the launch serves the loopback interface");

    // The launch stops on the terminate signal and prints its summary.
    terminate(child.id() as i32);
    let output = child.wait_with_output().expect("the launch stops");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(stdout.contains("\"command\": \"serve\""), "{stdout}");
    assert!(stdout.contains("\"status\": \"stopped\""), "{stdout}");
    assert!(
        stdout.contains("\"authentication\": \"bearer\""),
        "{stdout}"
    );
    assert!(stdout.contains("\"loopback_only\": true"), "{stdout}");
    assert!(stdout.contains(&format!("\"port\": {port}")), "{stdout}");
    assert!(
        stdout.contains(&format!("\"url\": \"http://127.0.0.1:{port}\"")),
        "{stdout}"
    );
}

/// A port nothing listens on.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port is free");
    let port = listener.local_addr().expect("the address is known").port();
    drop(listener);
    port
}

/// Ask a process to stop.
fn terminate(pid: i32) {
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
}

/// The data directory of the launch is the one the caller names.
#[test]
fn the_data_directory_of_the_launch_is_the_one_the_caller_names() {
    let temporary = tempfile::tempdir().expect("a temporary directory is free");
    let data_dir = temporary.path().join("named");
    let port = free_port();
    let mut child = Command::new(binary())
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--port")
        .arg(port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the launch starts");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !data_dir.join("api-token").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        data_dir.join("api-token").exists(),
        "the launch wrote its token in the named directory"
    );
    terminate(child.id() as i32);
    let _ = child.wait();
    let _ = Path::new("/");
}
