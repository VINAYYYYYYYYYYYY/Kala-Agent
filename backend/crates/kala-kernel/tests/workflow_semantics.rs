//! Native workflow branch, checkpoint, restore and event-sink tests.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Weak};
use std::thread;

use kala_contracts::{
    ApprovalKind, ApprovalStatus, ContractModel, Text, WorkflowDefinition, WorkflowStatus,
    WorkflowTransition,
};
use kala_kernel::KernelTimestamp;
use kala_kernel::workflow::{
    CheckpointContext, CollectingSink, StepAttempt, WorkflowApproval, WorkflowApprovalError,
    WorkflowEngine, WorkflowError, WorkflowEvent, WorkflowEventSink, WorkflowLimitExceeded,
    WorkflowState, WorkflowTransitionError,
};
use num_bigint::BigInt;

const NOW: &str = "2026-01-02T03:04:05Z";
const LATER: &str = "2026-01-02T03:05:05Z";

fn at(value: &str) -> KernelTimestamp {
    KernelTimestamp::parse_isoformat(value).expect("timestamp")
}

fn definition_json(checkpoint: bool) -> String {
    let checkpoint_type = if checkpoint {
        r#""pre_commit""#
    } else {
        "null"
    };
    format!(
        r#"{{"definition_id":"workflow-def","version":"1.0.0","name":"Single step","initial_state":"start","states":["start","done"],"transitions":[{{"transition_id":"transition-1","from_state":"start","to_state":"done","step_id":"step-1","checkpoint_type":{checkpoint_type}}}],"available_step_ids":["step-1"],"max_replanning_attempts":1}}"#
    )
}

fn definition(checkpoint: bool) -> WorkflowDefinition {
    WorkflowDefinition::from_json_str(&definition_json(checkpoint)).expect("definition")
}

fn branch_definition() -> WorkflowDefinition {
    WorkflowDefinition::from_json_str(
        r#"{"definition_id":"workflow-def","version":"1.0.0","name":"Branchable","initial_state":"start","states":["start","middle","done"],"transitions":[{"transition_id":"transition-1","from_state":"start","to_state":"middle","step_id":"step-1","checkpoint_type":null},{"transition_id":"transition-2","from_state":"middle","to_state":"done","step_id":"step-2","checkpoint_type":null}],"available_step_ids":["step-1","step-2"],"max_replanning_attempts":2}"#,
    )
    .expect("definition")
}

fn checkpoint_branch_definition() -> WorkflowDefinition {
    WorkflowDefinition::from_json_str(
        r#"{"definition_id":"workflow-def","version":"1.0.0","name":"Branchable checkpoint","initial_state":"start","states":["start","middle","done"],"transitions":[{"transition_id":"transition-1","from_state":"start","to_state":"middle","step_id":"step-1","checkpoint_type":"pre_commit"},{"transition_id":"transition-2","from_state":"middle","to_state":"done","step_id":"step-2","checkpoint_type":null}],"available_step_ids":["step-1","step-2"],"max_replanning_attempts":2}"#,
    )
    .expect("definition")
}

fn started(checkpoint: bool) -> (WorkflowEngine, WorkflowState) {
    let engine = WorkflowEngine::new(definition(checkpoint)).with_clock(Some(at(NOW)));
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    (engine, state)
}

fn waiting(checkpoint: bool) -> (WorkflowEngine, WorkflowState) {
    let (engine, state) = started(checkpoint);
    let state = engine.run(Some(&state), None, None, None).expect("run");
    (engine, state)
}

fn approval(
    approval_id: &str,
    transition_id: &str,
    step_id: &str,
    checkpoint_type: ApprovalKind,
) -> WorkflowApproval {
    WorkflowApproval::from_parts(
        Text::new(approval_id),
        Text::new(transition_id),
        Text::new(step_id),
        checkpoint_type,
        Text::new(&"a".repeat(64)),
        Text::new(&"b".repeat(64)),
        ApprovalStatus::Pending,
        None,
        None,
        None,
    )
    .expect("approval")
}

fn state_with_pending(pending: WorkflowApproval) -> WorkflowState {
    WorkflowState::from_parts(
        Text::new("workflow-1"),
        Text::new("workflow-def"),
        Text::new("1.0.0"),
        WorkflowStatus::WaitingForApproval,
        Text::new("start"),
        Vec::new(),
        Vec::new(),
        Some(pending),
        BigInt::from(0),
        Vec::new(),
        None,
    )
    .expect("state")
}

fn approve_context(now: &str) -> CheckpointContext {
    CheckpointContext::new().with_now(at(now))
}

#[test]
fn start_is_idempotent_and_restarts_for_another_id() {
    let (engine, first) = started(false);
    let again = engine.start(&Text::new("workflow-1"), None).expect("start");
    assert_eq!(again, first);
    let second = engine.start(&Text::new("workflow-2"), None).expect("start");
    assert_eq!(second.workflow_id().to_string_lossy(), "workflow-2");
    assert_ne!(second, first);
}

#[test]
fn start_validates_replanning_attempts() {
    let engine = WorkflowEngine::new(definition(false));
    assert!(matches!(
        engine.start(&Text::new("workflow-1"), Some(&BigInt::from(-1))),
        Err(WorkflowError::LimitExceeded(_))
    ));
    assert!(matches!(
        engine.start(&Text::new("workflow-1"), Some(&BigInt::from(9))),
        Err(WorkflowError::LimitExceeded(_))
    ));
    assert!(
        engine
            .start(&Text::new("workflow-1"), Some(&BigInt::from(1)))
            .is_ok()
    );
}

#[test]
fn start_emits_created_before_any_other_event() {
    let sink = Arc::new(CollectingSink::new());
    let engine = WorkflowEngine::new(definition(false)).with_sink(sink.clone());
    engine.start(&Text::new("workflow-1"), None).expect("start");
    let events = sink.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type.to_string_lossy(), "workflow.created");
    assert!(events[0].from_status.is_none());
    assert_eq!(events[0].to_status, WorkflowStatus::Created);
}

