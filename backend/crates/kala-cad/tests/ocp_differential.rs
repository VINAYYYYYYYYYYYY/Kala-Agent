#![cfg(feature = "native")]

//! Independent OpenCascade differential for the native compiler, measurement,
//! mesh and STEP/STL exchange.
//!
//! The oracle is the real Python Studio source (`compile_ocp.py`, `measure.py`,
//! `mesh.py`) run under the repository `.venv` with the `OCP` bindings. Every
//! frozen fixture case plus L-bracket/multi-feature/topology cases is compiled
//! by both implementations; native STEP and STL files are read back by
//! OpenCascade itself.
//!
//! The oracle is mandatory for this native gate. A missing interpreter or
//! missing `OCP` is a loud test failure, never a skip.
//! `KALA_CAD_ORACLE_PYTHON` selects another interpreter; `KALA_CAD_SKIP_OCP_ORACLE=1`
//! is not supported on purpose.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use kala_cad::{
    CompileOptions, DEFAULT_MAX_STEP_BYTES, MeshData, MeshOptions, Shape, StepImportOptions,
    StepImportOutcome, compile_json_str, import_step_view, measure_shape, normalise_step_bytes,
    step_bytes, stl_bytes, tessellate, topology,
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

fn oracle_python() -> PathBuf {
    if let Ok(value) = std::env::var("KALA_CAD_ORACLE_PYTHON") {
        return PathBuf::from(value);
    }
    common::project_root()
        .join(".venv")
        .join("bin")
        .join("python")
}

fn measurement_json(shape: &Shape) -> Value {
    let measured = measure_shape(shape).expect("native measurement");
    json!({
        "volume": measured.volume_mm3,
        "area": measured.surface_area_mm2,
        "centroid": measured.centroid_mm,
        "min": measured.bounds.min_mm,
        "max": measured.bounds.max_mm,
        "valid": measured.valid,
        "solid_count": measured.solid_count,
    })
}

fn topology_json(shape: &Shape) -> Value {
    let counts = topology(shape).expect("native topology");
    json!({
        "solids": counts.solids,
        "shells": counts.shells,
        "faces": counts.faces,
        "edges": counts.edges,
        "vertices": counts.vertices,
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
        "all_finite": mesh.positions.iter().all(|value| value.is_finite())
            && mesh.normals.iter().all(|value| value.is_finite()),
        "deflection_mm": mesh.deflection_mm,
        "angular_deflection_rad": mesh.angular_deflection_rad,
    })
}

fn import_view_json(outcome: &StepImportOutcome) -> Value {
    json!({
        "min": outcome.bounds.min_mm,
        "max": outcome.bounds.max_mm,
        "solids": outcome.solids,
        "warnings": outcome.warnings,
        "vertex_count": outcome.mesh.vertex_count(),
        "triangle_count": outcome.mesh.triangle_count(),
        "coarsened": outcome.coarsened,
    })
}

