//! The live Python differential of the mechanical reference simulator.
//!
//! The corpus covers the parameter parsing, the closed form calculation, the
//! reference report, the published artifact, the two handlers, the
//! registration and the misbehaviour entry points, over a curated and a
//! randomised set of arguments, including hostile ones: an unknown
//! operation, a dimension that is not a number, a dimension that is not
//! finite, a dimension outside its bounds and a report flag that is not a
//! boolean.
//!
//! Every operation is compared with the repository's own
//! `kala_agent.tools.simulator`, byte for byte, including the class of the
//! exception the Python implementation raises.

mod support;

use std::path::PathBuf;

use kala_contracts::{ContractModel, ToolDescriptor};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_tools::protocol::{
    PROTOCOL_VERSION, ToolHandler, ToolRequestEnvelope, canonical_json_dumps,
};
use kala_tools::registry::ToolRegistry;
use kala_tools::simulator::{
    ReferenceEvaluatorHandler, SimulatorToolHandler, compute_metrics, evaluator_descriptor,
    input_schema, output_schema, parse_parameters, register_reference_evaluator,
    register_simulator, report_payload, simulator_descriptor,
};

use support::{Lcg, canonical, run_oracle};

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

/// One operation of the corpus.
fn op(name: &str, arguments: CanonicalValue) -> CanonicalValue {
    CanonicalValue::object([("op", CanonicalValue::text(name)), ("arguments", arguments)])
}

/// One operation of the corpus with a call identifier.
fn op_with_call(name: &str, arguments: CanonicalValue, call_id: &str) -> CanonicalValue {
    CanonicalValue::object([
        ("op", CanonicalValue::text(name)),
        ("arguments", arguments),
        ("call_id", CanonicalValue::text(call_id)),
    ])
}

/// A non finite argument, as a corpus marker.
fn nonfinite(name: &str, marker: &str) -> CanonicalValue {
    arguments(&[(
        name,
        CanonicalValue::object([("__nonfinite__", CanonicalValue::text(marker))]),
    )])
}

/// An argument object.
fn arguments(entries: &[(&str, CanonicalValue)]) -> CanonicalValue {
    let mut object = CanonicalObject::new();
    for (key, value) in entries {
        object.insert(JsonText::new(key), value.clone());
    }
    CanonicalValue::Object(object)
}