#[test]
fn run_requires_succeeded_with_step_id() {
    let (engine, state) = started(false);
    let error = engine
        .run(Some(&state), None, Some(&Text::new("step-1")), None)
        .expect_err("missing succeeded");
    assert!(
        matches!(error, WorkflowError::Transition(message) if message.contains("succeeded must be supplied"))
    );
}

#[test]
fn run_infers_the_selected_transition_step() {
    let engine = WorkflowEngine::new(branch_definition());
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    let advanced = engine
        .run(Some(&state), None, None, Some(true))
        .expect("run");
    assert_eq!(advanced.current_state().to_string_lossy(), "middle");
    assert_eq!(advanced.completed_step_ids().len(), 1);
}

#[test]
fn run_completes_when_no_transition_remains() {
    let (engine, state) = started(false);
    let completed = engine.run(Some(&state), None, None, None).expect("run");
    assert_eq!(completed.status(), WorkflowStatus::Completed);
}

#[test]
fn terminal_states_are_never_advanced() {
    let (engine, state) = started(false);
    let cancelled = engine
        .cancel(Some(&state), Some(&Text::new("stop")))
        .expect("cancel");
    assert_eq!(
        engine.run(Some(&cancelled), None, None, None).expect("run"),
        cancelled
    );
}

#[test]
fn run_rejects_undeclared_transitions() {
    let (engine, state) = started(false);
    let error = engine
        .run(Some(&state), Some(&Text::new("transition-9")), None, None)
        .expect_err("undeclared");
    assert!(
        matches!(error, WorkflowError::Transition(message) if message.contains("not declared from the current state"))
    );
}

#[test]
fn run_rejects_states_from_another_definition() {
    let (engine, _) = started(false);
    let other = WorkflowState::from_parts(
        Text::new("workflow-1"),
        Text::new("other"),
        Text::new("1.0.0"),
        WorkflowStatus::Running,
        Text::new("start"),
        Vec::new(),
        Vec::new(),
        None,
        BigInt::from(0),
        Vec::new(),
        None,
    )
    .expect("state");
    assert!(matches!(
        engine.run(Some(&other), None, None, None),
        Err(WorkflowError::Transition(message)) if message.contains("definition_id does not match")
    ));
    let wrong_version = WorkflowState::from_parts(
        Text::new("workflow-1"),
        Text::new("workflow-def"),
        Text::new("9.9.9"),
        WorkflowStatus::Running,
        Text::new("start"),
        Vec::new(),
        Vec::new(),
        None,
        BigInt::from(0),
        Vec::new(),
        None,
    )
    .expect("state");
    assert!(engine.run(Some(&wrong_version), None, None, None).is_err());
    let wrong_state = WorkflowState::from_parts(
        Text::new("workflow-1"),
        Text::new("workflow-def"),
        Text::new("1.0.0"),
        WorkflowStatus::Running,
        Text::new("nowhere"),
        Vec::new(),
        Vec::new(),
        None,
        BigInt::from(0),
        Vec::new(),
        None,
    )
    .expect("state");
    assert!(engine.run(Some(&wrong_state), None, None, None).is_err());
}

#[test]
fn missing_state_is_reported() {
    let engine = WorkflowEngine::new(definition(false));
    assert!(engine.state().is_none());
    assert!(engine.current_state().is_none());
    assert!(engine.run(None, None, None, None).is_err());
    assert!(engine.cancel(None, None).is_err());
    assert!(engine.serialize(None).is_err());
    assert!(engine.state_digest(None).is_err());
    assert!(engine.snapshot(None).is_err());
}

#[test]
fn checkpoint_stops_start_and_resume_completes() {
    let (engine, waiting_state) = waiting(true);
    assert_eq!(waiting_state.status(), WorkflowStatus::WaitingForApproval);
    assert!(waiting_state.pending_approval().is_some());
    let approved = engine
        .approve_checkpoint(Some(&waiting_state), None, &approve_context(NOW))
        .expect("approve");
    let resumed = engine.resume(Some(&approved)).expect("resume");
    assert_eq!(resumed.status(), WorkflowStatus::Completed);
    assert!(resumed.pending_approval().is_none());
    assert_eq!(resumed.history().len(), 1);
}

#[test]
fn step_results_cannot_skip_or_run_before_approval() {
    let (engine, state) = started(true);
    let error = engine
        .record_step_result(&state, &Text::new("step-1"), true, None, None)
        .expect_err("checkpoint");
    assert!(
        matches!(error, WorkflowError::Approval(message) if message.contains("before checkpoint approval"))
    );
    let error = engine
        .run(Some(&state), None, Some(&Text::new("step-1")), Some(true))
        .expect_err("checkpoint");
    assert!(matches!(error, WorkflowError::Approval(_)));
    let (engine, waiting_state) = waiting(true);
    let error = engine
        .record_step_result(&waiting_state, &Text::new("step-1"), true, None, None)
        .expect_err("checkpoint");
    assert!(matches!(error, WorkflowError::Approval(_)));
}

#[test]
fn approved_checkpoint_allows_the_step() {
    let (engine, waiting_state) = waiting(true);
    let approved = engine
        .approve_checkpoint(Some(&waiting_state), None, &approve_context(NOW))
        .expect("approve");
    let completed = engine
        .record_step_result(&approved, &Text::new("step-1"), true, None, None)
        .expect("record");
    assert_eq!(completed.status(), WorkflowStatus::Completed);
}

#[test]
fn step_results_must_match_the_selected_transition() {
    let engine = WorkflowEngine::new(branch_definition());
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    let error = engine
        .record_step_result(&state, &Text::new("step-2"), true, None, None)
        .expect_err("mismatch");
    assert!(
        matches!(error, WorkflowError::Transition(message) if message.contains("does not match"))
    );
}

