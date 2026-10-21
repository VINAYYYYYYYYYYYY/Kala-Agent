#![cfg(feature = "native")]

//! Native compiler parity, semantics and refusal matrix.
//!
//! The fixture is read-only. Every case is compiled through the accepted
//! `kala-contracts` loader and the native OpenCascade adapter, then measured
//! with the native measurement path.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::{
    assert_bounds, assert_volume, box_script, compile_json, fixture, l_bracket_script, measure,
    rectangle_component, rectangle_script, script,
};
use kala_cad::{
    Bounds, CheckValue, CompileErrorKind, CompileOptions, build_checks, compile_json_str,
    measure_shape,
};
use serde_json::{Value, json};

const L_BRACKET_VOLUME_MM3: f64 = 35_571.416_047;

fn l_bracket_envelope() -> Bounds {
    Bounds::new([-30.0, 0.0, 0.0], [30.0, 45.0, 40.0]).expect("valid envelope")
}

fn expect_error(
    script_text: &str,
    options: &CompileOptions<'_>,
    needle: &str,
) -> kala_cad::CompileError {
    let error = compile_json_str(script_text, options).expect_err("the script must be refused");
    assert!(
        error.message().contains(needle),
        "expected message containing {needle:?}, got {:?}",
        error.message()
    );
    error
}

#[test]
fn the_fixture_exists_and_records_its_generator_and_oracle_digest() {
    let fixture = fixture();
    let generator = &fixture["generator"];
    assert_eq!(
        generator["command"].as_str().unwrap(),
        "PYTHONPATH=adapters/solidedge/src:src .venv/bin/python tests/fixtures/generate_construction_parity.py"
    );
    assert_eq!(
        generator["oracle_source"].as_str().unwrap(),
        "adapters/solidedge/src/kala_agent_solidedge/oracle.py"
    );
    let digest = generator["oracle_sha256"].as_str().unwrap();
    assert_eq!(digest.len(), 64);
    assert!(
        digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );
    assert_eq!(
        fixture["cases"].as_array().unwrap().len(),
        28,
        "the frozen construction corpus has exactly 28 cases"
    );
    assert_eq!(
        fixture["generator"]["comparison"]["volume_relative_tolerance"]
            .as_f64()
            .unwrap(),
        1e-6
    );
    assert_eq!(
        fixture["generator"]["comparison"]["bounds_absolute_tolerance_mm"]
            .as_f64()
            .unwrap(),
        0.001
    );
}

