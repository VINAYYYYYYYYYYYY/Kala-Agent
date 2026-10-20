//! The Rust port of the Python simulator branch tests.
//!
//! The source is `tests/unit/test_simulator_branches.py`. Every branch of the
//! simulator that the Python implementation covers is exercised here, with
//! the same expectations and the same messages.

mod support;

use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_tools::protocol::{ToolErrorCode, ToolHandler};
use kala_tools::registry::ToolRegistry;
use kala_tools::simulator::{
    EVALUATOR_TOOL_ID, ReferenceEvaluatorHandler, SimulatorParameters, SimulatorToolHandler,
    compute_metrics, escape_invoke, evaluator_descriptor, flood_invoke, liar_invoke,
    parse_parameters, register_reference_evaluator, register_simulator, report_payload,
    simulator_descriptor, simulator_handler, unrecognized_code_invoke, write_report,
};

use support::{request, symlink_directory};

/// The parameters of the tests.
fn parameters() -> SimulatorParameters {
    SimulatorParameters::new("beam_volume", 120.0, 20.0, 10.0, 2_700.0, false)
}

/// The arguments of a call.
fn arguments(entries: &[(&str, CanonicalValue)]) -> CanonicalObject {
    let mut object = CanonicalObject::new();
    for (key, value) in entries {
        object.insert(JsonText::new(key), value.clone());
    }
    object
}

/// The number of a metric.
fn number(metrics: &CanonicalValue, key: &str) -> f64 {
    let CanonicalValue::Object(metrics) = metrics else {
        panic!("metrics are an object");
    };
    match metrics.get(&JsonText::new(key)) {
        Some(CanonicalValue::Float(value)) => *value,
        _ => panic!("the metric is a number"),
    }
}

/// The value of a member of the result.
fn member(result: &CanonicalValue, key: &str) -> CanonicalValue {
    let CanonicalValue::Object(result) = result else {
        panic!("the result is an object");
    };
    result
        .get(&JsonText::new(key))
        .cloned()
        .unwrap_or(CanonicalValue::Null)
}

#[test]
fn the_parameters_payload_roundtrips() {
    let payload = parameters().to_payload();
    let CanonicalValue::Object(payload) = &payload else {
        panic!("the payload is an object");
    };
    assert_eq!(payload.len(), 6);
    assert_eq!(
        payload.get(&JsonText::new("density_kg_m3")),
        Some(&CanonicalValue::Float(2_700.0))
    );
    assert_eq!(
        payload.get(&JsonText::new("height_mm")),
        Some(&CanonicalValue::Float(10.0))
    );
    assert_eq!(
        payload.get(&JsonText::new("length_mm")),
        Some(&CanonicalValue::Float(120.0))
    );
    assert_eq!(
        payload.get(&JsonText::new("operation")),
        Some(&CanonicalValue::text("beam_volume"))
    );
    assert_eq!(
        payload.get(&JsonText::new("width_mm")),
        Some(&CanonicalValue::Float(20.0))
    );
    assert_eq!(
        payload.get(&JsonText::new("write_report")),
        Some(&CanonicalValue::Bool(false))
    );
}

#[test]
fn the_parse_rejects_a_non_mapping() {
    let error = parse_parameters(&CanonicalValue::array([CanonicalValue::text("operation")]))
        .expect_err("refused");
    assert_eq!(error.python_class_name(), "SimulatorInputError");
    assert!(error.to_string().contains("JSON object"));
}

#[test]
fn the_parse_rejects_an_unknown_operation() {
    for value in [
        CanonicalValue::text("shear_stress"),
        CanonicalValue::integer(7),
        CanonicalValue::Null,
    ] {
        let error = parse_parameters(&CanonicalValue::Object(arguments(&[("operation", value)])))
            .expect_err("refused");
        assert!(error.to_string().contains("operation must be one of"));
    }
}

#[test]
fn the_parse_defaults_every_dimension() {
    let parsed = parse_parameters(&CanonicalValue::Object(CanonicalObject::new())).expect("parsed");
    assert_eq!(parsed.operation(), "beam_volume");
    assert_eq!(parsed.length_mm(), 120.0);
    assert_eq!(parsed.width_mm(), 20.0);
    assert_eq!(parsed.height_mm(), 10.0);
    assert_eq!(parsed.density_kg_m3(), 2_700.0);
    assert!(!parsed.write_report());
}