#[test]
fn retry_bounds_fail_on_success_and_failure() {
    let engine = WorkflowEngine::new(branch_definition())
        .with_max_attempts([(Text::new("step-1"), BigInt::from(1))])
        .expect("bounds");
    let state = WorkflowState::from_parts(
        Text::new("workflow-1"),
        Text::new("workflow-def"),
        Text::new("1.0.0"),
        WorkflowStatus::Running,
        Text::new("start"),
        Vec::new(),
        vec![
            StepAttempt::from_parts(Text::new("step-1"), BigInt::from(1), false, None)
                .expect("attempt"),
        ],
        None,
        BigInt::from(0),
        Vec::new(),
        None,
    )
    .expect("state");
    let exhausted = engine
        .record_step_result(&state, &Text::new("step-1"), true, None, None)
        .expect("result");
    assert_eq!(exhausted.status(), WorkflowStatus::Failed);
    assert_eq!(
        exhausted.last_error().map(Text::to_string_lossy),
        Some("step_retry_bound_exhausted".to_owned())
    );

    let (engine, state) = started(false);
    let exhausted = engine
        .record_step_result(
            &state,
            &Text::new("step-1"),
            false,
            None,
            Some(&Text::new("flaky")),
        )
        .expect("result");
    assert_eq!(exhausted.status(), WorkflowStatus::Failed);
    assert_eq!(
        exhausted.last_error().map(Text::to_string_lossy),
        Some("flaky".to_owned())
    );
}

#[test]
fn failures_record_error_codes_and_emit_retry_events() {
    let engine = WorkflowEngine::new(branch_definition())
        .with_max_attempts([(Text::new("step-1"), BigInt::from(3))])
        .expect("bounds");
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    let retried = engine
        .record_step_failure(
            &state,
            &Text::new("step-1"),
            Some(&Text::new("solver_diverged")),
            None,
        )
        .expect("failure");
    assert_eq!(retried.status(), WorkflowStatus::Running);
    assert_eq!(
        retried.last_error().map(Text::to_string_lossy),
        Some("solver_diverged".to_owned())
    );
    assert_eq!(
        retried
            .attempts()
            .last()
            .expect("attempt")
            .error_code()
            .map(Text::to_string_lossy),
        Some("solver_diverged".to_owned())
    );
}

#[test]
fn success_advances_and_completes_the_chain() {
    let engine = WorkflowEngine::new(branch_definition());
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    let middle = engine
        .record_step_success(&state, &Text::new("step-1"), None)
        .expect("success");
    assert_eq!(middle.current_state().to_string_lossy(), "middle");
    let done = engine
        .record_step_success(&middle, &Text::new("step-2"), None)
        .expect("success");
    assert_eq!(done.status(), WorkflowStatus::Completed);
    assert_eq!(done.completed_step_ids().len(), 2);
    assert!(done.last_error().is_none());
}

#[test]
fn transition_application_rejects_illegal_origin_and_step() {
    let engine = WorkflowEngine::new(branch_definition());
    let state = WorkflowState::from_parts(
        Text::new("workflow-1"),
        Text::new("workflow-def"),
        Text::new("1.0.0"),
        WorkflowStatus::Running,
        Text::new("done"),
        Vec::new(),
        Vec::new(),
        None,
        BigInt::from(0),
        Vec::new(),
        None,
    )
    .expect("state");
    let transition = engine
        .get_transition(&Text::new("transition-1"))
        .expect("transition");
    assert!(matches!(
        engine.apply_transition(&state, &transition),
        Err(WorkflowError::Transition(message)) if message.contains("not legal")
    ));

    let ghost = WorkflowTransition::from_json_str(
        r#"{"transition_id":"ghost","from_state":"start","to_state":"middle","step_id":"ghost-step","checkpoint_type":null}"#,
    )
    .expect("transition");
    let start = WorkflowState::from_parts(
        Text::new("workflow-1"),
        Text::new("workflow-def"),
        Text::new("1.0.0"),
        WorkflowStatus::Running,
        Text::new("start"),
        Vec::new(),
        Vec::new(),
        None,
        BigInt::from(0),
        Vec::new(),
        None,
    )
    .expect("state");
    assert!(matches!(
        engine.apply_transition(&start, &ghost),
        Err(WorkflowError::Transition(message)) if message.contains("not declared by the workflow")
    ));
}

#[test]
fn event_order_matches_status_change_then_transition() {
    let sink = Arc::new(CollectingSink::new());
    let engine = WorkflowEngine::new(definition(false)).with_sink(sink.clone());
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    engine.run(Some(&state), None, None, None).expect("run");
    let types: Vec<String> = sink
        .events()
        .iter()
        .map(|event| event.event_type.to_string_lossy())
        .collect();
    assert_eq!(
        types,
        [
            "workflow.created",
            "workflow.status_changed",
            "workflow.transition"
        ]
    );
}

#[test]
fn stale_approval_is_invalidated_on_advance() {
    let (engine, waiting_state) = waiting(true);
    let approved = engine
        .approve_checkpoint(Some(&waiting_state), None, &approve_context(NOW))
        .expect("approve");
    let replanned = engine
        .request_replan(Some(&approved), Some(&Text::new("replan")))
        .expect("replan");
    let invalidated = engine.run(Some(&replanned), None, None, None).expect("run");
    assert_eq!(invalidated.status(), WorkflowStatus::WaitingForApproval);
    assert_eq!(
        invalidated.pending_approval().expect("pending").status(),
        ApprovalStatus::Invalidated
    );
    assert_eq!(
        invalidated.last_error().map(Text::to_string_lossy),
        Some("state_or_action_digest_changed".to_owned())
    );
}

