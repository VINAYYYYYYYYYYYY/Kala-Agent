//! Workflow restore differential and native malformed-state rejection.

mod support;

use kala_contracts::ContractModel;
use kala_json::{CanonicalValue, parse_strict};
use kala_kernel::workflow::WorkflowState;
use support::{
    array, assert_exact_oracle, get, int, object, run_oracle, simple_definition, text,
    workflow_native,
};

const NOW: &str = "2026-01-02T03:04:05Z";

fn last_result(output: &CanonicalValue) -> CanonicalValue {
    match get(output, "results").expect("results") {
        CanonicalValue::Array(values) => values.last().expect("result").clone(),
        _ => panic!("results array"),
    }
}

fn object_map(
    value: &CanonicalValue,
) -> std::collections::BTreeMap<kala_json::JsonText, CanonicalValue> {
    match value {
        CanonicalValue::Object(object) => object.clone(),
        _ => panic!("object"),
    }
}

fn with_key(value: &CanonicalValue, key: &str, replacement: CanonicalValue) -> CanonicalValue {
    let mut object = object_map(value);
    object.insert(kala_json::JsonText::new(key), replacement);
    CanonicalValue::Object(object)
}

fn without_key(value: &CanonicalValue, key: &str) -> CanonicalValue {
    let mut object = object_map(value);
    object.remove(&kala_json::JsonText::new(key));
    CanonicalValue::Object(object)
}

fn python_state() -> (CanonicalValue, String, String) {
    let xml_definition = simple_definition(true);
    let checkpoint_source = object(vec![
        ("clock", text(NOW)),
        ("definition", xml_definition.clone()),
        (
            "ops",
            array(vec![
                object(vec![
                    ("op", text("start")),
                    ("workflow_id", text("workflow-1")),
                ]),
                object(vec![("op", text("run"))]),
                object(vec![("op", text("serialize"))]),
            ]),
        ),
    ]);
    let parsed = parse_strict(
        run_oracle("workflow", &checkpoint_source).as_bytes(),
        &kala_contracts::model_json_limits(),
    )
    .expect("oracle output");
    let checkpoint_serialized = get(&last_result(&parsed), "result")
        .and_then(|value| value.as_text())
        .expect("serialized")
        .to_string_lossy();
    let progressed_source = object(vec![
        ("clock", text(NOW)),
        ("definition", xml_definition.clone()),
        (
            "ops",
            array(vec![
                object(vec![
                    ("op", text("start")),
                    ("workflow_id", text("workflow-1")),
                ]),
                object(vec![("op", text("run"))]),
                object(vec![
                    ("actor_id", text("reviewer")),
                    ("now", text(NOW)),
                    ("op", text("approve_checkpoint")),
                ]),
                object(vec![("op", text("resume"))]),
                object(vec![("op", text("serialize"))]),
            ]),
        ),
    ]);
    let parsed = parse_strict(
        run_oracle("workflow", &progressed_source).as_bytes(),
        &kala_contracts::model_json_limits(),
    )
    .expect("oracle output");
    let progressed_serialized = get(&last_result(&parsed), "result")
        .and_then(|value| value.as_text())
        .expect("serialized")
        .to_string_lossy();
    (xml_definition, checkpoint_serialized, progressed_serialized)
}

