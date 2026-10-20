//! The live Python differential over a real loopback socket.
//!
//! The oracle (`tests/oracle_socket.py`) starts the **live** Python FastAPI
//! application of `kala_agent.api` on a loopback port with uvicorn, and
//! replays the same raw HTTP/1.1 requests against it and against the native
//! server this test starts. The comparison is over the status, the body bytes
//! and the headers the application itself produces; the framing headers the
//! two servers add on their own are excluded, because they belong to the
//! servers and not to the application.
//!
//! Python is a test oracle only. The crate itself never starts an
//! interpreter.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use kala_api::probe::ProbeRouter;
use kala_api::server::{ApiState, serve};

const REPO: &str = "/home/v9i/Projects/Kala-Agent";

fn python() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("KALA_PYTHON") {
        let path = PathBuf::from(explicit);
        return path.exists().then_some(path);
    }
    let path = Path::new(REPO).join(".venv/bin/python");
    path.exists().then_some(path)
}

#[tokio::test]
async fn the_native_server_matches_the_live_python_server_over_a_socket() {
    let Some(interpreter) = python() else {
        eprintln!(
            "SKIPPED: the repository Python interpreter was not found; the live socket \
             differential did not run. Set KALA_PYTHON to run it."
        );
        return;
    };
    let work = tempfile::tempdir().expect("a temporary directory");
    let corpus_path = work.path().join("corpus.json");
    let results_path = work.path().join("results.json");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback listener");
    let port = listener.local_addr().expect("the local address").port();
    let state = Arc::new(
        ApiState::new(
            "unit-test-launch-token-0123456789abcdef",
            4096,
            Arc::new(ProbeRouter),
        )
        .expect("a valid state"),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let _ = serve(state, listener, async move {
            let _ = stopped.await;
        })
        .await;
    });

    let oracle = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle_socket.py");
    let output = tokio::task::spawn_blocking(move || {
        Command::new(&interpreter)
            .arg("-I")
            .arg(&oracle)
            .arg(&corpus_path)
            .arg(&results_path)
            .arg(port.to_string())
            .output()
    })
    .await
    .expect("the blocking task runs")
    .expect("the oracle runs");
    let _ = stop.send(());
    let _ = server.await;

    let report = String::from_utf8_lossy(&output.stdout).into_owned();
    eprintln!("{report}");
    assert!(
        output.status.success(),
        "the socket oracle reported mismatches:\n{report}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        report.contains("socket mismatches: 0"),
        "the socket oracle did not report a clean run:\n{report}"
    );
}