#[test]
fn resume_handles_unapproved_approved_and_stale() {
    let (engine, waiting_state) = waiting(true);
    assert_eq!(
        engine.resume(Some(&waiting_state)).expect("resume"),
        waiting_state
    );
    let approved = engine
        .approve_checkpoint(Some(&waiting_state), None, &approve_context(NOW))
        .expect("approve");
    let completed = engine.resume(Some(&approved)).expect("resume");
    assert_eq!(completed.status(), WorkflowStatus::Completed);
    let tampered = WorkflowState::from_parts(
        Text::new("workflow-1"),
        Text::new("workflow-def"),
        Text::new("1.0.0"),
        WorkflowStatus::WaitingForApproval,
        Text::new("start"),
        Vec::new(),
        Vec::new(),
        approved.pending_approval().cloned(),
        BigInt::from(1),
        Vec::new(),
        None,
    )
    .expect("state");
    let resumed = engine.resume(Some(&tampered)).expect("resume");
    assert_eq!(
        resumed.pending_approval().expect("pending").status(),
        ApprovalStatus::Invalidated
    );
}

#[test]
fn record_step_result_invalidates_stale_and_consumes_approved() {
    let (engine, waiting_state) = waiting(true);
    let approved = engine
        .approve_checkpoint(Some(&waiting_state), None, &approve_context(NOW))
        .expect("approve");
    let tampered = WorkflowState::from_parts(
        Text::new("workflow-1"),
        Text::new("workflow-def"),
        Text::new("1.0.0"),
        WorkflowStatus::WaitingForApproval,
        Text::new("start"),
        Vec::new(),
        Vec::new(),
        approved.pending_approval().cloned(),
        BigInt::from(1),
        Vec::new(),
        None,
    )
    .expect("state");
    let stale = engine
        .record_step_result(&tampered, &Text::new("step-1"), true, None, None)
        .expect("result");
    assert_eq!(
        stale.pending_approval().expect("pending").status(),
        ApprovalStatus::Invalidated
    );
    let completed = engine
        .record_step_result(&approved, &Text::new("step-1"), true, None, None)
        .expect("result");
    assert_eq!(completed.status(), WorkflowStatus::Completed);
}

#[test]
fn approve_requires_a_pending_checkpoint_and_matching_id() {
    let (engine, state) = started(false);
    for result in [
        engine.approve_checkpoint(Some(&state), None, &approve_context(NOW)),
        engine.reject_checkpoint(Some(&state), None, &approve_context(NOW)),
        engine.abort_checkpoint(Some(&state), None, &approve_context(NOW)),
    ] {
        assert!(
            matches!(result, Err(WorkflowError::Approval(message)) if message.contains("no pending checkpoint"))
        );
    }
    let (engine, waiting_state) = waiting(true);
    let pending = waiting_state.pending_approval().expect("pending");
    assert!(matches!(
        engine.approve_checkpoint(Some(&waiting_state), Some(&Text::new("other")), &approve_context(NOW)),
        Err(WorkflowError::Approval(message)) if message.contains("approval_id does not match")
    ));
    let approved = engine
        .approve_checkpoint(
            Some(&waiting_state),
            Some(pending.approval_id()),
            &approve_context(NOW),
        )
        .expect("approve");
    assert_eq!(
        engine
            .approve_checkpoint(Some(&approved), None, &approve_context(LATER))
            .expect("idempotent"),
        approved
    );
}

#[test]
fn approve_refuses_an_already_rejected_checkpoint() {
    let (engine, waiting_state) = waiting(true);
    let pending = waiting_state.pending_approval().expect("pending");
    let rejected_pending = WorkflowApproval::from_parts(
        pending.approval_id().clone(),
        pending.transition_id().clone(),
        pending.step_id().clone(),
        pending.checkpoint_type(),
        pending.state_digest().clone(),
        pending.action_digest().clone(),
        ApprovalStatus::Rejected,
        Some(Text::new("reviewer")),
        Some(at(NOW)),
        Some(Text::new("already resolved")),
    )
    .expect("approval");
    let revived = state_with_pending(rejected_pending);
    engine.restore(revived).expect("restore");
    assert!(matches!(
        engine.approve_checkpoint(None, None, &approve_context(NOW)),
        Err(WorkflowError::Approval(message)) if message.contains("already rejected")
    ));
    assert!(matches!(
        engine.reject_checkpoint(None, None, &approve_context(NOW)),
        Err(WorkflowError::Approval(message)) if message.contains("already rejected")
    ));
}

#[test]
fn checkpoint_digest_mismatch_invalidates() {
    let (engine, waiting_state) = waiting(true);
    let invalidated = engine
        .approve_checkpoint(
            Some(&waiting_state),
            None,
            &CheckpointContext::new()
                .with_state_digest(Text::new(&"c".repeat(64)))
                .with_now(at(NOW)),
        )
        .expect("decision");
    assert_eq!(
        invalidated.pending_approval().expect("pending").status(),
        ApprovalStatus::Invalidated
    );
    assert_eq!(
        invalidated.last_error().map(Text::to_string_lossy),
        Some("state_or_action_digest_changed".to_owned())
    );
}

#[test]
fn approve_rejects_unsafe_actor_but_naive_time_is_not_representable() {
    let (engine, waiting_state) = waiting(true);
    assert!(matches!(
        engine.approve_checkpoint(
            Some(&waiting_state),
            None,
            &CheckpointContext::new().with_actor_id(Text::new("bad actor")),
        ),
        Err(WorkflowError::Transition(message)) if message.contains("actor_id")
    ));
}

