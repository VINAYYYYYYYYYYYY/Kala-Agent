#![cfg(feature = "native")]

//! Exact native measurement, labels, units and declared-versus-measured checks.

mod common;

use common::{assert_bounds, assert_volume, box_script, compile_json, script};
use kala_cad::{
    Bounds, CheckValue, Shape, build_checks, measure_and_check, measure_shape, step_shape,
};
use serde_json::json;

fn box_shape(size_x: f64, size_y: f64, size_z: f64) -> Shape {
    compile_json(&box_script(size_x, size_y, size_z))
        .components
        .remove(0)
        .local_shape
}

fn cylinder_shape(radius: f64, height: f64) -> Shape {
    let text = script(
        json!([{
            "name": "pin",
            "steps": [
                {"op": "sketch", "profile": "circle", "plane": "XY", "diameter": radius * 2.0},
                {"op": "extrude", "length": height}
            ]
        }]),
        json!([]),
    );
    compile_json(&text).components.remove(0).local_shape
}

#[test]
fn a_box_is_measured_exactly() {
    let measured = measure_shape(&box_shape(20.0, 20.0, 10.0)).unwrap();
    assert_volume(measured.volume_mm3, 4000.0, 1e-12, "box volume");
    assert_volume(measured.surface_area_mm2, 1600.0, 1e-12, "box area");
    for (axis, expected) in [10.0, 10.0, 5.0].into_iter().enumerate() {
        assert!(
            (measured.centroid_mm[axis] - expected).abs() <= 1e-12,
            "centroid axis {axis}"
        );
    }
    assert_bounds(
        &measured.bounds,
        [0.0, 0.0, 0.0],
        [20.0, 20.0, 10.0],
        1e-9,
        "box",
    );
    assert_eq!(measured.bounds.size_mm(), [20.0, 20.0, 10.0]);
    assert!(measured.valid);
    assert_eq!(measured.solid_count, 1);
}

#[test]
fn a_cylinder_is_measured_exactly() {
    let measured = measure_shape(&cylinder_shape(2.0, 5.0)).unwrap();
    assert_volume(
        measured.volume_mm3,
        std::f64::consts::PI * 4.0 * 5.0,
        1e-12,
        "cylinder volume",
    );
    assert_volume(
        measured.surface_area_mm2,
        2.0 * std::f64::consts::PI * 4.0 + 2.0 * std::f64::consts::PI * 2.0 * 5.0,
        1e-12,
        "cylinder area",
    );
    for (axis, expected) in [0.0, 0.0, 2.5].into_iter().enumerate() {
        assert!(
            (measured.centroid_mm[axis] - expected).abs() <= 1e-12,
            "centroid axis {axis}"
        );
    }
    assert_bounds(
        &measured.bounds,
        [-2.0, -2.0, 0.0],
        [2.0, 2.0, 5.0],
        1e-9,
        "cylinder",
    );
    assert!(measured.valid);
    assert_eq!(measured.solid_count, 1);
}

#[test]
fn validity_and_single_solid_are_always_checked_with_source_labels() {
    let measured = measure_shape(&box_shape(20.0, 20.0, 10.0)).unwrap();
    let report = build_checks(&measured, None, true);
    assert!(report.verified);
    assert_eq!(report.checks.len(), 2);

    let validity = &report.checks[0];
    assert_eq!(validity.id, "brep_valid");
    assert_eq!(validity.label, "B-Rep valid");
    assert_eq!(validity.expected, CheckValue::Bool(true));
    assert_eq!(validity.measured, CheckValue::Bool(true));
    assert_eq!(validity.tolerance, 0.0);
    assert_eq!(validity.unit, "bool");

    let single = &report.checks[1];
    assert_eq!(single.id, "single_solid");
    assert_eq!(single.label, "Single solid");
    assert_eq!(single.expected, CheckValue::Float(1.0));
    assert_eq!(single.measured, CheckValue::Float(1.0));
    assert_eq!(single.tolerance, 0.0);
    assert_eq!(single.unit, "solid");
}

#[test]
fn the_envelope_is_compared_to_the_measured_bounds_in_millimetres() {
    let measured = measure_shape(&box_shape(20.0, 20.0, 10.0)).unwrap();
    let envelope = Bounds::new([0.0, 0.0, 0.0], [20.0, 20.0, 10.0]).unwrap();
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
    for check in &report.checks[2..] {
        assert_eq!(check.tolerance, 0.01);
        assert_eq!(check.unit, "mm");
        assert!(check.passed);
        assert!(matches!(check.expected, CheckValue::Pair(_, _)));
        assert!(matches!(check.measured, CheckValue::Pair(_, _)));
    }
}

#[test]
fn a_wrong_envelope_fails_verification_with_the_failed_row() {
    let measured = measure_shape(&box_shape(20.0, 20.0, 10.0)).unwrap();
    let envelope = Bounds::new([0.0, 0.0, 0.0], [20.0, 20.0, 11.0]).unwrap();
    let report = build_checks(&measured, Some(&envelope), true);
    assert!(!report.verified);
    let failed = report
        .checks
        .iter()
        .filter(|check| !check.passed)
        .collect::<Vec<_>>();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].id, "envelope_z");
    assert_eq!(failed[0].expected, CheckValue::Pair(0.0, 11.0));
    assert_eq!(failed[0].measured, CheckValue::Pair(0.0, 10.0));
}

