//! Native tests for the exact B-rep STEP writer.
//!
//! These tests check the document the crate produces — syntax, entity
//! numbering, analytic topology, centring, units, determinism, error typing,
//! boundedness, concurrency and the no-file-writes guarantee — without any
//! CAD kernel. The independent kernel verification lives in
//! `tests/ocp_differential.rs`; both are required.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use kala_step::{
    ISO_PREFIX, ISO_TERMINATOR, ISO_TRAILER, MAX_STEP_BYTES, NativeStepExporter, PrimitiveKind,
    STEP_EXPORTER_NAME, STEP_OUTPUT_MODE, StepError, StepExporter, export_kind_str,
    export_step_primitive, probe_capability, self_check,
};
use serde_json::{Value, json};

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("the document is ASCII")
}

/// Split one STEP parameter list at top-level commas.
fn split_parameters(list: &str) -> Vec<String> {
    let mut parameters = Vec::new();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut current = String::new();
    for character in list.chars() {
        match character {
            '\'' => {
                quoted = !quoted;
                current.push(character);
            }
            '(' if !quoted => {
                depth += 1;
                current.push(character);
            }
            ')' if !quoted => {
                depth -= 1;
                current.push(character);
            }
            ',' if !quoted && depth == 0 => {
                parameters.push(current.trim().to_owned());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    if !current.trim().is_empty() {
        parameters.push(current.trim().to_owned());
    }
    parameters
}

/// Parse `#n = body;` lines into `(number, body)` pairs.
fn entities(document: &str) -> Vec<(u32, String)> {
    document
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix('#')?;
            let (number, body) = rest.split_once(" = ")?;
            let body = body.strip_suffix(';')?;
            Some((number.parse().ok()?, body.to_owned()))
        })
        .collect()
}

fn entity_type(body: &str) -> String {
    if body.starts_with('(') {
        return "COMPLEX".to_owned();
    }
    body.split('(').next().unwrap_or_default().trim().to_owned()
}

fn type_counts(document: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for (_, body) in entities(document) {
        *counts.entry(entity_type(&body)).or_insert(0) += 1;
    }
    counts
}

fn references(body: &str) -> Vec<u32> {
    let mut found = Vec::new();
    let bytes = body.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'#' {
            let mut end = index + 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > index + 1 {
                if let Ok(number) = body[index + 1..end].parse() {
                    found.push(number);
                }
                index = end;
                continue;
            }
        }
        index += 1;
    }
    found
}

fn default_bytes(kind: PrimitiveKind) -> Vec<u8> {
    export_step_primitive(kind, None).expect("the default primitive exports")
}

#[test]
fn documents_are_complete_iso_10303_21_with_millimetre_units() {
    assert_eq!(STEP_OUTPUT_MODE, "exact_brep");
    for kind in [
        PrimitiveKind::Box,
        PrimitiveKind::Cylinder,
        PrimitiveKind::Sphere,
    ] {
        let bytes = default_bytes(kind);
        let document = text(&bytes);
        assert!(bytes.starts_with(ISO_PREFIX), "{kind}: ISO prefix");
        assert!(document.ends_with("ENDSEC;\nEND-ISO-10303-21;\n"), "{kind}");
        assert!(bytes.ends_with(&[ISO_TRAILER, ISO_TERMINATOR].concat()));
        assert!(document.contains("FILE_SCHEMA(('AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }'));"));
        assert!(document.contains("( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) )"));
        assert!(document.contains("( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) )"));
        assert!(document.contains("SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT()"));
        assert!(document.contains("UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-07),"));
        assert!(document.contains("ADVANCED_BREP_SHAPE_REPRESENTATION"));
        assert!(document.contains("MANIFOLD_SOLID_BREP"));
        assert!(document.contains("CLOSED_SHELL"));
        assert!(bytes.iter().all(u8::is_ascii));
    }
}