#[test]
fn reject_fails_and_abort_cancels() {
    let (engine, waiting_state) = waiting(true);
    let rejected = engine
        .reject_checkpoint(
            Some(&waiting_state),
            None,
            &CheckpointContext::new()
                .with_reason(Text::new("geometry rejected"))
                .with_now(at(NOW)),
        )
        .expect("reject");
    assert_eq!(rejected.status(), WorkflowStatus::Failed);
    assert_eq!(
        rejected.last_error().map(Text::to_string_lossy),
        Some("geometry rejected".to_owned())
    );
    assert!(rejected.pending_approval().is_none());

    let (engine, waiting_state) = waiting(true);
    let aborted = engine
        .abort_checkpoint(
            Some(&waiting_state),
            None,
            &CheckpointContext::new()
                .with_reason(Text::new("operator aborted"))
                .with_now(at(NOW)),
        )
        .expect("abort");
    assert_eq!(aborted.status(), WorkflowStatus::Cancelled);
    assert_eq!(
        aborted.last_error().map(Text::to_string_lossy),
        Some("operator aborted".to_owned())
    );
}

#[test]
fn every_decision_spelling_and_enum_is_supported() {
    for spelling in ["approve", "approved"] {
        let (engine, waiting_state) = waiting(true);
        let decided = engine
            .decide_checkpoint_str(Some(&waiting_state), None, spelling, &approve_context(NOW))
            .expect("decision");
        assert_eq!(decided.status(), WorkflowStatus::WaitingForApproval);
        assert_eq!(
            engine.resume(Some(&decided)).expect("resume").status(),
            WorkflowStatus::Completed
        );
    }
    for (spelling, expected) in [
        ("reject", WorkflowStatus::Failed),
        ("rejected", WorkflowStatus::Failed),
        ("abort", WorkflowStatus::Cancelled),
        ("aborted", WorkflowStatus::Cancelled),
    ] {
        let (engine, waiting_state) = waiting(true);
        let decided = engine
            .decide_checkpoint_str(Some(&waiting_state), None, spelling, &approve_context(NOW))
            .expect("decision");
        assert_eq!(decided.status(), expected);
    }
    let (engine, waiting_state) = waiting(true);
    assert_eq!(
        engine
            .decide_checkpoint(
                Some(&waiting_state),
                None,
                ApprovalStatus::Rejected,
                &approve_context(NOW)
            )
            .expect("decision")
            .status(),
        WorkflowStatus::Failed
    );
    assert!(matches!(
        engine.decide_checkpoint(
            Some(&waiting_state),
            None,
            ApprovalStatus::Pending,
            &approve_context(NOW)
        ),
        Err(WorkflowError::Approval(message)) if message.contains("must be approve, reject, or abort")
    ));
    assert!(matches!(
        engine.decide_checkpoint_str(Some(&waiting_state), None, "maybe", &approve_context(NOW)),
        Err(WorkflowError::Transition(message)) if message.contains("approval status is invalid")
    ));
}

#[test]
fn cancel_is_idempotent_and_aborts_pending_approval() {
    let (engine, state) = started(false);
    let cancelled = engine
        .cancel(Some(&state), Some(&Text::new("stopped")))
        .expect("cancel");
    assert_eq!(cancelled.status(), WorkflowStatus::Cancelled);
    assert_eq!(
        engine.cancel(Some(&cancelled), None).expect("cancel"),
        cancelled
    );
    let (engine, waiting_state) = waiting(true);
    let cancelled = engine
        .cancel(Some(&waiting_state), Some(&Text::new("operator stopped")))
        .expect("cancel");
    assert_eq!(cancelled.status(), WorkflowStatus::Cancelled);
    let pending = cancelled.pending_approval().expect("pending");
    assert_eq!(pending.status(), ApprovalStatus::Aborted);
    assert_eq!(
        pending.reason().map(Text::to_string_lossy),
        Some("operator stopped".to_owned())
    );
    assert_eq!(
        pending.actor_id().map(Text::to_string_lossy),
        Some("local-user".to_owned())
    );
    assert!(pending.decided_at().is_some());
}

#[test]
fn replanning_increments_and_enforces_the_definition_limit() {
    let engine = WorkflowEngine::new(branch_definition());
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    let once = engine
        .request_replan(Some(&state), Some(&Text::new("approach")))
        .expect("replan");
    assert_eq!(once.replanning_attempts(), &BigInt::from(1));
    assert!(once.last_error().is_none());
    let twice = engine.request_replan(Some(&once), None).expect("replan");
    assert_eq!(twice.replanning_attempts(), &BigInt::from(2));
    assert!(matches!(
        engine.request_replan(Some(&twice), None),
        Err(WorkflowError::LimitExceeded(_))
    ));
}

#[test]
fn serialize_snapshot_and_restore_round_trip() {
    let (engine, state) = started(false);
    assert_eq!(
        engine.serialize(Some(&state)).expect("serialize"),
        state.to_json().expect("json")
    );
    assert_eq!(engine.snapshot(Some(&state)).expect("snapshot"), state);
    let json = engine.serialize(Some(&state)).expect("serialize");
    let restored = engine.restore_json(&json).expect("restore");
    assert_eq!(restored, state);
    assert_eq!(engine.state(), Some(state.clone()));
    let mapping = state.as_value();
    assert_eq!(engine.restore_value(&mapping).expect("restore"), state);
    assert_eq!(engine.restore(state.clone()).expect("restore"), state);
}

#[test]
fn restore_rejects_mismatched_definition_and_malformed_json() {
    let (engine, _) = started(false);
    let other = WorkflowState::from_parts(
        Text::new("workflow-1"),
        Text::new("other"),
        Text::new("1.0.0"),
        WorkflowStatus::Running,
        Text::new("start"),
        Vec::new(),
        Vec::new(),
        None,
        BigInt::from(0),
        Vec::new(),
        None,
    )
    .expect("state");
    assert!(engine.restore(other).is_err());
    assert!(engine.restore_json("{").is_err());
    assert!(engine.restore_json("[]").is_err());
}