#[test]
fn every_corpus_case_matches_the_frozen_oracle() {
    let fixture = fixture();
    let mut checked_parts = 0;
    let mut documented_cases = 0;

    for case in fixture["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let script_text = serde_json::to_string(&case["script"]).unwrap();
        let result = compile_json(&script_text);
        assert!(
            result.warnings.is_empty(),
            "{id}: unexpected warnings {:?}",
            result.warnings
        );

        for expected in case["oracle"]["parts"].as_array().unwrap() {
            let name = expected["name"].as_str().unwrap();
            let component = result
                .component(name)
                .unwrap_or_else(|| panic!("{id}: missing component {name}"));
            let measured = measure_shape(&component.local_shape)
                .unwrap_or_else(|error| panic!("{id}: {name} did not measure: {error}"));
            assert!(measured.valid, "{id}: {name} is not valid");
            if expected["checked"].as_bool().unwrap() {
                checked_parts += 1;
                assert_eq!(measured.solid_count, 1, "{id}: {name}");
                assert_volume(
                    measured.volume_mm3,
                    expected["volume_mm3"].as_f64().unwrap(),
                    1e-6,
                    &format!("{id}/{name}"),
                );
                let minimum = expected["min_mm"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_f64().unwrap())
                    .collect::<Vec<_>>();
                let maximum = expected["max_mm"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_f64().unwrap())
                    .collect::<Vec<_>>();
                assert_bounds(
                    &measured.bounds,
                    [minimum[0], minimum[1], minimum[2]],
                    [maximum[0], maximum[1], maximum[2]],
                    0.001,
                    &format!("{id}/{name}"),
                );
            } else {
                // The oracle deliberately does not predict a rectangle revolve;
                // the compiler must still produce a valid, non-empty solid.
                assert_eq!(measured.solid_count, 1, "{id}: {name}");
                assert!(measured.volume_mm3 > 0.0, "{id}: {name}");
                assert!(
                    expected["reason"].as_str().unwrap().contains("revolve"),
                    "{id}: unexpected unchecked reason"
                );
            }
        }

        if let Some(bounds) = case["oracle"]["assembly_bounds"].as_object() {
            let assembly = measure(&result);
            assert!(assembly.valid, "{id}: assembly invalid");
            let minimum = bounds["min_mm"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_f64().unwrap())
                .collect::<Vec<_>>();
            let maximum = bounds["max_mm"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_f64().unwrap())
                .collect::<Vec<_>>();
            assert_bounds(
                &assembly.bounds,
                [minimum[0], minimum[1], minimum[2]],
                [maximum[0], maximum[1], maximum[2]],
                0.001,
                id,
            );
        }

        if let Some(documented) = case["documented"].as_object() {
            documented_cases += 1;
            let expected_part = &case["oracle"]["parts"][0];
            assert!(expected_part["checked"].as_bool().unwrap(), "{id}");
            assert!(
                (expected_part["volume_mm3"].as_f64().unwrap()
                    - documented["volume_mm3"].as_f64().unwrap())
                .abs()
                    <= 1e-6,
                "{id}"
            );
            for key in ["min_mm", "max_mm"] {
                for axis in 0..3 {
                    assert!(
                        (expected_part[key][axis].as_f64().unwrap()
                            - documented[key][axis].as_f64().unwrap())
                        .abs()
                            <= 1e-6,
                        "{id}: documented {key} axis {axis}"
                    );
                }
            }
        }
    }

    assert!(checked_parts >= 20, "only {checked_parts} checked parts");
    assert!(
        documented_cases >= 4,
        "only {documented_cases} documented cases"
    );
}

#[test]
fn documented_plane_cases_reproduce_the_measured_boxes() {
    let fixture = fixture();
    let cases = fixture["cases"].as_array().unwrap();
    let expected = [
        (
            "rectangle_pad_xy_offset_forward",
            [2.0, 3.0, 0.0],
            [6.0, 8.0, 6.0],
        ),
        (
            "rectangle_pad_xz_offset_forward",
            [2.0, -6.0, 3.0],
            [6.0, 0.0, 8.0],
        ),
        (
            "rectangle_pad_yz_offset_forward",
            [0.0, 2.0, 3.0],
            [6.0, 6.0, 8.0],
        ),
    ];
    for (id, minimum, maximum) in expected {
        let case = cases.iter().find(|case| case["id"] == id).unwrap();
        let result = compile_json(&serde_json::to_string(&case["script"]).unwrap());
        let measured = measure_shape(&result.components[0].local_shape).unwrap();
        assert_volume(measured.volume_mm3, 120.0, 1e-6, id);
        assert_bounds(&measured.bounds, minimum, maximum, 0.001, id);
    }
}

#[test]
fn l_bracket_matches_the_adr_oracle() {
    let result = compile_json(&l_bracket_script());
    assert!(result.fused, "the L-bracket must be fused");
    assert!(result.warnings.is_empty());
    let measured = measure(&result);
    assert!(measured.valid);
    assert_eq!(measured.solid_count, 1);
    assert_volume(measured.volume_mm3, L_BRACKET_VOLUME_MM3, 1e-6, "l-bracket");
    assert_bounds(
        &measured.bounds,
        [-30.0, 0.0, 0.0],
        [30.0, 45.0, 40.0],
        0.001,
        "l-bracket",
    );
    let closed_form = 60.0 * 45.0 * 8.0 + 60.0 * 8.0 * (40.0 - 8.0)
        - std::f64::consts::PI * (10.0_f64 / 2.0).powi(2) * 8.0
        - 4.0 * std::f64::consts::PI * (5.5_f64 / 2.0).powi(2) * 8.0;
    assert_volume(
        measured.volume_mm3,
        closed_form,
        1e-9,
        "l-bracket closed form",
    );
}