#[test]
fn file_name_parameters_are_well_formed() {
    // Regression guard for a real conformance bug: parameters 5 and 6 of
    // FILE_NAME are plain strings, not lists, and OpenCascade reports
    // "not a quoted String" when they are not (an unresolved reference on
    // load). The structural check below parses the parameter list instead of
    // matching one brittle literal.
    for kind in [
        PrimitiveKind::Box,
        PrimitiveKind::Cylinder,
        PrimitiveKind::Sphere,
    ] {
        let document = text(&default_bytes(kind));
        let line = document
            .lines()
            .find(|line| line.starts_with("FILE_NAME("))
            .expect("a FILE_NAME line exists");
        let parameters =
            split_parameters(line.trim_start_matches("FILE_NAME(").trim_end_matches(");"));
        assert_eq!(parameters.len(), 7, "{kind}: FILE_NAME parameter count");
        assert!(parameters[2].starts_with('('), "author is a list");
        assert!(parameters[3].starts_with('('), "organization is a list");
        for index in [0usize, 1, 4, 5, 6] {
            let parameter = &parameters[index];
            assert!(
                parameter.starts_with('\'') && parameter.ends_with('\''),
                "{kind}: FILE_NAME parameter {index} must be a quoted string, got {parameter}"
            );
        }
    }
}

#[test]
fn entity_numbering_is_sequential_and_every_reference_resolves() {
    for kind in [
        PrimitiveKind::Box,
        PrimitiveKind::Cylinder,
        PrimitiveKind::Sphere,
    ] {
        let document = text(&default_bytes(kind));
        let entities = entities(&document);
        assert!(!entities.is_empty());
        for (index, (number, _)) in entities.iter().enumerate() {
            assert_eq!(*number, index as u32 + 1, "{kind}: numbering gap");
        }
        let defined: BTreeSet<u32> = entities.iter().map(|(number, _)| *number).collect();
        for (number, body) in &entities {
            for reference in references(body) {
                assert!(
                    defined.contains(&reference),
                    "{kind}: entity #{number} references undefined #{reference} ({body})"
                );
            }
        }
    }
}

#[test]
fn primitive_topology_is_exact_and_analytic() {
    let box_document = text(&default_bytes(PrimitiveKind::Box));
    let counts = type_counts(&box_document);
    assert_eq!(counts.get("ADVANCED_FACE"), Some(&6));
    assert_eq!(counts.get("PLANE"), Some(&6));
    assert_eq!(counts.get("EDGE_CURVE"), Some(&12));
    // Twelve shared edges, each used once in each direction by two faces.
    assert_eq!(counts.get("ORIENTED_EDGE"), Some(&24));
    assert_eq!(counts.get("VERTEX_POINT"), Some(&8));
    // Eight box corners plus the placement origin of the shape representation.
    assert_eq!(counts.get("CARTESIAN_POINT"), Some(&9));
    assert_eq!(counts.get("CLOSED_SHELL"), Some(&1));
    assert_eq!(counts.get("MANIFOLD_SOLID_BREP"), Some(&1));

    let cylinder = text(&default_bytes(PrimitiveKind::Cylinder));
    let counts = type_counts(&cylinder);
    assert_eq!(counts.get("ADVANCED_FACE"), Some(&3));
    assert_eq!(counts.get("CYLINDRICAL_SURFACE"), Some(&1));
    assert_eq!(counts.get("PLANE"), Some(&2));
    assert_eq!(counts.get("EDGE_CURVE"), Some(&3));
    assert_eq!(counts.get("CIRCLE"), Some(&2));
    assert_eq!(counts.get("LINE"), Some(&1));
    // Six ORIENTED_EDGE entities: the seam line and both circles appear once
    // in the lateral loop and once in a cap, each direction being one entity.
    assert_eq!(counts.get("ORIENTED_EDGE"), Some(&6));
    assert_eq!(counts.get("EDGE_LOOP"), Some(&3));
    assert_eq!(counts.get("FACE_BOUND"), Some(&3));
    assert_eq!(counts.get("VERTEX_POINT"), Some(&2));
    // Two seam vertices, two cap centres, one placement origin.
    assert_eq!(counts.get("CARTESIAN_POINT"), Some(&5));

    let sphere = text(&default_bytes(PrimitiveKind::Sphere));
    let counts = type_counts(&sphere);
    assert_eq!(counts.get("ADVANCED_FACE"), Some(&1));
    assert_eq!(counts.get("SPHERICAL_SURFACE"), Some(&1));
    assert_eq!(counts.get("VERTEX_LOOP"), Some(&1));
    assert_eq!(counts.get("EDGE_CURVE"), None);
    assert_eq!(counts.get("CARTESIAN_POINT"), Some(&2));
}