#[test]
fn the_parse_accepts_integer_dimensions() {
    let parsed = parse_parameters(&CanonicalValue::Object(arguments(&[
        ("operation", CanonicalValue::text("bar_summary")),
        ("length_mm", CanonicalValue::integer(200)),
    ])))
    .expect("parsed");
    assert_eq!(parsed.operation(), "bar_summary");
    assert_eq!(parsed.length_mm(), 200.0);
}

#[test]
fn the_parse_rejects_non_numeric_dimensions() {
    let error = parse_parameters(&CanonicalValue::Object(arguments(&[(
        "length_mm",
        CanonicalValue::text("200"),
    )])))
    .expect_err("refused");
    assert!(error.to_string().contains("length_mm must be a number"));
    let error = parse_parameters(&CanonicalValue::Object(arguments(&[(
        "width_mm",
        CanonicalValue::Bool(true),
    )])))
    .expect_err("refused");
    assert!(error.to_string().contains("width_mm must be a number"));
}

#[test]
fn the_parse_rejects_non_finite_dimensions() {
    let error = parse_parameters(&CanonicalValue::Object(arguments(&[(
        "height_mm",
        CanonicalValue::Float(f64::INFINITY),
    )])))
    .expect_err("refused");
    assert!(error.to_string().contains("height_mm must be finite"));
    let error = parse_parameters(&CanonicalValue::Object(arguments(&[(
        "density_kg_m3",
        CanonicalValue::Float(f64::NAN),
    )])))
    .expect_err("refused");
    assert!(error.to_string().contains("density_kg_m3 must be finite"));
}

#[test]
fn the_parse_enforces_the_dimension_bounds() {
    for (name, value, expected) in [
        ("length_mm", 5_000.1, "length_mm must be between"),
        ("width_mm", 0.5, "width_mm must be between"),
        ("height_mm", 500.5, "height_mm must be between"),
        ("density_kg_m3", 20_000.5, "density_kg_m3 must be between"),
    ] {
        let error = parse_parameters(&CanonicalValue::Object(arguments(&[(
            name,
            CanonicalValue::Float(value),
        )])))
        .expect_err("refused");
        assert!(error.to_string().contains(expected), "{name}");
    }
}

#[test]
fn the_parse_rejects_a_non_boolean_write_report() {
    let error = parse_parameters(&CanonicalValue::Object(arguments(&[(
        "write_report",
        CanonicalValue::text("yes"),
    )])))
    .expect_err("refused");
    assert!(error.to_string().contains("write_report must be a boolean"));
}

#[test]
fn the_metrics_match_the_closed_form_reference() {
    let metrics = compute_metrics(&parameters());
    assert_eq!(number(&metrics, "volume_mm3"), 24_000.0);
    assert_eq!(number(&metrics, "mass_kg"), 0.0648);
    assert_eq!(number(&metrics, "surface_area_mm2"), 7_600.0);
    assert_eq!(number(&metrics, "slenderness_ratio"), 12.0);
    assert_eq!(number(&metrics, "second_moment_mm4"), 200_000.0);
    assert_eq!(
        member(&metrics, "within_limits"),
        CanonicalValue::Bool(true)
    );
}

#[test]
fn the_metrics_floor_the_slenderness_divisor() {
    let metrics = compute_metrics(&SimulatorParameters::new(
        "beam_volume",
        120.0,
        20.0,
        0.0,
        2_700.0,
        false,
    ));
    assert_eq!(number(&metrics, "slenderness_ratio"), 120.0);
}

#[test]
fn the_metrics_accept_the_largest_schema_bounded_body() {
    let parsed = parse_parameters(&CanonicalValue::Object(arguments(&[
        ("length_mm", CanonicalValue::Float(5_000.0)),
        ("width_mm", CanonicalValue::Float(500.0)),
        ("height_mm", CanonicalValue::Float(500.0)),
        ("density_kg_m3", CanonicalValue::Float(20_000.0)),
    ])))
    .expect("parsed");
    let metrics = compute_metrics(&parsed);
    assert_eq!(number(&metrics, "volume_mm3"), 1_250_000_000.0);
    assert!(number(&metrics, "mass_kg") <= 27_000.0);
    assert!(number(&metrics, "slenderness_ratio") <= 5_000.0);
    assert_eq!(
        member(&metrics, "within_limits"),
        CanonicalValue::Bool(true)
    );
}

#[test]
fn the_metrics_flag_the_volume_overflow() {
    let metrics = compute_metrics(&SimulatorParameters::new(
        "beam_volume",
        5_000.0,
        500.0,
        600.0,
        2_700.0,
        false,
    ));
    assert!(number(&metrics, "volume_mm3") > 1_250_000_000.0);
    assert_eq!(
        member(&metrics, "within_limits"),
        CanonicalValue::Bool(false)
    );
}

