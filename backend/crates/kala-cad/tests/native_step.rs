#![cfg(feature = "native")]

//! Reusable STEP export/import of final exact shapes.

mod common;

use std::process::Command;

use common::{
    assert_bounds, assert_volume, box_script, compile_json, l_bracket_script, measure,
    oracle_python, script,
};
use kala_cad::{
    DEFAULT_MAX_STEP_BYTES, Shape, StepImportErrorKind, StepImportOptions, StepWriteOptions,
    import_step_view, measure_shape, normalise_step_bytes, step_bytes, step_bytes_with_options,
    step_shape,
};
use serde_json::{Value, json};

fn l_bracket() -> (Shape, Vec<u8>) {
    let result = compile_json(&l_bracket_script());
    let data = step_bytes(&result.assembly_shape).expect("the L-bracket must export");
    (result.assembly_shape, data)
}

#[test]
fn the_l_bracket_step_round_trips_as_one_valid_solid_in_millimetres() {
    let (original, data) = l_bracket();
    assert!(data.starts_with(b"ISO-10303-21"), "not an ISO STEP file");
    let text = String::from_utf8_lossy(&data).to_ascii_lowercase();
    assert!(
        text.contains("milli") && text.contains("metre"),
        "the STEP file does not declare a millimetre length unit"
    );

    let imported = step_shape(&data).expect("the exported STEP must re-import");
    let before = measure_shape(&original).unwrap();
    let after = measure_shape(&imported).unwrap();
    assert!(after.valid);
    assert_eq!(after.solid_count, 1);
    assert_volume(after.volume_mm3, before.volume_mm3, 1e-6, "step volume");
    assert_volume(
        after.surface_area_mm2,
        before.surface_area_mm2,
        1e-6,
        "step area",
    );
    assert_bounds(
        &after.bounds,
        before.bounds.min_mm,
        before.bounds.max_mm,
        0.001,
        "step bounds",
    );
    for axis in 0..3 {
        assert!((after.centroid_mm[axis] - before.centroid_mm[axis]).abs() <= 0.001);
    }
}

