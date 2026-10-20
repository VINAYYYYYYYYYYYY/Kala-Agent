#![cfg(feature = "native")]

//! Live differential for HLR drawings and section cuts.
//!
//! The oracle is the real Python Studio drawing module
//! (`src/kala_agent/studio/design/drawing.py`) run under the repository
//! `.venv` with the `OCP` bindings. Both implementations draw the same
//! re-imported STEP document, so a difference is always a difference between
//! the two implementations and never a difference between two constructions.
//!
//! The corpus covers:
//!
//! * native compiler shapes: the ADR L-bracket, a box with a through hole, a
//!   box with a bore and an island, a cylinder, a multi-additive part, a
//!   translated part, four frozen fixture cases, and two extra tolerance /
//!   point-cap variants of the L-bracket;
//! * Python-built copies of the three shapes in
//!   `tests/unit/studio_design/test_drawing.py`, which are written to STEP and
//!   re-imported before either implementation draws them;
//! * every explicit refusal in the Python module (unknown view/plane, bad
//!   chord tolerance, bad point limit, non-finite offset, null shape).
//!
//! A missing interpreter or missing `OCP` is a loud test failure, never a
//! skip. `KALA_CAD_ORACLE_PYTHON` selects another interpreter.

mod common;

use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::process::Command;

use kala_cad::{
    CompileOptions, DEFAULT_CHORD_TOLERANCE_MM, DEFAULT_SECTION_CHORD_TOLERANCE_MM, Drawing,
    DrawingError, DrawingErrorKind, MAX_POINTS_PER_VIEW, Point2, Section, Shape, compile_json_str,
    default_section_offset, hlr_views_with_options, measure_shape, section_with_options,
    step_bytes, step_shape,
};
use serde_json::{Value, json};

const ORACLE_VIEWS: [&str; 4] = ["front", "top", "right", "iso"];
type SectionSeed = (&'static str, Option<f64>, f64);

#[derive(Clone)]
struct SectionSpec {
    plane: String,
    offset_mm: Option<f64>,
    chord_tolerance_mm: f64,
}

fn default_sections(shape: &Shape) -> Vec<SectionSpec> {
    let measured = measure_shape(shape).expect("native measurement");
    let minimum = measured.bounds.min_mm;
    let maximum = measured.bounds.max_mm;
    let xz_offset = minimum[1] + 0.3 * (maximum[1] - minimum[1]);
    let xy_offset = 0.5 * (minimum[2] + maximum[2]);
    let yz_offset = 0.5 * (minimum[0] + maximum[0]);
    vec![
        SectionSpec {
            plane: "XZ".to_owned(),
            offset_mm: None,
            chord_tolerance_mm: DEFAULT_SECTION_CHORD_TOLERANCE_MM,
        },
        SectionSpec {
            plane: "XZ".to_owned(),
            offset_mm: Some(xz_offset),
            chord_tolerance_mm: DEFAULT_SECTION_CHORD_TOLERANCE_MM,
        },
        SectionSpec {
            plane: "XY".to_owned(),
            offset_mm: Some(xy_offset),
            chord_tolerance_mm: 0.02,
        },
        SectionSpec {
            plane: "YZ".to_owned(),
            offset_mm: Some(yz_offset),
            chord_tolerance_mm: DEFAULT_SECTION_CHORD_TOLERANCE_MM,
        },
    ]
}

fn section_json(shape: &Shape, spec: &SectionSpec) -> Section {
    section_with_options(shape, &spec.plane, spec.offset_mm, spec.chord_tolerance_mm)
        .unwrap_or_else(|error| {
            panic!(
                "native section {} {:?} failed: {}",
                spec.plane, spec.offset_mm, error
            )
        })
}

fn request_sections(specs: &[SectionSpec]) -> Value {
    Value::Array(
        specs
            .iter()
            .map(|spec| {
                json!({
                    "plane": spec.plane,
                    "offset_mm": spec.offset_mm,
                    "chord_tolerance_mm": spec.chord_tolerance_mm,
                })
            })
            .collect(),
    )
}

fn l_bracket_script() -> String {
    common::l_bracket_script()
}

fn box_with_hole_script() -> String {
    common::script(
        json!([{
            "name": "block",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XZ",
                 "width": 40.0, "height": 20.0, "x": -20.0, "y": 0.0},
                {"op": "extrude", "length": 30.0},
                {"op": "sketch", "profile": "circle", "plane": "XZ",
                 "diameter": 10.0, "x": 0.0, "y": 10.0},
                {"op": "pocket"}
            ]
        }]),
        json!([{"op": "place", "component": "block", "y": 30.0}]),
    )
}