#[test]
fn the_metrics_flag_the_mass_overflow() {
    let metrics = compute_metrics(&SimulatorParameters::new(
        "beam_volume",
        5_000.0,
        500.0,
        500.0,
        30_000.0,
        false,
    ));
    assert!(number(&metrics, "volume_mm3") <= 1_250_000_000.0);
    assert!(number(&metrics, "mass_kg") > 27_000.0);
    assert_eq!(
        member(&metrics, "within_limits"),
        CanonicalValue::Bool(false)
    );
}

#[test]
fn the_metrics_flag_the_slenderness_overflow() {
    let metrics = compute_metrics(&SimulatorParameters::new(
        "beam_volume",
        10_000.0,
        20.0,
        0.5,
        2_700.0,
        false,
    ));
    assert!(number(&metrics, "volume_mm3") <= 1_250_000_000.0);
    assert!(number(&metrics, "mass_kg") <= 27_000.0);
    assert!(number(&metrics, "slenderness_ratio") > 5_000.0);
    assert_eq!(
        member(&metrics, "within_limits"),
        CanonicalValue::Bool(false)
    );
}

#[test]
fn the_report_payload_is_canonical_and_self_describing() {
    let parameters = parameters();
    let payload = report_payload(&parameters, &compute_metrics(&parameters), "call-7");
    assert_eq!(member(&payload, "call_id"), CanonicalValue::text("call-7"));
    assert_eq!(
        member(&payload, "tool_id"),
        CanonicalValue::text("mechanical.reference-simulator")
    );
    assert_eq!(
        member(&payload, "tool_version"),
        CanonicalValue::text("1.0.0")
    );
    let disclaimer = member(&payload, "disclaimer");
    let CanonicalValue::Text(ref disclaimer) = disclaimer else {
        panic!("the disclaimer is text");
    };
    assert!(
        disclaimer
            .to_string_lossy()
            .contains("not an engineering validation")
    );
    let inputs = member(&payload, "inputs");
    assert_eq!(
        member(&inputs, "operation"),
        CanonicalValue::text("beam_volume")
    );
    let metrics = member(&payload, "metrics");
    assert!(matches!(metrics, CanonicalValue::Object(_)));
}

#[test]
fn the_write_report_publishes_an_artifact() {
    let workspace = tempfile::tempdir().expect("workspace");
    let parameters = parameters();
    let (artifact, relative) = write_report(
        workspace.path(),
        &parameters,
        &compute_metrics(&parameters),
        "call-1",
    )
    .expect("report");
    assert_eq!(relative, "outputs/reference-report.json");
    let target = workspace
        .path()
        .join("outputs")
        .join("reference-report.json");
    assert!(target.is_file());
    assert_eq!(artifact.artifact_id(), "call-1-report");
    assert_eq!(
        artifact.uri().as_text().to_string_lossy(),
        format!(
            "file://{}",
            target.canonicalize().expect("target").to_string_lossy()
        )
    );
    assert_eq!(
        artifact.size_bytes().as_bigint().to_string(),
        std::fs::metadata(&target)
            .expect("metadata")
            .len()
            .to_string()
    );
    assert_eq!(artifact.media_type(), "application/json");
}

#[test]
fn the_write_report_overwrites_the_previous_report() {
    let workspace = tempfile::tempdir().expect("workspace");
    let parameters = parameters();
    let first = write_report(
        workspace.path(),
        &parameters,
        &compute_metrics(&parameters),
        "a",
    )
    .expect("report");
    let second = write_report(
        workspace.path(),
        &parameters,
        &compute_metrics(&parameters),
        "b",
    )
    .expect("report");
    assert_ne!(first.0.digest(), second.0.digest());
    let target = workspace
        .path()
        .join("outputs")
        .join("reference-report.json");
    let document = std::fs::read_to_string(&target).expect("report");
    assert!(document.contains("\"call_id\":\"b\""));
}

#[test]
fn the_write_report_reports_a_workspace_violation() {
    let workspace = tempfile::tempdir().expect("workspace");
    let parameters = parameters();
    let error = write_report(
        &workspace.path().join("missing"),
        &parameters,
        &compute_metrics(&parameters),
        "call-1",
    )
    .expect_err("refused");
    assert_eq!(error.python_class_name(), "SimulatorInputError");
    assert!(error.to_string().contains("not inside the workspace"));
}

