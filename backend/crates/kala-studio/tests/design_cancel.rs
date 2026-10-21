//! A cancelled design turn really stops.
//!
//! Three contracts are proved here:
//!
//! * a turn whose token is already set records exactly one
//!   `design.cancelled` event and no `design.*`/`provider.build.*`/
//!   `artifact.stored` event;
//! * a client that disconnects mid-build on `POST /design` drops the handler
//!   future, the cancel-on-drop guard sets the token, and the FreeCAD
//!   provider's whole process group is gone within two seconds — with no
//!   later build event;
//! * the same holds for a design turn that arrived through `POST /chat`.
//!
//! The two route tests use a fake `FreeCADCmd` that sleeps, so they are
//! hermetic and take about two seconds.

#![cfg(feature = "native-geometry")]

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use common::{RequestSpec, TOKEN, TestServer, empty_environ};
use kala_studio::native_geometry::{DesignError, NativeDesignService, NativeGeometryBackend};

/// The corpus L-bracket without the request-only `provider` field.
fn bracket_intent() -> serde_json::Value {
    json!({
        "template": {"template": "l_bracket", "width": 60.0, "depth": 45.0, "height": 40.0, "wall": 6.0}
    })
}

fn bracket_request() -> serde_json::Value {
    let mut request = bracket_intent();
    request["provider"] = json!("freecad");
    request
}

/// Write a fake `FreeCADCmd` that answers `--version` instantly and then
/// sleeps forever as a driver run, recording its own and its child's pid.
#[cfg(unix)]
fn fake_freecad(directory: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let pids = directory.join("freecad-pids.txt");
    let executable = directory.join("FreeCADCmd");
    let script = format!(
        "#!/bin/sh\n\
         case \"$1\" in\n\
           --version)\n\
             echo \"FreeCAD 1.1.4\"\n\
             exit 0\n\
             ;;\n\
           *kala_freecad_python_probe.py)\n\
             echo 'KALA_FREECAD_PYTHON={{\"cacheTag\": \"cpython-314\"}}'\n\
             exit 0\n\
             ;;\n\
         esac\n\
         echo \"$$\" >> '{pids}'\n\
         sleep 300 &\n\
         echo \"$!\" >> '{pids}'\n\
         wait\n",
        pids = pids.display()
    );
    std::fs::write(&executable, script).expect("write the fake FreeCADCmd");
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();
    pids
}

/// Start the front door over a fake FreeCADCmd on its provider PATH.
#[cfg(unix)]
async fn start_with_fake_freecad(work: &Path) -> (TestServer, std::path::PathBuf) {
    let pids = fake_freecad(work);
    let mut environ = BTreeMap::new();
    // The provider resolves FreeCADCmd from this PATH and runs it from the
    // sanitized child environment. The standard directories stay on it so the
    // fake driver's `sleep` is found.
    environ.insert(
        "PATH".to_owned(),
        format!("{}:/usr/bin:/bin", work.display()),
    );
    environ.insert("HOME".to_owned(), work.join("home").display().to_string());
    let data_dir = work.join("data");
    let backend = NativeGeometryBackend::new(
        data_dir.clone(),
        environ,
        64 * 1024 * 1024,
        kala_studio::geometry::default_step_exporter(),
        data_dir.join("geometry-work"),
    )
    .expect("the native geometry backend opens");
    let server = TestServer::start_with_backend(
        data_dir,
        TOKEN,
        262_144,
        None,
        empty_environ(),
        kala_studio::geometry::default_step_exporter(),
        None,
        backend as Arc<dyn kala_studio::geometry_backend::GeometryBackend>,
    )
    .await;
    (server, pids)
}