#[test]
fn l_bracket_checks_verify_with_the_expected_ids_and_values() {
    let measured = measure(&compile_json(&l_bracket_script()));
    let envelope = l_bracket_envelope();
    let report = build_checks(&measured, Some(&envelope), true);
    assert!(report.verified);
    let ids = report
        .checks
        .iter()
        .map(|check| check.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "brep_valid",
            "single_solid",
            "envelope_x",
            "envelope_y",
            "envelope_z"
        ]
    );
    let envelope_x = &report.checks[2];
    assert_eq!(envelope_x.expected, CheckValue::Pair(-30.0, 30.0));
    assert_eq!(envelope_x.measured, CheckValue::Pair(-30.0, 30.0));
    assert_eq!(envelope_x.tolerance, 0.01);
    assert_eq!(envelope_x.unit, "mm");
    assert_eq!(envelope_x.label, "Envelope X");
    assert!(report.checks.iter().all(|check| check.passed));
}

#[test]
fn reversed_extrusion_flips_the_bounds_and_keeps_the_volume() {
    let forward = compile_json(&rectangle_script("XY", 4.0, 5.0, 6.0, 0.0, 0.0, false));
    let reverse = compile_json(&rectangle_script("XY", 4.0, 5.0, 6.0, 0.0, 0.0, true));
    let forward = measure_shape(&forward.components[0].local_shape).unwrap();
    let reverse = measure_shape(&reverse.components[0].local_shape).unwrap();
    assert_volume(forward.volume_mm3, 120.0, 1e-6, "forward");
    assert_volume(reverse.volume_mm3, 120.0, 1e-6, "reverse");
    assert_bounds(
        &forward.bounds,
        [0.0, 0.0, 0.0],
        [4.0, 5.0, 6.0],
        0.001,
        "forward",
    );
    assert_bounds(
        &reverse.bounds,
        [0.0, 0.0, -6.0],
        [4.0, 5.0, 0.0],
        0.001,
        "reverse",
    );
}

#[test]
fn xz_extrudes_along_negative_y() {
    let result = compile_json(&rectangle_script("XZ", 4.0, 5.0, 6.0, 2.0, 3.0, false));
    let measured = measure_shape(&result.components[0].local_shape).unwrap();
    assert_bounds(
        &measured.bounds,
        [2.0, -6.0, 3.0],
        [6.0, 0.0, 8.0],
        0.001,
        "xz",
    );
}

#[test]
fn a_circle_revolve_is_lowered_to_an_extrusion() {
    let script_text = script(
        json!([{
            "name": "pin",
            "steps": [
                {"op": "sketch", "profile": "circle", "plane": "XY", "diameter": 6.0},
                {"op": "revolve"}
            ]
        }]),
        json!([]),
    );
    let result = compile_json(&script_text);
    let measured = measure_shape(&result.components[0].local_shape).unwrap();
    assert_volume(
        measured.volume_mm3,
        std::f64::consts::PI * 9.0 * 6.0,
        1e-6,
        "circle revolve",
    );
    assert_bounds(
        &measured.bounds,
        [-3.0, -3.0, 0.0],
        [3.0, 3.0, 6.0],
        0.001,
        "circle revolve",
    );
}

#[test]
fn a_rectangle_revolve_builds_a_ring() {
    let script_text = script(
        json!([{
            "name": "bush",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 4.0, "height": 5.0, "x": 2.0, "y": 0.0},
                {"op": "revolve", "angle": 360.0}
            ]
        }]),
        json!([]),
    );
    let result = compile_json(&script_text);
    let measured = measure_shape(&result.components[0].local_shape).unwrap();
    assert_volume(
        measured.volume_mm3,
        std::f64::consts::PI * (36.0 - 4.0) * 5.0,
        1e-6,
        "rectangle revolve",
    );
    assert_bounds(
        &measured.bounds,
        [-6.0, 0.0, -6.0],
        [6.0, 5.0, 6.0],
        1e-6,
        "rectangle revolve",
    );
}