/// The curated corpus.
fn curated() -> Vec<CanonicalValue> {
    let mut cases: Vec<CanonicalValue> = Vec::new();

    // The parameter parsing. The non finite values travel through the
    // corpus as a marker, because a canonical JSON document cannot carry
    // them.
    for value in [
        arguments(&[]),
        arguments(&[("operation", CanonicalValue::text("beam_volume"))]),
        arguments(&[("operation", CanonicalValue::text("bar_summary"))]),
        arguments(&[("operation", CanonicalValue::text("shear_stress"))]),
        arguments(&[("operation", CanonicalValue::integer(7))]),
        arguments(&[("operation", CanonicalValue::Null)]),
        arguments(&[("length_mm", CanonicalValue::text("200"))]),
        arguments(&[("width_mm", CanonicalValue::Bool(true))]),
        nonfinite("height_mm", "inf"),
        nonfinite("density_kg_m3", "nan"),
        arguments(&[("length_mm", CanonicalValue::Float(5_000.1))]),
        arguments(&[("width_mm", CanonicalValue::Float(0.5))]),
        arguments(&[("height_mm", CanonicalValue::Float(500.5))]),
        arguments(&[("density_kg_m3", CanonicalValue::Float(20_000.5))]),
        arguments(&[("write_report", CanonicalValue::text("yes"))]),
        arguments(&[("write_report", CanonicalValue::Bool(true))]),
        arguments(&[("write_report", CanonicalValue::integer(1))]),
        arguments(&[("length_mm", CanonicalValue::integer(200))]),
        arguments(&[("length_mm", CanonicalValue::Float(10.0))]),
        arguments(&[("length_mm", CanonicalValue::Float(5_000.0))]),
        arguments(&[("width_mm", CanonicalValue::Float(500.0))]),
        arguments(&[("height_mm", CanonicalValue::Float(1.0))]),
        arguments(&[("density_kg_m3", CanonicalValue::Float(100.0))]),
        arguments(&[("density_kg_m3", CanonicalValue::Float(20_000.0))]),
        arguments(&[
            ("density_kg_m3", CanonicalValue::Float(2_700.0)),
            ("height_mm", CanonicalValue::Float(10.0)),
            ("length_mm", CanonicalValue::Float(120.0)),
            ("width_mm", CanonicalValue::Float(20.0)),
        ]),
        CanonicalValue::array([CanonicalValue::text("operation")]),
        CanonicalValue::text("operation"),
        CanonicalValue::Null,
        CanonicalValue::integer(7),
    ] {
        cases.push(op("parse", value.clone()));
        cases.push(op("metrics", value));
    }

    // The report payload and the published artifact.
    for value in [
        arguments(&[]),
        arguments(&[("operation", CanonicalValue::text("bar_summary"))]),
        arguments(&[("length_mm", CanonicalValue::Float(1_000.0))]),
        arguments(&[("operation", CanonicalValue::text("nope"))]),
    ] {
        cases.push(op_with_call("report", value.clone(), "call-7"));
        cases.push(op_with_call("write_report", value, "call-1"));
    }

    // The two handlers.
    for value in [
        arguments(&[]),
        arguments(&[("operation", CanonicalValue::text("bar_summary"))]),
        arguments(&[("operation", CanonicalValue::text("nope"))]),
        arguments(&[("length_mm", CanonicalValue::Float(0.5))]),
        arguments(&[
            ("operation", CanonicalValue::text("beam_volume")),
            ("write_report", CanonicalValue::Bool(true)),
        ]),
    ] {
        cases.push(op_with_call("simulate", value.clone(), "call-1"));
        cases.push(op_with_call("evaluate", value, "call-1"));
    }

    // The descriptors and the registration.
    cases.push(CanonicalValue::object([(
        "op",
        CanonicalValue::text("descriptor"),
    )]));
    cases.push(CanonicalValue::object([
        ("op", CanonicalValue::text("descriptor")),
        ("tool_id", CanonicalValue::text("custom.tool")),
        ("version", CanonicalValue::text("9.9.9")),
    ]));
    cases.push(CanonicalValue::object([(
        "op",
        CanonicalValue::text("evaluator_descriptor"),
    )]));
    cases.push(CanonicalValue::object([
        ("op", CanonicalValue::text("evaluator_descriptor")),
        ("tool_id", CanonicalValue::text("custom.evaluator")),
        ("version", CanonicalValue::text("2.0.0")),
    ]));
    cases.push(CanonicalValue::object([(
        "op",
        CanonicalValue::text("handler"),
    )]));
    cases.push(CanonicalValue::object([
        ("op", CanonicalValue::text("handler")),
        ("tool_id", CanonicalValue::text("custom.tool")),
        ("version", CanonicalValue::text("3.0.0")),
    ]));
    cases.push(CanonicalValue::object([(
        "op",
        CanonicalValue::text("register"),
    )]));

    // The misbehaviour entry points.
    cases.push(op_with_call("liar", arguments(&[]), "call-1"));
    cases.push(op_with_call("unrecognized_code", arguments(&[]), "call-1"));
    cases.push(op_with_call("escape", arguments(&[]), "call-1"));

    cases
}

/// The randomised corpus.
fn randomised(seed: u64, count: usize) -> Vec<CanonicalValue> {
    let mut generator = Lcg::new(seed);
    let operations = ["beam_volume", "bar_summary"];
    let mut cases: Vec<CanonicalValue> = Vec::new();
    for _ in 0..count {
        let mut entries: Vec<(&str, CanonicalValue)> = Vec::new();
        if generator.below(2) == 0 {
            entries.push((
                "operation",
                CanonicalValue::text(operations[generator.below(2) as usize]),
            ));
        }
        if generator.below(2) == 0 {
            entries.push((
                "length_mm",
                CanonicalValue::Float(10.0 + generator.below(4_990) as f64),
            ));
        }
        if generator.below(2) == 0 {
            entries.push((
                "width_mm",
                CanonicalValue::Float(1.0 + generator.below(499) as f64),
            ));
        }
        if generator.below(2) == 0 {
            entries.push((
                "height_mm",
                CanonicalValue::Float(1.0 + generator.below(499) as f64),
            ));
        }
        if generator.below(2) == 0 {
            entries.push((
                "density_kg_m3",
                CanonicalValue::Float(100.0 + generator.below(19_900) as f64),
            ));
        }
        if generator.below(3) == 0 {
            entries.push((
                "write_report",
                CanonicalValue::Bool(generator.below(2) == 0),
            ));
        }
        let value = arguments(&entries);
        cases.push(op("parse", value.clone()));
        cases.push(op("metrics", value.clone()));
        cases.push(op_with_call("report", value.clone(), "call-3"));
        cases.push(op_with_call("simulate", value.clone(), "call-3"));
        cases.push(op_with_call("evaluate", value, "call-3"));
    }
    cases
}