#[test]
fn state_and_action_digests_are_deterministic_and_bind_the_transition() {
    let engine = WorkflowEngine::new(branch_definition());
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    let baseline = engine.state_digest_of(&state).expect("digest");
    assert_eq!(baseline, engine.state_digest(Some(&state)).expect("digest"));
    let advanced = engine
        .record_step_result(&state, &Text::new("step-1"), true, None, None)
        .expect("result");
    assert_ne!(engine.state_digest_of(&advanced).expect("digest"), baseline);
    let twin = WorkflowEngine::new(branch_definition());
    let twin_state = twin.start(&Text::new("workflow-1"), None).expect("start");
    assert_eq!(twin.state_digest_of(&twin_state).expect("digest"), baseline);
    let first = engine
        .get_transition(&Text::new("transition-1"))
        .expect("transition");
    let second = engine
        .get_transition(&Text::new("transition-2"))
        .expect("transition");
    assert_ne!(
        engine.action_digest(&first, "s").expect("digest"),
        engine.action_digest(&second, "s").expect("digest")
    );
    assert_ne!(
        engine.action_digest(&first, "s1").expect("digest"),
        engine.action_digest(&first, "s2").expect("digest")
    );
}

#[test]
fn approval_matching_rejects_unknown_origin_and_stale_digests() {
    let engine = WorkflowEngine::new(checkpoint_branch_definition());
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    let ghost = approval("approval-1", "ghost", "step-1", ApprovalKind::PreCommit);
    assert!(!engine.approval_matches(&state, &ghost));
    let wrong_origin = approval(
        "approval-1",
        "transition-2",
        "step-2",
        ApprovalKind::PreCommit,
    );
    assert!(!engine.approval_matches(&state, &wrong_origin));
    let transition = engine
        .get_transition(&Text::new("transition-1"))
        .expect("transition");
    let fresh = engine.make_approval(&state, &transition).expect("approval");
    assert!(engine.approval_matches(&state, &fresh));
    let stale = WorkflowApproval::from_parts(
        fresh.approval_id().clone(),
        fresh.transition_id().clone(),
        fresh.step_id().clone(),
        fresh.checkpoint_type(),
        Text::new(&"e".repeat(64)),
        fresh.action_digest().clone(),
        ApprovalStatus::Pending,
        None,
        None,
        None,
    )
    .expect("approval");
    assert!(!engine.approval_matches(&state, &stale));
}

#[test]
fn invalidate_approval_is_a_noop_without_pending_and_preserves_actor() {
    let engine = WorkflowEngine::new(branch_definition());
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    assert_eq!(
        engine
            .invalidate_approval(&state, &Text::new("reason"))
            .expect("noop"),
        state
    );

    let pending = approval(
        "approval-1",
        "transition-1",
        "step-1",
        ApprovalKind::PreCommit,
    );
    let waiting = state_with_pending(pending);
    let invalidated = engine
        .invalidate_approval(&waiting, &Text::new("stale"))
        .expect("invalidate");
    let record = invalidated.pending_approval().expect("pending");
    assert_eq!(record.status(), ApprovalStatus::Invalidated);
    assert_eq!(
        record.actor_id().map(Text::to_string_lossy),
        Some("local-user".to_owned())
    );
    assert_eq!(
        record.reason().map(Text::to_string_lossy),
        Some("stale".to_owned())
    );

    let resolved = WorkflowApproval::from_parts(
        Text::new("approval-1"),
        Text::new("transition-1"),
        Text::new("step-1"),
        ApprovalKind::PreCommit,
        Text::new(&"a".repeat(64)),
        Text::new(&"b".repeat(64)),
        ApprovalStatus::Invalidated,
        Some(Text::new("reviewer")),
        Some(at(NOW)),
        Some(Text::new("earlier")),
    )
    .expect("approval");
    let waiting = state_with_pending(resolved);
    let invalidated = engine
        .invalidate_approval(&waiting, &Text::new("stale"))
        .expect("invalidate");
    let record = invalidated.pending_approval().expect("pending");
    assert_eq!(
        record.actor_id().map(Text::to_string_lossy),
        Some("reviewer".to_owned())
    );
    assert_eq!(record.decided_at(), Some(at(NOW)));
}

#[test]
fn make_approval_requires_a_checkpoint_and_bound_digests() {
    let engine = WorkflowEngine::new(branch_definition());
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    let transition = engine
        .get_transition(&Text::new("transition-1"))
        .expect("transition");
    assert!(matches!(
        engine.make_approval(&state, &transition),
        Err(WorkflowError::Transition(message)) if message.contains("does not declare a checkpoint type")
    ));
    let checkpoint = definition(true);
    let engine = WorkflowEngine::new(checkpoint);
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    let transition = engine
        .get_transition(&Text::new("transition-1"))
        .expect("transition");
    let approval = engine.make_approval(&state, &transition).expect("approval");
    assert_eq!(
        approval.approval_id().to_string_lossy(),
        "approval-workflow-1-transition-1"
    );
    assert_eq!(
        approval.state_digest().to_string_lossy(),
        engine.state_digest_of(&state).expect("digest")
    );
}

#[test]
fn transition_selection_and_attempt_helpers() {
    let engine = WorkflowEngine::new(branch_definition());
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    assert!(
        engine
            .select_transition(&state, None)
            .expect("select")
            .is_some()
    );
    assert!(matches!(
        engine.select_transition(&state, Some(&Text::new("ghost"))),
        Err(WorkflowError::Transition(message)) if message.contains("not declared from the current state")
    ));
    assert!(engine.get_transition(&Text::new("ghost")).is_err());
    assert_eq!(engine.outgoing(&Text::new("start")).len(), 1);
    assert!(engine.outgoing(&Text::new("nowhere")).is_empty());
    assert_eq!(engine.attempt_limit(&Text::new("step-1")), BigInt::from(1));
    assert_eq!(
        engine.attempt_count(&state, &Text::new("step-1")),
        BigInt::from(0)
    );
}