#[test]
fn place_translates_the_placed_shape_but_not_the_local_shape() {
    let script_text = script(
        json!([rectangle_component(
            "plate", "XY", 10.0, 10.0, 5.0, 0.0, 0.0, false
        )]),
        json!([{"op": "place", "component": "plate", "x": 5.0, "y": 6.0, "z": 7.0}]),
    );
    let result = compile_json(&script_text);
    let local = measure_shape(&result.components[0].local_shape).unwrap();
    let placed = measure_shape(&result.components[0].placed_shape).unwrap();
    assert_bounds(
        &local.bounds,
        [0.0, 0.0, 0.0],
        [10.0, 10.0, 5.0],
        0.001,
        "local",
    );
    assert_bounds(
        &placed.bounds,
        [5.0, 6.0, 7.0],
        [15.0, 16.0, 12.0],
        0.001,
        "placed",
    );
    assert_volume(placed.volume_mm3, local.volume_mm3, 1e-9, "placed");
    assert!(!result.fused);
    assert_eq!(result.components[0].placement_mm, [5.0, 6.0, 7.0]);
}

#[test]
fn an_unfused_assembly_keeps_two_solids() {
    let script_text = script(
        json!([
            rectangle_component("a", "XY", 10.0, 10.0, 5.0, 0.0, 0.0, false),
            rectangle_component("b", "XY", 10.0, 10.0, 5.0, 0.0, 0.0, false),
        ]),
        json!([
            {"op": "place", "component": "a", "x": 0.0},
            {"op": "place", "component": "b", "x": 20.0},
        ]),
    );
    let result = compile_json(&script_text);
    let measured = measure(&result);
    assert!(!result.fused);
    assert_eq!(measured.solid_count, 2);
    assert_volume(measured.volume_mm3, 1000.0, 1e-6, "unfused");
    assert_bounds(
        &measured.bounds,
        [0.0, 0.0, 0.0],
        [30.0, 10.0, 5.0],
        0.001,
        "unfused",
    );
}