#[test]
fn the_write_workspace_file_reports_an_uncreatable_directory() {
    let workspace = tempfile::tempdir().expect("workspace");
    let blocker = workspace.path().join("blocked");
    std::fs::write(&blocker, "x").expect("blocker");
    let error = kala_tools::simulator::write_workspace_file(
        &blocker.join("nested").join("report.json"),
        b"{}",
    )
    .expect_err("refused");
    assert!(
        error
            .to_string()
            .contains("report directory could not be created")
    );
}

#[test]
fn the_write_workspace_file_rejects_a_symlinked_directory() {
    let workspace = tempfile::tempdir().expect("workspace");
    let real = workspace.path().join("real");
    std::fs::create_dir(&real).expect("directory");
    let linked = workspace.path().join("linked");
    symlink_directory(&real, &linked).expect("symlink");
    let error = kala_tools::simulator::write_workspace_file(&linked.join("report.json"), b"{}")
        .expect_err("refused");
    assert!(error.to_string().contains("must be a real directory"));
}

#[test]
#[cfg(unix)]
fn the_write_workspace_file_reports_a_failed_publication() {
    let workspace = tempfile::tempdir().expect("workspace");
    let outputs = workspace.path().join("outputs");
    std::fs::create_dir(&outputs).expect("directory");
    let mut permissions = std::fs::metadata(&outputs).expect("metadata").permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o500);
    }
    std::fs::set_permissions(&outputs, permissions).expect("permissions");
    let error = kala_tools::simulator::write_workspace_file(&outputs.join("report.json"), b"{}")
        .expect_err("refused");
    // The read only directory refuses the creation of the temporary file.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&outputs).expect("metadata").permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&outputs, permissions).expect("permissions");
    }
    assert!(error.to_string().contains("report could not be written"));
    let leftovers: Vec<_> = std::fs::read_dir(&outputs)
        .expect("directory")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".partial"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn the_write_workspace_file_overwrites_an_existing_target() {
    let workspace = tempfile::tempdir().expect("workspace");
    let target = workspace.path().join("outputs").join("report.json");
    kala_tools::simulator::write_workspace_file(&target, b"first").expect("written");
    kala_tools::simulator::write_workspace_file(&target, b"second").expect("written");
    assert_eq!(std::fs::read(&target).expect("target"), b"second");
}

#[test]
fn the_simulator_descriptor_declares_the_side_effects() {
    let descriptor = simulator_descriptor("custom.tool", "9.9.9").expect("descriptor");
    assert_eq!(descriptor.tool_id(), "custom.tool");
    assert_eq!(descriptor.version().as_str(), Some("9.9.9"));
    assert_eq!(descriptor.side_effect_level().as_str(), "local_write");
    assert_eq!(descriptor.timeout_seconds(), 20.0);
    assert!(!descriptor.destructive());
    assert!(descriptor.supports_structured_output());
    assert!(descriptor.idempotent());
    assert!(
        descriptor
            .required_capabilities()
            .iter()
            .any(|capability| capability.as_str() == "artifact.write")
    );
}

#[test]
fn the_evaluator_descriptor_declares_no_side_effects() {
    let descriptor = evaluator_descriptor("custom.evaluator", "2.0.0").expect("descriptor");
    assert_eq!(descriptor.tool_id(), "custom.evaluator");
    assert_eq!(descriptor.version().as_str(), Some("2.0.0"));
    assert_eq!(descriptor.side_effect_level().as_str(), "none");
    assert!(descriptor.required_capabilities().is_empty());
    assert_eq!(descriptor.timeout_seconds(), 10.0);
    assert!(
        descriptor
            .description()
            .as_str()
            .unwrap_or_default()
            .contains("filesystem")
    );
}

#[test]
fn the_simulator_handler_wraps_the_descriptor() {
    let handler = simulator_handler("custom.tool", "3.0.0").expect("handler");
    assert_eq!(handler.descriptor().tool_id(), "custom.tool");
    assert_eq!(handler.descriptor().version().as_str(), Some("3.0.0"));
}

#[test]
fn the_simulator_tool_handler_invokes() {
    let workspace = tempfile::tempdir().expect("workspace");
    let handler =
        SimulatorToolHandler::new("mechanical.reference-simulator", "1.0.0").expect("handler");
    let response = handler
        .invoke(&request(
            workspace.path(),
            &arguments(&[("operation", CanonicalValue::text("beam_volume"))]),
            "call-1",
        ))
        .expect("response");
    assert!(response.succeeded());
    assert_eq!(
        handler.descriptor().tool_id(),
        "mechanical.reference-simulator"
    );
    let response = handler
        .invoke(&request(
            workspace.path(),
            &arguments(&[("operation", CanonicalValue::text("unknown"))]),
            "call-1",
        ))
        .expect("response");
    assert!(!response.succeeded());
}

