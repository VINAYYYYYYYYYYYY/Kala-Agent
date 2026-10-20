//! Real HTTP STEP export/handoff verified by a test-only OpenCascade import.
//!
//! This is the required end-to-end evidence for the native STEP adapter: the
//! release/debug server is started on a real loopback socket, `POST
//! /api/studio/export` and `POST /api/studio/handoff` bytes are captured, and
//! the *test-only* `tests/step_ocp_oracle.py` (repository virtualenv,
//! `cadquery-ocp`) re-imports every response body and reports validity,
//! topology, analytic surface types and volume/area/bounds in millimetres.
//! The oracle also exports each primitive with the current Python studio
//! exporter, so the native bytes are compared numerically against the
//! implementation being replaced.
//!
//! No product Python or OCP dependency exists: the service never runs an
//! interpreter, and the oracle is skipped nowhere — a missing interpreter or
//! OCP fails this test loudly (`KALA_STUDIO_SKIP_OCP_ORACLE=1` is the only,
//! explicitly loud opt-out).
//!
//! The parked native STEP writer is the subject of this file, so it runs in
//! the `native-primitives` build. The default build answers the STEP routes
//! from the supervised geometry worker; the live Python differential covers
//! that path.

#![cfg(feature = "native-primitives")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use serde_json::{Value, json};

mod common;

use common::{RequestSpec, TOKEN, TestServer, empty_environ};

const REL_TOLERANCE: f64 = 1e-9;
const ABS_BOUND_TOLERANCE: f64 = 1e-6;

struct Case {
    id: &'static str,
    kind: &'static str,
    parameters: Value,
    faces: i64,
    surfaces: &'static [(&'static str, i64)],
    volume: f64,
    area: f64,
    bounds_min: [f64; 3],
    bounds_max: [f64; 3],
    /// `export` uses `/api/studio/export`, `handoff` uses a registry target.
    route: &'static str,
}

fn box_case(id: &'static str, size_x: f64, size_y: f64, size_z: f64) -> Case {
    Case {
        id,
        kind: "box",
        parameters: json!({"size_x": size_x, "size_y": size_y, "size_z": size_z}),
        faces: 6,
        surfaces: &[("plane", 6)],
        volume: size_x * size_y * size_z,
        area: 2.0 * (size_x * size_y + size_y * size_z + size_z * size_x),
        bounds_min: [-size_x / 2.0, -size_y / 2.0, -size_z / 2.0],
        bounds_max: [size_x / 2.0, size_y / 2.0, size_z / 2.0],
        route: "export",
    }
}

fn cylinder_case(id: &'static str, radius: f64, height: f64) -> Case {
    Case {
        id,
        kind: "cylinder",
        parameters: json!({"radius": radius, "height": height}),
        faces: 3,
        surfaces: &[("cylinder", 1), ("plane", 2)],
        volume: std::f64::consts::PI * radius * radius * height,
        area: 2.0 * std::f64::consts::PI * radius * (radius + height),
        bounds_min: [-radius, -radius, -height / 2.0],
        bounds_max: [radius, radius, height / 2.0],
        route: "export",
    }
}

fn sphere_case(id: &'static str, radius: f64) -> Case {
    Case {
        id,
        kind: "sphere",
        parameters: json!({"radius": radius}),
        faces: 1,
        surfaces: &[("sphere", 1)],
        volume: 4.0 / 3.0 * std::f64::consts::PI * radius * radius * radius,
        area: 4.0 * std::f64::consts::PI * radius * radius,
        bounds_min: [-radius, -radius, -radius],
        bounds_max: [radius, radius, radius],
        route: "export",
    }
}

fn cases() -> Vec<Case> {
    let mut handoff = box_case("handoff-solidworks-box", 10.0, 10.0, 10.0);
    handoff.route = "handoff";
    vec![
        box_case("export-box-default", 10.0, 10.0, 10.0),
        box_case("export-box-non-cube", 20.0, 10.0, 5.0),
        cylinder_case("export-cylinder-default", 5.0, 10.0),
        sphere_case("export-sphere-default", 5.0),
        handoff,
    ]
}