/// Send one authenticated JSON request and drop the socket once the fake
/// FreeCAD driver has started; return the pids that were running.
#[cfg(unix)]
async fn abort_after_the_build_starts(
    server: &TestServer,
    path: &str,
    request: &serde_json::Value,
    pids: &Path,
) -> Vec<u32> {
    let body = serde_json::to_vec(request).expect("request body");
    let mut stream = TcpStream::connect(server.addr).await.unwrap();
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\
         Authorization: Bearer {TOKEN}\r\ncontent-type: application/json\r\n\
         content-length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(&body).await.unwrap();

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut recorded: Vec<u32> = Vec::new();
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(pids) {
            recorded = text
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .collect();
            if recorded.len() >= 2 {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(recorded.len(), 2, "the fake FreeCAD tree did not start");
    for pid in &recorded {
        assert!(
            Path::new(&format!("/proc/{pid}")).exists(),
            "the fake FreeCAD process {pid} is running before the abort"
        );
    }

    // The client disconnects mid-build.
    let aborted_at = Instant::now();
    drop(stream);
    loop {
        let alive: Vec<u32> = recorded
            .iter()
            .copied()
            .filter(|pid| Path::new(&format!("/proc/{pid}")).exists())
            .collect();
        if alive.is_empty() {
            break;
        }
        assert!(
            aborted_at.elapsed() < Duration::from_secs(2),
            "the FreeCAD process tree survived the abort: {alive:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    recorded
}

#[cfg(unix)]
async fn journal_events(server: &TestServer) -> Vec<serde_json::Value> {
    let page = common::request(
        server.addr,
        "GET",
        "/api/studio/journal?after=0&limit=500",
        RequestSpec {
            headers: vec![("Authorization".to_owned(), format!("Bearer {TOKEN}"))],
            body: None,
            chunked: None,
        },
    )
    .await;
    assert_eq!(page.status, 200, "{}", page.body_text());
    page.json()["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[cfg(unix)]
fn assert_one_cancellation_and_no_build_record(events: &[serde_json::Value]) {
    let types: Vec<String> = events
        .iter()
        .map(|event| event["event_type"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert!(
        types.iter().any(|kind| kind == "provider.build.requested"),
        "the build started: {types:?}"
    );
    assert_eq!(
        types
            .iter()
            .filter(|kind| *kind == "design.cancelled")
            .count(),
        1,
        "exactly one cancellation event: {types:?}"
    );
    for forbidden in [
        "provider.build.completed",
        "provider.build.failed",
        "artifact.stored",
        "view.imported",
        "design.compiled",
        "design.measured",
        "design.checked",
        "design.rejected",
    ] {
        assert!(
            !types.iter().any(|kind| kind == forbidden),
            "a cancelled turn produced {forbidden}: {types:?}"
        );
    }
    let cancelled = events
        .iter()
        .find(|event| event["event_type"] == json!("design.cancelled"))
        .expect("the cancellation event");
    assert_eq!(
        cancelled["payload"]["reason"],
        json!("the FreeCAD build was cancelled; the FreeCADCmd process group was stopped")
    );
    // No prompt text and no secret reaches the cancellation payload.
    assert!(cancelled["payload"].get("prompt").is_none());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_client_disconnect_kills_the_freecad_process_group() {
    let work = tempfile::tempdir().unwrap();
    let (server, pids) = start_with_fake_freecad(work.path()).await;

    // The provider must be available: the fake answers the probe.
    let probe = common::request(
        server.addr,
        "GET",
        "/api/studio/providers/cad",
        RequestSpec {
            headers: vec![("Authorization".to_owned(), format!("Bearer {TOKEN}"))],
            body: None,
            chunked: None,
        },
    )
    .await;
    assert_eq!(probe.status, 200, "{}", probe.body_text());
    let freecad = probe.json()["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|row| row["id"] == json!("freecad"))
        .cloned()
        .expect("the FreeCAD row");
    assert_eq!(freecad["available"], json!(true), "{freecad}");
    assert_eq!(freecad["version"], json!("1.1.4"));

    let recorded =
        abort_after_the_build_starts(&server, "/api/studio/design", &bracket_request(), &pids)
            .await;
    assert_eq!(recorded.len(), 2);

    // Give the cancellation event a moment to land, then read the journal.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let events = journal_events(&server).await;
    assert_one_cancellation_and_no_build_record(&events);
    server.stop().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_client_disconnect_cancels_a_design_turn_sent_through_chat() {
    let work = tempfile::tempdir().unwrap();
    let (server, pids) = start_with_fake_freecad(work.path()).await;

    let request = json!({
        "provider": "local",
        "prompt": "Design a machined aluminum L-bracket for a 40 mm mounting plate, 8 mm wall thickness, with a 10 mm centered bore and 4x M5 corner mounting holes.",
        "cad_provider": "freecad",
    });
    let recorded = abort_after_the_build_starts(&server, "/api/studio/chat", &request, &pids).await;
    assert_eq!(recorded.len(), 2);

    tokio::time::sleep(Duration::from_millis(300)).await;
    let events = journal_events(&server).await;
    let types: Vec<String> = events
        .iter()
        .map(|event| event["event_type"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(
        types
            .iter()
            .filter(|kind| *kind == "design.cancelled")
            .count(),
        1,
        "the chat turn is cancelled once: {types:?}"
    );
    // The chat turn was never completed, so no chat.turn row claims it.
    assert!(!types.iter().any(|kind| kind == "chat.turn"), "{types:?}");
    for forbidden in [
        "provider.build.completed",
        "artifact.stored",
        "design.measured",
        "design.checked",
    ] {
        assert!(!types.iter().any(|kind| kind == forbidden), "{types:?}");
    }
    server.stop().await;
}

#[test]
fn a_pre_cancelled_turn_records_only_its_cancellation() {
    let work = tempfile::tempdir().unwrap();
    let service = NativeDesignService::new(work.path(), BTreeMap::new(), 64 * 1024 * 1024)
        .expect("the design service opens");
    let intent = kala_design_intent::DesignIntent::from_json_str(
        &serde_json::to_string(&bracket_intent()).unwrap(),
    )
    .expect("the intent parses");
    let token = kala_process::CancelToken::new();
    token.cancel();

    let error = service
        .design_cancellable(&intent, "opencascade", &token)
        .expect_err("a cancelled turn does not build");
    assert!(
        matches!(error, DesignError::Cancelled(_)),
        "expected a cancellation, got {error:?}"
    );

    let events = service.journal_page(0, 100).expect("the journal page");
    let types: Vec<&str> = events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect();
    assert_eq!(types, vec!["design.cancelled"]);
    assert_eq!(events[0].aggregate_id.len(), 64);
}
