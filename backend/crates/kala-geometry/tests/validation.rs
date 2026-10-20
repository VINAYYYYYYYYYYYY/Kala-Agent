//! Validation, type-distinction, and resource-cap tests.
//!
//! These are native Rust tests, not fixture mirrors: they exercise the public
//! typed builders and the serde/JSON adapter directly, including boundary
//! values, booleans, integral floats used as counts, unknown names, and
//! sanitized messages.

mod common;

use kala_geometry::{
    GeometryError, MAX_MESH_TRIANGLES, MAX_MESH_VERTICES, Mesh, PrimitiveParameters, PrimitiveSpec,
    build_box, build_cylinder, build_primitive, build_primitive_json, build_spec, build_sphere,
};
use serde_json::json;

fn error_message(result: Result<impl std::fmt::Debug, GeometryError>) -> String {
    result.unwrap_err().message().to_owned()
}

#[test]
fn typed_dimension_boundaries_and_non_finite_values() {
    assert!(build_box(0.1, 2000.0, 10.0, 1).is_ok());
    assert_eq!(
        error_message(build_box(0.099_999_999_999_999_99, 10.0, 10.0, 1)),
        "size_x must be between 0.1 and 2000.0 millimetres"
    );
    assert_eq!(
        error_message(build_box(10.0, 2_000.000_000_000_1, 10.0, 1)),
        "size_y must be between 0.1 and 2000.0 millimetres"
    );
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            error_message(build_box(value, 10.0, 10.0, 1)),
            "size_x must be a finite number"
        );
        assert_eq!(
            error_message(build_cylinder(5.0, value, 48, 1)),
            "height must be a finite number"
        );
        assert_eq!(
            error_message(build_sphere(value, 24, 32)),
            "radius must be a finite number"
        );
    }
}

#[test]
fn typed_tessellation_boundaries_match_python() {
    assert!(build_box(10.0, 10.0, 10.0, 1).is_ok());
    assert!(build_box(10.0, 10.0, 10.0, 64).is_ok());
    assert_eq!(
        error_message(build_box(10.0, 10.0, 10.0, 0)),
        "subdivisions must be between 1 and 64"
    );
    assert_eq!(
        error_message(build_box(10.0, 10.0, 10.0, 65)),
        "subdivisions must be between 1 and 64"
    );

    assert!(build_cylinder(5.0, 10.0, 6, 1).is_ok());
    assert!(build_cylinder(5.0, 10.0, 256, 64).is_ok());
    assert_eq!(
        error_message(build_cylinder(5.0, 10.0, 5, 1)),
        "radial_segments must be between 6 and 256"
    );
    assert_eq!(
        error_message(build_cylinder(5.0, 10.0, 257, 1)),
        "radial_segments must be between 6 and 256"
    );
    assert_eq!(
        error_message(build_cylinder(5.0, 10.0, 48, 0)),
        "height_segments must be between 1 and 64"
    );
    assert_eq!(
        error_message(build_cylinder(5.0, 10.0, 48, 65)),
        "height_segments must be between 1 and 64"
    );

    assert!(build_sphere(5.0, 3, 4).is_ok());
    assert!(build_sphere(5.0, 96, 128).is_ok());
    assert_eq!(
        error_message(build_sphere(5.0, 2, 32)),
        "rings must be between 3 and 96"
    );
    assert_eq!(
        error_message(build_sphere(5.0, 97, 32)),
        "rings must be between 3 and 96"
    );
    assert_eq!(
        error_message(build_sphere(5.0, 24, 3)),
        "sectors must be between 4 and 128"
    );
    assert_eq!(
        error_message(build_sphere(5.0, 24, 129)),
        "sectors must be between 4 and 128"
    );
}

#[test]
fn json_adapter_rejects_bool_string_and_float_in_integer_slots() {
    // Dimensions accept JSON integers and floats, but never booleans, strings,
    // nulls, arrays, or objects.
    for value in [
        json!(true),
        json!(false),
        json!("10"),
        json!(null),
        json!([]),
        json!({}),
    ] {
        let parameters = PrimitiveParameters::from_json_value(&json!({ "size_x": value })).unwrap();
        assert_eq!(
            error_message(build_primitive("box", &parameters)),
            "size_x must be a finite number",
            "size_x value {value}"
        );
    }

    // Counts accept JSON integers only. Integral JSON floats are still floats
    // and must be refused exactly as Python's isinstance(value, int) does.
    for value in [
        json!(true),
        json!(false),
        json!(2.0),
        json!(2.5),
        json!("2"),
        json!(null),
        json!([]),
        json!({}),
    ] {
        let parameters =
            PrimitiveParameters::from_json_value(&json!({ "subdivisions": value })).unwrap();
        assert_eq!(
            error_message(build_primitive("box", &parameters)),
            "subdivisions must be an integer",
            "subdivisions value {value}"
        );
    }

    let integer_dimension =
        PrimitiveParameters::from_json_value(&json!({"size_x": 10, "size_y": 20.0})).unwrap();
    let mesh = build_primitive("box", &integer_dimension).unwrap();
    assert_eq!(mesh.vertex_count(), 24);
    let bounds = kala_geometry::compute_bounds(mesh.positions()).unwrap();
    assert_eq!(bounds.min, [-5.0, -10.0, -5.0]);
    assert_eq!(bounds.max, [5.0, 10.0, 5.0]);
}