fn native_case(id: &str, script: &str) -> Value {
    let result = kala_cad::compile_json_str(script, &CompileOptions::default())
        .unwrap_or_else(|error| panic!("{id} must compile: {error}"));
    let parts = result
        .components
        .iter()
        .map(|component| {
            json!({
                "name": component.name,
                "measurement": measurement_json(&component.local_shape),
                "topology": topology_json(&component.local_shape),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "id": id,
        "script": serde_json::from_str::<Value>(script).unwrap(),
        "native": {
            "parts": parts,
            "assembly": measurement_json(&result.assembly_shape),
            "assembly_topology": topology_json(&result.assembly_shape),
            "warnings": result.warnings,
            "fused": result.fused,
        },
    })
}

fn extra_scripts() -> Vec<(&'static str, String)> {
    let multi_additive = common::script(
        json!([{
            "name": "part",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 20.0, "height": 10.0},
                {"op": "extrude", "length": 5.0},
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 5.0, "height": 5.0, "x": 2.0, "y": 2.0},
                {"op": "extrude", "length": 10.0}
            ]
        }]),
        json!([]),
    );
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
    let translated = common::script(
        json!([common::rectangle_component(
            "plate", "XY", 10.0, 10.0, 5.0, 0.0, 0.0, false
        )]),
        json!([{"op": "place", "component": "plate", "x": 5.0, "y": 6.0, "z": 7.0}]),
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
    let reversed = common::rectangle_script("XY", 4.0, 5.0, 6.0, 2.0, 3.0, true);
    let no_op_pocket = common::script(
        json!([{
            "name": "plate",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 20.0, "height": 20.0},
                {"op": "extrude", "length": 10.0, "reversed": true},
                {"op": "sketch", "profile": "circle", "plane": "XY",
                 "diameter": 2.0, "x": 5.0, "y": 5.0},
                {"op": "pocket", "length": 4.0}
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
    vec![
        ("l_bracket", common::l_bracket_script()),
        ("multi_additive", multi_additive),
        ("hole_plate", hole_plate),
        ("translated", translated),
        ("cylinder", cylinder),
        ("reversed", reversed),
        ("no_op_pocket", no_op_pocket),
        ("compound", compound),
    ]
}

/// A tiny deterministic LCG. The generated corpus is reproducible on every
/// host and in every test run; no external RNG or clock participates.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }

    fn range(&mut self, low: i64, high: i64) -> i64 {
        debug_assert!(low <= high);
        low + i64::from(self.next()) % (high - low + 1)
    }

    fn chance(&mut self, numerator: u32, denominator: u32) -> bool {
        self.next() % denominator < numerator
    }

    fn pick<'a, T>(&mut self, values: &'a [T]) -> &'a T {
        &values[self.next() as usize % values.len()]
    }
}

/// Deterministic randomized valid construction scripts.
///
/// Every case uses only the v1 subset and stays inside the frozen contract
/// bounds. The generated operations cover XY/XZ/YZ planes, forward and
/// reversed extrusions, circular and rectangular profiles, through-all and
/// finite pockets, additive fuses, placements, and unfused compounds.
fn random_scripts(count: usize, seed: u64) -> Vec<(String, String)> {
    let mut rng = Lcg::new(seed);
    let planes = ["XY", "XZ", "YZ"];
    let mut cases = Vec::with_capacity(count);
    for case_index in 0..count {
        let component_count = rng.range(1, 3) as usize;
        let mut components = Vec::new();
        for component_index in 0..component_count {
            let plane = *rng.pick(&planes);
            let circle = rng.chance(1, 3);
            let width = rng.range(6, 24) as f64;
            let height = rng.range(5, 20) as f64;
            let diameter = rng.range(4, 18) as f64;
            let x = rng.range(-6, 4) as f64;
            let y = rng.range(-6, 4) as f64;
            let length = rng.range(2, 12) as f64;
            let reversed = rng.chance(1, 4);
            let mut steps = Vec::new();
            if circle {
                steps.push(json!({
                    "op": "sketch", "profile": "circle", "plane": plane,
                    "diameter": diameter, "x": x, "y": y,
                }));
            } else {
                steps.push(json!({
                    "op": "sketch", "profile": "rectangle", "plane": plane,
                    "width": width, "height": height, "x": x, "y": y,
                }));
            }
            steps.push(json!({"op": "extrude", "length": length, "reversed": reversed}));

            if !circle {
                let pocket_count = rng.range(0, 2);
                for pocket_index in 0..pocket_count {
                    let pocket_diameter = (width.min(height) / 2.0 - 0.5).clamp(1.0, 8.0);
                    // A centre at 30% / 70% of the plate keeps the whole cutter
                    // inside the profile for both pockets.
                    let fraction = if pocket_index == 0 { 0.3 } else { 0.7 };
                    let u = x + width * fraction;
                    let v = y + height * 0.5;
                    steps.push(json!({
                        "op": "sketch", "profile": "circle", "plane": plane,
                        "diameter": pocket_diameter, "x": u, "y": v,
                    }));
                    if rng.chance(1, 2) {
                        steps.push(json!({"op": "pocket", "length": rng.range(1, 8) as f64}));
                    } else {
                        steps.push(json!({"op": "pocket"}));
                    }
                }
            }

            if rng.chance(1, 3) {
                // A second additive feature on the same plane, fused into the
                // component by the compiler.
                let second_width = (width / 2.0).max(2.0);
                let second_height = (height / 2.0).max(2.0);
                steps.push(json!({
                    "op": "sketch", "profile": "rectangle", "plane": plane,
                    "width": second_width, "height": second_height,
                    "x": x + 1.0, "y": y + 1.0,
                }));
                steps.push(json!({
                    "op": "extrude", "length": rng.range(1, 6) as f64,
                }));
            }

            components.push(json!({
                "name": format!("part{component_index}"),
                "steps": steps,
            }));
        }

        let mut assembly = Vec::new();
        for component_index in 0..component_count {
            assembly.push(json!({
                "op": "place",
                "component": format!("part{component_index}"),
                "x": rng.range(-8, 8) as f64,
                "y": rng.range(-8, 8) as f64,
                "z": rng.range(-8, 8) as f64,
            }));
        }
        if component_count >= 2 && rng.chance(1, 2) {
            let member_count = rng.range(2, component_count as i64) as usize;
            let members = (0..member_count)
                .map(|index| json!(format!("part{index}")))
                .collect::<Vec<_>>();
            assembly.push(json!({"op": "fuse", "components": members}));
        }

        let script = common::script(json!(components), json!(assembly));
        cases.push((format!("random_{case_index:02}"), script));
    }
    cases
}

