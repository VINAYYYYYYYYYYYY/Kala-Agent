//! Real `kala-studio` process launch contract: banner, readiness, token
//! sourcing, static UI, loopback validation, exit codes and SIGTERM shutdown.

mod common;

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use common::{RequestSpec, TOKEN};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_kala-studio")
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

async fn read_banner(child: &mut Child) -> serde_json::Value {
    let stdout = child.stdout.take().expect("piped stdout");
    let mut lines = BufReader::new(stdout).lines();
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        while let Ok(Some(line)) = lines.next_line().await {
            text.push_str(&line);
            text.push('\n');
            if line.trim() == "}" {
                return;
            }
        }
    })
    .await
    .expect("banner before 15s");
    serde_json::from_str(&text).expect("banner JSON")
}

async fn wait_ready(port: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if tokio::time::Instant::now() > deadline {
            panic!("health did not become ready within 15s");
        }
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn terminate(child: &mut Child) {
    let pid = child.id().expect("child pid");
    let status = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .unwrap();
    assert!(status.success());
    let exit = tokio::time::timeout(Duration::from_secs(15), child.wait())
        .await
        .expect("graceful shutdown within Electron's 15s readiness window")
        .unwrap();
    assert!(exit.success(), "exit status {exit:?}");
}

/// The geometry worker command the launch tests use.
///
/// The command never starts, so the geometry routes answer 503 while the
/// native routes keep working. That is the ADR-0010 contract for a studio
/// whose worker cannot start.
const ABSENT_WORKER: &str = "/nonexistent/kala-studio-geometry-worker";

fn write_static(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("index.html"), "<html>cli-spa</html>").unwrap();
    std::fs::write(root.join("asset.txt"), "cli-asset\n").unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launches_serves_and_shuts_down_without_python() {
    let work = tempfile::tempdir().unwrap();
    let data_dir = work.path().join("data");
    let static_dir = work.path().join("static");
    write_static(&static_dir);
    let port = free_port();
    let mut child = Command::new(binary())
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--static-dist")
        .arg(&static_dir)
        .env_clear()
        .env("PATH", "")
        .env("KALA_STUDIO_TOKEN", TOKEN)
        .env("KALA_STUDIO_GEOMETRY_WORKER", ABSENT_WORKER)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kala-studio");
    let banner = read_banner(&mut child).await;
    assert_eq!(banner["host"], "127.0.0.1");
    assert_eq!(banner["port"].as_u64(), Some(u64::from(port)));
    assert_eq!(banner["loopback"], true);
    assert_eq!(banner["url"], format!("http://127.0.0.1:{port}"));
    assert_eq!(banner["token_environment_variable"], "KALA_STUDIO_TOKEN");
    assert_eq!(
        banner["static_dist_environment_variable"],
        "KALA_STUDIO_DIST"
    );
    assert_eq!(
        banner["data_dir"].as_str(),
        Some(data_dir.display().to_string().as_str())
    );
    wait_ready(port).await;

    let address = format!("127.0.0.1:{port}").parse().unwrap();
    let health =
        common::request(address, "GET", "/api/studio/health", RequestSpec::default()).await;
    assert_eq!(health.status, 200);
    assert_eq!(health.json()["service"], "kala-studio");
    // The health route publishes the worker state, so a studio without a
    // worker says so instead of pretending the geometry is there.
    let worker_state = health.json()["geometry_worker"]["state"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        ["starting", "ready", "restarting", "stopped", "failed"].contains(&worker_state.as_str()),
        "unexpected geometry worker state: {worker_state}"
    );

    let mut preview = RequestSpec::default();
    preview
        .headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    preview
        .headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    preview.body = Some(br#"{"kind":"box"}"#.to_vec());
    let response = common::request(address, "POST", "/api/studio/preview", preview).await;
    // The geometry route is answered by the worker. Without one it is a 503
    // with a clear message, never a fabricated mesh.
    assert_eq!(response.status, 503);
    assert_eq!(response.json()["code"], "geometry_worker_unavailable");

    let index = common::request(address, "GET", "/", RequestSpec::default()).await;
    assert_eq!(index.status, 200);
    assert_eq!(index.body_text(), "<html>cli-spa</html>");
    let asset = common::request(address, "GET", "/asset.txt", RequestSpec::default()).await;
    assert_eq!(asset.body_text(), "cli-asset\n");

    assert!(!data_dir.join("studio-token").exists());
    terminate(&mut child).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mints_and_uses_the_token_file_when_the_environment_is_absent() {
    let work = tempfile::tempdir().unwrap();
    let data_dir = work.path().join("data");
    let port = free_port();
    let mut child = Command::new(binary())
        .arg("--port")
        .arg(port.to_string())
        .arg("--data-dir")
        .arg(&data_dir)
        .env_clear()
        .env("PATH", "")
        .env("KALA_STUDIO_GEOMETRY_WORKER", ABSENT_WORKER)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kala-studio");
    let _banner = read_banner(&mut child).await;
    wait_ready(port).await;
    let token = std::fs::read_to_string(data_dir.join("studio-token")).unwrap();
    let token = token.trim();
    assert_eq!(token.len(), 43);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(data_dir.join("studio-token"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let address = format!("127.0.0.1:{port}").parse().unwrap();
    let mut preview = RequestSpec::default();
    preview
        .headers
        .push(("Authorization".to_owned(), format!("Bearer {token}")));
    preview
        .headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    preview.body = Some(br#"{"kind":"box"}"#.to_vec());
    let response = common::request(address, "POST", "/api/studio/preview", preview).await;
    assert_eq!(response.status, 503);
    assert_eq!(response.json()["code"], "geometry_worker_unavailable");
    terminate(&mut child).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_data_dir_flag_wins_over_every_environment_row() {
    // `--data-dir` is used as-is; KALA_STUDIO_DATA_DIR, KALA_AGENT_DATA_DIR,
    // XDG_DATA_HOME and HOME all point somewhere else and must be ignored.
    let work = tempfile::tempdir().unwrap();
    let flag_dir = work.path().join("flag-data");
    let studio_env = work.path().join("studio-env-data");
    let agent_env = work.path().join("agent-env-data");
    let xdg = work.path().join("xdg-data");
    let home = work.path().join("home");
    for directory in [&home, &xdg, &agent_env] {
        std::fs::create_dir_all(directory).unwrap();
    }
    let port = free_port();
    let mut child = Command::new(binary())
        .arg("--port")
        .arg(port.to_string())
        .arg("--data-dir")
        .arg(&flag_dir)
        .env_clear()
        .env("PATH", "")
        .env("HOME", &home)
        .env("XDG_DATA_HOME", &xdg)
        .env("KALA_AGENT_DATA_DIR", &agent_env)
        .env("KALA_STUDIO_DATA_DIR", &studio_env)
        .env("KALA_STUDIO_TOKEN", TOKEN)
        .env("KALA_STUDIO_GEOMETRY_WORKER", ABSENT_WORKER)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kala-studio");
    let banner = read_banner(&mut child).await;
    assert_eq!(
        banner["data_dir"].as_str(),
        Some(flag_dir.display().to_string().as_str())
    );
    wait_ready(port).await;
    // The real home and the ignored environment directories are untouched.
    assert!(!home.join(".local").exists());
    assert!(!studio_env.exists());
    assert!(!xdg.join("kala-agent").exists());
    assert!(!agent_env.join("studio").exists());
    assert!(flag_dir.is_dir());
    terminate(&mut child).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kala_studio_data_dir_is_the_studio_directory_itself() {
    // The native service honours KALA_STUDIO_DATA_DIR (B2): the value *is*
    // the studio directory, so no `studio` child is appended, and a temporary
    // HOME still is never read or written.
    let work = tempfile::tempdir().unwrap();
    let studio_env = work.path().join("studio-env-data");
    let home = work.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let port = free_port();
    let mut child = Command::new(binary())
        .arg("--port")
        .arg(port.to_string())
        .env_clear()
        .env("PATH", "")
        .env("HOME", &home)
        .env("KALA_STUDIO_DATA_DIR", &studio_env)
        .env("KALA_STUDIO_GEOMETRY_WORKER", ABSENT_WORKER)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kala-studio");
    let banner = read_banner(&mut child).await;
    assert_eq!(
        banner["data_dir"].as_str(),
        Some(studio_env.display().to_string().as_str())
    );
    wait_ready(port).await;
    assert!(studio_env.join("studio-token").is_file());
    assert!(!home.join(".local").exists());
    terminate(&mut child).await;
}

#[test]
fn refuses_non_loopback_hosts_and_bad_ports_with_exit_2() {
    for (args, needle) in [
        (
            vec!["--host", "0.0.0.0", "--port", "8765"],
            "loopback interfaces only",
        ),
        (
            vec!["--host", "192.168.1.5", "--port", "8765"],
            "loopback interfaces only",
        ),
        (
            vec!["--host", "127.0.0.1", "--port", "0"],
            "port must be between 1 and 65535",
        ),
        (
            vec!["--host", "127.0.0.1", "--port", "70000"],
            "port must be between 1 and 65535",
        ),
    ] {
        let output = std::process::Command::new(binary())
            .args(args)
            .env_clear()
            .env("PATH", "")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(needle), "stderr: {stderr}");
    }
}

#[test]
fn rejects_an_invalid_token_with_exit_1_without_echoing_it() {
    let work = tempfile::tempdir().unwrap();
    let bad = "bad token with spaces and enough length";
    let output = std::process::Command::new(binary())
        .arg("--data-dir")
        .arg(work.path().join("data"))
        .env_clear()
        .env("PATH", "")
        .env("KALA_STUDIO_TOKEN", bad)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unsupported characters"));
    assert!(!stderr.contains(bad));
}

#[test]
fn rejects_an_invalid_step_import_limit_with_exit_2() {
    let work = tempfile::tempdir().unwrap();
    for (value, needle) in [
        (
            "abc",
            "KALA_STEP_IMPORT_MAX_BYTES must be an integer number of bytes",
        ),
        (
            "0",
            "the STEP import limit must be between 1 and 536870912 bytes",
        ),
        (
            "536870913",
            "the STEP import limit must be between 1 and 536870912 bytes",
        ),
    ] {
        let output = std::process::Command::new(binary())
            .arg("--data-dir")
            .arg(work.path().join("data"))
            .env_clear()
            .env("PATH", "")
            .env("KALA_STEP_IMPORT_MAX_BYTES", value)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{value:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(needle), "{value:?}: {stderr}");
    }
}

#[test]
fn help_is_available_with_exit_0() {
    let output = std::process::Command::new(binary())
        .arg("--help")
        .env_clear()
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    for flag in ["--host", "--port", "--data-dir", "--static-dist"] {
        assert!(stdout.contains(flag), "help is missing {flag}");
    }
}