#[allow(clippy::vec_init_then_push)]
#[test]
fn workflow_restore_rejection_matrix_matches_python() {
    let (definition, serialized, progressed_serialized) = python_state();
    let state = parse_strict(serialized.as_bytes(), &kala_contracts::model_json_limits())
        .expect("state value");
    let pending = get(&state, "pending_approval").expect("pending").clone();
    let progressed = parse_strict(
        progressed_serialized.as_bytes(),
        &kala_contracts::model_json_limits(),
    )
    .expect("progressed state value");
    let attempts = get(&progressed, "attempts").expect("attempts").clone();
    let history = get(&progressed, "history").expect("history").clone();

    let variant = |value: &CanonicalValue| text(&support::canonical(value));
    let mut variants: Vec<(&str, CanonicalValue)> = Vec::new();
    variants.push(("valid", text(&serialized)));
    variants.push((
        "unknown-key",
        variant(&with_key(&state, "unexpected", int(1))),
    ));
    variants.push((
        "definition-id",
        variant(&with_key(&state, "definition_id", text("other"))),
    ));
    variants.push((
        "definition-version",
        variant(&with_key(&state, "definition_version", text("9.9.9"))),
    ));
    variants.push((
        "current-state",
        variant(&with_key(&state, "current_state", text("nowhere"))),
    ));
    variants.push((
        "status",
        variant(&with_key(&state, "status", text("unknown"))),
    ));
    variants.push(("status-type", variant(&with_key(&state, "status", int(7)))));
    variants.push((
        "replanning-bool",
        variant(&with_key(
            &state,
            "replanning_attempts",
            CanonicalValue::Bool(true),
        )),
    ));
    variants.push((
        "replanning-string",
        variant(&with_key(&state, "replanning_attempts", text("1"))),
    ));
    variants.push((
        "replanning-negative",
        variant(&with_key(&state, "replanning_attempts", int(-1))),
    ));
    variants.push((
        "attempts-shape",
        variant(&with_key(&state, "attempts", text("none"))),
    ));
    variants.push((
        "history-shape",
        variant(&with_key(&state, "history", object(vec![]))),
    ));
    variants.push((
        "pending-shape",
        variant(&with_key(&state, "pending_approval", array(Vec::new()))),
    ));
    variants.push((
        "pending-status",
        variant(&with_key(
            &state,
            "pending_approval",
            with_key(&pending, "status", text("unknown")),
        )),
    ));
    variants.push((
        "pending-naive-time",
        variant(&with_key(
            &state,
            "pending_approval",
            with_key(&pending, "decided_at", text("2026-01-02T03:04:05")),
        )),
    ));
    variants.push((
        "pending-bad-time",
        variant(&with_key(
            &state,
            "pending_approval",
            with_key(&pending, "decided_at", text("not-a-time")),
        )),
    ));
    variants.push((
        "pending-actor",
        variant(&with_key(
            &state,
            "pending_approval",
            with_key(&pending, "actor_id", text("bad actor")),
        )),
    ));
    variants.push((
        "attempt-zero",
        variant(&with_key(
            &progressed,
            "attempts",
            array(vec![with_key(
                &attempts_value(&attempts),
                "attempt",
                int(0),
            )]),
        )),
    ));
    variants.push((
        "attempt-bool",
        variant(&with_key(
            &progressed,
            "attempts",
            array(vec![with_key(
                &attempts_value(&attempts),
                "attempt",
                CanonicalValue::Bool(true),
            )]),
        )),
    ));
    variants.push((
        "attempt-succeeded",
        variant(&with_key(
            &progressed,
            "attempts",
            array(vec![with_key(
                &attempts_value(&attempts),
                "succeeded",
                text("yes"),
            )]),
        )),
    ));
    variants.push((
        "history-sequence",
        variant(&with_key(
            &progressed,
            "history",
            array(vec![with_key(&history_value(&history), "sequence", int(0))]),
        )),
    ));
    variants.push((
        "missing-current",
        variant(&without_key(&state, "current_state")),
    ));
    variants.push((
        "missing-workflow",
        variant(&without_key(&state, "workflow_id")),
    ));
    variants.push((
        "workflow-unsafe",
        variant(&with_key(&state, "workflow_id", text("bad id"))),
    ));
    variants.push(("not-an-object", text("[]")));
    variants.push(("not-json", text("{")));

    let mut ops = Vec::new();
    for (_, variant) in &variants {
        ops.push(object(vec![
            ("op", text("restore")),
            ("state", variant.clone()),
        ]));
        ops.push(object(vec![("op", text("serialize"))]));
    }
    let document = object(vec![
        ("clock", text(NOW)),
        ("definition", definition),
        ("ops", array(ops)),
    ]);
    let oracle = run_oracle("workflow", &document);
    let native = workflow_native(&document);
    assert_exact_oracle(&native, &oracle);
}