fn box_with_island_script() -> String {
    common::script(
        json!([
            {
                "name": "block",
                "steps": [
                    {"op": "sketch", "profile": "rectangle", "plane": "XZ",
                     "width": 40.0, "height": 24.0, "x": -20.0, "y": 0.0},
                    {"op": "extrude", "length": 30.0},
                    {"op": "sketch", "profile": "circle", "plane": "XZ",
                     "diameter": 20.0, "x": 0.0, "y": 12.0},
                    {"op": "pocket"}
                ]
            },
            {
                "name": "island",
                "steps": [
                    {"op": "sketch", "profile": "circle", "plane": "XZ",
                     "diameter": 8.0, "x": 0.0, "y": 12.0},
                    {"op": "extrude", "length": 10.0, "reversed": true}
                ]
            }
        ]),
        json!([
            {"op": "place", "component": "block", "y": 30.0},
            {"op": "place", "component": "island", "y": 10.0}
        ]),
    )
}

fn cylinder_script() -> String {
    common::script(
        json!([{
            "name": "pin",
            "steps": [
                {"op": "sketch", "profile": "circle", "plane": "XY", "diameter": 6.0},
                {"op": "extrude", "length": 8.0}
            ]
        }]),
        json!([]),
    )
}

fn multi_additive_script() -> String {
    common::script(
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
    )
}

fn translated_script() -> String {
    common::script(
        json!([common::rectangle_component(
            "plate", "XY", 10.0, 10.0, 5.0, 0.0, 0.0, false
        )]),
        json!([{"op": "place", "component": "plate", "x": 5.0, "y": 6.0, "z": 7.0}]),
    )
}

