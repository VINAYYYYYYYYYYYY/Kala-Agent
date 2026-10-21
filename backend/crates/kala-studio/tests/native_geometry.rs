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
    // The backend publishes its own worst-case design-turn budget: two model
    // calls (180 s) plus two FreeCAD builds (1 800 s) = 3 960 s.
    assert_eq!(document["design_turn_budget_seconds"], json!(3_960));

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

// ---------------------------------------------------------------------------
// The cut guard: a pocket that removed nothing is never verified.
// ---------------------------------------------------------------------------

/// The normal 80 x 40 x 8 plate with four Ø6 through holes.
fn four_hole_plate() -> Value {
    json!({
        "provider": "opencascade",
        "script": {
            "summary": "80 x 40 x 8 plate with four 6 mm through holes",
            "components": [{
                "name": "plate",
                "steps": [
                    {"op": "sketch", "plane": "XY", "profile": "rectangle", "width": 80, "height": 40},
                    {"op": "extrude", "length": 8},
                    {"op": "sketch", "plane": "XY", "profile": "circle", "diameter": 6, "x": 8, "y": 8},
                    {"op": "pocket"},
                    {"op": "sketch", "plane": "XY", "profile": "circle", "diameter": 6, "x": 72, "y": 8},
                    {"op": "pocket"},
                    {"op": "sketch", "plane": "XY", "profile": "circle", "diameter": 6, "x": 72, "y": 32},
                    {"op": "pocket"},
                    {"op": "sketch", "plane": "XY", "profile": "circle", "diameter": 6, "x": 8, "y": 32},
                    {"op": "pocket"}
                ]
            }]
        }
    })
}

/// The same plate, but its one Ø6 hole is sketched entirely outside the body.
fn off_plate_hole() -> Value {
    json!({
        "provider": "opencascade",
        "script": {
            "summary": "80 x 40 x 8 plate with one hole outside the body",
            "components": [{
                "name": "plate",
                "steps": [
                    {"op": "sketch", "plane": "XY", "profile": "rectangle", "width": 80, "height": 40},
                    {"op": "extrude", "length": 8},
                    {"op": "sketch", "plane": "XY", "profile": "circle", "diameter": 6, "x": 100, "y": 20},
                    {"op": "pocket"}
                ]
            }]
        }
    })
}

/// One 12-step half of the 2026-10-20 MiniMax flange repro: a full disc with
/// the Ø30 bore, then a pocket that removes its own side, then three Ø8 holes
/// that land in the removed material.
fn flange_repro_half(name: &str, rectangle_x: f64, holes: [(f64, f64); 3]) -> Value {
    let mut steps = vec![
        json!({"op": "sketch", "plane": "XY", "profile": "circle", "diameter": 100}),
        json!({"op": "extrude", "length": 12}),
        json!({"op": "sketch", "plane": "XY", "profile": "circle", "diameter": 30}),
        json!({"op": "pocket"}),
        json!({"op": "sketch", "plane": "XY", "profile": "rectangle",
               "width": 100, "height": 100, "x": rectangle_x, "y": -50}),
        json!({"op": "pocket"}),
    ];
    for (x, y) in holes {
        steps.push(
            json!({"op": "sketch", "plane": "XY", "profile": "circle", "diameter": 8, "x": x, "y": y}),
        );
        steps.push(json!({"op": "pocket"}));
    }
    json!({"name": name, "steps": steps})
}

/// The two-half MiniMax flange the wave's live finding recorded.
fn flange_repro() -> Value {
    let components = vec![
        flange_repro_half(
            "flange_right",
            0.0,
            [
                (40.0, 0.0),
                (20.0, 34.641_016_151_377_55),
                (20.0, -34.641_016_151_377_55),
            ],
        ),
        flange_repro_half(
            "flange_left",
            -100.0,
            [
                (-40.0, 0.0),
                (-20.0, 34.641_016_151_377_55),
                (-20.0, -34.641_016_151_377_55),
            ],
        ),
    ];
    json!({
        "script": {
            "summary": "100 mm flange in two halves",
            "components": components,
            "assembly": [
                {"op": "place", "component": "flange_right"},
                {"op": "place", "component": "flange_left"},
                {"op": "fuse", "components": ["flange_right", "flange_left"]},
            ],
        },
    })
}

