//! Independent OpenCascade differential for the native STEP exporter.
//!
//! Every representative, extreme and random primitive is exported by this
//! crate, written to a temporary directory, and re-opened by the *test-only*
//! `tests/ocp_oracle.py` script under the repository virtualenv (the same
//! `cadquery-ocp` bindings the Python studio used as its CAD kernel). The
//! oracle also re-exports the same primitive with the current Python studio
//! exporter, so the assertions compare the native writer against an
//! independent reader *and* against the implementation being replaced.
//!
//! The oracle is mandatory in this environment: when the interpreter or `OCP`
//! is missing the test fails with an explicit message rather than skipping.
//! `KALA_STEP_SKIP_OCP_ORACLE=1` is the only opt-out, and it prints a loud
//! notice so a skipped differential can never look like a pass.
//!
//! Set `KALA_STEP_ORACLE_PYTHON` to use another interpreter; the default is
//! the repository `.venv`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use kala_step::{PrimitiveKind, export_step_primitive};
use serde::Deserialize;
use serde_json::{Value, json};

mod common;

use common::TempDir;

/// Relative tolerance for volumes and surface areas; the analytic formulas and
/// the OCCT integrator agree far more closely than this.
const REL_TOLERANCE: f64 = 1e-9;
/// Absolute tolerance for bounds in millimetres.
///
/// `Bnd_Box` from `BRepBndLib` grows the exact bounding box by the shape
/// tolerance (1e-7 here), so a measurement up to 1e-6 away from the analytic
/// bound is still exact geometry in millimetres.
const ABS_BOUND_TOLERANCE: f64 = 1e-6;

#[derive(Debug)]
struct Case {
    id: String,
    kind: PrimitiveKind,
    parameters: Value,
    /// Expected face count and analytic surface histogram.
    faces: i64,
    surfaces: &'static [(&'static str, i64)],
    /// Analytic volume, surface area and bounds in millimetres.
    volume: f64,
    area: f64,
    bounds_min: [f64; 3],
    bounds_max: [f64; 3],
}

fn box_case(id: &str, size_x: f64, size_y: f64, size_z: f64) -> Case {
    Case {
        id: id.to_owned(),
        kind: PrimitiveKind::Box,
        parameters: json!({"size_x": size_x, "size_y": size_y, "size_z": size_z}),
        faces: 6,
        surfaces: &[("plane", 6)],
        volume: size_x * size_y * size_z,
        area: 2.0 * (size_x * size_y + size_y * size_z + size_z * size_x),
        bounds_min: [-size_x / 2.0, -size_y / 2.0, -size_z / 2.0],
        bounds_max: [size_x / 2.0, size_y / 2.0, size_z / 2.0],
    }
}

fn cylinder_case(id: &str, radius: f64, height: f64) -> Case {
    Case {
        id: id.to_owned(),
        kind: PrimitiveKind::Cylinder,
        parameters: json!({"radius": radius, "height": height}),
        faces: 3,
        surfaces: &[("cylinder", 1), ("plane", 2)],
        volume: std::f64::consts::PI * radius * radius * height,
        area: 2.0 * std::f64::consts::PI * radius * (radius + height),
        bounds_min: [-radius, -radius, -height / 2.0],
        bounds_max: [radius, radius, height / 2.0],
    }
}

fn sphere_case(id: &str, radius: f64) -> Case {
    Case {
        id: id.to_owned(),
        kind: PrimitiveKind::Sphere,
        parameters: json!({"radius": radius}),
        faces: 1,
        surfaces: &[("sphere", 1)],
        volume: 4.0 / 3.0 * std::f64::consts::PI * radius * radius * radius,
        area: 4.0 * std::f64::consts::PI * radius * radius,
        bounds_min: [-radius, -radius, -radius],
        bounds_max: [radius, radius, radius],
    }
}

/// Deterministic splitmix64 so the random cases are reproducible everywhere.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Log-uniform dimension in `[0.1, 2000]`, the studio's valid range.
    fn dimension(&mut self) -> f64 {
        let unit = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        (0.1 * (2000.0_f64 / 0.1_f64).powf(unit)).clamp(0.1, 2000.0)
    }
}