/// The corpus of the differential.
fn corpus() -> Vec<CanonicalValue> {
    let mut cases = curated();
    cases.extend(randomised(0x513c_2026_1019, 60));
    cases
}

// ---------------------------------------------------------------------------
// The native interpreter
// ---------------------------------------------------------------------------

/// The native result of one case.
fn native_case(case: &CanonicalValue, workspace: &str) -> CanonicalValue {
    let CanonicalValue::Object(case) = case else {
        panic!("case is an object");
    };
    let name = case
        .get(&JsonText::new("op"))
        .and_then(|value| value.as_text())
        .map(|text| text.to_string_lossy())
        .unwrap_or_default();
    let arguments = case
        .get(&JsonText::new("arguments"))
        .cloned()
        .unwrap_or(CanonicalValue::Object(CanonicalObject::new()));
    let call_id = case
        .get(&JsonText::new("call_id"))
        .and_then(|value| value.as_text())
        .map(|text| text.to_string_lossy())
        .unwrap_or_else(|| "call-1".to_owned());
    let text = |key: &str| {
        case.get(&JsonText::new(key))
            .and_then(|value| value.as_text())
            .map(|text| text.to_string_lossy())
    };
    match name.as_str() {
        "parse" => match parse_parameters(&decode_nonfinite(&arguments)) {
            Ok(parameters) => ok(parameters.to_payload()),
            Err(error) => failed(error.python_class_name(), &error.to_string()),
        },
        "metrics" => match parse_parameters(&decode_nonfinite(&arguments)) {
            Ok(parameters) => ok(compute_metrics(&parameters)),
            Err(error) => failed(error.python_class_name(), &error.to_string()),
        },
        "report" => match parse_parameters(&arguments) {
            Ok(parameters) => {
                let metrics = compute_metrics(&parameters);
                ok(report_payload(&parameters, &metrics, &call_id))
            }
            Err(error) => failed(error.python_class_name(), &error.to_string()),
        },
        "write_report" => match parse_parameters(&arguments) {
            Ok(parameters) => {
                let metrics = compute_metrics(&parameters);
                let root = PathBuf::from(workspace);
                match kala_tools::simulator::write_report(&root, &parameters, &metrics, &call_id) {
                    Ok((artifact, relative)) => ok(CanonicalValue::object([
                        (
                            "artifact",
                            artifact
                                .to_wire_value()
                                .to_canonical_value()
                                .expect("artifact"),
                        ),
                        ("relative", CanonicalValue::text(relative)),
                    ])),
                    Err(error) => failed(error.python_class_name(), &error.to_string()),
                }
            }
            Err(error) => failed(error.python_class_name(), &error.to_string()),
        },
        "simulate" => {
            let request = request(&arguments, &call_id, workspace);
            match request {
                Err(error) => failed(&error.0, &error.1),
                Ok(request) => match kala_tools::simulator::invoke(&request) {
                    Ok(envelope) => ok(envelope.to_payload().expect("payload")),
                    Err(error) => failed("ToolProtocolError", &error.to_string()),
                },
            }
        }
        "evaluate" => {
            let request = request(&arguments, &call_id, workspace);
            match request {
                Err(error) => failed(&error.0, &error.1),
                Ok(request) => {
                    let handler =
                        ReferenceEvaluatorHandler::new("mechanical.reference-evaluator", "1.0.0")
                            .expect("evaluator");
                    match kala_tools::protocol::ToolHandler::invoke(&handler, &request) {
                        Ok(envelope) => ok(envelope.to_payload().expect("payload")),
                        Err(error) => failed("ToolProtocolError", &error.to_string()),
                    }
                }
            }
        }
        "descriptor" => {
            let descriptor = simulator_descriptor(
                &text("tool_id").unwrap_or_else(|| "mechanical.reference-simulator".to_owned()),
                &text("version").unwrap_or_else(|| "1.0.0".to_owned()),
            );
            match descriptor {
                Ok(descriptor) => ok(descriptor_payload(&descriptor)),
                Err(error) => failed(error.python_class_name(), &error.to_string()),
            }
        }
        "evaluator_descriptor" => {
            let descriptor = evaluator_descriptor(
                &text("tool_id").unwrap_or_else(|| "mechanical.reference-evaluator".to_owned()),
                &text("version").unwrap_or_else(|| "1.0.0".to_owned()),
            );
            match descriptor {
                Ok(descriptor) => ok(descriptor_payload(&descriptor)),
                Err(error) => failed(error.python_class_name(), &error.to_string()),
            }
        }
        "handler" => {
            let handler = SimulatorToolHandler::new(
                &text("tool_id").unwrap_or_else(|| "mechanical.reference-simulator".to_owned()),
                &text("version").unwrap_or_else(|| "1.0.0".to_owned()),
            );
            match handler {
                Ok(handler) => ok(descriptor_payload(handler.descriptor())),
                Err(error) => failed(error.python_class_name(), &error.to_string()),
            }
        }
        "register" => {
            let registry = ToolRegistry::new();
            match (
                register_simulator(&registry, "mechanical.reference-simulator", "1.0.0"),
                register_reference_evaluator(&registry, "mechanical.reference-evaluator", "1.0.0"),
            ) {
                (Ok(simulator), Ok(evaluator)) => {
                    let identifiers = registry.tool_ids().unwrap_or_default();
                    ok(CanonicalValue::object([
                        ("evaluator", descriptor_payload(&evaluator)),
                        ("simulator", descriptor_payload(&simulator)),
                        (
                            "tool_ids",
                            CanonicalValue::array(
                                identifiers
                                    .iter()
                                    .map(CanonicalValue::text)
                                    .collect::<Vec<_>>(),
                            ),
                        ),
                    ]))
                }
                (Err(error), _) | (_, Err(error)) => {
                    failed(error.python_class_name(), &error.to_string())
                }
            }
        }
        "liar" => {
            let request = request(&arguments, &call_id, workspace);
            match request {
                Err(error) => failed(&error.0, &error.1),
                Ok(request) => match kala_tools::simulator::liar_invoke(&request) {
                    Ok(envelope) => ok(envelope.to_payload().expect("payload")),
                    Err(error) => failed("ToolProtocolError", &error.to_string()),
                },
            }
        }
        "unrecognized_code" => {
            let request = request(&arguments, &call_id, workspace);
            match request {
                Err(error) => failed(&error.0, &error.1),
                Ok(request) => match kala_tools::simulator::unrecognized_code_invoke(&request) {
                    Ok(envelope) => ok(envelope.to_payload().expect("payload")),
                    Err(error) => failed("ToolProtocolError", &error.to_string()),
                },
            }
        }
        "escape" => {
            let request = request(&arguments, &call_id, workspace);
            match request {
                Err(error) => failed(&error.0, &error.1),
                Ok(request) => match kala_tools::simulator::escape_invoke(&request) {
                    Ok(envelope) => ok(envelope.to_payload().expect("payload")),
                    Err(error) => failed("ToolProtocolError", &error.to_string()),
                },
            }
        }
        other => panic!("unknown operation: {other}"),
    }
}