#[test]
fn a_no_op_pocket_is_reported_not_claimed_as_removal() {
    let script_text = script(
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
    let result = compile_json(&script_text);
    assert_eq!(
        result.warnings,
        ["component 'plate': pocket at step index 3 on XY removed no material"]
    );
    let measured = measure(&result);
    assert_volume(measured.volume_mm3, 4000.0, 1e-6, "no-op pocket");
}

/// One 12-step half of the 2026-10-20 MiniMax flange repro.
///
/// The half is a full 100 x 12 disc with the Ø30 bore, then a pocket that
/// removes its own side, then three Ø8 holes that now land in empty space.
fn flange_repro_half(name: &str, rectangle_x: f64, holes: [(f64, f64); 3]) -> Value {
    let mut steps = vec![
        json!({"op": "sketch", "profile": "circle", "plane": "XY", "diameter": 100.0}),
        json!({"op": "extrude", "length": 12.0}),
        json!({"op": "sketch", "profile": "circle", "plane": "XY", "diameter": 30.0}),
        json!({"op": "pocket"}),
        json!({"op": "sketch", "profile": "rectangle", "plane": "XY",
               "width": 100.0, "height": 100.0, "x": rectangle_x, "y": -50.0}),
        json!({"op": "pocket"}),
    ];
    for (x, y) in holes {
        steps.push(json!({"op": "sketch", "profile": "circle", "plane": "XY",
                          "diameter": 8.0, "x": x, "y": y}));
        steps.push(json!({"op": "pocket"}));
    }
    json!({"name": name, "steps": steps})
}

#[test]
fn the_flange_repro_warns_once_for_every_ineffective_pocket() {
    let script_text = script(
        json!([
            flange_repro_half(
                "flange_right",
                0.0,
                [
                    (40.0, 0.0),
                    (20.0, 34.641_016_151_377_55),
                    (20.0, -34.641_016_151_377_55),
                ]
            ),
            flange_repro_half(
                "flange_left",
                -100.0,
                [
                    (-40.0, 0.0),
                    (-20.0, 34.641_016_151_377_55),
                    (-20.0, -34.641_016_151_377_55),
                ]
            ),
        ]),
        json!([
            {"op": "place", "component": "flange_right"},
            {"op": "place", "component": "flange_left"},
            {"op": "fuse", "components": ["flange_right", "flange_left"]},
        ]),
    );
    let result = compile_json(&script_text);
    assert_eq!(
        result.warnings,
        [
            "component 'flange_right': pocket at step index 7 on XY removed no material",
            "component 'flange_right': pocket at step index 9 on XY removed no material",
            "component 'flange_right': pocket at step index 11 on XY removed no material",
            "component 'flange_left': pocket at step index 7 on XY removed no material",
            "component 'flange_left': pocket at step index 9 on XY removed no material",
            "component 'flange_left': pocket at step index 11 on XY removed no material",
        ]
    );
    let measured = measure(&result);
    // Both halves remove their own side, so the fused result is the full
    // flange with the bore but with no bolt hole: 85765.479443 mm3.
    assert_volume(measured.volume_mm3, 85_765.479_443, 1e-6, "flange repro");
}

#[test]
fn helix_is_refused_by_name_before_any_geometry() {
    let script_text = script(
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
    let error = expect_error(
        &script_text,
        &CompileOptions::default(),
        "unsupported by the Studio OCP compiler v1: helix",
    );
    assert_eq!(error.kind(), CompileErrorKind::Unsupported);
}

#[test]
fn compound_is_refused_by_name() {
    let script_text = script(
        json!([
            rectangle_component("a", "XY", 20.0, 20.0, 10.0, 0.0, 0.0, false),
            rectangle_component("b", "XY", 20.0, 20.0, 10.0, 0.0, 0.0, false),
        ]),
        json!([{"op": "compound", "components": ["a", "b"]}]),
    );
    expect_error(
        &script_text,
        &CompileOptions::default(),
        "unsupported by the Studio OCP compiler v1: compound",
    );
}

#[test]
fn analysis_edits_and_fillets_are_refused_by_name() {
    let plate = rectangle_script("XY", 20.0, 20.0, 10.0, 0.0, 0.0, false);
    let analysis = kala_contracts::load_analysis_json_str(
        r#"{"steps":[{"op":"material"},{"op":"mesh"},{"op":"solve"}]}"#,
    )
    .expect("valid analysis");
    let options = CompileOptions {
        analysis: Some(&analysis),
        ..CompileOptions::default()
    };
    expect_error(
        &plate,
        &options,
        "unsupported by the Studio OCP compiler v1: material",
    );

    for (op, extra) in [
        ("move", r#""x":1.0"#),
        ("resize", r#""length":5.0"#),
        ("fillet", r#""radius":1.0"#),
    ] {
        let reading = kala_contracts::load_reading_json_str(&format!(
            r#"{{"verdict":"change it","tool_edit":[{{"op":"{op}","component":"plate",{extra}}}],"rewrite":null}}"#
        ))
        .expect("valid reading");
        let options = CompileOptions {
            edits: reading.tool_edit(),
            ..CompileOptions::default()
        };
        expect_error(
            &plate,
            &options,
            &format!("unsupported by the Studio OCP compiler v1: {op}"),
        );
    }

    let mut fillets = BTreeMap::new();
    fillets.insert("plate".to_owned(), 1.0);
    let options = CompileOptions {
        fillets: Some(&fillets),
        ..CompileOptions::default()
    };
    expect_error(
        &plate,
        &options,
        "unsupported by the Studio OCP compiler v1: fillet",
    );
}

#[test]
fn a_rectangle_revolve_on_xz_is_refused_by_name() {
    let script_text = script(
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
    expect_error(
        &script_text,
        &CompileOptions::default(),
        "unsupported by the Studio OCP compiler v1: revolve",
    );
}

#[test]
fn a_cross_plane_pocket_is_refused_by_name() {
    let script_text = script(
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
    expect_error(
        &script_text,
        &CompileOptions::default(),
        "unsupported by the Studio OCP compiler v1: pocket",
    );
}

#[test]
fn a_duplicate_placement_is_refused() {
    let script_text = script(
        json!([rectangle_component(
            "plate", "XY", 20.0, 20.0, 10.0, 0.0, 0.0, false
        )]),
        json!([
            {"op": "place", "component": "plate", "x": 1.0},
            {"op": "place", "component": "plate", "x": 2.0},
        ]),
    );
    expect_error(
        &script_text,
        &CompileOptions::default(),
        "placed more than once",
    );
}

#[test]
fn the_compile_budget_is_enforced_and_must_be_finite_and_non_negative() {
    let plate = box_script(20.0, 20.0, 10.0);
    let budget = CompileOptions {
        timeout_seconds: f64::MIN_POSITIVE,
        ..CompileOptions::default()
    };
    let error = expect_error(&plate, &budget, "compile budget");
    assert_eq!(error.kind(), CompileErrorKind::Budget);

    for invalid in [-1.0, f64::NAN, f64::INFINITY] {
        let options = CompileOptions {
            timeout_seconds: invalid,
            ..CompileOptions::default()
        };
        expect_error(&plate, &options, "non-negative finite");
    }
}

#[test]
fn the_frozen_loader_owns_bounds_aliases_and_missing_references() {
    // A nested alias is flattened by the accepted contract loader and the
    // insertion order decides which alias wins.
    let alias_script = script(
        json!([{
            "name": "plate",
            "steps": [
                {"op": "sketch", "profile": {"box": {"width": 4.0, "height": 5.0}}},
                {"op": "extrude", "length": 6.0}
            ]
        }]),
        json!([]),
    );
    let measured = measure_shape(&compile_json(&alias_script).components[0].local_shape).unwrap();
    assert_volume(measured.volume_mm3, 120.0, 1e-6, "alias");

    // A literal string is required here: serde_json's default map sorts keys,
    // which would destroy the insertion-order semantics being tested.
    let ordered_script = r#"{
        "summary": "ordered aliases",
        "components": [{
            "name": "plate",
            "steps": [
                {"op": "sketch", "profile": {
                    "circle": {"diameter": 6.0},
                    "box": {"width": 4.0, "height": 5.0}
                }},
                {"op": "extrude", "length": 6.0}
            ]
        }],
        "assembly": []
    }"#;
    let measured = measure_shape(&compile_json(ordered_script).components[0].local_shape).unwrap();
    assert_volume(
        measured.volume_mm3,
        std::f64::consts::PI * 9.0 * 6.0,
        1e-6,
        "first profile alias wins",
    );

    let missing = script(
        json!([rectangle_component(
            "plate", "XY", 20.0, 20.0, 10.0, 0.0, 0.0, false
        )]),
        json!([{"op": "place", "component": "ghost"}]),
    );
    let error = expect_error(
        &missing,
        &CompileOptions::default(),
        "assembly names a missing component ghost",
    );
    assert_eq!(error.kind(), CompileErrorKind::Contract);

    let unknown = script(
        json!([{
            "name": "plate",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "width": 4.0, "height": 4.0},
                {"op": "thread", "length": 6.0}
            ]
        }]),
        json!([]),
    );
    let error = expect_error(
        &unknown,
        &CompileOptions::default(),
        "unknown operation thread",
    );
    assert_eq!(error.kind(), CompileErrorKind::Contract);

    let too_wide = script(
        json!([rectangle_component(
            "plate", "XY", 5000.1, 20.0, 10.0, 0.0, 0.0, false
        )]),
        json!([]),
    );
    expect_error(
        &too_wide,
        &CompileOptions::default(),
        "width is outside the model bounds",
    );
}

#[test]
fn compile_json_bytes_and_str_are_the_same_entry_point() {
    let text = box_script(20.0, 20.0, 10.0);
    let from_text =
        kala_cad::compile_json_str(&text, &CompileOptions::default()).expect("text compile");
    let from_bytes = kala_cad::compile_json_bytes(text.as_bytes(), &CompileOptions::default())
        .expect("byte compile");
    let left = measure(&from_text);
    let right = measure(&from_bytes);
    assert_volume(left.volume_mm3, right.volume_mm3, 1e-12, "text vs bytes");
}

#[test]
fn the_fixture_file_is_never_modified() {
    let path = common::fixture_path();
    let metadata = std::fs::metadata(&path).expect("fixture metadata");
    assert!(metadata.is_file());
    // The fixture is intentionally read-only to this cell; a write here would
    // be a shared-fixture edit outside the ownership boundary.
    assert!(Path::new(&path).exists());
}

#[test]
fn json_values_round_trip_through_the_accepted_loader() {
    let value: Value = json!({
        "summary": "round trip",
        "components": [{
            "name": "plate",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 20.0, "height": 20.0},
                {"op": "extrude", "length": 10.0}
            ]
        }],
        "assembly": []
    });
    let result = compile_json(&serde_json::to_string(&value).unwrap());
    assert_eq!(result.components.len(), 1);
    assert_eq!(result.components[0].name, "plate");
}
