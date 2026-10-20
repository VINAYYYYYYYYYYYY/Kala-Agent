//! Independent review probe 3: workflow restore/advance with forged history,
//! unknown/out-of-order steps, changed definition or native transition
//! identity, and stale approvals. Valid Python-exported states must restore
//! and re-serialize byte-identically.

mod support;

use kala_contracts::{ContractModel, Text, WorkflowDefinition, WorkflowStatus};
use kala_json::{CanonicalValue, JsonText, parse_strict};
use kala_kernel::workflow::{CheckpointContext, WorkflowEngine, WorkflowState};
use kala_kernel::{ApprovalStatus, KernelTimestamp};
use support::{array, get, object, run_oracle, simple_definition, text, three_state_definition};

const NOW: &str = "2026-01-02T03:04:05Z";

fn object_map(value: &CanonicalValue) -> std::collections::BTreeMap<JsonText, CanonicalValue> {
    match value {
        CanonicalValue::Object(object) => object.clone(),
        _ => panic!("object"),
    }
}

fn with_key(value: &CanonicalValue, key: &str, replacement: CanonicalValue) -> CanonicalValue {
    let mut object = object_map(value);
    object.insert(JsonText::new(key), replacement);
    CanonicalValue::Object(object)
}

fn push_item(value: &CanonicalValue, extra: CanonicalValue) -> CanonicalValue {
    match value {
        CanonicalValue::Array(values) => {
            let mut values = values.clone();
            values.push(extra);
            CanonicalValue::Array(values)
        }
        _ => panic!("array"),
    }
}

fn item(value: &CanonicalValue, index: usize) -> CanonicalValue {
    match value {
        CanonicalValue::Array(values) => values[index].clone(),
        _ => panic!("array"),
    }
}

fn definition(value: &CanonicalValue) -> WorkflowDefinition {
    WorkflowDefinition::from_value(value).expect("definition")
}

fn last_result_value(output: &CanonicalValue) -> CanonicalValue {
    match get(output, "results").expect("results") {
        CanonicalValue::Array(values) => values.last().expect("last").clone(),
        _ => panic!("results"),
    }
}

/// Run one scenario through the live Python reference and return its final
/// serialized state (the state itself, plus its exact JSON bytes).
fn python_state(
    definition_value: &CanonicalValue,
    ops: Vec<CanonicalValue>,
) -> (CanonicalValue, String) {
    let document = object(vec![
        ("clock", text(NOW)),
        ("definition", definition_value.clone()),
        ("ops", array(ops)),
    ]);
    let parsed = parse_strict(
        run_oracle("workflow", &document).as_bytes(),
        &kala_contracts::model_json_limits(),
    )
    .expect("oracle");
    let last = last_result_value(&parsed);
    let state = get(&last, "state").expect("state").clone();
    let serialized = get(&last, "state_json")
        .and_then(|value| value.as_text())
        .expect("state json")
        .to_string_lossy();
    (state, serialized)
}

fn start_ops() -> Vec<CanonicalValue> {
    vec![object(vec![
        ("op", text("start")),
        ("workflow_id", text("workflow-1")),
    ])]
}

fn progressed_state() -> (CanonicalValue, CanonicalValue, String) {
    let definition_value = three_state_definition();
    let mut ops = start_ops();
    ops.push(object(vec![("op", text("run"))]));
    let (state, serialized) = python_state(&definition_value, ops);
    (definition_value, state, serialized)
}

fn waiting_state() -> (CanonicalValue, CanonicalValue, String) {
    let definition_value = simple_definition(true);
    let mut ops = start_ops();
    ops.push(object(vec![("op", text("run"))]));
    let (state, serialized) = python_state(&definition_value, ops);
    (definition_value, state, serialized)
}

fn checkpoint_context(actor: &str) -> CheckpointContext {
    CheckpointContext::new()
        .with_actor_id(Text::new(actor))
        .with_now(KernelTimestamp::parse_isoformat(NOW).expect("time"))
}