#[test]
fn json_adapter_range_messages_are_exact() {
    let cases = [
        (
            "box",
            json!({"subdivisions": 0}),
            "subdivisions must be between 1 and 64",
        ),
        (
            "box",
            json!({"subdivisions": 65}),
            "subdivisions must be between 1 and 64",
        ),
        (
            "box",
            json!({"size_x": 0.01}),
            "size_x must be between 0.1 and 2000.0 millimetres",
        ),
        (
            "box",
            json!({"size_z": 2000.01}),
            "size_z must be between 0.1 and 2000.0 millimetres",
        ),
        (
            "cylinder",
            json!({"radial_segments": 5}),
            "radial_segments must be between 6 and 256",
        ),
        (
            "cylinder",
            json!({"height_segments": 65}),
            "height_segments must be between 1 and 64",
        ),
        (
            "cylinder",
            json!({"radius": -1}),
            "radius must be between 0.1 and 2000.0 millimetres",
        ),
        (
            "sphere",
            json!({"rings": 2}),
            "rings must be between 3 and 96",
        ),
        (
            "sphere",
            json!({"sectors": 129}),
            "sectors must be between 4 and 128",
        ),
    ];
    for (kind, parameters, expected) in cases {
        let parameters = PrimitiveParameters::from_json_value(&parameters).unwrap();
        assert_eq!(
            error_message(build_primitive(kind, &parameters)),
            expected,
            "{kind} {parameters:?}"
        );
    }
}

#[test]
fn unknown_kinds_and_parameters_are_refused() {
    assert_eq!(
        error_message(build_primitive("", &PrimitiveParameters::new())),
        "kind must be a non-empty string"
    );
    assert_eq!(
        error_message(build_primitive("torus", &PrimitiveParameters::new())),
        "unsupported kind: torus"
    );
    assert_eq!(
        error_message(build_primitive(
            "box",
            &PrimitiveParameters::from_json_value(&json!({"radius": 5})).unwrap()
        )),
        "box parameters must be a subset of size_x, size_y, size_z, subdivisions"
    );
    assert_eq!(
        error_message(build_primitive(
            "cylinder",
            &PrimitiveParameters::from_json_value(&json!({"size_x": 5})).unwrap()
        )),
        "cylinder parameters must be a subset of radius, height, radial_segments, height_segments"
    );
    assert_eq!(
        error_message(build_primitive(
            "sphere",
            &PrimitiveParameters::from_json_value(&json!({"radial_segments": 8})).unwrap()
        )),
        "sphere parameters must be a subset of radius, rings, sectors"
    );
}

#[test]
fn invalid_parameter_shape_is_reported_without_echoing_it() {
    assert_eq!(
        PrimitiveParameters::from_json_value(&json!([1, 2, 3]))
            .unwrap_err()
            .message(),
        "parameters must be a JSON object"
    );
    let parameters =
        PrimitiveParameters::from_json_value(&json!({"size_x": "super-secret-token"})).unwrap();
    let message = error_message(build_primitive("box", &parameters));
    assert_eq!(message, "size_x must be a finite number");
    assert!(!message.contains("super-secret-token"));

    let parameters = PrimitiveParameters::from_json_value(&json!({"size_x": 123_456_789})).unwrap();
    let message = error_message(build_primitive("box", &parameters));
    assert_eq!(message, "size_x must be between 0.1 and 2000.0 millimetres");
    assert!(!message.contains("123456789"));
}

#[test]
fn json_helpers_and_typed_specs_build_defaults() {
    assert_eq!(
        build_primitive_json("box", None).unwrap().vertex_count(),
        24
    );
    assert_eq!(
        build_primitive_json("box", Some(&json!(null)))
            .unwrap()
            .vertex_count(),
        24
    );
    assert_eq!(
        build_primitive_json("box", Some(&json!({"size_x": 10})))
            .unwrap()
            .vertex_count(),
        24
    );
    assert_eq!(
        build_spec(&PrimitiveSpec::default_box())
            .unwrap()
            .vertex_count(),
        24
    );
    assert_eq!(
        build_spec(&PrimitiveSpec::default_cylinder())
            .unwrap()
            .vertex_count(),
        198
    );
    assert_eq!(
        build_spec(&PrimitiveSpec::default_sphere())
            .unwrap()
            .vertex_count(),
        761
    );
    assert_eq!(
        error_message(build_spec(&PrimitiveSpec::Box {
            size: [0.0, 10.0, 10.0],
            subdivisions: 1
        })),
        "size_x must be between 0.1 and 2000.0 millimetres"
    );
}

#[test]
fn resource_caps_and_mesh_structure_are_enforced() {
    let vertex_error = Mesh::from_parts(
        vec![0.0; (MAX_MESH_VERTICES + 1) * 3],
        vec![0.0; (MAX_MESH_VERTICES + 1) * 3],
        Vec::new(),
    )
    .unwrap_err();
    assert_eq!(
        vertex_error.message(),
        "the requested primitive would produce 200001 vertices; the limit is 200000"
    );

    let triangle_error = Mesh::from_parts(
        vec![0.0; 3],
        vec![0.0; 3],
        vec![0; (MAX_MESH_TRIANGLES + 1) * 3],
    )
    .unwrap_err();
    assert_eq!(
        triangle_error.message(),
        "the requested primitive would produce 400001 triangles; the limit is 400000"
    );
}
