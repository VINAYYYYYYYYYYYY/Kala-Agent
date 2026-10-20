#![cfg(feature = "native")]

//! Optional second-reader differential: FreeCAD 1.1.4 re-imports native STEP
//! and STL files with its own application-level reader.
//!
//! The mandatory reader remains the OCP oracle in `ocp_differential.rs`. This
//! test is skipped only when no FreeCAD executable exists; if one exists and
//! fails, the test fails. It never claims Windows or production execution.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use kala_cad::{
    CompileOptions, MeshData, MeshOptions, Shape, measure_shape, step_bytes, stl_bytes, tessellate,
};
use serde_json::{Value, json};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("kala-cad-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn measurement_json(shape: &Shape) -> Value {
    let measured = measure_shape(shape).expect("native measurement");
    json!({
        "volume": measured.volume_mm3,
        "area": measured.surface_area_mm2,
        "min": measured.bounds.min_mm,
        "max": measured.bounds.max_mm,
        "valid": measured.valid,
        "solid_count": measured.solid_count,
    })
}

fn mesh_json(mesh: &MeshData) -> Value {
    let mut minimum = [f64::INFINITY; 3];
    let mut maximum = [f64::NEG_INFINITY; 3];
    for point in mesh.positions.chunks_exact(3) {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(point[axis]);
            maximum[axis] = maximum[axis].max(point[axis]);
        }
    }
    json!({
        "vertex_count": mesh.vertex_count(),
        "triangle_count": mesh.triangle_count(),
        "min": minimum,
        "max": maximum,
    })
}

fn python_literal(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy().to_string()).expect("path literal")
}

fn freecad_binary() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("KALA_CAD_FREECAD") {
        return Some(PathBuf::from(value));
    }
    let probe = Command::new("freecadcmd")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match probe {
        Ok(status) if status.success() => Some(PathBuf::from("freecadcmd")),
        _ => None,
    }
}

fn compile(script: &str) -> kala_cad::CompilationResult {
    kala_cad::compile_json_str(script, &CompileOptions::default()).expect("native compile")
}

#[test]
fn freecad_reimports_native_step_and_stl_geometry() {
    let Some(freecad) = freecad_binary() else {
        eprintln!(
            "FreeCAD is not installed on this host; the optional second-reader check was not executed. \
             The mandatory OCP oracle still ran."
        );
        return;
    };

    let hole_plate = common::script(
        json!([{
            "name": "plate",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 20.0, "height": 20.0},
                {"op": "extrude", "length": 10.0},
                {"op": "sketch", "profile": "circle", "plane": "XY",
                 "diameter": 2.0, "x": 5.0, "y": 5.0},
                {"op": "pocket"}
            ]
        }]),
        json!([]),
    );
    let compound = common::script(
        json!([
            {
                "name": "a",
                "steps": [
                    {"op": "sketch", "profile": "rectangle", "plane": "XY",
                     "width": 10.0, "height": 10.0},
                    {"op": "extrude", "length": 2.0}
                ]
            },
            {
                "name": "b",
                "steps": [
                    {"op": "sketch", "profile": "rectangle", "plane": "XY",
                     "width": 10.0, "height": 10.0},
                    {"op": "extrude", "length": 2.0}
                ]
            }
        ]),
        json!([
            {"op": "place", "component": "a", "x": 0.0},
            {"op": "place", "component": "b", "x": 15.0}
        ]),
    );
    let cylinder = common::script(
        json!([{
            "name": "pin",
            "steps": [
                {"op": "sketch", "profile": "circle", "plane": "XY", "diameter": 6.0},
                {"op": "extrude", "length": 8.0}
            ]
        }]),
        json!([]),
    );

    let temp = TempDir::new("freecad");
    let step_scripts = [
        ("l_bracket", common::l_bracket_script()),
        ("compound", compound),
        ("hole_plate", hole_plate),
    ];
    let mut steps = Vec::new();
    for (id, script) in step_scripts {
        let result = compile(&script);
        let data = step_bytes(&result.assembly_shape).expect("native STEP");
        let path = temp.path().join(format!("native-{id}.step"));
        std::fs::write(&path, &data).expect("write STEP");
        steps.push(json!({
            "id": id,
            "path": path,
            "native": measurement_json(&result.assembly_shape),
        }));
    }

    let stl_scripts = [
        ("box", common::box_script(20.0, 20.0, 10.0)),
        ("cylinder", cylinder),
    ];
    let mut stls = Vec::new();
    for (id, script) in stl_scripts {
        let result = compile(&script);
        let mesh = tessellate(&result.assembly_shape, MeshOptions::default()).expect("mesh");
        let data = stl_bytes(&mesh, kala_cad::STL_HEADER_FILL).expect("native STL");
        let path = temp.path().join(format!("native-{id}.stl"));
        std::fs::write(&path, &data).expect("write STL");
        stls.push(json!({
            "id": id,
            "path": path,
            "native": mesh_json(&mesh),
        }));
    }

    let request_path = temp.path().join("freecad-request.json");
    let result_path = temp.path().join("freecad-result.json");
    std::fs::write(
        &request_path,
        serde_json::to_vec_pretty(&json!({"steps": steps, "stls": stls})).expect("request JSON"),
    )
    .expect("write request");

    let oracle =
        common::project_root().join("backend/crates/kala-cad/tests/oracle/freecad_oracle.py");
    let driver = format!(
        "import sys\nsys.argv=[{name}, {request}, {result}]\nexec(compile(open({oracle}).read(), {oracle}, 'exec'))\n",
        name = python_literal(Path::new("kala-cad-freecad")),
        request = python_literal(&request_path),
        result = python_literal(&result_path),
        oracle = python_literal(&oracle),
    );

    let mut child = Command::new(&freecad)
        .arg("-c")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start FreeCADCmd");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(driver.as_bytes())
        .expect("write FreeCAD driver");
    let output = child.wait_with_output().expect("wait for FreeCADCmd");

    let result_bytes = std::fs::read(&result_path).unwrap_or_else(|error| {
        panic!(
            "FreeCAD produced no result file ({error}); status {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let result: Value = serde_json::from_slice(&result_bytes).expect("parse FreeCAD result");
    eprintln!(
        "FreeCAD oracle: {}",
        serde_json::to_string_pretty(&result["checked"]).expect("checked")
    );
    eprintln!(
        "FreeCAD versions: {}",
        serde_json::to_string_pretty(&result["versions"]).expect("versions")
    );
    assert!(
        result["ok"].as_bool().unwrap_or(false),
        "FreeCAD re-import mismatches:\n{}",
        serde_json::to_string_pretty(&result["mismatches"]).expect("mismatches")
    );
    assert!(result["checked"]["steps"].as_u64().unwrap_or(0) >= 3);
    assert!(result["checked"]["stls"].as_u64().unwrap_or(0) >= 2);
}