#[test]
fn an_unfused_assembly_round_trips_as_a_compound_with_two_solids() {
    let text = script(
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
    let result = compile_json(&text);
    let original = measure(&result);
    assert_eq!(original.solid_count, 2);
    let data = step_bytes(&result.assembly_shape).unwrap();
    let imported = step_shape(&data).unwrap();
    let after = measure_shape(&imported).unwrap();
    assert!(after.valid);
    assert_eq!(after.solid_count, 2);
    assert_volume(after.volume_mm3, 400.0, 1e-6, "compound volume");
    assert_bounds(
        &after.bounds,
        [0.0, 0.0, 0.0],
        [25.0, 10.0, 2.0],
        0.001,
        "compound bounds",
    );
}

#[test]
fn a_fused_assembly_round_trips_as_one_solid() {
    let result = compile_json(&l_bracket_script());
    assert!(result.fused);
    let data = step_bytes(&result.assembly_shape).unwrap();
    let imported = step_shape(&data).unwrap();
    assert_eq!(measure_shape(&imported).unwrap().solid_count, 1);
}

#[test]
fn two_exports_of_the_same_shape_decode_to_the_same_geometry() {
    let result = compile_json(&box_script(20.0, 20.0, 10.0));
    let first = step_bytes(&result.assembly_shape).unwrap();
    let second = step_bytes(&result.assembly_shape).unwrap();
    let first_shape = step_shape(&first).unwrap();
    let second_shape = step_shape(&second).unwrap();
    let left = measure_shape(&first_shape).unwrap();
    let right = measure_shape(&second_shape).unwrap();
    assert_volume(left.volume_mm3, right.volume_mm3, 1e-12, "first vs second");
    assert_bounds(
        &right.bounds,
        left.bounds.min_mm,
        left.bounds.max_mm,
        1e-9,
        "first vs second bounds",
    );
    // Timestamps/entity numbering are metadata and may differ. The file is a
    // fresh valid document either way; no byte-parity claim is made.
    assert!(first.starts_with(b"ISO-10303-21"));
    assert!(second.starts_with(b"ISO-10303-21"));
}

#[test]
fn empty_garbage_and_truncated_step_data_are_refused() {
    for data in [
        b"".as_slice(),
        b"not a step file".as_slice(),
        b"ISO-10303-21;\nHEADER;\n".as_slice(),
    ] {
        let error = step_shape(data).expect_err("must refuse");
        assert!(!error.message().is_empty());
    }
    let (_, good) = l_bracket();
    let truncated = &good[..good.len() / 2];
    let error = step_shape(truncated).expect_err("truncated STEP must be refused");
    assert!(!error.message().is_empty());
}

#[test]
fn a_null_shape_cannot_be_exported() {
    let error = step_bytes(&Shape::null().unwrap()).expect_err("null must fail");
    assert_eq!(error.message(), "the STEP export needs a non-null shape");
}

#[test]
fn python_and_rust_written_step_import_with_identical_bounds() {
    let python = oracle_python();
    assert!(
        python.is_file(),
        "the mandatory Python STEP oracle was not found at {}; set \
         KALA_CAD_ORACLE_PYTHON",
        python.display()
    );
    let temp = common::TempDir::new("python-step");
    let step_path = temp.path().join("python-l-bracket.step");
    let request_path = temp.path().join("request.json");
    let result_path = temp.path().join("result.json");
    let script_json: Value = serde_json::from_str(&l_bracket_script()).unwrap();
    std::fs::write(
        &request_path,
        serde_json::to_vec_pretty(&json!({
            "script": script_json,
            "step_path": step_path,
        }))
        .unwrap(),
    )
    .unwrap();

    let output = Command::new(&python)
        .arg(common::project_root().join("backend/crates/kala-cad/tests/oracle/step_writer.py"))
        .arg(&request_path)
        .arg(&result_path)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("start the Python STEP writer");
    let result: Value =
        serde_json::from_slice(&std::fs::read(&result_path).unwrap_or_else(|error| {
            panic!(
                "the Python STEP writer produced no result ({error}); status {:?}\n\
                 stdout:\n{}\nstderr:\n{}",
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        }))
        .expect("parse the Python STEP result");
    assert!(
        result["ok"].as_bool().unwrap_or(false),
        "the Python STEP writer failed: {}",
        result["error"]
    );

    let python_bytes = std::fs::read(&step_path).expect("read the Python STEP");
    assert_eq!(
        python_bytes.len() as u64,
        result["step_size"].as_u64().unwrap()
    );
    let python_view = import_step_view(&python_bytes, StepImportOptions::default())
        .expect("the Python-written STEP must import");
    assert!(
        python_view.warnings.is_empty(),
        "{:?}",
        python_view.warnings
    );
    assert_eq!(
        python_view.solids,
        result["measurement"]["solid_count"].as_u64().unwrap() as usize
    );
    let expected = &result["measurement"];
    let minimum = expected["min"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_f64().unwrap())
        .collect::<Vec<_>>();
    let maximum = expected["max"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_f64().unwrap())
        .collect::<Vec<_>>();
    assert_bounds(
        &python_view.bounds,
        [minimum[0], minimum[1], minimum[2]],
        [maximum[0], maximum[1], maximum[2]],
        0.001,
        "python step view",
    );
    let imported = measure_shape(&python_view.shape).unwrap();
    assert_volume(
        imported.volume_mm3,
        expected["volume"].as_f64().unwrap(),
        1e-6,
        "python step volume",
    );

    let native = compile_json(&l_bracket_script());
    let rust_bytes = step_bytes(&native.assembly_shape).unwrap();
    let rust_view =
        import_step_view(&rust_bytes, StepImportOptions::default()).expect("Rust STEP import");
    assert_bounds(
        &rust_view.bounds,
        python_view.bounds.min_mm,
        python_view.bounds.max_mm,
        0.001,
        "python vs rust step view",
    );
    let rust_measurement = measure_shape(&rust_view.shape).unwrap();
    assert_volume(
        rust_measurement.volume_mm3,
        imported.volume_mm3,
        1e-6,
        "python vs rust step volume",
    );
    assert_eq!(rust_view.solids, python_view.solids);
}

#[test]
fn normalised_exports_apply_the_corpus_replacement_without_changing_geometry() {
    let result = compile_json(&l_bracket_script());
    let raw = step_bytes(&result.assembly_shape).unwrap();
    let normalised = normalise_step_bytes(&raw);
    // The corpus replacement is length-preserving and idempotent. It does not
    // claim that two exports are byte-identical: OCCT's entity ordering is a
    // kernel fact, and the corpus only normalises the timestamp and the
    // per-process translator counter.
    assert_eq!(normalised.len(), raw.len());
    assert_eq!(
        normalise_step_bytes(&normalised),
        normalised,
        "normalisation must be idempotent"
    );
    assert!(
        normalised
            .windows(19)
            .any(|window| window == b"2026-01-01T00:00:00")
    );
    assert!(normalised.starts_with(b"ISO-10303-21"));

    let optioned =
        step_bytes_with_options(&result.assembly_shape, StepWriteOptions { normalise: true })
            .unwrap();
    assert!(optioned.starts_with(b"ISO-10303-21"));
    assert!(
        optioned
            .windows(19)
            .any(|window| window == b"2026-01-01T00:00:00")
    );
    let optioned_view = import_step_view(&optioned, StepImportOptions::default()).unwrap();
    assert_eq!(optioned_view.solids, 1);

    let normalised_view = import_step_view(&normalised, StepImportOptions::default()).unwrap();
    let raw_view = import_step_view(&raw, StepImportOptions::default()).unwrap();
    assert_eq!(normalised_view.solids, raw_view.solids);
    assert_bounds(
        &normalised_view.bounds,
        raw_view.bounds.min_mm,
        raw_view.bounds.max_mm,
        0.0,
        "normalised vs raw step bounds",
    );
    assert_volume(
        measure_shape(&normalised_view.shape).unwrap().volume_mm3,
        measure_shape(&raw_view.shape).unwrap().volume_mm3,
        1e-12,
        "normalised vs raw step volume",
    );
}

#[test]
fn view_import_refuses_malformed_and_oversized_bytes_with_exact_messages() {
    let empty = import_step_view(&[], StepImportOptions::default()).unwrap_err();
    assert_eq!(empty.kind(), StepImportErrorKind::Malformed);
    assert_eq!(empty.message(), "the STEP file is empty");

    let bad_header =
        import_step_view(b"not a step file", StepImportOptions::default()).unwrap_err();
    assert_eq!(bad_header.kind(), StepImportErrorKind::Malformed);
    assert_eq!(
        bad_header.message(),
        "the file does not start with an ISO-10303-21 header"
    );

    let header_only =
        import_step_view(b"ISO-10303-21;\nHEADER;\n", StepImportOptions::default()).unwrap_err();
    assert_eq!(header_only.kind(), StepImportErrorKind::Malformed);
    assert_eq!(header_only.message(), "the STEP reader refused the file");

    let (_, data) = l_bracket();
    let limit = (data.len() - 1) as u64;
    let too_large = import_step_view(
        &data,
        StepImportOptions {
            max_step_bytes: limit,
            ..StepImportOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(too_large.kind(), StepImportErrorKind::TooLarge);
    assert_eq!(
        too_large.message(),
        format!("the STEP file must not exceed {limit} bytes")
    );

    for invalid in [0, kala_cad::MAX_STEP_IMPORT_BYTES + 1] {
        let error = import_step_view(
            &data,
            StepImportOptions {
                max_step_bytes: invalid,
                ..StepImportOptions::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), StepImportErrorKind::Config);
        assert_eq!(
            error.message(),
            "the STEP import limit must be between 1 and 536870912 bytes"
        );
    }
    assert_eq!(DEFAULT_MAX_STEP_BYTES, 64 * 1024 * 1024);
}

#[test]
fn view_import_reports_surface_only_step_without_inventing_solids() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("non_solid_surface.step");
    let data = std::fs::read(path).expect("read the surface fixture");
    let view = import_step_view(&data, StepImportOptions::default()).expect("surface import");
    assert_eq!(view.solids, 0);
    assert_eq!(
        view.warnings,
        vec!["the STEP file contains no solids; only its surfaces are shown"]
    );
    assert!(view.mesh.triangle_count() > 0);
    assert!(view.mesh.positions.iter().all(|value| value.is_finite()));
}

#[test]
fn view_import_retries_coarser_or_refuses_the_same_budget_as_python() {
    let result = compile_json(&l_bracket_script());
    let data = step_bytes(&result.assembly_shape).unwrap();
    let error = import_step_view(
        &data,
        StepImportOptions {
            max_vertices: 1,
            ..StepImportOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), StepImportErrorKind::TooComplex);
    assert_eq!(
        error.message(),
        "the mesh would exceed the 1 vertex limit; refusing to truncate it"
    );

    // OCCT keeps the triangulation it attached to a face. A coarser retry on
    // the same imported shape therefore reuses that triangulation instead of
    // replacing it, exactly as `importer.py` does, so the same budget is still
    // refused. This is the honest behaviour; the retry exists for a shape
    // whose first tessellation failed before attaching a triangulation.
    let cylinder = script(
        json!([{
            "name": "barrel",
            "steps": [
                {"op": "sketch", "profile": "circle", "plane": "XY", "diameter": 200.0},
                {"op": "extrude", "length": 200.0}
            ]
        }]),
        json!([]),
    );
    let result = compile_json(&cylinder);
    let data = step_bytes(&result.assembly_shape).unwrap();
    let default_mesh =
        kala_cad::tessellate(&result.assembly_shape, kala_cad::MeshOptions::default()).unwrap();
    let coarse_mesh = kala_cad::tessellate(
        &result.assembly_shape,
        kala_cad::MeshOptions {
            deflection_mm: 0.5,
            angular_deflection_rad: 0.8,
            ..kala_cad::MeshOptions::default()
        },
    )
    .unwrap();
    assert_eq!(
        coarse_mesh.vertex_count(),
        default_mesh.vertex_count(),
        "OCCT reuses an attached triangulation"
    );
    let limit = default_mesh.vertex_count() - 1;
    let error = import_step_view(
        &data,
        StepImportOptions {
            max_vertices: limit,
            ..StepImportOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), StepImportErrorKind::TooComplex);
    assert_eq!(
        error.message(),
        format!("the mesh would exceed the {limit} vertex limit; refusing to truncate it")
    );
    assert_eq!(normalise_step_bytes(&data).len(), data.len());
}

#[test]
fn step_import_preserves_the_face_topology_of_a_hole() {
    let text = script(
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
    let result = compile_json(&text);
    let original = measure(&result);
    let data = step_bytes(&result.assembly_shape).unwrap();
    let imported = step_shape(&data).unwrap();
    let after = measure_shape(&imported).unwrap();
    assert_eq!(after.solid_count, 1);
    assert!(after.valid);
    assert_volume(after.volume_mm3, original.volume_mm3, 1e-6, "hole volume");
    assert_volume(
        after.surface_area_mm2,
        original.surface_area_mm2,
        1e-6,
        "hole area",
    );
    assert!(
        after.surface_area_mm2 > 1600.0 + 1.0,
        "the through hole must add cylindrical surface area"
    );
}