#[test]
fn the_simulator_tool_handler_honours_a_custom_identity() {
    let workspace = tempfile::tempdir().expect("workspace");
    let handler = SimulatorToolHandler::new("custom.tool", "4.0.0").expect("handler");
    assert_eq!(handler.descriptor().tool_id(), "custom.tool");
    let response = handler
        .invoke(&request(
            workspace.path(),
            &arguments(&[("operation", CanonicalValue::text("beam_volume"))]),
            "call-1",
        ))
        .expect("response");
    assert!(response.succeeded());
}

#[test]
fn the_evaluator_handler_returns_the_metrics() {
    let workspace = tempfile::tempdir().expect("workspace");
    let handler = ReferenceEvaluatorHandler::new(EVALUATOR_TOOL_ID, "1.0.0").expect("handler");
    let response = handler
        .invoke(&request(
            workspace.path(),
            &arguments(&[("operation", CanonicalValue::text("beam_volume"))]),
            "call-1",
        ))
        .expect("response");
    assert!(response.succeeded());
    let result = response.result().expect("result");
    assert_eq!(
        result.get(&JsonText::new("operation")),
        Some(&CanonicalValue::text("beam_volume"))
    );
    assert_eq!(
        result.get(&JsonText::new("report_path")),
        Some(&CanonicalValue::Null)
    );
    assert_eq!(
        result.get(&JsonText::new("within_limits")),
        Some(&CanonicalValue::Bool(true))
    );
}

#[test]
fn the_evaluator_handler_honours_a_custom_identity() {
    let workspace = tempfile::tempdir().expect("workspace");
    let handler = ReferenceEvaluatorHandler::new("custom.evaluator", "5.0.0").expect("handler");
    assert_eq!(handler.descriptor().tool_id(), "custom.evaluator");
    let response = handler
        .invoke(&request(
            workspace.path(),
            &arguments(&[("operation", CanonicalValue::text("beam_volume"))]),
            "call-1",
        ))
        .expect("response");
    assert!(response.succeeded());
}

#[test]
fn the_evaluator_refuses_to_write_reports() {
    let workspace = tempfile::tempdir().expect("workspace");
    let handler = ReferenceEvaluatorHandler::new(EVALUATOR_TOOL_ID, "1.0.0").expect("handler");
    let response = handler
        .invoke(&request(
            workspace.path(),
            &arguments(&[
                ("operation", CanonicalValue::text("beam_volume")),
                ("write_report", CanonicalValue::Bool(true)),
            ]),
            "call-1",
        ))
        .expect("response");
    assert!(!response.succeeded());
    assert_eq!(response.error_code(), Some("workspace_violation"));
    assert_eq!(
        response.error_code(),
        Some(ToolErrorCode::WorkspaceViolation.as_str())
    );
    assert!(
        response
            .error_message()
            .unwrap_or_default()
            .contains("cannot write a report")
    );
}

#[test]
fn the_evaluator_reports_invalid_arguments() {
    let workspace = tempfile::tempdir().expect("workspace");
    let handler = ReferenceEvaluatorHandler::new(EVALUATOR_TOOL_ID, "1.0.0").expect("handler");
    let response = handler
        .invoke(&request(
            workspace.path(),
            &arguments(&[("operation", CanonicalValue::text("nope"))]),
            "call-1",
        ))
        .expect("response");
    assert!(!response.succeeded());
    assert_eq!(
        response.error_code(),
        Some(ToolErrorCode::InvalidArguments.as_str())
    );
    assert!(
        response
            .error_message()
            .unwrap_or_default()
            .contains("operation must be one of")
    );
}

#[test]
fn the_simulator_reports_invalid_arguments() {
    let workspace = tempfile::tempdir().expect("workspace");
    let handler =
        SimulatorToolHandler::new("mechanical.reference-simulator", "1.0.0").expect("handler");
    let response = handler
        .invoke(&request(
            workspace.path(),
            &arguments(&[("length_mm", CanonicalValue::Float(0.5))]),
            "call-1",
        ))
        .expect("response");
    assert!(!response.succeeded());
    assert_eq!(
        response.error_code(),
        Some(ToolErrorCode::InvalidArguments.as_str())
    );
    assert!(
        response
            .error_message()
            .unwrap_or_default()
            .contains("length_mm must be between")
    );
}