fn sanitise(id: &str) -> String {
    id.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn fixture_scripts() -> Vec<(String, String)> {
    let fixture = common::fixture();
    fixture["cases"]
        .as_array()
        .expect("fixture cases")
        .iter()
        .take(4)
        .enumerate()
        .map(|(index, case)| {
            let label = case["id"].as_str().unwrap_or("case");
            (
                format!("fixture_{index}_{}", sanitise(label)),
                serde_json::to_string(&case["script"]).expect("fixture script"),
            )
        })
        .collect()
}

struct NativeCase {
    id: String,
    drawing: Drawing,
    sections: Vec<Section>,
    default_offset: f64,
    tolerance: f64,
}

struct OracleCase {
    id: String,
    step_path: PathBuf,
    views: Vec<String>,
    tolerance: f64,
    max_points: usize,
    sections: Vec<SectionSpec>,
}

fn oracle_python() -> PathBuf {
    common::oracle_python()
}

fn refusal_specs(shape: &Shape, shape_path: &Path) -> Vec<(Value, (String, String))> {
    let shape_path = shape_path.to_string_lossy().into_owned();
    let mut specs: Vec<(Value, (String, String))> = Vec::new();
    let mut add = |id: &str, call: Value, expected: Result<(), DrawingError>| {
        let (class_name, message) = match expected {
            Ok(()) => panic!("refusal {id} unexpectedly succeeded"),
            Err(error) => (
                python_error_type(error.kind()).to_owned(),
                error.message().to_owned(),
            ),
        };
        let mut spec = call;
        spec["id"] = json!(id);
        specs.push((spec, (class_name, message)));
    };

    add(
        "unknown_view",
        json!({
            "call": "hlr_views", "shape_path": shape_path,
            "views": ["side"], "chord_tolerance_mm": 0.05, "max_points": 20_000,
        }),
        hlr_views_with_options(shape, &["side"], 0.05, 20_000).map(|_| ()),
    );
    add(
        "unknown_view_second",
        json!({
            "call": "hlr_views", "shape_path": shape_path,
            "views": ["front", "side"], "chord_tolerance_mm": 0.05, "max_points": 20_000,
        }),
        hlr_views_with_options(shape, &["front", "side"], 0.05, 20_000).map(|_| ()),
    );
    add(
        "chord_zero",
        json!({
            "call": "hlr_views", "shape_path": shape_path,
            "views": ["front"], "chord_tolerance_mm": 0.0, "max_points": 20_000,
        }),
        hlr_views_with_options(shape, &["front"], 0.0, 20_000).map(|_| ()),
    );
    add(
        "chord_negative",
        json!({
            "call": "hlr_views", "shape_path": shape_path,
            "views": ["front"], "chord_tolerance_mm": -1.0, "max_points": 20_000,
        }),
        hlr_views_with_options(shape, &["front"], -1.0, 20_000).map(|_| ()),
    );
    add(
        "chord_nan",
        json!({
            "call": "hlr_views", "shape_path": shape_path,
            "views": ["front"], "chord_tolerance_mm": "nan", "max_points": 20_000,
        }),
        hlr_views_with_options(shape, &["front"], f64::NAN, 20_000).map(|_| ()),
    );
    add(
        "chord_inf",
        json!({
            "call": "hlr_views", "shape_path": shape_path,
            "views": ["front"], "chord_tolerance_mm": "inf", "max_points": 20_000,
        }),
        hlr_views_with_options(shape, &["front"], f64::INFINITY, 20_000).map(|_| ()),
    );
    add(
        "point_limit_zero",
        json!({
            "call": "hlr_views", "shape_path": shape_path,
            "views": ["front"], "chord_tolerance_mm": 0.05, "max_points": 0,
        }),
        hlr_views_with_options(shape, &["front"], 0.05, 0).map(|_| ()),
    );
    add(
        "unknown_plane",
        json!({
            "call": "section", "shape_path": shape_path,
            "plane": "AB", "offset_mm": 0.0, "chord_tolerance_mm": 0.01,
        }),
        section_with_options(shape, "AB", Some(0.0), 0.01).map(|_| ()),
    );
    add(
        "unknown_plane_lower",
        json!({
            "call": "section", "shape_path": shape_path,
            "plane": "ab", "offset_mm": 0.0, "chord_tolerance_mm": 0.01,
        }),
        section_with_options(shape, "ab", Some(0.0), 0.01).map(|_| ()),
    );
    add(
        "offset_nan",
        json!({
            "call": "section", "shape_path": shape_path,
            "plane": "XZ", "offset_mm": "nan", "chord_tolerance_mm": 0.01,
        }),
        section_with_options(shape, "XZ", Some(f64::NAN), 0.01).map(|_| ()),
    );
    add(
        "offset_inf",
        json!({
            "call": "section", "shape_path": shape_path,
            "plane": "XZ", "offset_mm": "inf", "chord_tolerance_mm": 0.01,
        }),
        section_with_options(shape, "XZ", Some(f64::INFINITY), 0.01).map(|_| ()),
    );
    add(
        "offset_neg_inf",
        json!({
            "call": "section", "shape_path": shape_path,
            "plane": "XZ", "offset_mm": "-inf", "chord_tolerance_mm": 0.01,
        }),
        section_with_options(shape, "XZ", Some(f64::NEG_INFINITY), 0.01).map(|_| ()),
    );

    // The null-shape cases need a second shape value, so they are computed
    // separately from the imported refusal shape.
    let null = Shape::null().expect("a null kernel handle");
    add(
        "null_shape_view",
        json!({
            "call": "hlr_views", "shape": "null",
            "views": ["front"], "chord_tolerance_mm": 0.05, "max_points": 20_000,
        }),
        hlr_views_with_options(&null, &["front"], 0.05, 20_000).map(|_| ()),
    );
    add(
        "null_shape_section",
        json!({
            "call": "section", "shape": "null",
            "plane": "XZ", "offset_mm": 0.0, "chord_tolerance_mm": 0.01,
        }),
        section_with_options(&null, "XZ", Some(0.0), 0.01).map(|_| ()),
    );
    add(
        "null_shape_offset",
        json!({"call": "default_section_offset", "shape": "null"}),
        default_section_offset(&null).map(|_| ()),
    );

    specs
}

fn python_error_type(kind: DrawingErrorKind) -> &'static str {
    match kind {
        DrawingErrorKind::Argument => "ValueError",
        DrawingErrorKind::Shape => "DrawingError",
        // Python has no stable wrapper for a kernel exception; these cases
        // must not occur in the refusal corpus.
        DrawingErrorKind::Kernel => "Standard_Failure",
        DrawingErrorKind::Unavailable => "DrawingError",
    }
}