fn cases() -> Vec<Case> {
    let mut cases = vec![
        // canonical defaults
        box_case("box-default", 10.0, 10.0, 10.0),
        cylinder_case("cylinder-default", 5.0, 10.0),
        sphere_case("sphere-default", 5.0),
        // dimension bounds and extreme ratios
        box_case("box-min", 0.1, 0.1, 0.1),
        box_case("box-max", 2000.0, 2000.0, 2000.0),
        box_case("box-non-cube", 20.0, 30.0, 40.0),
        box_case("box-extreme-ratio", 0.1, 2000.0, 5.0),
        box_case("box-extreme-ratio-2", 2000.0, 0.1, 0.2),
        cylinder_case("cylinder-min", 0.1, 0.1),
        cylinder_case("cylinder-max", 2000.0, 2000.0),
        cylinder_case("cylinder-extreme-ratio", 0.1, 2000.0),
        cylinder_case("cylinder-extreme-ratio-2", 2000.0, 0.1),
        cylinder_case("cylinder-non-default", 3.25, 17.5),
        sphere_case("sphere-min", 0.1),
        sphere_case("sphere-max", 2000.0),
        sphere_case("sphere-non-default", 42.5),
    ];
    let mut random = SplitMix64(0x5EED_2026_1019);
    for index in 0..12 {
        match index % 3 {
            0 => cases.push(box_case(
                &format!("box-random-{index}"),
                random.dimension(),
                random.dimension(),
                random.dimension(),
            )),
            1 => cases.push(cylinder_case(
                &format!("cylinder-random-{index}"),
                random.dimension(),
                random.dimension(),
            )),
            _ => cases.push(sphere_case(
                &format!("sphere-random-{index}"),
                random.dimension(),
            )),
        }
    }
    cases
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
    std::env::var_os("KALA_STEP_ORACLE_PYTHON")
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

#[test]
fn native_export_matches_ocp_reimport_and_python_export() {
    if std::env::var_os("KALA_STEP_SKIP_OCP_ORACLE").is_some_and(|value| value == "1") {
        println!(
            "kala-step: OCP differential SKIPPED by explicit KALA_STEP_SKIP_OCP_ORACLE=1 opt-out; \
             this is not STEP evidence"
        );
        return;
    }

    let root = repo_root();
    let python = oracle_python(&root);
    assert!(
        python.is_file(),
        "the STEP differential needs the repository virtualenv at {} (or KALA_STEP_ORACLE_PYTHON); \
         set KALA_STEP_SKIP_OCP_ORACLE=1 to opt out explicitly, but the native writer is then unverified",
        python.display()
    );
    let oracle = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("ocp_oracle.py");
    assert!(oracle.is_file(), "missing {}", oracle.display());

    let cases = cases();
    let workdir = TempDir::new("kala-step-differential");

    let mut request_cases = Vec::with_capacity(cases.len());
    for case in &cases {
        let bytes = export_step_primitive(case.kind, Some(&case.parameters))
            .unwrap_or_else(|error| panic!("export {} failed: {error}", case.id));
        let path = workdir.join(&format!("{}.step", case.id));
        std::fs::write(&path, &bytes).expect("the STEP file can be written");
        request_cases.push(json!({
            "id": case.id,
            "kind": case.kind.as_str(),
            "parameters": case.parameters,
            "step_path": path.to_string_lossy(),
        }));
    }

    let request = json!({
        "source_root": root.to_string_lossy(),
        "python_reference": true,
        "cases": request_cases,
    });
    let request_path = workdir.join("request.json");
    let report_path = workdir.join("report.json");
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
        "kala-step: OCP differential via {} / OCP {} on {} cases ({} python)",
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
        let observed = by_id
            .get(case.id.as_str())
            .unwrap_or_else(|| panic!("the oracle did not report case {}", case.id));
        assert_eq!(observed.kind, case.kind.as_str());
        if let Some(error) = &observed.oracle_error {
            panic!("oracle error for {}: {error}", case.id);
        }

        // Transfer status: IFSelect_RetDone == 1.
        assert_eq!(
            observed.read_status,
            Some(1),
            "{}: STEPControl_Reader.ReadFile did not return IFSelect_RetDone",
            case.id
        );
        assert!(
            observed.transfer_status.unwrap_or(0) >= 1,
            "{}: TransferRoots transferred nothing",
            case.id
        );
        assert_eq!(
            observed.null_shape,
            Some(false),
            "{}: the reader returned a null shape",
            case.id
        );
        assert_eq!(
            observed.valid,
            Some(true),
            "{}: BRepCheck_Analyzer reported an invalid solid",
            case.id
        );
        assert_eq!(observed.solids, Some(1), "{}: solid count", case.id);
        assert_eq!(observed.shells, Some(1), "{}: shell count", case.id);
        assert_eq!(
            observed.free_edges,
            Some(0),
            "{}: ShapeAnalysis_Shell found free edges",
            case.id
        );
        assert_eq!(
            observed.shell_closed.as_deref(),
            Some(["BRepCheck_NoError".to_owned()].as_slice()),
            "{}: BRepCheck_Shell::Closed",
            case.id
        );
        assert_eq!(
            observed.shell_orientation.as_deref(),
            Some(["BRepCheck_NoError".to_owned()].as_slice()),
            "{}: BRepCheck_Shell::Orientation",
            case.id
        );
        assert_eq!(observed.faces, Some(case.faces), "{}: face count", case.id);

        let expected_surfaces: BTreeMap<String, i64> = case
            .surfaces
            .iter()
            .map(|(name, count)| ((*name).to_owned(), *count))
            .collect();
        assert_eq!(
            observed.surface_types.as_ref(),
            Some(&expected_surfaces),
            "{}: analytic face surface types",
            case.id
        );

        // Measured in millimetres: the numbers below are the millimetre
        // metric values, so a missing or wrong unit declaration cannot pass.
        let volume = observed.volume.expect("volume reported");
        let area = observed.area.expect("area reported");
        assert_close(
            &format!("{} volume", case.id),
            volume,
            case.volume,
            REL_TOLERANCE,
        );
        assert_close(&format!("{} area", case.id), area, case.area, REL_TOLERANCE);
        assert_bounds(
            &format!("{} minimum", case.id),
            observed
                .bounds_min
                .as_deref()
                .expect("minimum bounds reported"),
            &case.bounds_min,
        );
        assert_bounds(
            &format!("{} maximum", case.id),
            observed
                .bounds_max
                .as_deref()
                .expect("maximum bounds reported"),
            &case.bounds_max,
        );

        // The current Python exporter must describe the same analytic solid.
        let reference = observed
            .reference
            .as_ref()
            .unwrap_or_else(|| panic!("{}: no Python reference metrics", case.id));
        assert_eq!(
            reference.valid,
            Some(true),
            "{}: the Python reference solid is invalid",
            case.id
        );
        assert_eq!(
            reference.solids,
            Some(1),
            "{}: Python reference solid count",
            case.id
        );
        assert_eq!(
            reference.surface_types.as_ref(),
            Some(&expected_surfaces),
            "{}: Python reference surface types",
            case.id
        );
        assert!(
            reference.bytes.unwrap_or(0) > 128,
            "{}: the Python reference produced no STEP bytes",
            case.id
        );
        assert_close(
            &format!("{} volume vs Python", case.id),
            volume,
            reference.volume.expect("reference volume"),
            REL_TOLERANCE,
        );
        assert_close(
            &format!("{} area vs Python", case.id),
            area,
            reference.area.expect("reference area"),
            REL_TOLERANCE,
        );
        let observed_min = observed
            .bounds_min
            .as_deref()
            .expect("minimum bounds reported");
        for (index, (native, python)) in observed_min
            .iter()
            .zip(reference.bounds_min.as_deref().expect("reference minimum"))
            .enumerate()
        {
            let difference = (native - python).abs();
            assert!(
                difference <= ABS_BOUND_TOLERANCE,
                "{}: axis {index} minimum {native} vs Python {python}",
                case.id
            );
        }
        let observed_max = observed
            .bounds_max
            .as_deref()
            .expect("maximum bounds reported");
        for (index, (native, python)) in observed_max
            .iter()
            .zip(reference.bounds_max.as_deref().expect("reference maximum"))
            .enumerate()
        {
            let difference = (native - python).abs();
            assert!(
                difference <= ABS_BOUND_TOLERANCE,
                "{}: axis {index} maximum {native} vs Python {python}",
                case.id
            );
        }
        println!(
            "  {} ok: {} faces {:?} volume {:.9} mm^3 area {:.9} mm^2",
            case.id, case.faces, expected_surfaces, volume, area
        );
    }

    // `workdir` removes itself when it drops, on success and on failure.
}