struct RefusalSpec {
    id: &'static str,
    script: String,
    analysis: Option<&'static str>,
    edits: Option<&'static str>,
    fillets: Option<&'static str>,
}

fn native_refusal(spec: &RefusalSpec) -> String {
    let analysis = spec
        .analysis
        .map(|text| kala_contracts::load_analysis_json_str(text).expect("analysis"));
    let reading = spec.edits.map(|edits| {
        kala_contracts::load_reading_json_str(&format!(
            r#"{{"verdict":"apply","tool_edit":{edits},"rewrite":null}}"#
        ))
        .expect("reading")
    });
    let fillets = spec.fillets.map(|text| {
        let value: Value = serde_json::from_str(text).expect("fillets");
        value
            .as_object()
            .expect("fillets object")
            .iter()
            .map(|(name, value)| (name.clone(), value.as_f64().expect("fillet radius")))
            .collect::<std::collections::BTreeMap<_, _>>()
    });
    let options = CompileOptions {
        analysis: analysis.as_ref(),
        edits: reading.as_ref().map_or(&[], |reading| reading.tool_edit()),
        fillets: fillets.as_ref(),
        ..CompileOptions::default()
    };
    compile_json_str(&spec.script, &options)
        .expect_err("the refusal case must fail")
        .message()
        .to_owned()
}

