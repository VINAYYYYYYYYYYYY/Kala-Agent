//! Every public model has a minimal valid form and rejects extra fields.
//!
//! These tests do not need Python; the live differential test covers byte
//! parity separately.

use kala_contracts::{
    ApprovalRequest, ArtifactRef, ContractModel, DesignRevision, DesignSpec, DomainPackageManifest,
    EvaluationMetric, EvaluationResult, PlanStep, Provenance, Requirement, RunBudget, ToolCall,
    ToolDescriptor, ToolResult, WorkflowDefinition, WorkflowTransition,
};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const AT: &str = "2026-01-02T03:04:05Z";

fn minimal_cases() -> Vec<(&'static str, String)> {
    vec![
        (
            "Requirement",
            r#"{"id":"r","title":"t","kind":"functional"}"#.to_owned(),
        ),
        (
            "DesignSpec",
            r#"{"id":"s","name":"n","summary":"z"}"#.to_owned(),
        ),
        (
            "DomainPackageManifest",
            format!(r#"{{"package_id":"p","version":"1","digest":"{A}"}}"#),
        ),
        (
            "ArtifactRef",
            format!(
                r#"{{"artifact_id":"a","uri":"artifact://objects/a","digest":"{A}","media_type":"application/json","size_bytes":0}}"#
            ),
        ),
        (
            "DesignRevision",
            format!(
                r#"{{"revision_id":"r","design_id":"d","sequence":0,"spec":{{"id":"s","name":"n","summary":"z"}},"created_at":"{AT}"}}"#
            ),
        ),
        (
            "PlanStep",
            r#"{"step_id":"s","catalog_step_id":"c","intent":"i"}"#.to_owned(),
        ),
        (
            "WorkflowTransition",
            r#"{"transition_id":"t","from_state":"a","to_state":"b","step_id":"s"}"#.to_owned(),
        ),
        (
            "WorkflowDefinition",
            r#"{"definition_id":"w","version":"1","name":"n","initial_state":"a","states":["a"],"available_step_ids":["s"]}"#.to_owned(),
        ),
        (
            "ToolDescriptor",
            format!(
                r#"{{"tool_id":"t","version":"1","name":"n","description":"d","input_schema_digest":"{A}","output_schema_digest":"{B}","side_effect_level":"read"}}"#
            ),
        ),
        (
            "ToolCall",
            format!(
                r#"{{"call_id":"c","tool_id":"t","tool_version":"1","side_effect_level":"read","timeout_seconds":1,"issued_at":"{AT}"}}"#
            ),
        ),
        (
            "ToolResult",
            r#"{"call_id":"c","tool_id":"t","status":"succeeded"}"#.to_owned(),
        ),
        (
            "EvaluationMetric",
            r#"{"name":"n","value":1}"#.to_owned(),
        ),
        (
            "EvaluationResult",
            format!(
                r#"{{"evaluation_id":"e","subject_id":"s","passed":true,"evaluated_at":"{AT}"}}"#
            ),
        ),
        (
            "ApprovalRequest",
            format!(
                r#"{{"request_id":"a","checkpoint_type":"egress","status":"pending","workflow_id":"w","action_digest":"{A}","state_digest":"{B}","options":["approve"],"created_at":"{AT}"}}"#
            ),
        ),
        ("RunBudget", "{}".to_owned()),
        (
            "Provenance",
            format!(
                r#"{{"provenance_id":"p","subject_ids":["s"],"recorded_at":"{AT}"}}"#
            ),
        ),
    ]
}

fn validate(name: &str, source: &str) -> bool {
    match name {
        "Requirement" => Requirement::from_json_str(source).is_ok(),
        "DesignSpec" => DesignSpec::from_json_str(source).is_ok(),
        "DomainPackageManifest" => DomainPackageManifest::from_json_str(source).is_ok(),
        "ArtifactRef" => ArtifactRef::from_json_str(source).is_ok(),
        "DesignRevision" => DesignRevision::from_json_str(source).is_ok(),
        "PlanStep" => PlanStep::from_json_str(source).is_ok(),
        "WorkflowTransition" => WorkflowTransition::from_json_str(source).is_ok(),
        "WorkflowDefinition" => WorkflowDefinition::from_json_str(source).is_ok(),
        "ToolDescriptor" => ToolDescriptor::from_json_str(source).is_ok(),
        "ToolCall" => ToolCall::from_json_str(source).is_ok(),
        "ToolResult" => ToolResult::from_json_str(source).is_ok(),
        "EvaluationMetric" => EvaluationMetric::from_json_str(source).is_ok(),
        "EvaluationResult" => EvaluationResult::from_json_str(source).is_ok(),
        "ApprovalRequest" => ApprovalRequest::from_json_str(source).is_ok(),
        "RunBudget" => RunBudget::from_json_str(source).is_ok(),
        "Provenance" => Provenance::from_json_str(source).is_ok(),
        _ => panic!("unknown model {name}"),
    }
}

#[test]
fn all_models_have_a_valid_minimal_form_and_reject_extras() {
    for (name, source) in minimal_cases() {
        assert!(validate(name, &source), "{name} minimal form");
        let extra = source[..source.len() - 1].to_owned() + r#","unexpected":1}"#;
        assert!(!validate(name, &extra), "{name} extra field");
        if name != "RunBudget" {
            assert!(!validate(name, "{}"), "{name} missing required fields");
        }
    }
}

#[test]
fn wire_and_canonical_forms_are_distinct_but_consistent() {
    let spec = DesignSpec::from_json_str(
        r#"{"id":"s","name":"n","summary":"é","attributes_json":"{\"b\":2,\"a\":1}"}"#,
    )
    .expect("spec");
    assert!(spec.to_wire_json().unwrap().contains("é"));
    assert!(spec.to_wire_json_ascii().contains("\\u00e9"));
    assert_eq!(
        spec.to_canonical_json().unwrap(),
        r#"{"attributes_json":"{\"a\":1,\"b\":2}","constraints":[],"id":"s","name":"n","parameter_schema_digest":null,"requirement_ids":[],"summary":"\u00e9"}"#
    );
    assert_eq!(
        spec.canonical_digest().unwrap().to_hex(),
        "081b6469158d95a573d00e0cc522d89960ab9842ff3beade2eddcd2bdffe474e"
    );
}

#[test]
fn invalid_construction_is_rejected_with_the_python_message() {
    use kala_contracts::{ConstructionError, load_construction};
    let value = kala_json::parse_strict(
        br#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"bogus"}]}]}"#,
        &kala_contracts::model_json_limits(),
    )
    .unwrap();
    let error: ConstructionError = load_construction(&value).unwrap_err();
    assert_eq!(error.message(), "unknown operation bogus");
}
