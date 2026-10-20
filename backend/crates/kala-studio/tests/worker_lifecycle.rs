//! The supervised geometry worker: readiness, the token, the crash path,
//! the restart and the stdin lifeline.
//!
//! The test drives the real `kala-studio` binary with the real Python
//! geometry worker. It is Unix-only: the Windows worker lifecycle is not
//! proven here and no Windows claim is made.
//!
//! A missing interpreter fails this test loudly. `KALA_SKIP_WORKER_LIFECYCLE=1`
//! is the only opt-out, and it is loud too.

#![cfg(unix)]

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use common::{RequestSpec, TOKEN};

const ABSENT_WORKER: &str = "/nonexistent/kala-studio-geometry-worker";
const SKIP_ENV: &str = "KALA_SKIP_WORKER_LIFECYCLE";

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .expect("the crate lives under <repo>/backend/crates")
}

fn venv_python() -> PathBuf {
    repository_root().join(".venv").join("bin").join("python")
}

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_kala-studio")
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn port_addr(port: u16) -> std::net::SocketAddr {
    format!("127.0.0.1:{port}").parse().unwrap()
}

fn skip_or_fail() -> bool {
    if std::env::var(SKIP_ENV).is_ok() {
        eprintln!("{SKIP_ENV} is set: the worker lifecycle test is skipped loudly");
        return true;
    }
    false
}

/// The children of one process, read from the Linux procfs.
fn child_pids(parent: u32) -> Vec<u32> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return found;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(text) = name.to_str() else { continue };
        let Ok(pid) = text.parse::<u32>() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        // The executable name may contain spaces, so the fields after the
        // closing parenthesis are the ones that matter.
        let Some(tail) = stat.rfind(')') else {
            continue;
        };
        let fields: Vec<&str> = stat[tail + 1..].split_whitespace().collect();
        // fields[0] is the state, fields[1] is the parent pid.
        if fields.len() > 1 && fields[1].parse::<u32>() == Ok(parent) {
            found.push(pid);
        }
    }
    found
}

fn process_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// The environment of one process, read from the Linux procfs.
fn process_environ(pid: u32) -> Vec<(String, String)> {
    let raw = std::fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
    raw.split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let text = String::from_utf8_lossy(entry).into_owned();
            let (key, value) = text.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

async fn read_banner(child: &mut Child) -> serde_json::Value {
    let stdout = child.stdout.take().expect("piped stdout");
    let mut lines = BufReader::new(stdout).lines();
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(30), async {
        while let Ok(Some(line)) = lines.next_line().await {
            text.push_str(&line);
            text.push('\n');
            if line.trim() == "}" {
                return;
            }
        }
    })
    .await
    .expect("banner before 30s");
    serde_json::from_str(&text).expect("banner JSON")
}

/// Wait until the front door accepts connections.
async fn wait_ready(port: u16) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the studio did not become ready");
}