#[test]
fn there_is_no_tessellation_or_approximation_entity() {
    let allowed = BTreeSet::from([
        "ADVANCED_BREP_SHAPE_REPRESENTATION",
        "ADVANCED_FACE",
        "APPLICATION_CONTEXT",
        "APPLICATION_PROTOCOL_DEFINITION",
        "AXIS2_PLACEMENT_3D",
        "CARTESIAN_POINT",
        "CIRCLE",
        "CLOSED_SHELL",
        "COMPLEX",
        "CYLINDRICAL_SURFACE",
        "DIRECTION",
        "EDGE_CURVE",
        "EDGE_LOOP",
        "FACE_BOUND",
        "LINE",
        "MANIFOLD_SOLID_BREP",
        "ORIENTED_EDGE",
        "PLANE",
        "PRODUCT",
        "PRODUCT_CONTEXT",
        "PRODUCT_DEFINITION",
        "PRODUCT_DEFINITION_CONTEXT",
        "PRODUCT_DEFINITION_FORMATION",
        "PRODUCT_DEFINITION_SHAPE",
        "PRODUCT_RELATED_PRODUCT_CATEGORY",
        "SHAPE_DEFINITION_REPRESENTATION",
        "SPHERICAL_SURFACE",
        "UNCERTAINTY_MEASURE_WITH_UNIT",
        "VECTOR",
        "VERTEX_LOOP",
        "VERTEX_POINT",
    ]);
    for kind in [
        PrimitiveKind::Box,
        PrimitiveKind::Cylinder,
        PrimitiveKind::Sphere,
    ] {
        let document = text(&default_bytes(kind));
        for entity in type_counts(&document).keys() {
            assert!(
                allowed.contains(entity.as_str()),
                "{kind}: unexpected entity type {entity} (tessellation or approximation?)"
            );
        }
        for forbidden in [
            "POLY_LOOP",
            "FACETED_BREP",
            "TRIANGULATED_FACE_SET",
            "TESSELLATED",
            "B_SPLINE",
            "BEZIER",
            "MESH",
            "TRIANGLE",
        ] {
            assert!(
                !document.contains(forbidden),
                "{kind}: document contains forbidden {forbidden}"
            );
        }
    }
}

#[test]
fn default_primitives_are_origin_centred_in_millimetres() {
    let box_document = text(&default_bytes(PrimitiveKind::Box));
    assert!(box_document.contains("CARTESIAN_POINT('',(-5.,-5.,-5.))"));
    assert!(box_document.contains("CARTESIAN_POINT('',(5.,5.,5.))"));

    let cylinder = text(&default_bytes(PrimitiveKind::Cylinder));
    assert!(cylinder.contains("CARTESIAN_POINT('',(5.,0.,-5.))"));
    assert!(cylinder.contains("CARTESIAN_POINT('',(5.,0.,5.))"));
    assert!(cylinder.contains("CARTESIAN_POINT('',(0.,0.,-5.))"));
    assert!(cylinder.contains("CARTESIAN_POINT('',(0.,0.,5.))"));
    assert!(cylinder.contains("CYLINDRICAL_SURFACE('',#"));
    assert!(cylinder.contains(",5.)"));

    let sphere = text(&default_bytes(PrimitiveKind::Sphere));
    assert!(sphere.contains("CARTESIAN_POINT('',(0.,0.,-5.))"));
    assert!(sphere.contains("SPHERICAL_SURFACE('',#"));
}