async fn freecad_is_available(addr: std::net::SocketAddr) -> bool {
    let response = get(addr, "/api/studio/providers/cad").await;
    if response.status != 200 {
        return false;
    }
    response.json()["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|row| row["id"] == json!("freecad") && row["available"] == json!(true))
}

fn assert_ineffective_cut_is_not_verified(design: &Value) {
    assert_eq!(design["verified"], json!(false));
    let warnings = design["warnings"].as_array().expect("warnings");
    assert!(
        warnings.iter().any(|warning| warning
            .as_str()
            .is_some_and(|text| text.contains("removed no material"))),
        "{warnings:?}"
    );
    let checks = design["checks"].as_array().expect("checks");
    let row = checks.last().expect("the cut row is last");
    assert_eq!(row["id"], json!("cuts_effective"));
    assert_eq!(row["label"], json!("Cuts removed material"));
    assert_eq!(row["expected"], json!(1.0));
    assert_eq!(row["measured"], json!(0.0));
    assert_eq!(row["tolerance"], json!(0.0));
    assert_eq!(row["unit"], json!("cut"));
    assert_eq!(row["passed"], json!(false));
    assert_eq!(row["status"], json!("failed"));
    let volume = design["measured"]["volume_mm3"].as_f64().expect("a volume");
    assert!(
        (volume - 25_600.0).abs() < 1e-6,
        "the off-plate pocket removed material: {volume}"
    );
}

/// The six per-pocket warnings the two-half flange must report.
const FLANGE_REPRO_WARNINGS: [&str; 6] = [
    "component 'flange_right': pocket at step index 7 on XY removed no material",
    "component 'flange_right': pocket at step index 9 on XY removed no material",
    "component 'flange_right': pocket at step index 11 on XY removed no material",
    "component 'flange_left': pocket at step index 7 on XY removed no material",
    "component 'flange_left': pocket at step index 9 on XY removed no material",
    "component 'flange_left': pocket at step index 11 on XY removed no material",
];

fn assert_flange_repro_is_rejected(design: &Value) {
    assert_eq!(design["verified"], json!(false));
    let warnings = design["warnings"].as_array().expect("warnings");
    let expected: Vec<Value> = FLANGE_REPRO_WARNINGS
        .iter()
        .map(|warning| json!(warning))
        .collect();
    assert_eq!(warnings, &expected, "{warnings:?}");
    let checks = design["checks"].as_array().expect("checks");
    let row = checks.last().expect("the cut row is last");
    assert_eq!(row["id"], json!("cuts_effective"));
    assert_eq!(row["expected"], json!(10.0));
    assert_eq!(row["measured"], json!(4.0));
    assert_eq!(row["passed"], json!(false));
    assert_eq!(row["status"], json!("failed"));
    let volume = design["measured"]["volume_mm3"].as_f64().expect("a volume");
    assert!(
        (volume - 85_765.479_443).abs() < 1e-3,
        "the flange repro volume changed: {volume}"
    );
}

fn assert_effective_cuts_are_verified(design: &Value) {
    assert_eq!(design["verified"], json!(true));
    let volume = design["measured"]["volume_mm3"].as_f64().expect("a volume");
    assert!(
        (volume - 24_695.221_316_345).abs() < 1e-3,
        "the four-hole plate volume changed: {volume}"
    );
    assert!(design["warnings"].as_array().is_some_and(Vec::is_empty));
    assert!(
        design["checks"].as_array().is_some_and(|checks| checks
            .iter()
            .all(|row| row["id"] != json!("cuts_effective"))),
        "a passing design gained the cuts_effective row"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_off_plate_hole_fails_verification_on_opencascade() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;

    let response = post(server.addr, "/api/studio/design", &off_plate_hole()).await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    assert_ineffective_cut_is_not_verified(&response.json());

    let response = post(server.addr, "/api/studio/design", &four_hole_plate()).await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    assert_effective_cuts_are_verified(&response.json());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_off_plate_hole_fails_verification_on_freecad() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;
    if !freecad_is_available(server.addr).await {
        eprintln!(
            "FreeCADCmd is not available on this host: the live FreeCAD cut guard is skipped loudly"
        );
        return;
    }

    let mut request = off_plate_hole();
    request["provider"] = json!("freecad");
    let response = post(server.addr, "/api/studio/design", &request).await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let design = response.json();
    assert_ineffective_cut_is_not_verified(&design);
    assert_eq!(design["measured_by"], json!("FreeCAD"));
    assert!(design["warnings"].as_array().is_some_and(|warnings| {
        warnings.iter().any(|warning| {
            warning.as_str()
                == Some("component 'plate': pocket at step index 3 on XY removed no material")
        })
    }));

    let mut request = four_hole_plate();
    request["provider"] = json!("freecad");
    let response = post(server.addr, "/api/studio/design", &request).await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    assert_effective_cuts_are_verified(&response.json());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_flange_repro_names_every_ineffective_pocket_on_opencascade() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;

    let mut request = flange_repro();
    request["provider"] = json!("opencascade");
    let response = post(server.addr, "/api/studio/design", &request).await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    assert_flange_repro_is_rejected(&response.json());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_flange_repro_names_every_ineffective_pocket_on_freecad() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;
    if !freecad_is_available(server.addr).await {
        eprintln!(
            "FreeCADCmd is not available on this host: the live flange repro is skipped loudly"
        );
        return;
    }

    let mut request = flange_repro();
    request["provider"] = json!("freecad");
    let response = post(server.addr, "/api/studio/design", &request).await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let design = response.json();
    assert_flange_repro_is_rejected(&design);
    assert_eq!(design["measured_by"], json!("FreeCAD"));
}

/// The 200 x 200 x 5 mm plate with a 10 x 10 grid of Ø3 through holes: 202 steps.
fn hundred_hole_plate() -> Value {
    let mut steps = vec![
        json!({"op": "sketch", "plane": "XY", "profile": "rectangle", "width": 200, "height": 200}),
        json!({"op": "extrude", "length": 5}),
    ];
    for row in 0..10 {
        for column in 0..10 {
            steps.push(json!({
                "op": "sketch",
                "plane": "XY",
                "profile": "circle",
                "diameter": 3,
                "x": 20 + 16 * column,
                "y": 20 + 16 * row,
            }));
            steps.push(json!({"op": "pocket"}));
        }
    }
    json!({
        "script": {
            "summary": "200 x 200 x 5 mm plate with a 10 x 10 grid of 3 mm through holes",
            "components": [{"name": "plate", "steps": steps}],
        },
        "provider": "opencascade",
    })
}

/// The same plate, but three of its holes lie entirely outside the body.
fn hundred_hole_plate_with_three_off_plate() -> Value {
    let mut request = hundred_hole_plate();
    let steps = request["script"]["components"][0]["steps"]
        .as_array_mut()
        .expect("steps");
    // The hole index is its position in the 10 x 10 grid; each hole is a
    // sketch-and-pocket pair after the first two steps.
    for hole in [0usize, 11, 99] {
        let index = 2 + 2 * hole;
        steps[index]["x"] = json!(300 + hole);
        steps[index]["y"] = json!(300);
    }
    request["script"]["summary"] =
        json!("200 x 200 x 5 mm plate with three holes outside the body");
    request
}

fn expected_hundred_hole_volume(holes: f64) -> f64 {
    200.0 * 200.0 * 5.0 - holes * std::f64::consts::PI * 1.5 * 1.5 * 5.0
}

fn assert_hundred_hole_design(design: &Value, holes: f64) {
    assert_eq!(design["verified"], json!(true), "{design}");
    let volume = design["measured"]["volume_mm3"].as_f64().expect("a volume");
    assert!(
        (volume - expected_hundred_hole_volume(holes)).abs() < 1e-3,
        "the {holes}-hole plate volume is {volume}"
    );
    assert!(design["warnings"].as_array().is_some_and(Vec::is_empty));
    assert!(
        design["checks"].as_array().is_some_and(|checks| checks
            .iter()
            .all(|row| row["id"] != json!("cuts_effective"))),
        "a passing design gained the cuts_effective row"
    );
    let vertices = design["view"]["vertex_count"].as_u64().expect("vertices");
    let triangles = design["view"]["triangle_count"]
        .as_u64()
        .expect("triangles");
    eprintln!("100-hole plate view: {vertices} vertices, {triangles} triangles");
    assert!(vertices > 0 && triangles > 0);
    assert!(vertices <= 200_000, "the view exceeded the vertex bound");
    assert!(triangles <= 400_000, "the view exceeded the triangle bound");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hundred_hole_plate_builds_and_verifies_on_opencascade() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;

    let started = std::time::Instant::now();
    let response = post(server.addr, "/api/studio/design", &hundred_hole_plate()).await;
    eprintln!(
        "OpenCascade 100-hole plate (202 steps) built in {:?}",
        started.elapsed()
    );
    assert_eq!(response.status, 200, "{}", response.body_text());
    let design = response.json();
    assert_hundred_hole_design(&design, 100.0);
    assert_eq!(design["measured_by"], json!("OpenCascade (opt-in)"));

    let design_id = design["design_id"].as_str().expect("design id").to_owned();
    let drawing = get(
        server.addr,
        &format!("/api/studio/design/{design_id}/drawing"),
    )
    .await;
    assert_eq!(drawing.status, 200, "{}", drawing.body_text());
    let drawing = drawing.json();
    let truncated = drawing["views"]
        .as_array()
        .is_some_and(|views| views.iter().any(|view| view["truncated"] == json!(true)));
    eprintln!(
        "100-hole plate drawing: {} points, truncated {truncated}",
        drawing["point_count"]
    );

    let section = get(
        server.addr,
        &format!("/api/studio/design/{design_id}/section?plane=XZ&offset=100"),
    )
    .await;
    assert_eq!(section.status, 200, "{}", section.body_text());
    let section = section.json();
    eprintln!(
        "100-hole plate XZ section: {} outer loops, {} inner loops, area {:?}",
        section["outer_loop_count"], section["inner_loop_count"], section["area_mm2"]
    );
    assert_eq!(section["outer_loop_count"], json!(11));
    assert_eq!(section["inner_loop_count"], json!(0));
    assert_eq!(section["area_exact"], json!(true));
    let area = section["area_mm2"].as_f64().expect("area");
    assert!(
        (area - 850.0).abs() < 1e-6,
        "the section through the hole row is 200 x 5 minus ten 3 x 5 slots: {area}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hundred_hole_plate_builds_and_verifies_on_freecad() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;
    if !freecad_is_available(server.addr).await {
        eprintln!(
            "FreeCADCmd is not available on this host: the live 100-hole plate is skipped loudly"
        );
        return;
    }

    let mut request = hundred_hole_plate();
    request["provider"] = json!("freecad");
    let started = std::time::Instant::now();
    let response = post(server.addr, "/api/studio/design", &request).await;
    eprintln!(
        "FreeCAD 100-hole plate (202 steps) built in {:?}",
        started.elapsed()
    );
    assert_eq!(response.status, 200, "{}", response.body_text());
    let design = response.json();
    assert_hundred_hole_design(&design, 100.0);
    assert_eq!(design["measured_by"], json!("FreeCAD"));
}

#[tokio::test(flavor = "multi_thread")]
async fn three_off_plate_holes_are_counted_exactly_on_opencascade() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;

    let response = post(
        server.addr,
        "/api/studio/design",
        &hundred_hole_plate_with_three_off_plate(),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let design = response.json();
    assert_eq!(design["verified"], json!(false));
    let checks = design["checks"].as_array().expect("checks");
    let row = checks.last().expect("the cut row is last");
    assert_eq!(row["id"], json!("cuts_effective"));
    assert_eq!(row["expected"], json!(100.0));
    assert_eq!(row["measured"], json!(97.0));
    assert_eq!(row["passed"], json!(false));
    let volume = design["measured"]["volume_mm3"].as_f64().expect("a volume");
    assert!(
        (volume - expected_hundred_hole_volume(97.0)).abs() < 1e-2,
        "the 97-hole volume is {volume}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn three_off_plate_holes_are_counted_exactly_on_freecad() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;
    if !freecad_is_available(server.addr).await {
        eprintln!(
            "FreeCADCmd is not available on this host: the live 97-hole plate is skipped loudly"
        );
        return;
    }

    let mut request = hundred_hole_plate_with_three_off_plate();
    request["provider"] = json!("freecad");
    let response = post(server.addr, "/api/studio/design", &request).await;
    assert_eq!(response.status, 200, "{}", response.body_text());
    let design = response.json();
    assert_eq!(design["verified"], json!(false));
    let checks = design["checks"].as_array().expect("checks");
    let row = checks.last().expect("the cut row is last");
    assert_eq!(row["id"], json!("cuts_effective"));
    assert_eq!(row["expected"], json!(100.0));
    assert_eq!(row["measured"], json!(97.0));
    assert_eq!(
        design["warnings"].as_array().map(Vec::len),
        Some(3),
        "{design}"
    );
}

// ------------------------------------------------------- artifact downloads

/// Read one stored artifact through the public download route.
async fn download(addr: std::net::SocketAddr, design_id: &str, kind: &str) -> common::TestResponse {
    get(
        addr,
        &format!("/api/studio/design/{design_id}/artifact/{kind}"),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_artifact_route_downloads_the_exact_stored_bytes() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;

    let design = post(server.addr, "/api/studio/design", &bracket()).await;
    assert_eq!(design.status, 200, "{}", design.body_text());
    let design = design.json();
    let design_id = design["design_id"].as_str().expect("design id").to_owned();

    // The construction script: exact bytes, verified digest, real filename.
    let script = download(server.addr, &design_id, "script").await;
    assert_eq!(script.status, 200, "{}", script.body_text());
    assert_eq!(script.header("content-type"), Some("application/json"));
    let script_digest = design["artifacts"]["script_sha256"]
        .as_str()
        .expect("script digest");
    assert_eq!(
        script.header("content-disposition"),
        Some(
            format!(
                r#"attachment; filename="l-bracket-60-x-45-x-40-{}.json""#,
                &script_digest[..8]
            )
            .as_str()
        )
    );
    assert_eq!(script.header("x-studio-digest"), Some(script_digest));
    assert_eq!(script.header("x-studio-artifact-kind"), Some("script"));
    assert_eq!(
        script.header("x-studio-design-id"),
        Some(design_id.as_str())
    );
    assert_eq!(script.body, {
        let path = work
            .path()
            .join("data")
            .join("artifacts")
            .join("objects")
            .join(&script_digest[..2])
            .join(script_digest);
        std::fs::read(path).expect("the stored script object")
    });
    let stored_script: Value = serde_json::from_slice(&script.body).expect("script JSON");
    assert!(stored_script["components"].as_array().is_some());

    // The provider's STEP: exactly the bytes whose digest the design reports.
    let step = download(server.addr, &design_id, "step").await;
    assert_eq!(step.status, 200, "{}", step.body_text());
    assert_eq!(step.header("content-type"), Some("application/step"));
    let step_digest = design["artifacts"]["step_sha256"]
        .as_str()
        .expect("step digest");
    assert_eq!(step.header("x-studio-digest"), Some(step_digest));
    assert_eq!(step.body, {
        let path = work
            .path()
            .join("data")
            .join("artifacts")
            .join("objects")
            .join(&step_digest[..2])
            .join(step_digest);
        std::fs::read(path).expect("the stored STEP object")
    });
    assert!(step.body.starts_with(b"ISO-10303-21"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_artifact_route_refuses_a_kind_the_design_did_not_store() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;
    let design = post(server.addr, "/api/studio/design", &bracket()).await;
    let design = design.json();
    let design_id = design["design_id"].as_str().expect("design id");

    for kind in ["fcstd", "stl"] {
        let response = download(server.addr, design_id, kind).await;
        assert_eq!(response.status, 404, "{}", response.body_text());
        let body = response.json();
        assert_eq!(body["code"], json!("artifact_not_found"));
        assert_eq!(
            body["message"],
            json!("no artifact of that kind is stored for this design")
        );
        assert_eq!(
            body["details"],
            json!([format!("design_id={design_id}; kind={kind}")])
        );
    }

    let unknown = download(server.addr, design_id, "bogus").await;
    assert_eq!(unknown.status, 422);
    assert_eq!(
        unknown.json()["details"],
        json!(["the artifact kind must be one of script, step, fcstd, stl"])
    );

    let invalid = get(server.addr, "/api/studio/design/not-a-digest/artifact/step").await;
    assert_eq!(invalid.status, 422);
    assert_eq!(
        invalid.json()["details"],
        json!(["the design id is not a sha256 digest"])
    );

    let denied = common::request(
        server.addr,
        "GET",
        &format!("/api/studio/design/{design_id}/artifact/step"),
        RequestSpec {
            headers: vec![("Authorization".to_owned(), "Bearer wrong".to_owned())],
            body: None,
            chunked: None,
        },
    )
    .await;
    assert_eq!(denied.status, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_artifact_route_refuses_a_tampered_store_object() {
    let work = tempfile::tempdir().unwrap();
    let server = start_native(work.path().join("data")).await;
    let design = post(server.addr, "/api/studio/design", &bracket()).await;
    let design = design.json();
    let design_id = design["design_id"].as_str().expect("design id").to_owned();
    let digest = design["artifacts"]["script_sha256"]
        .as_str()
        .expect("script digest")
        .to_owned();
    let object = work
        .path()
        .join("data")
        .join("artifacts")
        .join("objects")
        .join(&digest[..2])
        .join(&digest);
    std::fs::write(&object, b"{\"tampered\":true}").expect("tamper the stored object");

    let response = download(server.addr, &design_id, "script").await;
    assert_eq!(response.status, 500, "{}", response.body_text());
    assert_eq!(response.json()["code"], json!("artifact_digest_mismatch"));
    assert_eq!(
        response.json()["message"],
        json!("the stored artifact does not match its journalled digest")
    );
}