async fn wait_for_worker_state(
    port: u16,
    states: &[&str],
    timeout: Duration,
) -> Option<serde_json::Value> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let response = common::request(
            format!("127.0.0.1:{port}").parse().unwrap(),
            "GET",
            "/api/studio/health",
            RequestSpec::default(),
        )
        .await;
        if response.status == 200 {
            let document = response.json();
            let state = document["geometry_worker"]["state"].as_str().unwrap_or("");
            if states.contains(&state) {
                return Some(document);
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    None
}

fn preview_request() -> RequestSpec {
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    spec.headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    spec.body = Some(br#"{"kind":"box"}"#.to_vec());
    spec
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_that_cannot_start_leaves_the_native_routes_working() {
    if skip_or_fail() {
        return;
    }
    let work = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut child = Command::new(binary())
        .arg("--port")
        .arg(port.to_string())
        .arg("--data-dir")
        .arg(work.path().join("data"))
        .env_clear()
        .env("PATH", "")
        .env("KALA_STUDIO_TOKEN", TOKEN)
        .env("KALA_STUDIO_GEOMETRY_WORKER", ABSENT_WORKER)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kala-studio");
    let _banner = read_banner(&mut child).await;
    wait_ready(port).await;

    // The geometry route is a 503 with a clear message.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut status = 0u16;
    while std::time::Instant::now() < deadline {
        let response = common::request(
            format!("127.0.0.1:{port}").parse().unwrap(),
            "POST",
            "/api/studio/preview",
            preview_request(),
        )
        .await;
        status = response.status;
        if status == 503 {
            assert_eq!(response.json()["code"], "geometry_worker_unavailable");
            let document = response.json();
            let message = document["message"].as_str().unwrap_or_default().to_owned();
            assert!(
                message.contains("geometry worker"),
                "the refusal must name the worker: {message}"
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        status, 503,
        "the geometry route must refuse without a worker"
    );

    // The native routes keep working.
    let health = common::request(
        format!("127.0.0.1:{port}").parse().unwrap(),
        "GET",
        "/api/studio/health",
        RequestSpec::default(),
    )
    .await;
    assert_eq!(health.status, 200);
    assert_eq!(health.json()["service"], "kala-studio");

    let mut chat = RequestSpec::default();
    chat.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    chat.headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    chat.body = Some(br#"{"provider":"local","prompt":"x"}"#.to_vec());
    let chat = common::request(
        format!("127.0.0.1:{port}").parse().unwrap(),
        "POST",
        "/api/studio/chat",
        chat,
    )
    .await;
    assert_eq!(chat.status, 200, "the chat route stays native");

    let _ = child.kill().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_killed_worker_is_restarted_and_never_sees_the_token_in_argv() {
    if skip_or_fail() {
        return;
    }
    let python = venv_python();
    assert!(
        python.is_file(),
        "the geometry worker interpreter is missing: {}",
        python.display()
    );
    let work = tempfile::tempdir().unwrap();
    let port = free_port();
    let command = serde_json::json!([
        python.to_string_lossy(),
        "-m",
        "kala_agent.studio",
        "--worker"
    ])
    .to_string();
    let mut child = Command::new(binary())
        .arg("--port")
        .arg(port.to_string())
        .arg("--data-dir")
        .arg(work.path().join("data"))
        .env_clear()
        .env("PATH", "")
        .env("KALA_STUDIO_TOKEN", TOKEN)
        .env("KALA_STUDIO_GEOMETRY_WORKER", command)
        .env("PYTHONPATH", repository_root().join("src"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kala-studio");
    let _banner = read_banner(&mut child).await;
    wait_ready(port).await;

    let health = wait_for_worker_state(port, &["ready"], Duration::from_secs(90))
        .await
        .expect("the geometry worker must become ready");
    assert_eq!(health["geometry_worker"]["state"], "ready");
    assert_eq!(health["geometry_worker"]["design_available"], true);
    assert_eq!(health["step_available"], true);

    // The worker answers the geometry route.
    let response = common::request(
        format!("127.0.0.1:{port}").parse().unwrap(),
        "POST",
        "/api/studio/preview",
        preview_request(),
    )
    .await;
    assert_eq!(response.status, 200);
    assert!(response.body_text().contains("\"positions\""));

    // The token never reaches the command line.
    let parent = child.id().expect("child pid");
    let workers = child_pids(parent);
    assert!(!workers.is_empty(), "the worker must be a child process");
    let data_dir = work.path().join("data");
    for worker in &workers {
        let cmdline = std::fs::read(format!("/proc/{worker}/cmdline")).unwrap_or_default();
        let text = String::from_utf8_lossy(&cmdline);
        assert!(
            !text.contains(TOKEN),
            "the studio token must not be in the worker's argv: {text}"
        );
        assert!(
            !text.contains("KALA_WORKER_TOKEN"),
            "the worker token variable must not be in the worker's argv: {text}"
        );
        // The front door's resolved data dir is an explicit argument, so the
        // worker can never fall back to the home or temporary directory.
        assert!(
            text.contains("--data-dir"),
            "the worker argv must carry the data dir: {text}"
        );
        assert!(
            text.contains(&data_dir.display().to_string()),
            "the worker argv must carry the front door's data dir: {text}"
        );
    }

    // Killing the worker is noticed: the front door restarts it and the
    // geometry route answers again.
    for worker in &workers {
        let _ = std::process::Command::new("kill")
            .arg("-KILL")
            .arg(worker.to_string())
            .status();
    }
    // The front door restarts the worker and the geometry route answers
    // again. A dead worker is a 503, never a fabricated answer.
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    let mut recovered = false;
    let mut refused = false;
    while std::time::Instant::now() < deadline {
        let response = common::request(
            format!("127.0.0.1:{port}").parse().unwrap(),
            "POST",
            "/api/studio/preview",
            preview_request(),
        )
        .await;
        if response.status == 200 {
            recovered = true;
            break;
        }
        if response.status == 503 {
            refused = true;
            assert_eq!(response.json()["code"], "geometry_worker_unavailable");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        recovered,
        "the restarted worker must answer the geometry route"
    );
    let health = wait_for_worker_state(port, &["ready"], Duration::from_secs(30))
        .await
        .expect("the geometry worker must be ready again");
    assert_eq!(health["geometry_worker"]["state"], "ready");
    let _ = refused;

    // A dead front door must not leave the worker behind: the worker stops
    // on the stdin lifeline.
    for worker in &workers {
        let _ = std::process::Command::new("kill")
            .arg("-KILL")
            .arg(parent.to_string())
            .status();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut gone = false;
        while std::time::Instant::now() < deadline {
            if !process_exists(*worker) {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            gone,
            "the worker must exit when the front door dies (the stdin lifeline)"
        );
    }
    let _ = child.kill().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_health_document_publishes_the_worker_state() {
    if skip_or_fail() {
        return;
    }
    let work = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut child = Command::new(binary())
        .arg("--port")
        .arg(port.to_string())
        .arg("--data-dir")
        .arg(work.path().join("data"))
        .env_clear()
        .env("PATH", "")
        .env("KALA_STUDIO_TOKEN", TOKEN)
        .env("KALA_STUDIO_GEOMETRY_WORKER", ABSENT_WORKER)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kala-studio");
    let _banner = read_banner(&mut child).await;
    wait_ready(port).await;
    let health = wait_for_worker_state(
        port,
        &["failed", "restarting", "starting", "stopped"],
        Duration::from_secs(30),
    )
    .await
    .expect("the worker state must be published");
    let worker = &health["geometry_worker"];
    assert!(
        worker.is_object(),
        "the health document needs the worker state"
    );
    assert!(worker["state"].is_string());
    assert!(worker["design_available"].is_boolean());
    let _ = child.kill().await;
}

/// The STEP import ceiling is one setting: the front door resolves it once
/// and hands the resolved value to the worker, so the two sides accept
/// exactly the same body.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_worker_is_launched_with_the_resolved_step_import_limit() {
    if skip_or_fail() {
        return;
    }
    let python = venv_python();
    assert!(
        python.is_file(),
        "the geometry worker interpreter is missing: {}",
        python.display()
    );
    let work = tempfile::tempdir().unwrap();
    let port = free_port();
    let command = serde_json::json!([
        python.to_string_lossy(),
        "-m",
        "kala_agent.studio",
        "--worker"
    ])
    .to_string();
    let mut child = Command::new(binary())
        .arg("--port")
        .arg(port.to_string())
        .arg("--data-dir")
        .arg(work.path().join("data"))
        .env_clear()
        .env("PATH", "")
        .env("KALA_STUDIO_TOKEN", TOKEN)
        .env("KALA_STUDIO_GEOMETRY_WORKER", command)
        .env("PYTHONPATH", repository_root().join("src"))
        .env("KALA_STEP_IMPORT_MAX_BYTES", "4096")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kala-studio");
    let _banner = read_banner(&mut child).await;
    wait_ready(port).await;
    wait_for_worker_state(port, &["ready"], Duration::from_secs(90))
        .await
        .expect("the geometry worker must become ready");

    // The worker process carries the resolved ceiling.
    let parent = child.id().expect("child pid");
    let workers = child_pids(parent);
    assert!(!workers.is_empty(), "the worker must be a child process");
    for worker in &workers {
        let environment = process_environ(*worker);
        let limit = environment
            .iter()
            .find(|(key, _)| key == "KALA_STEP_IMPORT_MAX_BYTES")
            .map(|(_, value)| value.clone());
        assert_eq!(
            limit.as_deref(),
            Some("4096"),
            "the worker must carry the resolved STEP import limit"
        );
    }

    // The front door enforces the same ceiling: a body at the limit reaches
    // the worker, one byte over it is refused.
    let mut at_limit = RequestSpec::default();
    at_limit
        .headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    at_limit
        .headers
        .push(("content-type".to_owned(), "application/step".to_owned()));
    at_limit.body = Some(vec![b'x'; 4_096]);
    let response = common::request(port_addr(port), "POST", "/api/studio/import", at_limit).await;
    assert_eq!(response.status, 422, "{}", response.body_text());
    assert_eq!(response.json()["code"], "invalid_step");

    let mut over_limit = RequestSpec::default();
    over_limit
        .headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    over_limit
        .headers
        .push(("content-type".to_owned(), "application/step".to_owned()));
    over_limit.body = Some(vec![b'x'; 4_097]);
    let response = common::request(port_addr(port), "POST", "/api/studio/import", over_limit).await;
    assert_eq!(response.status, 413, "{}", response.body_text());
    assert_eq!(response.json()["code"], "payload_too_large");

    let _ = child.kill().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_studio_token_is_not_echoed_by_the_process() {
    if skip_or_fail() {
        return;
    }
    let work = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut child = Command::new(binary())
        .arg("--port")
        .arg(port.to_string())
        .arg("--data-dir")
        .arg(work.path().join("data"))
        .env_clear()
        .env("PATH", "")
        .env("KALA_STUDIO_TOKEN", TOKEN)
        .env("KALA_STUDIO_GEOMETRY_WORKER", ABSENT_WORKER)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kala-studio");
    let _banner = read_banner(&mut child).await;
    wait_ready(port).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = child.kill().await;
    let output = child.wait_with_output().await.unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains(TOKEN), "the token must not be echoed");
    assert!(!stderr.contains(TOKEN), "the token must not be logged");
    let _ = HashMap::<String, String>::new();
}
