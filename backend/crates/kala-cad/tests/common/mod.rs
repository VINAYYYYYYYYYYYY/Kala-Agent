//! Shared helpers for the native CAD test suites.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use kala_cad::{Bounds, CompilationResult, CompileOptions, Measurement};
use serde_json::{Value, json};

pub fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("backend/crates/kala-cad is three levels below the repository root")
        .to_path_buf()
}

pub fn fixture_path() -> PathBuf {
    project_root()
        .join("tests")
        .join("fixtures")
        .join("construction_parity.json")
}

pub fn fixture() -> Value {
    let path = fixture_path();
    assert!(
        path.is_file(),
        "the frozen parity fixture is missing: {}; it is read-only and must not be regenerated here",
        path.display()
    );
    let bytes = std::fs::read(&path).expect("read the parity fixture");
    serde_json::from_slice(&bytes).expect("parse the parity fixture")
}

pub fn compile_json(script: &str) -> CompilationResult {
    kala_cad::compile_json_str(script, &CompileOptions::default())
        .expect("the script should compile")
}

pub fn measure(result: &CompilationResult) -> Measurement {
    kala_cad::measure_shape(result.shape()).expect("the assembly should measure")
}

pub fn assert_volume(actual: f64, expected: f64, relative: f64, label: &str) {
    let difference = (actual - expected).abs();
    assert!(
        difference <= relative * expected.abs(),
        "{label}: volume {actual} differs from {expected} by {difference} (relative tolerance {relative})"
    );
}

pub fn assert_bounds(
    measured: &Bounds,
    minimum: [f64; 3],
    maximum: [f64; 3],
    tolerance: f64,
    label: &str,
) {
    for axis in 0..3 {
        assert!(
            (measured.min_mm[axis] - minimum[axis]).abs() <= tolerance,
            "{label}: minimum axis {axis} was {} not {} (tolerance {tolerance})",
            measured.min_mm[axis],
            minimum[axis]
        );
        assert!(
            (measured.max_mm[axis] - maximum[axis]).abs() <= tolerance,
            "{label}: maximum axis {axis} was {} not {} (tolerance {tolerance})",
            measured.max_mm[axis],
            maximum[axis]
        );
    }
}

pub fn script(components: Value, assembly: Value) -> String {
    serde_json::to_string(&json!({
        "summary": "native cad test",
        "components": components,
        "assembly": assembly,
    }))
    .expect("serialise the script")
}

#[allow(clippy::too_many_arguments)]
pub fn rectangle_component(
    name: &str,
    plane: &str,
    width: f64,
    height: f64,
    length: f64,
    x: f64,
    y: f64,
    reversed: bool,
) -> Value {
    json!({
        "name": name,
        "steps": [
            {
                "op": "sketch",
                "profile": "rectangle",
                "plane": plane,
                "width": width,
                "height": height,
                "x": x,
                "y": y,
            },
            {"op": "extrude", "length": length, "reversed": reversed},
        ],
    })
}

pub fn rectangle_script(
    plane: &str,
    width: f64,
    height: f64,
    length: f64,
    x: f64,
    y: f64,
    reversed: bool,
) -> String {
    script(
        json!([rectangle_component(
            "plate", plane, width, height, length, x, y, reversed
        )]),
        json!([]),
    )
}

pub fn l_bracket_script() -> String {
    let mut base_steps = vec![
        json!({
            "op": "sketch",
            "profile": "rectangle",
            "plane": "XY",
            "width": 60.0,
            "height": 45.0,
            "x": -30.0,
            "y": 0.0,
        }),
        json!({"op": "extrude", "length": 8.0}),
    ];
    for y in [16.25, 36.75] {
        for x in [-21.75, 21.75] {
            base_steps.push(json!({
                "op": "sketch",
                "profile": "circle",
                "plane": "XY",
                "diameter": 5.5,
                "x": x,
                "y": y,
            }));
            base_steps.push(json!({"op": "pocket"}));
        }
    }
    let upright_steps = vec![
        json!({
            "op": "sketch",
            "profile": "rectangle",
            "plane": "XZ",
            "width": 60.0,
            "height": 40.0,
            "x": -30.0,
            "y": 0.0,
        }),
        json!({"op": "extrude", "length": 8.0}),
        json!({
            "op": "sketch",
            "profile": "circle",
            "plane": "XZ",
            "diameter": 10.0,
            "x": 0.0,
            "y": 24.0,
        }),
        json!({"op": "pocket"}),
    ];
    script(
        json!([
            {"name": "base", "steps": base_steps},
            {"name": "upright", "steps": upright_steps},
        ]),
        json!([
            {"op": "place", "component": "upright", "y": 8.0},
            {"op": "fuse", "components": ["base", "upright"]},
        ]),
    )
}

pub fn box_script(size_x: f64, size_y: f64, size_z: f64) -> String {
    rectangle_script("XY", size_x, size_y, size_z, 0.0, 0.0, false)
}

/// The repository `.venv` interpreter used by every live differential.
///
/// `KALA_CAD_ORACLE_PYTHON` overrides it. A missing interpreter is a loud test
/// failure in the differential suites, never a silent skip.
pub fn oracle_python() -> PathBuf {
    if let Ok(value) = std::env::var("KALA_CAD_ORACLE_PYTHON") {
        return PathBuf::from(value);
    }
    project_root().join(".venv").join("bin").join("python")
}

/// A private temporary directory removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("kala-cad-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