#[test]
fn a_multi_solid_compound_is_measured_and_fails_single_solid() {
    let text = script(
        json!([
            {
                "name": "a",
                "steps": [
                    {"op": "sketch", "profile": "rectangle", "plane": "XY",
                     "width": 10.0, "height": 10.0},
                    {"op": "extrude", "length": 5.0}
                ]
            },
            {
                "name": "b",
                "steps": [
                    {"op": "sketch", "profile": "rectangle", "plane": "XY",
                     "width": 10.0, "height": 10.0},
                    {"op": "extrude", "length": 5.0}
                ]
            }
        ]),
        json!([
            {"op": "place", "component": "a", "x": 0.0},
            {"op": "place", "component": "b", "x": 20.0}
        ]),
    );
    let result = compile_json(&text);
    let measured = measure_shape(&result.assembly_shape).unwrap();
    assert_volume(measured.volume_mm3, 1000.0, 1e-12, "compound volume");
    assert_bounds(
        &measured.bounds,
        [0.0, 0.0, 0.0],
        [30.0, 10.0, 5.0],
        1e-9,
        "compound",
    );
    assert_eq!(measured.solid_count, 2);
    assert!(measured.valid);

    let report = build_checks(&measured, None, true);
    assert!(!report.verified);
    assert_eq!(
        report
            .checks
            .iter()
            .filter(|check| !check.passed)
            .map(|check| check.id.as_str())
            .collect::<Vec<_>>(),
        ["single_solid"]
    );
    let without_single = build_checks(&measured, None, false);
    assert_eq!(without_single.checks.len(), 1);
    assert!(without_single.verified);
}

#[test]
fn measure_and_check_returns_the_same_measurement_and_report() {
    let shape = box_shape(20.0, 20.0, 10.0);
    let envelope = Bounds::new([0.0, 0.0, 0.0], [20.0, 20.0, 10.0]).unwrap();
    let (measured, report) = measure_and_check(&shape, Some(&envelope), true).unwrap();
    assert_volume(measured.volume_mm3, 4000.0, 1e-12, "measure_and_check");
    assert!(report.verified);
    assert_eq!(report, build_checks(&measured, Some(&envelope), true));
}

#[test]
fn a_zero_volume_surface_is_measured_honestly_but_is_not_a_part() {
    let path = common::project_root()
        .join("backend/crates/kala-cad/tests/fixtures/non_solid_surface.step");
    let data = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("read non-solid STEP fixture {}: {error}", path.display()));
    let shape = step_shape(&data).expect("the surface STEP must import");
    let measured = measure_shape(&shape).unwrap();
    assert_volume(measured.volume_mm3, 0.0, 1e-12, "surface volume");
    assert_eq!(measured.solid_count, 0);
    assert_bounds(
        &measured.bounds,
        [0.0, 0.0, 0.0],
        [20.0, 10.0, 0.0],
        1e-6,
        "surface",
    );
    assert_volume(measured.surface_area_mm2, 200.0, 1e-9, "surface area");
    let report = build_checks(&measured, None, true);
    assert!(!report.verified);
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.id == "single_solid" && !check.passed)
    );
}

#[test]
fn a_null_shape_raises_the_exact_measurement_error() {
    let error = measure_shape(&Shape::null().unwrap()).expect_err("null must fail");
    assert_eq!(error.message(), "the shape is null and cannot be measured");
}

#[test]
fn bounds_reject_inverted_non_finite_and_wrong_length_values() {
    let inverted = Bounds::new([1.0, 0.0, 0.0], [0.0, 1.0, 1.0]).unwrap_err();
    assert_eq!(inverted.message(), "bounds minimum exceeds maximum");
    let non_finite = Bounds::new([0.0, 0.0, 0.0], [f64::INFINITY, 1.0, 1.0]).unwrap_err();
    assert_eq!(non_finite.message(), "bounds must be finite numbers");
    let wrong_length = Bounds::from_slices(&[0.0, 0.0], &[1.0, 1.0, 1.0]).unwrap_err();
    assert_eq!(
        wrong_length.message(),
        "bounds require three minimum and three maximum values"
    );
}

#[test]
fn topology_counts_describe_the_exact_brep() {
    let box_counts = kala_cad::topology(&box_shape(20.0, 20.0, 10.0)).unwrap();
    assert_eq!(box_counts.solids, 1);
    assert_eq!(box_counts.shells, 1);
    assert_eq!(box_counts.faces, 6);
    assert_eq!(box_counts.edges, 12);
    assert_eq!(box_counts.vertices, 8);

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
    let hole = compile_json(&text);
    let hole_counts = kala_cad::topology(&hole.assembly_shape).unwrap();
    assert_eq!(hole_counts.solids, 1);
    assert_eq!(hole_counts.shells, 1);
    assert!(
        hole_counts.faces >= 7,
        "a through hole adds a cylindrical face: {:?}",
        hole_counts
    );
    assert!(hole_counts.edges > box_counts.edges);
    assert!(hole_counts.vertices > box_counts.vertices);

    let error = kala_cad::topology(&Shape::null().unwrap()).unwrap_err();
    assert_eq!(error.message(), "the shape is null and cannot be measured");
}