#[test]
fn non_default_and_extreme_dimensions_keep_exact_half_extents() {
    let box_document = text(
        &export_step_primitive(
            PrimitiveKind::Box,
            Some(&json!({"size_x": 20.0, "size_y": 0.1, "size_z": 5.0})),
        )
        .unwrap(),
    );
    assert!(box_document.contains("CARTESIAN_POINT('',(-10.,-0.05,-2.5))"));
    assert!(box_document.contains("CARTESIAN_POINT('',(10.,0.05,2.5))"));

    let cylinder = text(
        &export_step_primitive(
            PrimitiveKind::Cylinder,
            Some(&json!({"radius": 0.1, "height": 2000.0})),
        )
        .unwrap(),
    );
    assert!(cylinder.contains("CARTESIAN_POINT('',(0.1,0.,-1000.))"));
    assert!(cylinder.contains("CARTESIAN_POINT('',(0.1,0.,1000.))"));
    assert!(cylinder.contains("CYLINDRICAL_SURFACE('',#"));

    let sphere = text(
        &export_step_primitive(PrimitiveKind::Sphere, Some(&json!({"radius": 2000.0}))).unwrap(),
    );
    assert!(sphere.contains("CARTESIAN_POINT('',(0.,0.,-2000.))"));
    assert!(sphere.contains(",2000.)"));
}

#[test]
fn exports_are_deterministic_and_bounded() {
    for (kind, parameters) in [
        (
            PrimitiveKind::Box,
            json!({"size_x": 2000.0, "size_y": 2000.0, "size_z": 2000.0}),
        ),
        (
            PrimitiveKind::Cylinder,
            json!({"radius": 2000.0, "height": 2000.0}),
        ),
        (PrimitiveKind::Sphere, json!({"radius": 2000.0})),
    ] {
        let first = export_step_primitive(kind, Some(&parameters)).unwrap();
        let second = export_step_primitive(kind, Some(&parameters)).unwrap();
        assert_eq!(first, second, "{kind}: repeated exports must be identical");
        assert!(
            first.len() <= MAX_STEP_BYTES,
            "{kind}: {} bytes exceed the bound",
            first.len()
        );
    }
}

#[test]
fn invalid_parameters_are_refused_with_python_messages() {
    let expect_invalid = |value: Option<&Value>, message: &str| {
        let error = export_step_primitive(PrimitiveKind::Box, value).unwrap_err();
        match error {
            StepError::Invalid(geometry) => assert_eq!(geometry.message(), message),
            other => panic!("expected an invalid-dimension error, got {other:?}"),
        }
    };

    expect_invalid(
        Some(&json!({"size_x": 0.09})),
        "size_x must be between 0.1 and 2000.0 millimetres",
    );
    expect_invalid(
        Some(&json!({"size_y": 2000.1})),
        "size_y must be between 0.1 and 2000.0 millimetres",
    );
    expect_invalid(
        Some(&json!({"size_z": -1})),
        "size_z must be between 0.1 and 2000.0 millimetres",
    );
    expect_invalid(
        Some(&json!({"size_x": 0})),
        "size_x must be between 0.1 and 2000.0 millimetres",
    );
    expect_invalid(Some(&json!([])), "parameters must be a JSON object");
    expect_invalid(Some(&json!("box")), "parameters must be a JSON object");
}