fn refusal_specs() -> Vec<RefusalSpec> {
    let helix = common::script(
        json!([{
            "name": "screw",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "width": 4.0, "height": 4.0},
                {"op": "extrude", "length": 10.0},
                {"op": "helix", "diameter": 4.0, "pitch": 1.0, "length": 5.0}
            ]
        }]),
        json!([]),
    );
    let compound = common::script(
        json!([
            common::rectangle_component("a", "XY", 20.0, 20.0, 10.0, 0.0, 0.0, false),
            common::rectangle_component("b", "XY", 20.0, 20.0, 10.0, 0.0, 0.0, false),
        ]),
        json!([{"op": "compound", "components": ["a", "b"]}]),
    );
    let xz_revolve = common::script(
        json!([{
            "name": "wheel",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XZ",
                 "width": 4.0, "height": 5.0},
                {"op": "revolve"}
            ]
        }]),
        json!([]),
    );
    let cross_plane = common::script(
        json!([{
            "name": "plate",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 20.0, "height": 20.0},
                {"op": "extrude", "length": 10.0},
                {"op": "sketch", "profile": "circle", "plane": "XZ",
                 "diameter": 2.0, "x": 5.0, "y": 5.0},
                {"op": "pocket"}
            ]
        }]),
        json!([]),
    );
    let duplicate_place = common::script(
        json!([common::rectangle_component(
            "plate", "XY", 20.0, 20.0, 10.0, 0.0, 0.0, false
        )]),
        json!([
            {"op": "place", "component": "plate", "x": 1.0},
            {"op": "place", "component": "plate", "x": 2.0}
        ]),
    );
    let cut_all = common::script(
        json!([{
            "name": "plate",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 10.0, "height": 10.0},
                {"op": "extrude", "length": 5.0},
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 20.0, "height": 20.0},
                {"op": "pocket"}
            ]
        }]),
        json!([]),
    );
    let plate = common::box_script(20.0, 20.0, 10.0);
    vec![
        RefusalSpec {
            id: "helix",
            script: helix,
            analysis: None,
            edits: None,
            fillets: None,
        },
        RefusalSpec {
            id: "compound",
            script: compound,
            analysis: None,
            edits: None,
            fillets: None,
        },
        RefusalSpec {
            id: "xz_revolve",
            script: xz_revolve,
            analysis: None,
            edits: None,
            fillets: None,
        },
        RefusalSpec {
            id: "cross_plane_pocket",
            script: cross_plane,
            analysis: None,
            edits: None,
            fillets: None,
        },
        RefusalSpec {
            id: "duplicate_place",
            script: duplicate_place,
            analysis: None,
            edits: None,
            fillets: None,
        },
        RefusalSpec {
            id: "cut_all",
            script: cut_all,
            analysis: None,
            edits: None,
            fillets: None,
        },
        RefusalSpec {
            id: "analysis_material",
            script: plate.clone(),
            analysis: Some(r#"{"steps":[{"op":"material"},{"op":"mesh"},{"op":"solve"}]}"#),
            edits: None,
            fillets: None,
        },
        RefusalSpec {
            id: "analysis_fix",
            script: plate.clone(),
            analysis: Some(r#"{"steps":[{"op":"fix"},{"op":"solve"}]}"#),
            edits: None,
            fillets: None,
        },
        RefusalSpec {
            id: "analysis_force",
            script: plate.clone(),
            analysis: Some(r#"{"steps":[{"op":"force","force_n":10.0},{"op":"solve"}]}"#),
            edits: None,
            fillets: None,
        },
        RefusalSpec {
            id: "analysis_pressure",
            script: plate.clone(),
            analysis: Some(r#"{"steps":[{"op":"pressure","pressure_mpa":1.0},{"op":"solve"}]}"#),
            edits: None,
            fillets: None,
        },
        RefusalSpec {
            id: "analysis_mesh",
            script: plate.clone(),
            analysis: Some(r#"{"steps":[{"op":"mesh"},{"op":"solve"}]}"#),
            edits: None,
            fillets: None,
        },
        RefusalSpec {
            id: "analysis_solve",
            script: plate.clone(),
            analysis: Some(r#"{"steps":[{"op":"solve"}]}"#),
            edits: None,
            fillets: None,
        },
        RefusalSpec {
            id: "edit_move",
            script: plate.clone(),
            analysis: None,
            edits: Some(r#"[{"op":"move","component":"plate","x":1.0}]"#),
            fillets: None,
        },
        RefusalSpec {
            id: "edit_resize",
            script: plate.clone(),
            analysis: None,
            edits: Some(r#"[{"op":"resize","component":"plate","length":5.0}]"#),
            fillets: None,
        },
        RefusalSpec {
            id: "edit_fillet",
            script: plate.clone(),
            analysis: None,
            edits: Some(r#"[{"op":"fillet","component":"plate","radius":0.5}]"#),
            fillets: None,
        },
        RefusalSpec {
            id: "fillet",
            script: plate,
            analysis: None,
            edits: None,
            fillets: Some(r#"{"plate":1.0}"#),
        },
    ]
}

#[test]
fn native_results_match_the_mandatory_ocp_oracle() {
    let python = oracle_python();
    assert!(
        python.is_file(),
        "the mandatory OCP oracle interpreter was not found at {}; set \
         KALA_CAD_ORACLE_PYTHON. This gate fails rather than skipping.",
        python.display()
    );
    let probe = Command::new(&python)
        .arg("-c")
        .arg("import OCP; print(getattr(OCP, '__version__', 'unknown'))")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("start the oracle interpreter");
    assert!(
        probe.status.success(),
        "the oracle interpreter {} cannot import OCP:\n{}",
        python.display(),
        String::from_utf8_lossy(&probe.stderr)
    );

    let fixture = common::fixture();
    let mut cases = fixture["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .map(|case| {
            native_case(
                case["id"].as_str().expect("case id"),
                &serde_json::to_string(&case["script"]).expect("script"),
            )
        })
        .collect::<Vec<_>>();
    let extras = extra_scripts();
    for (id, script) in &extras {
        cases.push(native_case(id, script));
    }
    // A deterministic randomized corpus: no clock, no external RNG, and every
    // case is compiled by both implementations.
    let random = random_scripts(24, 0x4b41_4c41_4341_4421);
    for (id, script) in &random {
        cases.push(native_case(id, script));
    }

    let refusals = refusal_specs()
        .into_iter()
        .map(|spec| {
            let native_error = native_refusal(&spec);
            json!({
                "id": spec.id,
                "script": serde_json::from_str::<Value>(&spec.script).unwrap(),
                "analysis": spec.analysis.map(|text| serde_json::from_str::<Value>(text).unwrap()),
                "edits": spec.edits.map(|text| serde_json::from_str::<Value>(text).unwrap()).unwrap_or_else(|| json!([])),
                "fillets": spec.fillets.map(|text| serde_json::from_str::<Value>(text).unwrap()),
                "native_error": native_error,
            })
        })
        .collect::<Vec<_>>();

    let temp = TempDir::new("oracle");
    let mesh_ids = [
        "box",
        "cylinder",
        "l_bracket",
        "hole_plate",
        "multi_additive",
    ];
    let mut meshes = Vec::new();
    for id in mesh_ids {
        let script = if id == "box" {
            common::box_script(20.0, 20.0, 10.0)
        } else {
            extras
                .iter()
                .find(|(name, _)| *name == id)
                .map(|(_, script)| script.clone())
                .unwrap_or_else(|| panic!("missing mesh script {id}"))
        };
        let result = kala_cad::compile_json_str(&script, &CompileOptions::default()).unwrap();
        let mesh = tessellate(&result.assembly_shape, MeshOptions::default()).unwrap();
        meshes.push(json!({
            "id": id,
            "script": serde_json::from_str::<Value>(&script).unwrap(),
            "native": mesh_json(&mesh),
        }));
    }

    let mut steps = Vec::new();
    let mut imports = Vec::new();
    let mut import_failures = Vec::new();
    let mut normalisations = Vec::new();
    let mut l_bracket_step: Option<(Vec<u8>, PathBuf)> = None;
    for id in [
        "l_bracket",
        "compound",
        "hole_plate",
        "translated",
        "multi_additive",
    ] {
        let script = extras
            .iter()
            .find(|(name, _)| *name == id)
            .map(|(_, script)| script.clone())
            .unwrap_or_else(|| panic!("missing STEP script {id}"));
        let result = kala_cad::compile_json_str(&script, &CompileOptions::default()).unwrap();
        let data = step_bytes(&result.assembly_shape).unwrap();
        let path = temp.path().join(format!("native-{id}.step"));
        std::fs::write(&path, &data).expect("write STEP");
        steps.push(json!({
            "id": id,
            "path": path,
            "native_measurement": measurement_json(&result.assembly_shape),
            "native_topology": topology_json(&result.assembly_shape),
        }));

        // The view-only importer must agree with `importer.py` on the exact
        // bounds, solid count, warning list and mesh counts.
        let view = import_step_view(&data, StepImportOptions::default())
            .unwrap_or_else(|error| panic!("{id} native import failed: {error}"));
        imports.push(json!({
            "id": id,
            "path": path,
            "max_vertices": kala_cad::MAX_MESH_VERTICES,
            "max_triangles": kala_cad::MAX_MESH_TRIANGLES,
            "native": import_view_json(&view),
        }));

        let normalised = normalise_step_bytes(&data);
        assert_eq!(normalised.len(), data.len(), "{id} normalisation length");
        let normalised_path = temp.path().join(format!("native-{id}.normalised.step"));
        std::fs::write(&normalised_path, &normalised).expect("write normalised STEP");
        normalisations.push(json!({
            "id": id,
            "raw_path": path,
            "native_path": normalised_path,
        }));

        if id == "l_bracket" {
            l_bracket_step = Some((data, path));
        }
    }

    // A real surface-only STEP file exercises the honest "no solids" warning.
    let surface_fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("non_solid_surface.step");
    let surface_data = std::fs::read(&surface_fixture).expect("read the surface STEP fixture");
    let surface_view = import_step_view(&surface_data, StepImportOptions::default())
        .expect("the surface STEP fixture must import for display");
    imports.push(json!({
        "id": "non_solid_surface",
        "path": surface_fixture,
        "max_vertices": kala_cad::MAX_MESH_VERTICES,
        "max_triangles": kala_cad::MAX_MESH_TRIANGLES,
        "native": import_view_json(&surface_view),
    }));

    let (l_bracket_data, l_bracket_path) =
        l_bracket_step.expect("the L-bracket STEP must have been written");
    let failure_cases: [(String, &str, Vec<u8>, u64); 4] = [
        (
            "empty".to_owned(),
            "empty",
            Vec::new(),
            DEFAULT_MAX_STEP_BYTES,
        ),
        (
            "bad_header".to_owned(),
            "bad_header",
            b"not a step file".to_vec(),
            DEFAULT_MAX_STEP_BYTES,
        ),
        (
            "header_only".to_owned(),
            "header_only",
            b"ISO-10303-21;\nHEADER;\n".to_vec(),
            DEFAULT_MAX_STEP_BYTES,
        ),
        (
            "truncated".to_owned(),
            "truncated",
            l_bracket_data[..l_bracket_data.len() / 2].to_vec(),
            DEFAULT_MAX_STEP_BYTES,
        ),
    ];
    for (id, tag, data, max_step_bytes) in failure_cases {
        let path = temp.path().join(format!("failure-{tag}.step"));
        std::fs::write(&path, &data).expect("write failure STEP");
        let error = import_step_view(
            &data,
            StepImportOptions {
                max_step_bytes,
                ..StepImportOptions::default()
            },
        )
        .expect_err("the failure case must be refused");
        import_failures.push(json!({
            "id": id,
            "path": path,
            "max_step_bytes": max_step_bytes,
            "native_error": {"code": error.code(), "message": error.message()},
        }));
    }
    let too_large_limit = (l_bracket_data.len() - 1) as u64;
    let too_large_error = import_step_view(
        &l_bracket_data,
        StepImportOptions {
            max_step_bytes: too_large_limit,
            ..StepImportOptions::default()
        },
    )
    .expect_err("one byte below the document size must be refused");
    import_failures.push(json!({
        "id": "too_large",
        "path": l_bracket_path,
        "max_step_bytes": too_large_limit,
        "native_error": {"code": too_large_error.code(), "message": too_large_error.message()},
    }));

    let mut stls = Vec::new();
    for id in ["box", "cylinder", "l_bracket", "hole_plate"] {
        let script = if id == "box" {
            common::box_script(20.0, 20.0, 10.0)
        } else {
            extras
                .iter()
                .find(|(name, _)| *name == id)
                .map(|(_, script)| script.clone())
                .unwrap_or_else(|| panic!("missing STL script {id}"))
        };
        let result = kala_cad::compile_json_str(&script, &CompileOptions::default()).unwrap();
        let mesh = tessellate(&result.assembly_shape, MeshOptions::default()).unwrap();
        let data = stl_bytes(&mesh, kala_cad::STL_HEADER_FILL).unwrap();
        let path = temp.path().join(format!("native-{id}.stl"));
        std::fs::write(&path, &data).expect("write STL");
        stls.push(json!({
            "id": id,
            "path": path,
            "native_mesh": mesh_json(&mesh),
        }));
    }

    let request = json!({
        "cases": cases,
        "refusals": refusals,
        "meshes": meshes,
        "steps": steps,
        "stls": stls,
        "imports": imports,
        "import_failures": import_failures,
        "normalisations": normalisations,
    });
    let request_path = temp.path().join("request.json");
    let result_path = temp.path().join("result.json");
    std::fs::write(
        &request_path,
        serde_json::to_vec_pretty(&request).expect("request JSON"),
    )
    .expect("write request");

    let output = Command::new(&python)
        .arg(common::project_root().join("backend/crates/kala-cad/tests/oracle/ocp_oracle.py"))
        .arg(&request_path)
        .arg(&result_path)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("run the OCP oracle");
    let result_bytes = std::fs::read(&result_path).unwrap_or_else(|error| {
        panic!(
            "the mandatory OCP oracle produced no result file ({error}); status {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let result: Value = serde_json::from_slice(&result_bytes).expect("parse oracle result");
    if !output.status.success() && result["ok"].as_bool().unwrap_or(false) {
        panic!(
            "the OCP oracle reported success but exited with {:?}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    eprintln!(
        "OCP oracle: {}",
        serde_json::to_string_pretty(&result["checked"]).expect("checked")
    );
    eprintln!(
        "OCP oracle versions: {}",
        serde_json::to_string_pretty(&result["versions"]).expect("versions")
    );
    eprintln!(
        "OCP oracle source digests: {}",
        serde_json::to_string_pretty(&result["source_digests"]).expect("digests")
    );
    assert!(
        result["ok"].as_bool().unwrap_or(false),
        "OCP oracle found mismatches:\n{}",
        serde_json::to_string_pretty(&result["mismatches"]).expect("mismatches")
    );

    let checked = &result["checked"];
    assert!(
        checked["cases"].as_u64().unwrap_or(0) >= 60,
        "the oracle checked too few cases: {checked}"
    );
    assert!(
        checked["parts"].as_u64().unwrap_or(0) >= 50,
        "the oracle checked too few parts: {checked}"
    );
    assert!(checked["refusals"].as_u64().unwrap_or(0) >= 16);
    assert!(checked["meshes"].as_u64().unwrap_or(0) >= 5);
    assert!(checked["steps"].as_u64().unwrap_or(0) >= 5);
    assert!(checked["stls"].as_u64().unwrap_or(0) >= 4);
    assert!(checked["imports"].as_u64().unwrap_or(0) >= 6);
    assert!(checked["import_failures"].as_u64().unwrap_or(0) >= 5);
    assert!(checked["normalisations"].as_u64().unwrap_or(0) >= 5);
}