#[test]
fn engine_exposes_definition_and_alias_methods() {
    let engine = WorkflowEngine::new(branch_definition());
    assert_eq!(engine.definition().definition_id(), "workflow-def");
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    assert_eq!(
        engine
            .advance(Some(&state), None, None, None)
            .expect("advance"),
        engine
            .execute(Some(&state), None, None, None)
            .expect("execute")
    );
}

fn failing_sink_after(fail_at: usize) -> Arc<dyn WorkflowEventSink> {
    struct FailingSink {
        calls: std::sync::atomic::AtomicUsize,
        fail_at: usize,
    }
    impl WorkflowEventSink for FailingSink {
        fn emit(&self, _event: &WorkflowEvent) -> Result<(), WorkflowError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call + 1 == self.fail_at {
                Err(WorkflowError::Callback("sink refused".to_owned()))
            } else {
                Ok(())
            }
        }
    }
    Arc::new(FailingSink {
        calls: std::sync::atomic::AtomicUsize::new(0),
        fail_at,
    })
}

#[test]
fn sink_failure_preserves_source_mutation_order() {
    let sink = failing_sink_after(1);
    let engine = WorkflowEngine::new(definition(false)).with_sink(sink);
    assert!(engine.start(&Text::new("workflow-1"), None).is_err());
    // `start` stores before emitting, exactly as the Python source does.
    assert!(engine.state().is_some());

    let sink = failing_sink_after(2);
    let engine = WorkflowEngine::new(definition(false)).with_sink(sink);
    let created = engine
        .start(&Text::new("workflow-1"), None)
        .expect("created");
    assert!(engine.run(Some(&created), None, None, None).is_err());
    // `run` stores only after `_advance` has emitted every event.
    assert_eq!(engine.state(), Some(created));
}

#[test]
fn reentrant_sink_can_read_engine_state_without_deadlock() {
    struct ReentrantSink {
        engine: Weak<WorkflowEngine>,
        seen: AtomicBool,
    }
    impl WorkflowEventSink for ReentrantSink {
        fn emit(&self, _event: &WorkflowEvent) -> Result<(), WorkflowError> {
            if let Some(engine) = self.engine.upgrade() {
                assert!(engine.state().is_some());
                self.seen.store(true, Ordering::SeqCst);
            }
            Ok(())
        }
    }
    let engine = Arc::new_cyclic(|weak| {
        let sink = Arc::new(ReentrantSink {
            engine: weak.clone(),
            seen: AtomicBool::new(false),
        });
        WorkflowEngine::new(definition(false)).with_sink(sink)
    });
    engine.start(&Text::new("workflow-1"), None).expect("start");
    let state = engine.state().expect("state");
    engine.run(Some(&state), None, None, None).expect("run");
}

#[test]
fn collecting_and_callback_sinks_receive_payloads() {
    let sink = Arc::new(CollectingSink::new());
    let engine = WorkflowEngine::new(definition(false)).with_sink(sink.clone());
    engine.start(&Text::new("workflow-1"), None).expect("start");
    assert_eq!(sink.events().len(), 1);

    let payloads = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = payloads.clone();
    let engine = WorkflowEngine::new(definition(false)).with_payload_callback(
        move |event_type: &Text, payload: &kala_json::CanonicalValue| {
            captured
                .lock()
                .unwrap()
                .push((event_type.to_string_lossy(), payload.clone()));
            Ok(())
        },
    );
    engine.start(&Text::new("workflow-1"), None).expect("start");
    assert_eq!(payloads.lock().unwrap().len(), 1);

    let journal = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = journal.clone();
    let engine = WorkflowEngine::new(definition(false)).with_journal_callback(
        move |event_type: &Text, workflow_id: &Text, _payload: &kala_json::CanonicalValue| {
            captured
                .lock()
                .unwrap()
                .push((event_type.to_string_lossy(), workflow_id.to_string_lossy()));
            Ok(())
        },
    );
    engine.start(&Text::new("workflow-1"), None).expect("start");
    assert_eq!(
        journal.lock().unwrap().as_slice(),
        [("workflow.created".to_owned(), "workflow-1".to_owned())]
    );
}

#[test]
fn concurrent_state_access_does_not_deadlock() {
    let engine = Arc::new(WorkflowEngine::new(branch_definition()));
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();
    for index in 0..8 {
        let engine = engine.clone();
        let barrier = barrier.clone();
        let state = state.clone();
        handles.push(thread::spawn(move || {
            barrier.wait();
            for _ in 0..100 {
                let _ = engine.state();
                let _ = engine.serialize(Some(&state));
                if index == 0 {
                    let _ = engine.state_digest_of(&state);
                }
            }
        }));
    }
    for handle in handles {
        handle.join().expect("thread");
    }
}

#[test]
fn type_aliases_are_available() {
    let _: Option<WorkflowTransitionError> = None;
    let _: Option<WorkflowLimitExceeded> = None;
    let _: Option<WorkflowApprovalError> = None;
}

#[test]
fn resolution_and_cancel_sink_failures_leave_the_previous_state() {
    // start=1, run status_changed=2, run checkpoint=3, approve=4.
    let sink = failing_sink_after(4);
    let engine = WorkflowEngine::new(definition(true))
        .with_sink(sink)
        .with_clock(Some(at(NOW)));
    let created = engine.start(&Text::new("workflow-1"), None).expect("start");
    let waiting_state = engine.run(Some(&created), None, None, None).expect("run");
    let pending_before = waiting_state.pending_approval().cloned();
    assert!(
        engine
            .approve_checkpoint(Some(&waiting_state), None, &approve_context(NOW))
            .is_err()
    );
    let current = engine.state().expect("state");
    assert_eq!(current.status(), WorkflowStatus::WaitingForApproval);
    assert_eq!(current.pending_approval().cloned(), pending_before);
    assert_eq!(
        current.pending_approval().expect("pending").status(),
        ApprovalStatus::Pending
    );

    // start=1, run=2/3, cancel=4.
    let sink = failing_sink_after(4);
    let engine = WorkflowEngine::new(definition(true))
        .with_sink(sink)
        .with_clock(Some(at(NOW)));
    let created = engine.start(&Text::new("workflow-1"), None).expect("start");
    let waiting_state = engine.run(Some(&created), None, None, None).expect("run");
    assert!(
        engine
            .cancel(Some(&waiting_state), Some(&Text::new("stop")))
            .is_err()
    );
    let current = engine.state().expect("state");
    assert_eq!(current.status(), WorkflowStatus::WaitingForApproval);
    assert_eq!(
        current.pending_approval().expect("pending").status(),
        ApprovalStatus::Pending
    );
}