#[test]
fn non_numeric_and_non_finite_dimension_values_are_refused() {
    let expect_invalid = |value: Value, message: &str| {
        let error = export_step_primitive(PrimitiveKind::Sphere, Some(&value)).unwrap_err();
        assert_eq!(error.message(), message, "for {value}");
    };

    expect_invalid(json!({"radius": true}), "radius must be a finite number");
    expect_invalid(json!({"radius": "5"}), "radius must be a finite number");
    expect_invalid(json!({"radius": null}), "radius must be a finite number");
    expect_invalid(json!({"radius": [5]}), "radius must be a finite number");
    expect_invalid(
        json!({"radius": {"value": 5}}),
        "radius must be a finite number",
    );
    // A JSON number beyond the f64 range cannot be written as a Rust literal,
    // so it is parsed from text; the geometry adapter sees an infinity and
    // refuses it as non-finite.
    let overflowing: Value =
        serde_json::from_str(r#"{"radius": 1e400}"#).expect("serde_json accepts wide numbers");
    expect_invalid(overflowing, "radius must be a finite number");
}

#[test]
fn unknown_kinds_and_unknown_parameters_behave_like_python() {
    let error = export_kind_str("torus", None).unwrap_err();
    assert_eq!(error.message(), "unsupported kind: torus");

    // Unknown parameter names are ignored by the geometry layer, exactly as
    // `resolve_dimension_parameters` ignores them.
    let bytes = export_step_primitive(
        PrimitiveKind::Box,
        Some(&json!({"size_x": 20, "unused": true, "other": "x"})),
    )
    .unwrap();
    let document = text(&bytes);
    assert!(document.contains("CARTESIAN_POINT('',(10.,-5.,-5.))"));

    // Integer JSON dimensions are accepted like floats.
    let other = export_step_primitive(PrimitiveKind::Box, Some(&json!({"size_x": 20}))).unwrap();
    assert_eq!(bytes, other);

    // A JSON null parameter block means "no parameters": the defaults apply.
    let defaults = export_step_primitive(PrimitiveKind::Box, Some(&Value::Null)).unwrap();
    assert_eq!(defaults, default_bytes(PrimitiveKind::Box));
}

#[test]
fn capability_is_honest_and_the_self_check_runs() {
    let capability = probe_capability();
    assert!(
        capability.available,
        "the native writer is always available"
    );
    assert_eq!(capability.exporter_name, Some(STEP_EXPORTER_NAME));
    self_check().expect("the bounded self-check passes");

    let exporter = NativeStepExporter;
    assert_eq!(exporter.capability(), capability);
    let workdir = std::env::temp_dir();
    let via_trait = exporter
        .export_primitive(PrimitiveKind::Cylinder, None, &workdir)
        .unwrap();
    assert_eq!(via_trait, default_bytes(PrimitiveKind::Cylinder));
}

#[test]
fn the_exporter_never_writes_to_the_work_path() {
    let workdir = std::env::temp_dir().join(format!(
        "kala-step-media-{}-{}",
        std::process::id(),
        line!()
    ));
    let _ = std::fs::remove_dir_all(&workdir);
    let exporter = NativeStepExporter;
    for kind in [
        PrimitiveKind::Box,
        PrimitiveKind::Cylinder,
        PrimitiveKind::Sphere,
    ] {
        exporter
            .export_primitive(kind, None, &workdir)
            .expect("the export succeeds even though nothing is written");
    }
    // The native path assembles the document in memory: it must not create the
    // directory, a temporary file, or anything else.
    assert!(
        !workdir.exists(),
        "the exporter wrote to {}",
        workdir.display()
    );
}

#[test]
fn concurrent_exports_are_deterministic_and_isolated() {
    let expected = Arc::new(text(&default_bytes(PrimitiveKind::Box)));
    let mut handles = Vec::new();
    for worker in 0..8u32 {
        let expected = Arc::clone(&expected);
        handles.push(thread::spawn(move || {
            for iteration in 0..25 {
                let parameters = json!({"size_x": 10.0, "size_y": 10.0, "size_z": 10.0});
                let bytes = export_step_primitive(PrimitiveKind::Box, Some(&parameters)).unwrap();
                assert_eq!(
                    text(&bytes),
                    *expected,
                    "worker {worker} iteration {iteration}"
                );
            }
        }));
    }
    for handle in handles {
        handle.join().expect("no worker panics");
    }
}

#[test]
fn validation_errors_never_echo_the_submitted_value() {
    let secret = "kala-secret-value";
    let error =
        export_step_primitive(PrimitiveKind::Sphere, Some(&json!({"radius": secret}))).unwrap_err();
    assert!(!error.to_string().contains(secret));
    assert_eq!(error.to_string(), "radius must be a finite number");
}

#[test]
fn workdir_types_are_paths_and_never_touched() {
    // A path that cannot be created is still accepted: the exporter has no
    // filesystem dependency at all.
    let impossible = PathBuf::from("/proc/definitely-not-writable/kala-step");
    assert!(Path::new("/proc").exists());
    NativeStepExporter
        .export_primitive(PrimitiveKind::Sphere, None, &impossible)
        .expect("no filesystem access happens");
}
