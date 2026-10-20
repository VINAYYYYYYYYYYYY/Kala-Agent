//! The native geometry backend: the front door serves design, import,
//! journal, intent and CAD-provider routes with no Python process.
//!
//! These tests exercise the backend over a real socket with the real
//! OpenCascade kernel. The cold-reload test writes its journal with the
//! **Python** implementation and then reads it back with the native service,
//! which is the interoperability claim the cutover rests on.

#![cfg(feature = "native-geometry")]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};

use common::{RequestSpec, TOKEN, TestServer, empty_environ};
use kala_studio::native_geometry::NativeGeometryBackend;

/// The corpus L-bracket, built with the opt-in OpenCascade provider.
fn bracket() -> Value {
    json!({
        "template": {"template": "l_bracket", "width": 60.0, "depth": 45.0, "height": 40.0, "wall": 6.0},
        "provider": "opencascade",
    })
}

/// The repository root, derived from the crate manifest.
fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// The repository virtualenv interpreter, which is the mandatory oracle.
fn venv_python() -> PathBuf {
    repository_root().join(".venv").join("bin").join("python")
}

/// The controlled environment the CAD provider subprocesses see.
fn provider_environment(workspace: &Path) -> BTreeMap<String, String> {
    let _ = std::fs::create_dir_all(workspace);
    match kala_process::ChildEnvironment::legacy_python_worker(workspace, &[]) {
        Ok(environment) => environment
            .entries()
            .iter()
            .map(|entry| (entry.key.clone(), entry.value.clone()))
            .collect(),
        Err(_) => BTreeMap::new(),
    }
}

/// Start the front door with the native geometry backend.
async fn start_native(data_dir: PathBuf) -> TestServer {
    let backend = NativeGeometryBackend::new(
        data_dir.clone(),
        provider_environment(&data_dir.join("geometry-worker")),
        64 * 1024 * 1024,
        kala_studio::geometry::default_step_exporter(),
        data_dir.join("geometry-work"),
    )
    .expect("the native geometry backend opens");
    TestServer::start_with_backend(
        data_dir,
        TOKEN,
        262_144,
        None,
        empty_environ(),
        kala_studio::geometry::default_step_exporter(),
        None,
        backend as Arc<dyn kala_studio::geometry_backend::GeometryBackend>,
    )
    .await
}

async fn post(addr: std::net::SocketAddr, path: &str, body: &Value) -> common::TestResponse {
    let spec = RequestSpec {
        headers: vec![
            ("Authorization".to_owned(), format!("Bearer {TOKEN}")),
            ("content-type".to_owned(), "application/json".to_owned()),
        ],
        body: Some(serde_json::to_vec(body).expect("request body")),
        chunked: None,
    };
    common::request(addr, "POST", path, spec).await
}

async fn get(addr: std::net::SocketAddr, path: &str) -> common::TestResponse {
    let spec = RequestSpec {
        headers: vec![("Authorization".to_owned(), format!("Bearer {TOKEN}"))],
        body: None,
        chunked: None,
    };
    common::request(addr, "GET", path, spec).await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_native_backend_builds_a_design_without_python() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;

    let health = get(server.addr, "/api/studio/health").await;
    assert_eq!(health.status, 200);
    let document = health.json();
    assert_eq!(document["design_available"], json!(true));
    assert_eq!(document["step_available"], json!(true));
    assert_eq!(document["geometry_worker"]["state"], json!("ready"));

    let response = post(server.addr, "/api/studio/design", &bracket()).await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let design = response.json();
    assert_eq!(design["provider"], json!("opencascade"));
    assert_eq!(design["measured_by"], json!("OpenCascade (opt-in)"));
    assert_eq!(design["verified"], json!(true));
    assert_eq!(design["measured"]["valid"], json!(true));
    assert_eq!(design["measured"]["solid_count"], json!(1));
    assert_eq!(design["component_count"], json!(2));
    assert_eq!(design["fused"], json!(true));
    assert_eq!(design["cached"], json!(false));
    // The L-bracket of the corpus: 60 x 45 x 40 with a 6 mm wall.
    assert_eq!(design["design_id"], json!(BRACKET_DESIGN_ID));
    assert_eq!(design["measured"]["volume_mm3"], json!(28439.999999999993));
    assert_eq!(design["measured"]["surface_area_mm2"], json!(11148.0));
    assert_eq!(
        design["measured"]["bounds"],
        json!({"min": [-30.0, 0.0, 0.0], "max": [30.0, 45.0, 40.0]})
    );
    assert_eq!(
        design["checks"]
            .as_array()
            .expect("checks")
            .iter()
            .map(|row| row["id"].as_str().expect("check id"))
            .collect::<Vec<_>>(),
        vec![
            "brep_valid",
            "single_solid",
            "envelope_x",
            "envelope_y",
            "envelope_z"
        ]
    );
    // The view is the OpenCascade import of the provider's own STEP bytes.
    assert_eq!(design["view"]["engine"], json!("ocp"));
    assert_eq!(design["view"]["solids"], json!(1));
    assert_eq!(design["view"]["vertex_count"], json!(56));
    assert_eq!(design["view"]["triangle_count"], json!(28));
    assert_eq!(design["view"]["warnings"], json!([]));
}