/// Decode the non finite marker of the corpus.
///
/// A canonical JSON document cannot carry a non finite number, so the corpus
/// marks one and both sides decode it into the same double.
fn decode_nonfinite(value: &CanonicalValue) -> CanonicalValue {
    match value {
        CanonicalValue::Object(entries) => {
            if entries.len() == 1 {
                if let Some(CanonicalValue::Text(marker)) =
                    entries.get(&JsonText::new("__nonfinite__"))
                {
                    match marker.to_string_lossy().as_str() {
                        "nan" => return CanonicalValue::Float(f64::NAN),
                        "inf" => return CanonicalValue::Float(f64::INFINITY),
                        "-inf" => return CanonicalValue::Float(f64::NEG_INFINITY),
                        _ => {}
                    }
                }
            }
            CanonicalValue::Object(
                entries
                    .iter()
                    .map(|(key, item)| (key.clone(), decode_nonfinite(item)))
                    .collect(),
            )
        }
        CanonicalValue::Array(items) => {
            CanonicalValue::Array(items.iter().map(decode_nonfinite).collect())
        }
        other => other.clone(),
    }
}

/// The canonical payload of a descriptor.
fn descriptor_payload(descriptor: &ToolDescriptor) -> CanonicalValue {
    descriptor
        .to_wire_value()
        .to_canonical_value()
        .expect("descriptor")
}