#[test]
fn native_drawings_and_sections_match_the_python_oracle() {
    let python = oracle_python();
    assert!(
        python.is_file(),
        "the mandatory drawing oracle interpreter was not found at {}; set \
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

    let temp = common::TempDir::new("drawing-oracle");
    let mut shapes_request = Vec::new();
    let mut native_cases = Vec::new();

    let mut scripts = vec![
        ("l_bracket".to_owned(), l_bracket_script()),
        ("box_with_hole".to_owned(), box_with_hole_script()),
        ("box_with_island".to_owned(), box_with_island_script()),
        ("cylinder".to_owned(), cylinder_script()),
        ("multi_additive".to_owned(), multi_additive_script()),
        ("translated".to_owned(), translated_script()),
    ];
    scripts.extend(fixture_scripts());

    for (id, script) in scripts {
        let result = compile_json_str(&script, &CompileOptions::default())
            .unwrap_or_else(|error| panic!("{id} must compile: {error}"));
        let data = step_bytes(result.shape()).expect("native STEP export");
        let path = temp.path().join(format!("native-{id}.step"));
        std::fs::write(&path, &data).expect("write native STEP");
        // Both implementations draw the re-imported document.
        let shape = step_shape(&data).expect("native STEP re-import");
        let mut sections = default_sections(&shape);
        if id == "l_bracket" {
            // Case-insensitive plane names and two plane positions that lie
            // exactly on a part face, where the cut can be degenerate.
            sections.push(SectionSpec {
                plane: "xz".to_owned(),
                offset_mm: Some(4.0),
                chord_tolerance_mm: DEFAULT_SECTION_CHORD_TOLERANCE_MM,
            });
            sections.push(SectionSpec {
                plane: "XZ".to_owned(),
                offset_mm: Some(0.0),
                chord_tolerance_mm: DEFAULT_SECTION_CHORD_TOLERANCE_MM,
            });
        }
        let drawing = hlr_views_with_options(
            &shape,
            &ORACLE_VIEWS,
            DEFAULT_CHORD_TOLERANCE_MM,
            MAX_POINTS_PER_VIEW,
        )
        .expect("native HLR drawing");
        let offset = default_section_offset(&shape).expect("native default offset");
        shapes_request.push(json!({
            "id": id.clone(),
            "step_path": path,
            "views": ORACLE_VIEWS,
            "chord_tolerance_mm": DEFAULT_CHORD_TOLERANCE_MM,
            "max_points": MAX_POINTS_PER_VIEW,
            "sections": request_sections(&sections),
        }));
        native_cases.push(NativeCase {
            id,
            drawing,
            sections: sections
                .iter()
                .map(|spec| section_json(&shape, spec))
                .collect(),
            default_offset: offset,
            tolerance: DEFAULT_CHORD_TOLERANCE_MM,
        });
    }

    // Tolerance and point-cap variants of the L-bracket.
    for (id, views, tolerance, max_points) in [
        (
            "l_bracket_coarse",
            vec!["front"],
            0.5_f64,
            MAX_POINTS_PER_VIEW,
        ),
        (
            "l_bracket_fine",
            vec!["front"],
            0.001_f64,
            MAX_POINTS_PER_VIEW,
        ),
        ("l_bracket_truncated", vec!["front"], 0.05_f64, 5_usize),
    ] {
        let result = compile_json_str(&l_bracket_script(), &CompileOptions::default())
            .expect("the L-bracket compiles");
        let data = step_bytes(result.shape()).expect("native STEP export");
        let path = temp.path().join(format!("native-{id}.step"));
        std::fs::write(&path, &data).expect("write native STEP");
        let shape = step_shape(&data).expect("native STEP re-import");
        let drawing = hlr_views_with_options(&shape, &views, tolerance, max_points)
            .expect("native HLR drawing");
        let offset = default_section_offset(&shape).expect("native default offset");
        shapes_request.push(json!({
            "id": id,
            "step_path": path,
            "views": views,
            "chord_tolerance_mm": tolerance,
            "max_points": max_points,
            "sections": [],
        }));
        native_cases.push(NativeCase {
            id: id.to_owned(),
            drawing,
            sections: Vec::new(),
            default_offset: offset,
            tolerance,
        });
    }

    struct OracleSeed {
        id: &'static str,
        builder: &'static str,
        sections: Vec<SectionSeed>,
    }
    let mut max_point_movement = 0.0_f64;
    let mut oracle_cases = Vec::new();
    let mut oracle_request = Vec::new();
    let oracle_specs = [
        OracleSeed {
            id: "python_l_bracket",
            builder: "l_bracket",
            sections: vec![
                ("XZ", None, DEFAULT_SECTION_CHORD_TOLERANCE_MM),
                ("XZ", Some(4.0), DEFAULT_SECTION_CHORD_TOLERANCE_MM),
                ("XZ", Some(0.0), DEFAULT_SECTION_CHORD_TOLERANCE_MM),
                ("XZ", Some(15.0), DEFAULT_SECTION_CHORD_TOLERANCE_MM),
                ("XY", Some(4.0), DEFAULT_SECTION_CHORD_TOLERANCE_MM),
                ("YZ", Some(0.0), DEFAULT_SECTION_CHORD_TOLERANCE_MM),
            ],
        },
        OracleSeed {
            id: "python_box_with_hole",
            builder: "box_with_through_hole",
            sections: vec![
                ("XZ", None, DEFAULT_SECTION_CHORD_TOLERANCE_MM),
                ("XZ", Some(15.0), DEFAULT_SECTION_CHORD_TOLERANCE_MM),
                ("XY", Some(5.0), DEFAULT_SECTION_CHORD_TOLERANCE_MM),
                ("YZ", Some(0.0), DEFAULT_SECTION_CHORD_TOLERANCE_MM),
            ],
        },
        OracleSeed {
            id: "python_box_with_island",
            builder: "box_with_an_island_in_a_bore",
            sections: vec![
                ("XZ", None, DEFAULT_SECTION_CHORD_TOLERANCE_MM),
                ("XZ", Some(10.0), DEFAULT_SECTION_CHORD_TOLERANCE_MM),
                ("XZ", Some(4.0), DEFAULT_SECTION_CHORD_TOLERANCE_MM),
            ],
        },
    ];
    for OracleSeed {
        id,
        builder,
        sections,
    } in oracle_specs
    {
        let sections = sections
            .into_iter()
            .map(|(plane, offset_mm, chord_tolerance_mm)| SectionSpec {
                plane: plane.to_owned(),
                offset_mm,
                chord_tolerance_mm,
            })
            .collect::<Vec<_>>();
        let step_path = temp.path().join(format!("oracle-{id}.step"));
        let views = ORACLE_VIEWS
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>();
        oracle_request.push(json!({
            "id": id,
            "builder": builder,
            "step_path": step_path,
            "views": views,
            "chord_tolerance_mm": DEFAULT_CHORD_TOLERANCE_MM,
            "max_points": MAX_POINTS_PER_VIEW,
            "sections": request_sections(&sections),
        }));
        oracle_cases.push(OracleCase {
            id: id.to_owned(),
            step_path,
            views,
            tolerance: DEFAULT_CHORD_TOLERANCE_MM,
            max_points: MAX_POINTS_PER_VIEW,
            sections,
        });
    }

    let refusal_shape_data = {
        let result = compile_json_str(&l_bracket_script(), &CompileOptions::default())
            .expect("the L-bracket compiles");
        step_bytes(result.shape()).expect("native STEP export")
    };
    let refusal_path = temp.path().join("refusal-shape.step");
    std::fs::write(&refusal_path, &refusal_shape_data).expect("write refusal STEP");
    let refusal_shape = step_shape(&refusal_shape_data).expect("refusal STEP re-import");
    let refusals = refusal_specs(&refusal_shape, &refusal_path);

    let request = json!({
        "shapes": shapes_request,
        "oracle_shapes": oracle_request,
        "refusals": refusals.iter().map(|(spec, _)| spec.clone()).collect::<Vec<_>>(),
    });
    let request_path = temp.path().join("request.json");
    let result_path = temp.path().join("result.json");
    std::fs::write(
        &request_path,
        serde_json::to_vec_pretty(&request).expect("request JSON"),
    )
    .expect("write request");

    let output = Command::new(&python)
        .arg(common::project_root().join("backend/crates/kala-cad/tests/oracle/drawing_oracle.py"))
        .arg(&request_path)
        .arg(&result_path)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("run the drawing oracle");
    let result_bytes = std::fs::read(&result_path).unwrap_or_else(|error| {
        panic!(
            "the drawing oracle produced no result file ({error}); status {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let result: Value = serde_json::from_slice(&result_bytes).expect("parse oracle result");
    assert!(
        result["ok"].as_bool().unwrap_or(false),
        "the drawing oracle failed:\n{}",
        serde_json::to_string_pretty(&result["mismatches"]).expect("mismatches")
    );
    eprintln!(
        "drawing oracle: {}",
        serde_json::to_string_pretty(&result["checked"]).expect("checked")
    );
    eprintln!(
        "drawing oracle versions: {}",
        serde_json::to_string_pretty(&result["versions"]).expect("versions")
    );
    eprintln!(
        "drawing oracle source digests: {}",
        serde_json::to_string_pretty(&result["source_digests"]).expect("digests")
    );

    let checked = &result["checked"];
    assert!(checked["shapes"].as_u64().unwrap_or(0) >= 13);
    assert!(checked["views"].as_u64().unwrap_or(0) >= 50);
    assert!(checked["sections"].as_u64().unwrap_or(0) >= 50);
    assert!(checked["oracle_shapes"].as_u64().unwrap_or(0) >= 3);
    assert!(checked["refusals"].as_u64().unwrap_or(0) >= 15);

    let oracle_shapes = result["shapes"].as_array().expect("shapes array");
    let oracle_by_id = oracle_shapes
        .iter()
        .map(|payload| {
            (
                payload["id"].as_str().expect("shape id").to_owned(),
                payload,
            )
        })
        .collect::<BTreeMap<String, &Value>>();
    for case in &native_cases {
        let payload = oracle_by_id
            .get(&case.id)
            .unwrap_or_else(|| panic!("missing oracle shape {}", case.id));
        max_point_movement = max_point_movement.max(assert_drawing_matches(
            &case.id,
            &case.drawing,
            payload,
            case.tolerance,
        ));
        let sections = payload["sections"].as_array().expect("sections");
        assert_eq!(sections.len(), case.sections.len(), "{}", case.id);
        for (native, oracle) in case.sections.iter().zip(sections) {
            max_point_movement =
                max_point_movement.max(assert_section_matches(&case.id, native, oracle));
        }
        assert_close(
            case.default_offset,
            payload["default_section_offset"].as_f64().expect("offset"),
            1e-9,
            &format!("{} default offset", case.id),
        );
    }

    let oracle_shapes = result["oracle_shapes"]
        .as_array()
        .expect("oracle shapes array");
    let oracle_by_id = oracle_shapes
        .iter()
        .map(|payload| {
            (
                payload["id"].as_str().expect("shape id").to_owned(),
                payload,
            )
        })
        .collect::<BTreeMap<String, &Value>>();
    for case in &oracle_cases {
        let payload = oracle_by_id
            .get(&case.id)
            .unwrap_or_else(|| panic!("missing oracle shape {}", case.id));
        let data = std::fs::read(&case.step_path).expect("read oracle STEP");
        let shape = step_shape(&data).expect("native STEP re-import");
        let drawn = case.views.iter().map(String::as_str).collect::<Vec<_>>();
        let drawing = hlr_views_with_options(&shape, &drawn, case.tolerance, case.max_points)
            .expect("native HLR drawing");
        max_point_movement = max_point_movement.max(assert_drawing_matches(
            &case.id,
            &drawing,
            payload,
            case.tolerance,
        ));
        let sections = payload["sections"].as_array().expect("sections");
        assert_eq!(sections.len(), case.sections.len(), "{}", case.id);
        let native_sections = case
            .sections
            .iter()
            .map(|spec| section_json(&shape, spec))
            .collect::<Vec<_>>();
        for (native, oracle) in native_sections.iter().zip(sections) {
            max_point_movement =
                max_point_movement.max(assert_section_matches(&case.id, native, oracle));
        }
        assert_close(
            default_section_offset(&shape).expect("native default offset"),
            payload["default_section_offset"].as_f64().expect("offset"),
            1e-9,
            &format!("{} default offset", case.id),
        );
        // The explicit Python-test expectations, checked on the native cut of
        // the exact Python shape.
        match case.id.as_str() {
            "python_l_bracket" => {
                let cut = native_sections
                    .iter()
                    .find(|section| section.plane.as_str() == "XZ" && section.offset_mm == 4.0)
                    .expect("the L-bracket XZ@4 section");
                assert_eq!(cut.outer_loop_count(), 1);
                assert_eq!(cut.inner_loop_count(), 1);
                let outer = cut.outer_loops().next().unwrap();
                let inner = cut.inner_loops().next().unwrap();
                assert_relative(outer.area_mm2, 2400.0, 1e-9, "L-bracket outer area");
                assert_relative(inner.area_mm2, 25.0 * PI, 1e-9, "L-bracket bore area");
                assert_relative(cut.area_mm2, 2400.0 - 25.0 * PI, 1e-9, "L-bracket net area");
                assert!(outer.area_exact && inner.area_exact && cut.area_exact);
            }
            "python_box_with_hole" => {
                let cut = native_sections
                    .iter()
                    .find(|section| section.plane.as_str() == "XY" && section.offset_mm == 5.0)
                    .expect("the tangent section");
                assert_eq!(cut.loops.len(), 1);
                assert_relative(cut.area_mm2, 1200.0, 1e-9, "tangent area");
                assert!(!cut.area_exact);
                assert!(!cut.loops[0].area_exact);
            }
            "python_box_with_island" => {
                let cut = native_sections
                    .iter()
                    .find(|section| section.plane.as_str() == "XZ" && section.offset_mm == 10.0)
                    .expect("the island section");
                assert_eq!(cut.outer_loop_count(), 2);
                assert_eq!(cut.inner_loop_count(), 1);
                assert_relative(
                    cut.area_mm2,
                    800.0 - 100.0 * PI + 16.0 * PI,
                    1e-9,
                    "island net area",
                );
                assert!(cut.area_exact);
                assert!(cut.loops.iter().all(|loop_| loop_.area_exact));
            }
            _ => {}
        }
    }

    eprintln!("drawing oracle maximum sampled point movement: {max_point_movement} mm");

    let refusal_results = result["refusals"].as_array().expect("refusals array");
    let refusal_by_id = refusal_results
        .iter()
        .map(|payload| {
            (
                payload["id"].as_str().expect("refusal id").to_owned(),
                payload,
            )
        })
        .collect::<BTreeMap<String, &Value>>();
    for (spec, (expected_type, expected_message)) in &refusals {
        let id = spec["id"].as_str().expect("refusal id");
        let payload = refusal_by_id
            .get(id)
            .unwrap_or_else(|| panic!("missing refusal {id}"));
        assert_eq!(
            payload["error_type"].as_str(),
            Some(expected_type.as_str()),
            "refusal {id} type"
        );
        assert_eq!(
            payload["message"].as_str(),
            Some(expected_message.as_str()),
            "refusal {id} message"
        );
    }
}

fn assert_drawing_matches(id: &str, native: &Drawing, payload: &Value, tolerance: f64) -> f64 {
    let mut movement = 0.0_f64;
    let views = payload["views"].as_array().expect("views");
    assert_eq!(views.len(), native.views.len(), "{id} view count");
    for (view, oracle) in native.views.iter().zip(views) {
        let label = format!("{id}/{}", view.name.as_str());
        assert_eq!(
            oracle["name"].as_str(),
            Some(view.name.as_str()),
            "{label} name"
        );
        assert_eq!(
            oracle["point_count"].as_u64().unwrap_or(u64::MAX),
            view.point_count() as u64,
            "{label} point count"
        );
        assert_eq!(
            oracle["truncated"].as_bool(),
            Some(view.truncated),
            "{label} truncated"
        );
        movement = movement.max(assert_polylines(
            &format!("{label}/visible"),
            &view.visible,
            oracle["visible"].as_array().expect("visible"),
            tolerance,
        ));
        movement = movement.max(assert_polylines(
            &format!("{label}/hidden"),
            &view.hidden,
            oracle["hidden"].as_array().expect("hidden"),
            tolerance,
        ));
    }
    movement
}

fn assert_polylines(
    label: &str,
    native: &[kala_cad::Polyline],
    oracle: &[Value],
    bound: f64,
) -> f64 {
    let mut movement = 0.0_f64;
    assert_eq!(oracle.len(), native.len(), "{label} polyline count");
    for (index, (line, payload)) in native.iter().zip(oracle).enumerate() {
        assert_eq!(
            payload["closed"].as_bool(),
            Some(line.closed),
            "{label}[{index}] closed"
        );
        let points = payload["points"].as_array().expect("points");
        assert_eq!(
            points.len(),
            line.points.len(),
            "{label}[{index}] point count"
        );
        for (point, payload_point) in line.points.iter().zip(points) {
            let pair = payload_point.as_array().expect("point pair");
            let u = pair[0].as_f64().expect("u");
            let v = pair[1].as_f64().expect("v");
            let distance = point.distance(Point2::new(u, v));
            movement = movement.max(distance);
            assert!(
                distance <= bound,
                "{label}[{index}] point moved by {distance}: native ({}, {}) oracle ({u}, {v})",
                point.u,
                point.v
            );
        }
    }
    // The polyline structure matched above; also verify the point-set
    // Hausdorff bound the task requires across the whole classification.
    let native_cloud = native
        .iter()
        .flat_map(|line| line.points.iter().copied())
        .collect::<Vec<_>>();
    let oracle_cloud = oracle
        .iter()
        .flat_map(|payload| payload["points"].as_array().into_iter().flatten())
        .map(|point| {
            let pair = point.as_array().expect("point pair");
            Point2::new(pair[0].as_f64().expect("u"), pair[1].as_f64().expect("v"))
        })
        .collect::<Vec<_>>();
    if !native_cloud.is_empty() && !oracle_cloud.is_empty() {
        let distance = symmetric_hausdorff(&native_cloud, &oracle_cloud);
        assert!(
            distance <= bound,
            "{label} point-set Hausdorff distance {distance} exceeds {bound}"
        );
    }
    movement
}

fn symmetric_hausdorff(left: &[Point2], right: &[Point2]) -> f64 {
    directed_hausdorff(left, right).max(directed_hausdorff(right, left))
}

fn directed_hausdorff(left: &[Point2], right: &[Point2]) -> f64 {
    left.iter()
        .map(|point| {
            right
                .iter()
                .map(|other| point.distance(*other))
                .fold(f64::INFINITY, f64::min)
        })
        .fold(0.0, f64::max)
}

fn assert_section_matches(id: &str, native: &Section, payload: &Value) -> f64 {
    let mut movement = 0.0_f64;
    let label = format!("{id}/{}@{:?}", native.plane.as_str(), native.offset_mm);
    assert_eq!(
        payload["plane"].as_str(),
        Some(native.plane.as_str()),
        "{label} plane"
    );
    assert_close(
        payload["offset_mm"].as_f64().expect("offset"),
        native.offset_mm,
        1e-9,
        &format!("{label} offset"),
    );
    assert_close(
        payload["chord_tolerance_mm"].as_f64().expect("tolerance"),
        native.chord_tolerance_mm,
        0.0,
        &format!("{label} tolerance"),
    );
    assert_eq!(
        payload["truncated"].as_bool(),
        Some(native.truncated),
        "{label} truncated"
    );
    assert_eq!(
        payload["area_exact"].as_bool(),
        Some(native.area_exact),
        "{label} area_exact"
    );
    assert_area(
        payload["area_mm2"].as_f64().expect("area"),
        native.area_mm2,
        &format!("{label} net area"),
    );
    let loops = payload["loops"].as_array().expect("loops");
    assert_eq!(loops.len(), native.loops.len(), "{label} loop count");
    for (index, (loop_, oracle)) in native.loops.iter().zip(loops).enumerate() {
        let loop_label = format!("{label} loop {index}");
        assert_eq!(
            oracle["outer"].as_bool(),
            Some(loop_.outer),
            "{loop_label} outer"
        );
        assert_eq!(
            oracle["area_exact"].as_bool(),
            Some(loop_.area_exact),
            "{loop_label} area_exact"
        );
        assert_area(
            oracle["area_mm2"].as_f64().expect("loop area"),
            loop_.area_mm2,
            &format!("{loop_label} area"),
        );
        let points = oracle["points"].as_array().expect("loop points");
        assert_eq!(points.len(), loop_.points.len(), "{loop_label} point count");
        let mut oracle_min_u = f64::INFINITY;
        let mut oracle_min_v = f64::INFINITY;
        let mut oracle_max_u = f64::NEG_INFINITY;
        let mut oracle_max_v = f64::NEG_INFINITY;
        for (point, payload_point) in loop_.points.iter().zip(points) {
            let pair = payload_point.as_array().expect("point pair");
            let u = pair[0].as_f64().expect("u");
            let v = pair[1].as_f64().expect("v");
            oracle_min_u = oracle_min_u.min(u);
            oracle_min_v = oracle_min_v.min(v);
            oracle_max_u = oracle_max_u.max(u);
            oracle_max_v = oracle_max_v.max(v);
            let distance = point.distance(Point2::new(u, v));
            movement = movement.max(distance);
            assert!(
                distance <= native.chord_tolerance_mm,
                "{loop_label} point moved by {distance}"
            );
        }
        let bounds = loop_.bounds().expect("a loop has points");
        assert_close(
            oracle_min_u,
            bounds.min_u,
            1e-6,
            &format!("{loop_label} min_u"),
        );
        assert_close(
            oracle_min_v,
            bounds.min_v,
            1e-6,
            &format!("{loop_label} min_v"),
        );
        assert_close(
            oracle_max_u,
            bounds.max_u,
            1e-6,
            &format!("{loop_label} max_u"),
        );
        assert_close(
            oracle_max_v,
            bounds.max_v,
            1e-6,
            &format!("{loop_label} max_v"),
        );
    }
    movement
}

fn assert_area(actual: f64, expected: f64, label: &str) {
    if expected.abs() < 1e-12 {
        assert!(
            actual.abs() <= 1e-12,
            "{label}: {actual} is not a zero area"
        );
    } else {
        let difference = (actual - expected).abs();
        assert!(
            difference <= 1e-9 * expected.abs(),
            "{label}: {actual} differs from {expected} by {difference}"
        );
    }
}

fn assert_close(actual: f64, expected: f64, absolute: f64, label: &str) {
    assert!(
        (actual - expected).abs() <= absolute,
        "{label}: {actual} differs from {expected} by more than {absolute}"
    );
}

fn assert_relative(actual: f64, expected: f64, relative: f64, label: &str) {
    let difference = (actual - expected).abs();
    assert!(
        difference <= relative * expected.abs(),
        "{label}: {actual} differs from {expected} by {difference} (relative tolerance {relative})"
    );
}