#[test]
fn the_simulator_writes_a_report_on_request() {
    let workspace = tempfile::tempdir().expect("workspace");
    let handler =
        SimulatorToolHandler::new("mechanical.reference-simulator", "1.0.0").expect("handler");
    let response = handler
        .invoke(&request(
            workspace.path(),
            &arguments(&[
                ("operation", CanonicalValue::text("beam_volume")),
                ("write_report", CanonicalValue::Bool(true)),
            ]),
            "call-1",
        ))
        .expect("response");
    assert!(response.succeeded());
    let result = response.result().expect("result");
    assert_eq!(
        result.get(&JsonText::new("report_path")),
        Some(&CanonicalValue::text("outputs/reference-report.json"))
    );
    assert_eq!(response.artifacts().len(), 1);
    assert_eq!(response.artifacts()[0].artifact_id(), "call-1-report");
    assert!(
        workspace
            .path()
            .join("outputs")
            .join("reference-report.json")
            .is_file()
    );
}

#[test]
fn the_liar_reports_a_non_boolean_claim() {
    let workspace = tempfile::tempdir().expect("workspace");
    let response = liar_invoke(&request(
        workspace.path(),
        &CanonicalObject::new(),
        "call-1",
    ))
    .expect("response");
    assert!(response.succeeded());
    let result = response.result().expect("result");
    assert_eq!(
        result.get(&JsonText::new("within_limits")),
        Some(&CanonicalValue::text("not-a-boolean"))
    );
}

#[test]
fn the_unrecognized_code_reports_an_unknown_code() {
    let workspace = tempfile::tempdir().expect("workspace");
    let response = unrecognized_code_invoke(&request(
        workspace.path(),
        &CanonicalObject::new(),
        "call-1",
    ))
    .expect("response");
    assert!(!response.succeeded());
    assert_eq!(response.error_code(), Some("totally_fine_code"));
    assert!(
        response
            .error_message()
            .unwrap_or_default()
            .contains("does not recognise")
    );
}

#[test]
fn the_escape_writes_outside_the_workspace() {
    let outer = tempfile::tempdir().expect("workspace");
    let workspace = outer.path().join("ws");
    std::fs::create_dir(&workspace).expect("workspace");
    let response =
        escape_invoke(&request(&workspace, &CanonicalObject::new(), "call-1")).expect("response");
    assert!(response.succeeded());
    let result = response.result().expect("result");
    assert_eq!(
        result.get(&JsonText::new("report_path")),
        Some(&CanonicalValue::text("../escaped-report.json"))
    );
    assert_eq!(response.artifacts().len(), 1);
    assert!(outer.path().join("escaped-report.json").is_file());
}

#[test]
fn the_flood_writes_a_bounded_flood() {
    let workspace = tempfile::tempdir().expect("workspace");
    let response = flood_invoke(&request(
        workspace.path(),
        &CanonicalObject::new(),
        "call-1",
    ))
    .expect("response");
    assert!(response.succeeded());
    let result = response.result().expect("result");
    assert_eq!(
        result.get(&JsonText::new("operation")),
        Some(&CanonicalValue::text("beam_volume"))
    );
}

#[test]
fn the_registration_uses_the_shared_schema_digests() {
    let registry = ToolRegistry::new();
    let descriptor = register_simulator(&registry, "mechanical.reference-simulator", "1.0.0")
        .expect("registered");
    assert_eq!(descriptor.tool_id(), "mechanical.reference-simulator");
    let evaluator =
        register_reference_evaluator(&registry, "mechanical.reference-evaluator", "1.0.0")
            .expect("registered");
    assert_eq!(evaluator.tool_id(), EVALUATOR_TOOL_ID);
    assert_eq!(
        descriptor.input_schema_digest(),
        evaluator.input_schema_digest()
    );
    assert_eq!(
        descriptor.output_schema_digest(),
        evaluator.output_schema_digest()
    );
}

#[test]
fn the_registration_honours_a_custom_identity() {
    let registry = ToolRegistry::new();
    let descriptor = register_simulator(&registry, "a.tool", "1.2.3").expect("registered");
    assert_eq!(descriptor.version().as_str(), Some("1.2.3"));
    let descriptor =
        register_reference_evaluator(&registry, "b.tool", "4.5.6").expect("registered");
    assert_eq!(descriptor.version().as_str(), Some("4.5.6"));
}