#[test]
fn replan_and_invalidation_sink_failures_leave_the_previous_state() {
    let sink = failing_sink_after(2);
    let engine = WorkflowEngine::new(definition(false))
        .with_sink(sink)
        .with_clock(Some(at(NOW)));
    let created = engine.start(&Text::new("workflow-1"), None).expect("start");
    assert!(
        engine
            .request_replan(Some(&created), Some(&Text::new("again")))
            .is_err()
    );
    assert_eq!(engine.state(), Some(created));

    let sink = failing_sink_after(4);
    let engine = WorkflowEngine::new(definition(true))
        .with_sink(sink)
        .with_clock(Some(at(NOW)));
    let created = engine.start(&Text::new("workflow-1"), None).expect("start");
    let waiting_state = engine.run(Some(&created), None, None, None).expect("run");
    assert!(
        engine
            .invalidate_approval(&waiting_state, &Text::new("stale"))
            .is_err()
    );
    let current = engine.state().expect("state");
    assert_eq!(
        current.pending_approval().expect("pending").status(),
        ApprovalStatus::Pending
    );
}

#[test]
fn surrogate_workflow_ids_survive_digests_and_serialization() {
    use kala_contracts::Text as ContractText;
    let workflow_id =
        ContractText::from_code_points(vec![u32::from(b'w'), 0xD800, u32::from(b'x')])
            .expect("surrogate");
    let engine = WorkflowEngine::new(definition(false));
    let state = engine.start(&workflow_id, None).expect("start");
    assert_eq!(state.workflow_id().code_points(), workflow_id.code_points());
    let json = state.to_json().expect("json");
    assert!(json.contains("\\ud800"));
    let restored = engine.restore_json(&json).expect("restore");
    assert_eq!(
        restored.workflow_id().code_points(),
        workflow_id.code_points()
    );
    assert_eq!(
        engine.state_digest_of(&state).expect("digest"),
        engine.state_digest_of(&restored).expect("digest")
    );
}

#[test]
fn exact_cpython_state_action_and_canonical_bytes() {
    let engine = WorkflowEngine::new(definition(true));
    let state = engine.start(&Text::new("workflow-1"), None).expect("start");
    assert_eq!(
        state.to_json().expect("json"),
        r#"{"attempts":[],"completed_step_ids":[],"current_state":"start","definition_id":"workflow-def","definition_version":"1.0.0","history":[],"last_error":null,"pending_approval":null,"replanning_attempts":0,"status":"created","workflow_id":"workflow-1"}"#
    );
    assert_eq!(
        engine.state_digest_of(&state).expect("state digest"),
        "fb1cc7d28b1f133e058cc8debc9f91c16a99a83d1a6afdcd032b6e9b85502252"
    );
    let transition = engine
        .get_transition(&Text::new("transition-1"))
        .expect("transition");
    assert_eq!(
        engine
            .action_digest(
                &transition,
                &engine.state_digest_of(&state).expect("state digest")
            )
            .expect("action digest"),
        "4dd3b1f3859ad11aa8e216e1c5a6797d71e814c4c2c1806117c24bed77caab9d"
    );
}

#[test]
fn canonical_digest_failure_propagates_without_mutating_state() {
    let engine = WorkflowEngine::new(definition(true));
    let huge = BigInt::from(10).pow(4_301);
    let pending = WorkflowApproval::from_parts(
        Text::new("approval-workflow-1-transition-1"),
        Text::new("transition-1"),
        Text::new("step-1"),
        ApprovalKind::PreCommit,
        Text::new("a"),
        Text::new("b"),
        ApprovalStatus::Approved,
        Some(Text::new("reviewer")),
        Some(at(NOW)),
        None,
    )
    .expect("approval");
    let state = WorkflowState::from_parts(
        Text::new("workflow-1"),
        Text::new("workflow-def"),
        Text::new("1.0.0"),
        WorkflowStatus::WaitingForApproval,
        Text::new("start"),
        Vec::new(),
        vec![StepAttempt::from_parts(Text::new("step-1"), huge, false, None).expect("attempt")],
        Some(pending),
        BigInt::from(0),
        Vec::new(),
        None,
    )
    .expect("state");
    engine.restore(state.clone()).expect("restore");
    let error = engine.resume(None).expect_err("digest failure");
    assert!(matches!(error, WorkflowError::Transition(_)));
    assert_eq!(engine.state(), Some(state));
}

#[test]
fn surrogate_workflow_ids_survive_checkpoint_approval_ids() {
    let workflow_id =
        Text::from_code_points(vec![u32::from(b'w'), 0xD800, u32::from(b'x')]).expect("surrogate");
    let engine = WorkflowEngine::new(definition(true));
    let state = engine.start(&workflow_id, None).expect("start");
    let waiting_state = engine.run(Some(&state), None, None, None).expect("run");
    let approval = waiting_state.pending_approval().expect("pending");
    assert!(approval.approval_id().code_points().contains(&0xD800));
    let json = waiting_state.to_json().expect("json");
    assert!(json.contains("\\ud800"));
    assert!(engine.restore_json(&json).is_ok());
}