#[derive(Debug, Deserialize)]
struct Reference {
    valid: Option<bool>,
    solids: Option<i64>,
    surface_types: Option<BTreeMap<String, i64>>,
    volume: Option<f64>,
    area: Option<f64>,
    bounds_min: Option<Vec<f64>>,
    bounds_max: Option<Vec<f64>>,
    bytes: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CaseReport {
    id: String,
    kind: String,
    read_status: Option<i64>,
    transfer_status: Option<i64>,
    null_shape: Option<bool>,
    valid: Option<bool>,
    solids: Option<i64>,
    shells: Option<i64>,
    shell_closed: Option<Vec<String>>,
    shell_orientation: Option<Vec<String>>,
    free_edges: Option<i64>,
    faces: Option<i64>,
    surface_types: Option<BTreeMap<String, i64>>,
    volume: Option<f64>,
    area: Option<f64>,
    bounds_min: Option<Vec<f64>>,
    bounds_max: Option<Vec<f64>>,
    reference: Option<Reference>,
    oracle_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Report {
    python_version: String,
    ocp_version: String,
    cases: Vec<CaseReport>,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .canonicalize()
        .expect("the crate lives inside the repository")
}

fn oracle_python(root: &Path) -> PathBuf {
    std::env::var_os("KALA_STUDIO_OCP_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".venv").join("bin").join("python"))
}

fn relative_difference(native: f64, reference: f64) -> f64 {
    (native - reference).abs() / native.abs().max(reference.abs()).max(f64::MIN_POSITIVE)
}

fn assert_close(label: &str, native: f64, expected: f64, tolerance: f64) {
    let difference = relative_difference(native, expected);
    assert!(
        difference <= tolerance,
        "{label}: native {native} differs from expected {expected} by {difference:e} (tolerance {tolerance:e})"
    );
}

fn assert_bounds(label: &str, measured: &[f64], expected: &[f64; 3]) {
    assert_eq!(measured.len(), 3, "{label}: expected three coordinates");
    for (index, (measured, expected)) in measured.iter().zip(expected).enumerate() {
        let difference = (measured - expected).abs();
        assert!(
            difference <= ABS_BOUND_TOLERANCE,
            "{label}: axis {index} measured {measured} mm, expected {expected} mm (difference {difference:e} mm)"
        );
    }
}

fn post(body: &Value) -> RequestSpec {
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    spec.headers
        .push(("content-type".to_owned(), "application/json".to_owned()));
    spec.body = Some(serde_json::to_vec(body).unwrap());
    spec
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_step_responses_reimport_as_exact_mm_solids() {
    if std::env::var_os("KALA_STUDIO_SKIP_OCP_ORACLE").is_some_and(|value| value == "1") {
        println!(
            "kala-studio: HTTP STEP reimport SKIPPED by explicit KALA_STUDIO_SKIP_OCP_ORACLE=1 \
             opt-out; this is not STEP evidence"
        );
        return;
    }
    if cfg!(not(feature = "native-step")) {
        println!("kala-studio: HTTP STEP reimport requires the native-step feature");
        return;
    }

    let root = repo_root();
    let python = oracle_python(&root);
    assert!(
        python.is_file(),
        "the HTTP STEP reimport needs the repository virtualenv at {} (or KALA_STUDIO_OCP_PYTHON); \
         set KALA_STUDIO_SKIP_OCP_ORACLE=1 to opt out explicitly, but the route bytes are then unverified",
        python.display()
    );
    let oracle = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("step_ocp_oracle.py");
    assert!(oracle.is_file(), "missing {}", oracle.display());

    let work = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_backend(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        empty_environ(),
        kala_studio::geometry::default_step_exporter(),
        None,
        std::sync::Arc::new(kala_studio::native_backend::NativePrimitivesBackend::new(
            kala_studio::geometry::default_step_exporter(),
            std::env::temp_dir().join("kala-studio-work"),
        )),
    )
    .await;

    let cases = cases();
    let mut request_cases = Vec::with_capacity(cases.len());
    let mut exported = Vec::with_capacity(cases.len());
    for case in &cases {
        let response = if case.route == "handoff" {
            common::request(
                server.addr,
                "POST",
                "/api/studio/handoff",
                post(&json!({
                    "integration_id": "solidworks",
                    "kind": case.kind,
                    "format": "step",
                    "parameters": case.parameters,
                })),
            )
            .await
        } else {
            common::request(
                server.addr,
                "POST",
                "/api/studio/export",
                post(&json!({
                    "kind": case.kind,
                    "format": "step",
                    "parameters": case.parameters,
                })),
            )
            .await
        };
        assert_eq!(
            response.status,
            200,
            "{}: {} {}",
            case.id,
            response.status,
            response.body_text()
        );
        assert_eq!(response.header("content-type"), Some("application/step"));
        assert!(response.body.starts_with(b"ISO-10303-21;"), "{}", case.id);
        assert_eq!(
            count_occurrences(&response.body, b"MANIFOLD_SOLID_BREP"),
            1,
            "{} must be one manifold solid",
            case.id
        );
        assert!(
            response.body.iter().all(u8::is_ascii),
            "{} must be ASCII",
            case.id
        );
        if case.route == "handoff" {
            assert_eq!(
                response.header("x-studio-handoff").unwrap(),
                format!("solidworks;format=step;bytes={}", response.body.len())
            );
            assert_eq!(
                response.header("content-disposition").unwrap(),
                "attachment; filename=\"solidworks.step\""
            );
        } else {
            assert_eq!(
                response.header("content-disposition").unwrap(),
                format!("attachment; filename=\"kala-studio-{}.step\"", case.kind)
            );
        }
        let path = work.path().join(format!("{}.step", case.id));
        std::fs::write(&path, &response.body).unwrap();
        exported.push(response.body);
        request_cases.push(json!({
            "id": case.id,
            "kind": case.kind,
            "parameters": case.parameters,
            "step_path": path.to_string_lossy(),
        }));
    }
    server.stop().await;

    // The native writer is deterministic, so the same request twice is the
    // same bytes; a second box export proves the HTTP body is not mutated.
    let work2 = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_backend(
        work2.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        empty_environ(),
        kala_studio::geometry::default_step_exporter(),
        None,
        std::sync::Arc::new(kala_studio::native_backend::NativePrimitivesBackend::new(
            kala_studio::geometry::default_step_exporter(),
            std::env::temp_dir().join("kala-studio-work"),
        )),
    )
    .await;
    let repeat = common::request(
        server.addr,
        "POST",
        "/api/studio/export",
        post(&json!({"kind": "box", "format": "step", "parameters": cases[0].parameters})),
    )
    .await;
    assert_eq!(repeat.status, 200);
    assert_eq!(
        repeat.body, exported[0],
        "native STEP export must be deterministic"
    );
    server.stop().await;

    let request = json!({
        "source_root": root.to_string_lossy(),
        "python_reference": true,
        "cases": request_cases,
    });
    let request_path = work.path().join("request.json");
    let report_path = work.path().join("report.json");
    std::fs::write(
        &request_path,
        serde_json::to_vec_pretty(&request).expect("the request serialises"),
    )
    .expect("the request can be written");

    let output = Command::new(&python)
        .arg(&oracle)
        .arg(&request_path)
        .arg(&report_path)
        .current_dir(&root)
        .output()
        .unwrap_or_else(|error| panic!("could not run {}: {error}", python.display()));
    assert!(
        output.status.success(),
        "the oracle failed ({}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let raw = std::fs::read(&report_path).expect("the oracle wrote a report");
    let report: Report = serde_json::from_slice(&raw).expect("the report parses");
    assert_eq!(report.cases.len(), cases.len());
    println!(
        "kala-studio: HTTP STEP reimport via {} / OCP {} on {} cases ({} python)",
        python.display(),
        report.ocp_version,
        report.cases.len(),
        report.python_version.lines().next().unwrap_or("unknown")
    );

    let by_id: BTreeMap<&str, &CaseReport> = report
        .cases
        .iter()
        .map(|case| (case.id.as_str(), case))
        .collect();
    for case in &cases {
        let measured = by_id
            .get(case.id)
            .unwrap_or_else(|| panic!("{} missing from the oracle report", case.id));
        assert!(
            measured.oracle_error.is_none(),
            "{}: oracle error {:?}",
            case.id,
            measured.oracle_error
        );
        assert_eq!(measured.kind, case.kind);
        assert_eq!(measured.read_status, Some(1), "{}: read status", case.id);
        assert!(
            measured.transfer_status.is_some_and(|value| value > 0),
            "{}: no roots transferred",
            case.id
        );
        assert_ne!(measured.null_shape, Some(true), "{}: null shape", case.id);
        assert_eq!(measured.valid, Some(true), "{}: invalid B-rep", case.id);
        assert_eq!(measured.solids, Some(1), "{}: solids", case.id);
        assert_eq!(measured.shells, Some(1), "{}: shells", case.id);
        assert!(
            measured
                .shell_closed
                .as_deref()
                .is_some_and(|values| values == ["BRepCheck_NoError"]),
            "{}: shell must be closed ({:?})",
            case.id,
            measured.shell_closed
        );
        assert!(
            measured
                .shell_orientation
                .as_deref()
                .is_some_and(|values| values == ["BRepCheck_NoError"]),
            "{}: shell orientation ({:?})",
            case.id,
            measured.shell_orientation
        );
        assert_eq!(measured.free_edges, Some(0), "{}: free edges", case.id);
        assert_eq!(measured.faces, Some(case.faces), "{}: faces", case.id);
        let expected_surfaces: BTreeMap<String, i64> = case
            .surfaces
            .iter()
            .map(|(name, count)| ((*name).to_owned(), *count))
            .collect();
        assert_eq!(
            measured.surface_types.as_ref(),
            Some(&expected_surfaces),
            "{}: analytic surfaces",
            case.id
        );

        // Millimetres: the measured volume, area and bounds are the analytic
        // values of the requested dimensions.
        let volume = measured.volume.expect("volume");
        let area = measured.area.expect("area");
        assert_close(
            &format!("{} volume", case.id),
            volume,
            case.volume,
            REL_TOLERANCE,
        );
        assert_close(&format!("{} area", case.id), area, case.area, REL_TOLERANCE);
        assert_bounds(
            &format!("{} bounds min", case.id),
            measured.bounds_min.as_ref().expect("bounds min"),
            &case.bounds_min,
        );
        assert_bounds(
            &format!("{} bounds max", case.id),
            measured.bounds_max.as_ref().expect("bounds max"),
            &case.bounds_max,
        );

        // The native body and the Python OCP body describe the same solid:
        // OpenCascade reads both and must report the same measurements.
        let reference = measured.reference.as_ref().expect("python reference");
        assert_eq!(
            reference.valid,
            Some(true),
            "{}: reference invalid",
            case.id
        );
        assert_eq!(reference.solids, Some(1), "{}: reference solids", case.id);
        assert!(
            reference.bytes.is_some_and(|bytes| bytes > 0),
            "{}: the Python reference export was empty",
            case.id
        );
        assert_eq!(
            reference.surface_types.as_ref(),
            Some(&expected_surfaces),
            "{}: reference surfaces",
            case.id
        );
        assert_close(
            &format!("{} volume vs python", case.id),
            volume,
            reference.volume.expect("reference volume"),
            REL_TOLERANCE,
        );
        assert_close(
            &format!("{} area vs python", case.id),
            area,
            reference.area.expect("reference area"),
            REL_TOLERANCE,
        );
        assert_bounds(
            &format!("{} bounds min vs python", case.id),
            reference.bounds_min.as_ref().expect("reference bounds min"),
            &case.bounds_min,
        );
        assert_bounds(
            &format!("{} bounds max vs python", case.id),
            reference.bounds_max.as_ref().expect("reference bounds max"),
            &case.bounds_max,
        );
    }
}

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}
