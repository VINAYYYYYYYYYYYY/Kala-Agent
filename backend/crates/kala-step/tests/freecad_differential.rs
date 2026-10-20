//! Second-reader differential: FreeCAD re-import of the emitted files.
//!
//! The required independent verification is OpenCascade through
//! `cadquery-ocp` (`tests/ocp_differential.rs`). This test adds an
//! application-level consumer with its own import path — FreeCAD 1.1.4, which
//! is installed on this host — so an emitted file is shown to load as one
//! closed analytic solid outside the oracle used to design the writer.
//!
//! `FreeCADCmd` is expected on a development host; when it is missing the test
//! fails with an explicit message unless `KALA_STEP_SKIP_FREECAD=1` is set,
//! which prints a loud notice instead of pretending the check passed. Use
//! `KALA_STEP_FREECAD` to point at another binary.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use kala_step::{PrimitiveKind, export_step_primitive};
use serde::Deserialize;
use serde_json::{Value, json};

mod common;

use common::TempDir;

const REL_TOLERANCE: f64 = 1e-9;
/// FreeCAD's `BoundBox` is exact for these analytic solids, so this is tighter
/// than the OpenCascade `Bnd_Box` tolerance used in the OCP differential.
const ABS_BOUND_TOLERANCE: f64 = 1e-9;

struct Case {
    id: &'static str,
    kind: PrimitiveKind,
    parameters: Value,
    faces: i64,
    surfaces: &'static [(&'static str, i64)],
    volume: f64,
    area: f64,
    bounds_min: [f64; 3],
    bounds_max: [f64; 3],
}

fn cases() -> Vec<Case> {
    let half = |value: f64| value / 2.0;
    let box_case = |id, x: f64, y: f64, z: f64| Case {
        id,
        kind: PrimitiveKind::Box,
        parameters: json!({"size_x": x, "size_y": y, "size_z": z}),
        faces: 6,
        surfaces: &[("plane", 6)],
        volume: x * y * z,
        area: 2.0 * (x * y + y * z + z * x),
        bounds_min: [-half(x), -half(y), -half(z)],
        bounds_max: [half(x), half(y), half(z)],
    };
    let cylinder_case = |id, radius: f64, height: f64| Case {
        id,
        kind: PrimitiveKind::Cylinder,
        parameters: json!({"radius": radius, "height": height}),
        faces: 3,
        surfaces: &[("cylinder", 1), ("plane", 2)],
        volume: std::f64::consts::PI * radius * radius * height,
        area: 2.0 * std::f64::consts::PI * radius * (radius + height),
        bounds_min: [-radius, -radius, -half(height)],
        bounds_max: [radius, radius, half(height)],
    };
    let sphere_case = |id, radius: f64| Case {
        id,
        kind: PrimitiveKind::Sphere,
        parameters: json!({"radius": radius}),
        faces: 1,
        surfaces: &[("sphere", 1)],
        volume: 4.0 / 3.0 * std::f64::consts::PI * radius * radius * radius,
        area: 4.0 * std::f64::consts::PI * radius * radius,
        bounds_min: [-radius, -radius, -radius],
        bounds_max: [radius, radius, radius],
    };
    vec![
        box_case("box-default", 10.0, 10.0, 10.0),
        box_case("box-non-cube", 20.0, 30.0, 40.0),
        cylinder_case("cylinder-default", 5.0, 10.0),
        cylinder_case("cylinder-extreme-ratio", 0.1, 2000.0),
        sphere_case("sphere-default", 5.0),
        sphere_case("sphere-max", 2000.0),
    ]
}