fn attempts_value(value: &CanonicalValue) -> CanonicalValue {
    match value {
        CanonicalValue::Array(values) => values[0].clone(),
        _ => panic!("attempts"),
    }
}

fn history_value(value: &CanonicalValue) -> CanonicalValue {
    match value {
        CanonicalValue::Array(values) => values[0].clone(),
        _ => panic!("history"),
    }
}

#[test]
fn native_restore_rejects_forged_consistency_cases_python_accepts() {
    let (definition, serialized, _) = python_state();
    let parsed = parse_strict(serialized.as_bytes(), &kala_contracts::model_json_limits())
        .expect("state value");
    let state = WorkflowState::from_value(&parsed).expect("native state");
    let definition =
        kala_contracts::WorkflowDefinition::from_value(&definition).expect("definition");
    let engine = kala_kernel::workflow::WorkflowEngine::new(definition);

    // A valid exported state is accepted.
    engine.restore(state.clone()).expect("valid restore");

    // A history entry that does not describe a declared transition is forged.
    let mut forged_history = state.history().to_vec();
    forged_history.push(
        kala_kernel::workflow::WorkflowTransitionRecord::from_parts(
            num_bigint::BigInt::from(forged_history.len() + 1),
            kala_contracts::Text::new("transition-1"),
            kala_contracts::Text::new("start"),
            kala_contracts::Text::new("done"),
            kala_contracts::Text::new("step-1"),
            num_bigint::BigInt::from(1),
        )
        .expect("record"),
    );
    let forged = kala_kernel::workflow::WorkflowState::from_parts(
        state.workflow_id().clone(),
        state.definition_id().clone(),
        state.definition_version().clone(),
        state.status(),
        state.current_state().clone(),
        state.completed_step_ids().to_vec(),
        state.attempts().to_vec(),
        state.pending_approval().cloned(),
        state.replanning_attempts().clone(),
        forged_history,
        state.last_error().cloned(),
    )
    .expect("state");
    assert!(engine.restore(forged).is_err());

    // Replanning beyond the definition bound cannot be restored.
    let over_replanned = kala_kernel::workflow::WorkflowState::from_parts(
        state.workflow_id().clone(),
        state.definition_id().clone(),
        state.definition_version().clone(),
        state.status(),
        state.current_state().clone(),
        state.completed_step_ids().to_vec(),
        state.attempts().to_vec(),
        state.pending_approval().cloned(),
        num_bigint::BigInt::from(9),
        state.history().to_vec(),
        state.last_error().cloned(),
    )
    .expect("state");
    assert!(engine.restore(over_replanned).is_err());

    // Unknown completed step identifiers and unknown attempt steps are forged.
    let unknown_completed = kala_kernel::workflow::WorkflowState::from_parts(
        state.workflow_id().clone(),
        state.definition_id().clone(),
        state.definition_version().clone(),
        state.status(),
        state.current_state().clone(),
        vec![kala_contracts::Text::new("ghost-step")],
        state.attempts().to_vec(),
        state.pending_approval().cloned(),
        state.replanning_attempts().clone(),
        state.history().to_vec(),
        state.last_error().cloned(),
    )
    .expect("state");
    assert!(engine.restore(unknown_completed).is_err());

    // A successful attempt without its history/completed projections is forged.
    let mismatched = kala_kernel::workflow::WorkflowState::from_parts(
        state.workflow_id().clone(),
        state.definition_id().clone(),
        state.definition_version().clone(),
        kala_contracts::WorkflowStatus::Running,
        kala_contracts::Text::new("start"),
        Vec::new(),
        vec![
            kala_kernel::workflow::StepAttempt::from_parts(
                kala_contracts::Text::new("step-1"),
                num_bigint::BigInt::from(1),
                true,
                None,
            )
            .expect("attempt"),
        ],
        None,
        num_bigint::BigInt::from(0),
        Vec::new(),
        None,
    )
    .expect("state");
    assert!(engine.restore(mismatched).is_err());
}