/// The design id of the corpus L-bracket with the `opencascade` provider.
const BRACKET_DESIGN_ID: &str = "9ec29435f7b95f19641f548c1d29e398f0cbb4115e9e5e38c5d8ef07c83f4bf8";

#[tokio::test(flavor = "multi_thread")]
async fn the_native_backend_refuses_a_request_without_a_provider() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;

    let mut request = bracket();
    request.as_object_mut().expect("request").remove("provider");
    let response = post(server.addr, "/api/studio/design", &request).await;
    assert_eq!(response.status, 422);
    let document = response.json();
    assert_eq!(document["code"], json!("invalid_request"));
    assert_eq!(
        document["details"],
        json!([
            "body: value_error: Value error, provider is required (choose opencascade, freecad, solidedge)"
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_native_backend_refuses_an_unknown_provider() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;

    let mut request = bracket();
    request["provider"] = json!("nope");
    let response = post(server.addr, "/api/studio/design", &request).await;
    assert_eq!(response.status, 422);
    let document = response.json();
    assert_eq!(
        document["details"],
        json!(["body.provider: enum: Input should be 'opencascade', 'freecad' or 'solidedge'"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_native_backend_answers_the_journal_routes() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;

    // A chat turn is journalled through the append route.
    let response = post(
        server.addr,
        "/api/studio/chat",
        &json!({"provider": "local", "prompt": "hello"}),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body_text());

    let page = get(server.addr, "/api/studio/journal?after=0&limit=50").await;
    assert_eq!(page.status, 200);
    let document = page.json();
    let events = document["events"].as_array().expect("events");
    assert!(
        events
            .iter()
            .any(|event| event["event_type"] == json!("chat.turn")),
        "the chat turn is journalled: {events:?}"
    );
    assert_eq!(document["limit"], json!(50));

    let verify = get(server.addr, "/api/studio/journal/verify").await;
    assert_eq!(verify.status, 200);
    let verification = verify.json();
    assert_eq!(verification["ok"], json!(true));
    assert_eq!(verification["event_count"], json!(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_native_backend_refuses_an_unusable_journal_page() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;

    let response = get(server.addr, "/api/studio/journal?after=0&limit=900").await;
    assert_eq!(response.status, 422);
    let document = response.json();
    assert_eq!(
        document["details"],
        json!(["query.limit: less_than_equal: Input should be less than or equal to 500"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_native_backend_answers_the_intent_routes() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;

    let response = post(
        server.addr,
        "/api/studio/chat",
        &json!({
            "provider": "local",
            "prompt": "make me an L-bracket 60 x 45 x 40 mm with a 6 mm wall",
            "cad_provider": "opencascade",
        }),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let document = response.json();
    let design = &document["design"];
    assert_eq!(design["design_id"], json!(BRACKET_DESIGN_ID));
    assert!(
        document["text"]
            .as_str()
            .expect("text")
            .contains("Measured volume 28440.000 mm^3")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_native_backend_imports_a_step_file_for_display() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;

    // The design's own STEP export is imported back for display.
    let design = post(server.addr, "/api/studio/design", &bracket()).await;
    let design = design.json();
    let digest = design["artifacts"]["step_sha256"]
        .as_str()
        .expect("step digest")
        .to_owned();
    let artifact = std::fs::read(
        work.path()
            .join("data")
            .join("artifacts")
            .join("objects")
            .join(&digest[..2])
            .join(&digest),
    )
    .expect("the stored STEP artifact");
    assert!(artifact.starts_with(b"ISO-10303-21"));

    let spec = RequestSpec {
        headers: vec![("Authorization".to_owned(), format!("Bearer {TOKEN}"))],
        body: Some(artifact),
        chunked: None,
    };
    let response = common::request(server.addr, "POST", "/api/studio/import", spec).await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let document = response.json();
    assert_eq!(document["step_sha256"], json!(digest));
    assert_eq!(document["solids"], json!(1));
    assert_eq!(document["warnings"], json!([]));
    assert_eq!(document["view"]["bounds"]["min"], json!([-30.0, 0.0, 0.0]));
    assert_eq!(document["view"]["bounds"]["max"], json!([30.0, 45.0, 40.0]));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_native_backend_draws_and_cuts_a_stored_design() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;
    let response = post(server.addr, "/api/studio/design", &bracket()).await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let design_id = response.json()["design_id"]
        .as_str()
        .expect("design id")
        .to_owned();

    let drawing = get(
        server.addr,
        &format!("/api/studio/design/{design_id}/drawing"),
    )
    .await;
    assert_eq!(drawing.status, 200, "{}", drawing.body_text());
    let document = drawing.json();
    assert_eq!(document["design_id"], json!(design_id));
    let views = document["views"].as_array().expect("views");
    let names = views
        .iter()
        .map(|view| view["name"].as_str().expect("view name"))
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["front", "top", "right", "iso"]);
    assert_eq!(document["chord_tolerance_mm"], json!(0.05));
    assert_eq!(document["max_points"], json!(20000));
    assert!(document["point_count"].as_u64().expect("points") > 0);

    // One named view only.
    let single = get(
        server.addr,
        &format!("/api/studio/design/{design_id}/drawing?views=front"),
    )
    .await;
    assert_eq!(single.status, 200);
    let single = single.json();
    assert_eq!(single["views"].as_array().expect("views").len(), 1);

    // An unusable view selection is refused.
    let refused = get(
        server.addr,
        &format!("/api/studio/design/{design_id}/drawing?views=nope"),
    )
    .await;
    assert_eq!(refused.status, 422);
    assert_eq!(
        refused.json()["details"],
        json!(["views must be a subset of front, top, right, iso"])
    );

    // The XZ section at y = 4 cuts the base and the upright: 60 x 40 mm.
    let section = get(
        server.addr,
        &format!("/api/studio/design/{design_id}/section?plane=XZ&offset=4"),
    )
    .await;
    assert_eq!(section.status, 200, "{}", section.body_text());
    let document = section.json();
    assert_eq!(document["plane"], json!("XZ"));
    assert_eq!(document["offset_mm"], json!(4.0));
    assert_eq!(document["outer_loop_count"], json!(1));
    assert_eq!(document["inner_loop_count"], json!(0));
    assert_eq!(document["area_exact"], json!(true));
    let area = document["area_mm2"].as_f64().expect("area");
    assert!(
        (area - 2400.0).abs() < 1e-6,
        "the cut area is 2400 mm^2: {area}"
    );

    // An unusable plane is refused.
    let refused = get(
        server.addr,
        &format!("/api/studio/design/{design_id}/section?plane=QQ"),
    )
    .await;
    assert_eq!(refused.status, 422);
    assert_eq!(
        refused.json()["details"],
        json!(["plane must be one of XY, XZ, or YZ"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_native_backend_reports_an_unknown_design_on_the_drawing_route() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;
    let response = get(
        server.addr,
        &format!("/api/studio/design/{}/drawing", "a".repeat(64)),
    )
    .await;
    assert_eq!(response.status, 404);
    let document = response.json();
    assert_eq!(document["code"], json!("design_not_found"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_native_backend_cold_reloads_a_python_written_journal() {
    let python = venv_python();
    assert!(
        python.is_file(),
        "the mandatory Python oracle is missing at {}",
        python.display()
    );
    let work = tempfile::tempdir().unwrap();
    let data_dir = work.path().join("data");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let result_file = work.path().join("oracle.json");

    // The Python implementation writes the journal and the artifacts.
    let completed = Command::new(&python)
        .arg("-I")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/python_journal_oracle.py"))
        .arg(&data_dir)
        .arg(&result_file)
        .env("PYTHONPATH", repository_root().join("src"))
        .output()
        .expect("the Python oracle runs");
    assert!(
        completed.status.success(),
        "the Python oracle failed: {}",
        String::from_utf8_lossy(&completed.stderr)
    );
    let oracle: Value =
        serde_json::from_slice(&std::fs::read(&result_file).expect("the oracle writes its result"))
            .expect("the oracle prints one document");
    assert_eq!(oracle["design_id"], json!(BRACKET_DESIGN_ID));

    // The native service opens the *same* directory and loads the design
    // back: the design is a cold miss in memory, so it is reloaded from the
    // journal and the stored STEP artifact.
    let server = start_native(data_dir).await;
    let response = post(server.addr, "/api/studio/design", &bracket()).await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let design = response.json();
    assert_eq!(design["design_id"], json!(BRACKET_DESIGN_ID));
    assert_eq!(design["cached"], json!(true));
    assert_eq!(design["measured"]["volume_mm3"], oracle["volume_mm3"]);
    assert_eq!(
        design["measured"]["surface_area_mm2"],
        oracle["surface_area_mm2"]
    );
    assert_eq!(design["measured"]["solid_count"], oracle["solid_count"]);
    assert_eq!(design["verified"], oracle["verified"]);
    assert_eq!(
        design["artifacts"]["script_sha256"],
        oracle["script_sha256"]
    );
    assert_eq!(design["artifacts"]["step_sha256"], oracle["step_sha256"]);
    assert_eq!(design["view"]["vertex_count"], json!(56));
    assert_eq!(design["view"]["triangle_count"], json!(28));
}