#[test]
fn python_lifecycle_states_restore_and_reserialize_byte_identically() {
    type Scenario = (CanonicalValue, Vec<Vec<(&'static str, CanonicalValue)>>);
    let scenarios: Vec<Scenario> = vec![
        (
            three_state_definition(),
            vec![
                vec![("op", text("start")), ("workflow_id", text("workflow-1"))],
                vec![("op", text("run"))],
                vec![("op", text("request_replan"))],
                vec![("op", text("run"))],
            ],
        ),
        (
            simple_definition(true),
            vec![
                vec![("op", text("start")), ("workflow_id", text("workflow-1"))],
                vec![("op", text("run"))],
                vec![
                    ("op", text("approve_checkpoint")),
                    ("actor_id", text("reviewer")),
                    ("now", text(NOW)),
                ],
                vec![("op", text("resume"))],
            ],
        ),
        (
            simple_definition(true),
            vec![
                vec![("op", text("start")), ("workflow_id", text("workflow-1"))],
                vec![("op", text("run"))],
                vec![("op", text("cancel")), ("reason", text("stop"))],
            ],
        ),
        (
            simple_definition(true),
            vec![
                vec![("op", text("start")), ("workflow_id", text("workflow-1"))],
                vec![("op", text("run"))],
                vec![
                    ("op", text("reject_checkpoint")),
                    ("actor_id", text("reviewer")),
                    ("now", text(NOW)),
                    ("reason", text("no")),
                ],
            ],
        ),
        (
            simple_definition(true),
            vec![
                vec![("op", text("start")), ("workflow_id", text("workflow-1"))],
                vec![("op", text("run"))],
                vec![
                    ("op", text("abort_checkpoint")),
                    ("actor_id", text("reviewer")),
                    ("now", text(NOW)),
                    ("reason", text("no")),
                ],
            ],
        ),
    ];

    let mut documents = Vec::new();
    for (definition_value, steps) in scenarios {
        let definition = definition(&definition_value);
        let mut cumulative = Vec::new();
        let mut serialized_states: Vec<String> = Vec::new();
        for step in steps {
            cumulative.push(object(step));
            let (state_value, serialized) = python_state(&definition_value, cumulative.clone());
            let engine = WorkflowEngine::new(definition.clone());
            let restored = engine
                .restore_value(&state_value)
                .unwrap_or_else(|error| panic!("restore failed: {}", error.message()));
            assert_eq!(
                restored.to_json().expect("json"),
                serialized,
                "native serialization must match Python bytes"
            );
            assert_eq!(
                engine.serialize(None).expect("serialize"),
                restored.to_json().expect("json")
            );
            serialized_states.push(serialized);
        }
        let mut ops = Vec::new();
        for serialized in &serialized_states {
            ops.push(object(vec![
                ("op", text("restore")),
                ("state", text(serialized)),
            ]));
            ops.push(object(vec![("op", text("serialize"))]));
            ops.push(object(vec![("op", text("state_digest"))]));
        }
        documents.push(object(vec![
            ("clock", text(NOW)),
            ("definition", definition_value),
            ("ops", array(ops)),
        ]));
    }
    for document in documents {
        let oracle = run_oracle("workflow", &document);
        let native = support::workflow_native(&document);
        support::assert_exact_oracle(&native, &oracle);
    }
}

#[test]
fn forged_history_and_unknown_steps_are_rejected_transactionally() {
    let (definition_value, state_value, serialized) = progressed_state();
    let engine = WorkflowEngine::new(definition(&definition_value));
    let baseline = engine.restore_value(&state_value).expect("valid restore");
    let baseline_json = engine.serialize(None).expect("serialize");
    assert_eq!(baseline_json, serialized);

    let attempts = get(&state_value, "attempts").expect("attempts").clone();
    let history = get(&state_value, "history").expect("history").clone();
    let record = item(&history, 0);
    let attempt = item(&attempts, 0);

    let cases: Vec<(&str, CanonicalValue)> = vec![
        (
            "created-with-history",
            with_key(&state_value, "status", text("created")),
        ),
        (
            "unknown-status",
            with_key(&state_value, "status", text("bogus")),
        ),
        (
            "unknown-current-state",
            with_key(&state_value, "current_state", text("nowhere")),
        ),
        (
            "completion-without-history",
            with_key(&state_value, "current_state", text("done")),
        ),
        (
            "history-sequence-gap",
            with_key(
                &state_value,
                "history",
                array(vec![with_key(&record, "sequence", support::int(9))]),
            ),
        ),
        (
            "history-unknown-transition",
            with_key(
                &state_value,
                "history",
                array(vec![with_key(
                    &record,
                    "transition_id",
                    text("ghost-transition"),
                )]),
            ),
        ),
        (
            "history-replayed-later-transition",
            with_key(
                &state_value,
                "history",
                array(vec![with_key(
                    &record,
                    "transition_id",
                    text("transition-2"),
                )]),
            ),
        ),
        (
            "history-to-state-mismatch",
            with_key(
                &state_value,
                "history",
                array(vec![with_key(&record, "to_state", text("done"))]),
            ),
        ),
        (
            "history-from-state-mismatch",
            with_key(
                &state_value,
                "history",
                array(vec![with_key(&record, "from_state", text("middle"))]),
            ),
        ),
        (
            "history-step-mismatch",
            with_key(
                &state_value,
                "history",
                array(vec![with_key(&record, "step_id", text("step-2"))]),
            ),
        ),
        (
            "attempt-unknown-step",
            with_key(
                &state_value,
                "attempts",
                push_item(&attempts, support::attempt_value("ghost-step", 1, true)),
            ),
        ),
        (
            "attempt-extra-success-without-history",
            with_key(
                &state_value,
                "attempts",
                push_item(&attempts, support::attempt_value("step-2", 1, true)),
            ),
        ),
        (
            "attempt-sequence-mismatch",
            with_key(
                &state_value,
                "attempts",
                array(vec![with_key(&attempt, "attempt", support::int(7))]),
            ),
        ),
        (
            "completed-without-attempt",
            with_key(
                &state_value,
                "completed_step_ids",
                array(vec![text("step-2")]),
            ),
        ),
        (
            "completed-empty-with-success",
            with_key(&state_value, "completed_step_ids", array(vec![])),
        ),
        (
            "over-replanned",
            with_key(&state_value, "replanning_attempts", support::int(9)),
        ),
        (
            "changed-definition-id",
            with_key(&state_value, "definition_id", text("other-definition")),
        ),
        (
            "changed-definition-version",
            with_key(&state_value, "definition_version", text("9.9.9")),
        ),
        (
            "forged-pending-shape",
            with_key(&state_value, "pending_approval", array(vec![])),
        ),
        (
            "unknown-key-is-ignored",
            with_key(&state_value, "ignored", text("x")),
        ),
    ];

    for (label, case) in cases {
        let parsed = match WorkflowState::from_value(&case) {
            Ok(parsed) => parsed,
            Err(_) => {
                // The dynamic boundary itself refused a malformed record.
                assert!(engine.restore_value(&case).is_err(), "{label} was accepted");
                assert_eq!(engine.state().expect("state"), baseline);
                continue;
            }
        };
        let outcome = engine.restore(parsed);
        if label == "unknown-key-is-ignored" {
            // Python's loader ignores unknown keys; native does the same.
            assert!(outcome.is_ok(), "{label} should be accepted");
            engine.restore(baseline.clone()).expect("reset");
            continue;
        }
        assert!(outcome.is_err(), "{label} was accepted by restore");
        assert_eq!(
            engine.state().expect("state"),
            baseline,
            "{label} mutated the engine despite a failed restore"
        );
        assert_eq!(
            engine.serialize(None).expect("serialize"),
            baseline_json,
            "{label} changed the serialized state"
        );
    }
}

#[test]
fn changed_native_transition_identity_never_replays_an_approval() {
    let (definition_a_value, waiting, _) = waiting_state();

    // Definition B keeps the same id/version but changes transition-1's target.
    let transitions = get(&definition_a_value, "transitions")
        .expect("transitions")
        .clone();
    let transition = item(&transitions, 0);
    let definition_b_value = with_key(
        &with_key(
            &with_key(
                &definition_a_value,
                "states",
                array(vec![text("start"), text("done2")]),
            ),
            "transitions",
            array(vec![with_key(&transition, "to_state", text("done2"))]),
        ),
        "version",
        text("1.0.0"),
    );
    let definition_b = definition(&definition_b_value);
    assert_eq!(definition_b.definition_id(), "workflow-def");
    assert_eq!(definition_b.version().to_string_lossy(), "1.0.0");

    let engine = WorkflowEngine::new(definition_b);
    let restored = engine.restore_value(&waiting).expect("identity is equal");
    assert_eq!(restored.current_state().to_string_lossy(), "start");

    // Approving a stale restored approval is allowed (it is self-consistent),
    // but resuming must invalidate it instead of applying the changed
    // transition.
    let approved = engine
        .approve_checkpoint(None, None, &checkpoint_context("reviewer"))
        .expect("approve stale record");
    assert_eq!(
        approved.pending_approval().expect("pending").status(),
        ApprovalStatus::Approved
    );
    let resumed = engine.resume(None).expect("resume");
    assert_eq!(resumed.status(), WorkflowStatus::WaitingForApproval);
    assert_eq!(resumed.current_state().to_string_lossy(), "start");
    assert!(resumed.history().is_empty());
    assert!(resumed.completed_step_ids().is_empty());
    assert_eq!(
        resumed.pending_approval().expect("pending").status(),
        ApprovalStatus::Invalidated
    );
    assert_eq!(
        resumed.last_error().map(Text::to_string_lossy),
        Some("state_or_action_digest_changed".to_owned())
    );

    // Step results and repeated advances cannot force the old approval.
    let stuck = engine.state().expect("state");
    assert!(
        engine
            .record_step_success(&stuck, &Text::new("step-1"), None)
            .is_err()
    );
    let advanced = engine.run(None, None, None, None).expect("run");
    assert_eq!(advanced.status(), WorkflowStatus::WaitingForApproval);
    assert!(advanced.history().is_empty());
    assert!(
        engine
            .approve_checkpoint(None, None, &checkpoint_context("reviewer"))
            .is_err()
    );
    assert!(!engine.state().expect("state").is_terminal());

    // A fresh approval for the current definition still works: this is a new
    // approval, not a replay. Clear the invalidated pending record explicitly
    // (as a caller repairing the workflow would) and create a new checkpoint.
    let stuck = engine.state().expect("state");
    let clean = WorkflowState::from_parts(
        stuck.workflow_id().clone(),
        stuck.definition_id().clone(),
        stuck.definition_version().clone(),
        WorkflowStatus::Running,
        stuck.current_state().clone(),
        stuck.completed_step_ids().to_vec(),
        stuck.attempts().to_vec(),
        None,
        stuck.replanning_attempts().clone(),
        stuck.history().to_vec(),
        stuck.last_error().cloned(),
    )
    .expect("clean state");
    engine.restore(clean).expect("restore clean");
    let waiting_b = engine.run(None, None, None, None).expect("new checkpoint");
    assert_eq!(waiting_b.status(), WorkflowStatus::WaitingForApproval);
    assert_eq!(
        waiting_b.pending_approval().expect("pending").status(),
        ApprovalStatus::Pending
    );
    let approved_b = engine
        .approve_checkpoint(None, None, &checkpoint_context("reviewer"))
        .expect("approve new");
    assert_eq!(
        approved_b.pending_approval().expect("pending").status(),
        ApprovalStatus::Approved
    );
    let resumed_b = engine.resume(None).expect("resume new");
    assert_eq!(resumed_b.status(), WorkflowStatus::Completed);
    assert_eq!(resumed_b.current_state().to_string_lossy(), "done2");
    assert_eq!(resumed_b.history().len(), 1);

    // Changed checkpoint type, step, missing transition and identity changes
    // are rejected outright.
    let reject_cases = vec![
        with_key(&definition_a_value, "definition_id", text("other")),
        with_key(&definition_a_value, "version", text("9.9.9")),
        with_key(
            &definition_a_value,
            "transitions",
            array(vec![with_key(
                &transition,
                "checkpoint_type",
                CanonicalValue::Null,
            )]),
        ),
        with_key(
            &with_key(
                &definition_a_value,
                "transitions",
                array(vec![with_key(&transition, "step_id", text("step-2"))]),
            ),
            "available_step_ids",
            array(vec![text("step-1"), text("step-2")]),
        ),
        with_key(
            &definition_a_value,
            "transitions",
            array(vec![with_key(
                &transition,
                "transition_id",
                text("transition-x"),
            )]),
        ),
    ];
    for (index, case) in reject_cases.into_iter().enumerate() {
        let engine = WorkflowEngine::new(definition(&case));
        assert!(
            engine.restore_value(&waiting).is_err(),
            "definition mutation {index} was accepted"
        );
    }
}

#[test]
fn stale_restored_approval_is_invalidated_not_replayed() {
    let (definition_value, waiting, _) = waiting_state();
    let definition = definition(&definition_value);
    let pending = get(&waiting, "pending_approval").expect("pending").clone();
    let with_actor = with_key(
        &with_key(&pending, "actor_id", text("reviewer")),
        "decided_at",
        text(NOW),
    );

    let cases = vec![
        with_key(
            &waiting,
            "pending_approval",
            with_key(&pending, "state_digest", text("0")),
        ),
        with_key(
            &waiting,
            "pending_approval",
            with_key(&pending, "action_digest", text("0")),
        ),
        with_key(
            &waiting,
            "pending_approval",
            with_key(
                &with_key(&with_actor, "status", text("approved")),
                "state_digest",
                text("0"),
            ),
        ),
        with_key(
            &waiting,
            "pending_approval",
            with_key(&with_actor, "status", text("expired")),
        ),
    ];

    for (index, case) in cases.iter().enumerate() {
        let engine = WorkflowEngine::new(definition.clone());
        let restored = engine.restore_value(case).expect("restore stale approval");
        assert_eq!(
            restored.status(),
            WorkflowStatus::WaitingForApproval,
            "case {index} restore"
        );
        let resumed = engine.resume(None).expect("resume");
        assert_eq!(
            resumed.status(),
            WorkflowStatus::WaitingForApproval,
            "case {index} resume"
        );
        assert!(resumed.history().is_empty());
        let status = resumed.pending_approval().expect("pending").status();
        match index {
            // An unapproved stale record simply blocks until a new approval.
            0 | 1 => assert_eq!(status, ApprovalStatus::Pending),
            // An approved record whose digests no longer match is invalidated.
            2 => assert_eq!(status, ApprovalStatus::Invalidated),
            // An imported expired record stays expired and never runs.
            _ => assert_eq!(status, ApprovalStatus::Expired),
        }
    }

    // The exact-oracle form of the same mutations must match Python bytes.
    let mut ops = Vec::new();
    for case in &cases {
        ops.push(object(vec![
            ("op", text("restore")),
            ("state", text(&support::canonical(case))),
        ]));
        ops.push(object(vec![("op", text("resume"))]));
        ops.push(object(vec![("op", text("serialize"))]));
    }
    let document = object(vec![
        ("clock", text(NOW)),
        ("definition", definition_value),
        ("ops", array(ops)),
    ]);
    let oracle = run_oracle("workflow", &document);
    let native = support::workflow_native(&document);
    support::assert_exact_oracle(&native, &oracle);
}