#[derive(Debug, Deserialize)]
struct CaseReport {
    id: String,
    valid: Option<bool>,
    closed: Option<bool>,
    solids: Option<i64>,
    shells: Option<i64>,
    faces: Option<i64>,
    volume: Option<f64>,
    area: Option<f64>,
    surface_types: Option<BTreeMap<String, i64>>,
    bounds_min: Option<Vec<f64>>,
    bounds_max: Option<Vec<f64>>,
    oracle_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Report {
    freecad_version: String,
    cases: Vec<CaseReport>,
}

fn freecad_binary() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("KALA_STEP_FREECAD") {
        return Some(PathBuf::from(explicit));
    }
    for candidate in ["/usr/bin/FreeCADCmd", "/usr/local/bin/FreeCADCmd"] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn assert_close(label: &str, native: f64, expected: f64) {
    let difference =
        (native - expected).abs() / native.abs().max(expected.abs()).max(f64::MIN_POSITIVE);
    assert!(
        difference <= REL_TOLERANCE,
        "{label}: {native} differs from {expected} by {difference:e}"
    );
}

fn assert_bounds(label: &str, measured: &[f64], expected: &[f64; 3]) {
    assert_eq!(measured.len(), 3, "{label}: three coordinates expected");
    for (index, (measured, expected)) in measured.iter().zip(expected).enumerate() {
        let difference = (measured - expected).abs();
        assert!(
            difference <= ABS_BOUND_TOLERANCE,
            "{label}: axis {index} measured {measured} mm, expected {expected} mm"
        );
    }
}

#[test]
fn freecad_reads_the_emitted_files_as_exact_solids() {
    let Some(freecad) = freecad_binary() else {
        if std::env::var_os("KALA_STEP_SKIP_FREECAD").is_some_and(|value| value == "1") {
            println!(
                "kala-step: FreeCAD cross-check SKIPPED by explicit KALA_STEP_SKIP_FREECAD=1; \
                 the required OpenCascade differential still ran"
            );
            return;
        }
        panic!(
            "FreeCADCmd was not found; install FreeCAD, set KALA_STEP_FREECAD, or set \
             KALA_STEP_SKIP_FREECAD=1 to opt out of the second-reader check explicitly"
        );
    };

    let cases = cases();
    let workdir = TempDir::new("kala-step-freecad");

    let mut request_cases = Vec::with_capacity(cases.len());
    for case in &cases {
        let bytes = export_step_primitive(case.kind, Some(&case.parameters))
            .unwrap_or_else(|error| panic!("export {} failed: {error}", case.id));
        let path = workdir.join(&format!("{}.step", case.id));
        std::fs::write(&path, &bytes).expect("the STEP file can be written");
        request_cases.push(json!({"id": case.id, "step_path": path.to_string_lossy()}));
    }

    // FreeCAD imports the script as a module and caches its bytecode, so the
    // script is copied into the self-cleaning work directory and run from
    // there; nothing is left beside the repository's test-only oracle.
    let oracle = workdir.join("freecad_oracle.py");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("freecad_oracle.py"),
        &oracle,
    )
    .expect("the test-only oracle can be copied");
    let request_path = workdir.join("request.json");
    let report_path = workdir.join("report.json");
    std::fs::write(
        &request_path,
        serde_json::to_vec_pretty(&json!({"cases": request_cases})).unwrap(),
    )
    .expect("the request can be written");

    let output = Command::new(&freecad)
        .arg(&oracle)
        .arg(&request_path)
        .arg(&report_path)
        .output()
        .unwrap_or_else(|error| panic!("could not run {}: {error}", freecad.display()));
    assert!(
        output.status.success(),
        "FreeCADCmd failed ({}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report: Report =
        serde_json::from_slice(&std::fs::read(&report_path).expect("a report was written"))
            .expect("the report parses");
    assert_eq!(report.cases.len(), cases.len());
    println!(
        "kala-step: FreeCAD {} cross-check on {} cases",
        report.freecad_version,
        report.cases.len()
    );

    let by_id: BTreeMap<&str, &CaseReport> = report
        .cases
        .iter()
        .map(|case| (case.id.as_str(), case))
        .collect();
    for case in &cases {
        let observed = by_id
            .get(case.id)
            .unwrap_or_else(|| panic!("FreeCAD did not report {}", case.id));
        if let Some(error) = &observed.oracle_error {
            panic!("FreeCAD error for {}: {error}", case.id);
        }
        assert_eq!(observed.valid, Some(true), "{}: isValid", case.id);
        assert_eq!(observed.closed, Some(true), "{}: isClosed", case.id);
        assert_eq!(observed.solids, Some(1), "{}: solids", case.id);
        assert_eq!(observed.shells, Some(1), "{}: shells", case.id);
        assert_eq!(observed.faces, Some(case.faces), "{}: faces", case.id);
        let surfaces: BTreeMap<String, i64> = case
            .surfaces
            .iter()
            .map(|(name, count)| ((*name).to_owned(), *count))
            .collect();
        assert_eq!(
            observed.surface_types.as_ref(),
            Some(&surfaces),
            "{}: surface types",
            case.id
        );
        assert_close(
            &format!("{} volume", case.id),
            observed.volume.expect("volume"),
            case.volume,
        );
        assert_close(
            &format!("{} area", case.id),
            observed.area.expect("area"),
            case.area,
        );
        assert_bounds(
            &format!("{} minimum", case.id),
            observed.bounds_min.as_deref().expect("minimum bounds"),
            &case.bounds_min,
        );
        assert_bounds(
            &format!("{} maximum", case.id),
            observed.bounds_max.as_deref().expect("maximum bounds"),
            &case.bounds_max,
        );
        println!(
            "  {} ok: FreeCAD volume {:.9} mm^3 area {:.9} mm^2, bounds {} mm",
            case.id,
            observed.volume.unwrap_or_default(),
            observed.area.unwrap_or_default(),
            case.bounds_max[0]
        );
    }

    // `workdir` removes itself when it drops, on success and on failure.
}