/// The request envelope of a case.
fn request(
    arguments: &CanonicalValue,
    call_id: &str,
    workspace: &str,
) -> Result<ToolRequestEnvelope, (String, String)> {
    let CanonicalValue::Object(arguments) = arguments else {
        return Err((
            "ToolEnvelopeError".to_owned(),
            "arguments must be a JSON object".to_owned(),
        ));
    };
    ToolRequestEnvelope::new(
        PROTOCOL_VERSION,
        call_id,
        "mechanical.reference-simulator",
        "1.0.0",
        arguments,
        workspace,
        30.0,
        &num_bigint::BigInt::from(1_000_000),
        &num_bigint::BigInt::from(1_000_000),
        &[],
    )
    .map_err(|error| ("ToolEnvelopeError".to_owned(), error.to_string()))
}

/// A successful result.
fn ok(output: CanonicalValue) -> CanonicalValue {
    CanonicalValue::object([("ok", CanonicalValue::Bool(true)), ("output", output)])
}

/// A failed result.
fn failed(class: &str, message: &str) -> CanonicalValue {
    CanonicalValue::object([
        ("ok", CanonicalValue::Bool(false)),
        ("error", CanonicalValue::text(class)),
        ("message", CanonicalValue::text(message)),
    ])
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

#[test]
fn the_simulator_matches_live_python() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = workspace.path().join("workspace");
    std::fs::create_dir_all(&root).expect("workspace directory");
    let root = root.to_string_lossy().replace('\\', "/");

    let cases = corpus();
    let document = CanonicalValue::array(cases.clone());
    let oracle = run_oracle("simulator", &document, Some(PathBuf::from(&root).as_path()));

    assert_eq!(oracle.len(), cases.len(), "one oracle result per case");
    let mut comparisons = 0usize;
    let mut failures = 0usize;
    for (index, case) in cases.iter().enumerate() {
        let native = native_case(case, &root);
        comparisons += 1;
        if let CanonicalValue::Object(entries) = &native {
            if matches!(
                entries.get(&JsonText::new("ok")),
                Some(CanonicalValue::Bool(false))
            ) {
                failures += 1;
            }
        }
        let native_bytes = canonical(&native);
        let oracle_bytes = canonical(&oracle[index]);
        assert_eq!(
            native_bytes, oracle_bytes,
            "native and oracle differ for simulator case {index}\nnative: {native_bytes}\noracle: {oracle_bytes}"
        );
    }
    // The corpus exercises the refusal paths as well as the accepted ones.
    assert!(comparisons >= 380, "{comparisons} comparisons");
    assert!(failures >= 30, "{failures} refusals");
}

/// The corpus sizes are stable, so the report cannot drift away from the
/// corpus that runs.
#[test]
fn the_simulator_corpus_sizes_are_stable() {
    let cases = corpus();
    let curated_count = curated().len();
    assert_eq!(cases.len(), curated_count + 60 * 5);
    assert!(curated_count >= 80);
}

/// The canonical text of the schemas is stable.
#[test]
fn the_simulator_schemas_are_canonical() {
    let input = canonical_json_dumps(&input_schema(), kala_tools::protocol::MAX_JSON_BYTES)
        .expect("input schema");
    let output = canonical_json_dumps(&output_schema(), kala_tools::protocol::MAX_JSON_BYTES)
        .expect("output schema");
    assert!(input.starts_with("{\"additionalProperties\":false,"));
    assert!(output.contains("\"title\":\"mechanical.reference-simulator.output\""));
}
