//! The explicit unavailable mode must be honest and must not approximate CAD.
#![cfg(not(feature = "native"))]

use kala_cad::{
    Bounds, CheckValue, CompileErrorKind, CompileOptions, DrawingErrorKind, MeshOptions, Shape,
    StepImportErrorKind, StepImportOptions, StepWriteOptions, build_checks, compile_json_str,
    default_section_offset, design_available, hlr_views, hlr_views_with_options, import_step_view,
    measure_shape, normalise_step_bytes, probe_capability, section, section_with_options,
    step_bytes, step_bytes_with_options, step_shape, tessellate, topology,
    validate_step_import_max_bytes,
};
use serde_json::json;

fn box_script() -> String {
    serde_json::to_string(&json!({
        "summary": "unavailable mode",
        "components": [{
            "name": "plate",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 20.0, "height": 20.0},
                {"op": "extrude", "length": 10.0}
            ]
        }],
        "assembly": []
    }))
    .unwrap()
}

#[test]
fn capability_is_explicitly_unavailable() {
    let capability = probe_capability();
    assert!(!capability.available);
    assert_eq!(capability.backend, "none");
    assert_eq!(capability.kernel, "none");
    let reason = capability.reason.expect("a reason is mandatory");
    assert!(reason.contains("native"), "unexpected reason: {reason}");
    assert!(!design_available());
}

#[test]
fn every_geometry_entry_point_reports_unavailable() {
    let script = box_script();
    let error = compile_json_str(&script, &CompileOptions::default()).expect_err("must refuse");
    assert_eq!(error.kind(), CompileErrorKind::Unavailable);
    assert!(error.message().contains("unavailable"));

    let null = Shape::null();
    assert!(null.is_null());
    assert!(measure_shape(&null).is_err());
    assert!(topology(&null).is_err());
    assert!(tessellate(&null, MeshOptions::default()).is_err());
    assert!(step_bytes(&null).is_err());
    assert!(step_shape(b"ISO-10303-21;").is_err());
    assert!(step_bytes_with_options(&null, StepWriteOptions { normalise: true }).is_err());
    let error = import_step_view(b"ISO-10303-21;", StepImportOptions::default())
        .expect_err("a geometry view cannot exist without the kernel");
    assert_eq!(error.kind(), StepImportErrorKind::Malformed);
    assert!(error.message().contains("unavailable"));

    // The drawing and section surface is present but honestly unavailable.
    for error in [
        hlr_views(&null).expect_err("drawing needs the kernel"),
        hlr_views_with_options(&null, &["front"], 0.05, 1).expect_err("drawing needs the kernel"),
        section(&null).expect_err("sections need the kernel"),
        section_with_options(&null, "XZ", Some(0.0), 0.01).expect_err("sections need the kernel"),
        default_section_offset(&null).expect_err("offsets need the kernel"),
    ] {
        assert_eq!(error.kind(), DrawingErrorKind::Unavailable);
        assert!(error.message().contains("unavailable"));
    }

    // Limit validation and byte normalisation are policy, not geometry, and
    // stay available so an operator still gets the exact refusal message.
    assert_eq!(validate_step_import_max_bytes(4096).unwrap(), 4096);
    assert_eq!(
        normalise_step_bytes(b"x'2026-10-20T12:34:56'y")[2..21].to_vec(),
        b"2026-01-01T00:00:00".to_vec()
    );
}

#[test]
fn kernel_independent_checks_and_bounds_still_work() {
    let bounds = Bounds::new([0.0, 0.0, 0.0], [20.0, 20.0, 10.0]).unwrap();
    assert_eq!(bounds.size_mm(), [20.0, 20.0, 10.0]);
    let synthetic = kala_cad::Measurement {
        volume_mm3: 4000.0,
        surface_area_mm2: 1600.0,
        centroid_mm: [10.0, 10.0, 5.0],
        bounds,
        valid: true,
        solid_count: 1,
    };
    let report = build_checks(&synthetic, Some(&bounds), true);
    assert!(report.verified);
    assert_eq!(report.checks[0].expected, CheckValue::Bool(true));
    assert_eq!(report.checks[0].id, "brep_valid");
}
