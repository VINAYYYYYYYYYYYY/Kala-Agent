from __future__ import annotations

import json
from collections.abc import Mapping
from dataclasses import replace
from datetime import UTC, datetime, timedelta
from typing import Any, cast

import pytest

from kala_agent.contracts import (
    ApprovalKind,
    ApprovalStatus,
    WorkflowDefinition,
    WorkflowStatus,
    WorkflowTransition,
)
from kala_agent.kernel.workflow import (
    StepAttempt,
    WorkflowApproval,
    WorkflowApprovalError,
    WorkflowEngine,
    WorkflowEvent,
    WorkflowLimitExceeded,
    WorkflowState,
    WorkflowTransitionError,
    WorkflowTransitionRecord,
    _safe_identifier,
    _status,
)

DECIDED_AT = datetime(2026, 1, 2, 3, 4, 5, tzinfo=UTC)


def _definition(
    *,
    states: tuple[str, ...] = ("start", "middle", "done"),
    transitions: tuple[WorkflowTransition, ...] | None = None,
    available: tuple[str, ...] = ("step-1", "step-2"),
    max_replanning_attempts: int = 1,
) -> WorkflowDefinition:
    if transitions is None:
        transitions = (
            WorkflowTransition(
                transition_id="transition-1",
                from_state="start",
                to_state="middle",
                step_id="step-1",
            ),
            WorkflowTransition(
                transition_id="transition-2",
                from_state="middle",
                to_state="done",
                step_id="step-2",
            ),
        )
    return WorkflowDefinition(
        definition_id="workflow-def",
        version="1.0.0",
        name="Branchable workflow",
        initial_state="start",
        states=states,
        transitions=transitions,
        available_step_ids=available,
        max_replanning_attempts=max_replanning_attempts,
    )


def _single_step_definition(
    *,
    checkpoint: bool = False,
    transition_id: str = "transition-1",
    from_state: str = "start",
    to_state: str = "done",
    step_id: str = "step-1",
) -> WorkflowDefinition:
    return WorkflowDefinition(
        definition_id="workflow-def",
        version="1.0.0",
        name="Single step",
        initial_state=from_state,
        states=("start", "done"),
        transitions=(
            WorkflowTransition(
                transition_id=transition_id,
                from_state=from_state,
                to_state=to_state,
                step_id=step_id,
                checkpoint_type=ApprovalKind.PRE_COMMIT if checkpoint else None,
            ),
        ),
        available_step_ids=(step_id,),
        max_replanning_attempts=1,
    )


def _approval(**overrides: object) -> WorkflowApproval:
    values: dict[str, Any] = {
        "approval_id": "approval-1",
        "transition_id": "transition-1",
        "step_id": "step-1",
        "checkpoint_type": ApprovalKind.PRE_COMMIT,
        "state_digest": "a" * 64,
        "action_digest": "b" * 64,
    }
    values.update(cast("Mapping[str, Any]", overrides))
    return WorkflowApproval(**values)


def _state(**overrides: object) -> WorkflowState:
    values: dict[str, Any] = {
        "workflow_id": "workflow-1",
        "definition_id": "workflow-def",
        "definition_version": "1.0.0",
        "status": WorkflowStatus.RUNNING,
        "current_state": "start",
    }
    values.update(cast("Mapping[str, Any]", overrides))
    return WorkflowState(**values)


def _replaced_approval(state: WorkflowState, status: ApprovalStatus) -> WorkflowApproval:
    pending = state.pending_approval
    assert pending is not None
    return replace(
        pending,
        status=status,
        actor_id="reviewer",
        decided_at=DECIDED_AT,
        reason="already resolved",
    )


def test_safe_identifier_rejects_non_strings() -> None:
    with pytest.raises(WorkflowTransitionError, match="must be a string"):
        _safe_identifier(7, "workflow_id")


def test_safe_identifier_rejects_empty_and_oversized() -> None:
    with pytest.raises(WorkflowTransitionError, match="nonempty and bounded"):
        _safe_identifier("   ", "workflow_id")
    with pytest.raises(WorkflowTransitionError, match="nonempty and bounded"):
        _safe_identifier("x" * 129, "workflow_id")


def test_safe_identifier_rejects_unsafe_characters() -> None:
    with pytest.raises(WorkflowTransitionError, match="unsafe characters"):
        _safe_identifier("has space", "workflow_id")
    with pytest.raises(WorkflowTransitionError, match="unsafe characters"):
        _safe_identifier("has\x00null", "workflow_id")
    with pytest.raises(WorkflowTransitionError, match="unsafe characters"):
        _safe_identifier("has\nnewline", "workflow_id")


def test_safe_identifier_strips_surrounding_space() -> None:
    assert _safe_identifier("  padded  ", "workflow_id") == "padded"


def test_status_accepts_enum_and_string() -> None:
    assert _status(WorkflowStatus.RUNNING) is WorkflowStatus.RUNNING
    assert _status("running") is WorkflowStatus.RUNNING


def test_status_rejects_unknown_values() -> None:
    with pytest.raises(WorkflowTransitionError, match="status is invalid"):
        _status("nowhere")
    with pytest.raises(WorkflowTransitionError, match="status is invalid"):
        _status(cast(WorkflowStatus, 7))


def test_checkpoint_status_rejects_unknown_values() -> None:
    from kala_agent.kernel.workflow import _checkpoint_status

    assert _checkpoint_status(ApprovalStatus.PENDING) is ApprovalStatus.PENDING
    assert _checkpoint_status("pending") is ApprovalStatus.PENDING
    with pytest.raises(WorkflowTransitionError, match="approval status is invalid"):
        _checkpoint_status("undecided")


def test_checkpoint_kind_rejects_unknown_values() -> None:
    from kala_agent.kernel.workflow import _checkpoint_kind

    assert _checkpoint_kind(ApprovalKind.PRE_COMMIT) is ApprovalKind.PRE_COMMIT
    assert _checkpoint_kind("pre_commit") is ApprovalKind.PRE_COMMIT
    with pytest.raises(WorkflowTransitionError, match="checkpoint_type is invalid"):
        _checkpoint_kind("mid_flight")


def test_parse_datetime_rejects_non_strings() -> None:
    from kala_agent.kernel.workflow import _parse_datetime

    assert _parse_datetime(None) is None
    with pytest.raises(WorkflowTransitionError, match="timestamp is invalid"):
        _parse_datetime(7)


def test_parse_datetime_rejects_unparsable_text() -> None:
    from kala_agent.kernel.workflow import _parse_datetime

    with pytest.raises(WorkflowTransitionError, match="timestamp is invalid"):
        _parse_datetime("not-a-time")


def test_parse_datetime_requires_a_timezone() -> None:
    from kala_agent.kernel.workflow import _parse_datetime

    with pytest.raises(WorkflowTransitionError, match="must include a timezone"):
        _parse_datetime("2026-01-02T03:04:05")


def test_parse_datetime_normalizes_to_utc() -> None:
    from kala_agent.kernel.workflow import _parse_datetime

    assert _parse_datetime("2026-01-02T03:04:05Z") == DECIDED_AT


def test_approval_rejects_blank_identifiers() -> None:
    for field in ("approval_id", "transition_id", "step_id"):
        with pytest.raises(WorkflowTransitionError, match="nonempty and bounded"):
            _approval(**{field: "  "})


def test_approval_rejects_empty_digests() -> None:
    with pytest.raises(WorkflowTransitionError, match="state_digest"):
        _approval(state_digest="")
    with pytest.raises(WorkflowTransitionError, match="action_digest"):
        _approval(action_digest="")
    with pytest.raises(WorkflowTransitionError, match="state_digest"):
        _approval(state_digest=cast(str, 7))


def test_approval_rejects_naive_decision_time() -> None:
    with pytest.raises(WorkflowTransitionError, match="decided_at must include a timezone"):
        _approval(
            status=ApprovalStatus.APPROVED, actor_id="operator", decided_at=datetime(2026, 1, 1)
        )


def test_approval_normalizes_decision_time_to_utc() -> None:
    from datetime import timezone

    offset = datetime(2026, 1, 2, 5, 4, 5, tzinfo=timezone(timedelta(hours=2)))
    approval = _approval(status=ApprovalStatus.APPROVED, actor_id="operator", decided_at=offset)
    assert approval.decided_at == DECIDED_AT


def test_approval_rejects_decision_fields_while_pending() -> None:
    with pytest.raises(WorkflowTransitionError, match="cannot contain a decision"):
        _approval(actor_id="operator")
    with pytest.raises(WorkflowTransitionError, match="cannot contain a decision"):
        _approval(decided_at=DECIDED_AT)
    with pytest.raises(WorkflowTransitionError, match="cannot contain a decision"):
        _approval(reason="because")


def test_approval_requires_actor_and_time_when_resolved() -> None:
    for status in (ApprovalStatus.APPROVED, ApprovalStatus.REJECTED, ApprovalStatus.ABORTED):
        with pytest.raises(WorkflowTransitionError, match="requires actor and decision time"):
            _approval(status=status, actor_id="operator")
        with pytest.raises(WorkflowTransitionError, match="requires actor and decision time"):
            _approval(status=status, decided_at=DECIDED_AT)


def test_approval_exposes_status_predicates() -> None:
    pending = _approval()
    assert pending.is_pending is True
    assert pending.is_approved is False
    approved = _approval(status=ApprovalStatus.APPROVED, actor_id="op", decided_at=DECIDED_AT)
    assert approved.is_pending is False
    assert approved.is_approved is True


def test_approval_rejects_unsafe_actor() -> None:
    with pytest.raises(WorkflowTransitionError, match="actor_id"):
        _approval(
            status=ApprovalStatus.APPROVED,
            actor_id="has space",
            decided_at=DECIDED_AT,
        )


def test_approval_roundtrips_through_a_mapping() -> None:
    approved = _approval(
        status=ApprovalStatus.APPROVED,
        actor_id="operator",
        decided_at=DECIDED_AT,
        reason="reviewed",
    )
    payload = approved.to_dict()
    assert payload["status"] == "approved"
    assert payload["decided_at"] == DECIDED_AT.isoformat()
    assert WorkflowApproval.from_dict(payload) == approved


def test_approval_from_dict_requires_every_field() -> None:
    payload = dict(_approval().to_dict())
    del payload["transition_id"]
    with pytest.raises(WorkflowTransitionError, match="missing a field"):
        WorkflowApproval.from_dict(payload)


def test_approval_from_dict_rejects_bad_timestamp() -> None:
    payload = dict(_approval().to_dict())
    payload["decided_at"] = "not-a-time"
    with pytest.raises(WorkflowTransitionError, match="timestamp is invalid"):
        WorkflowApproval.from_dict(payload)


def test_step_attempt_validates_its_fields() -> None:
    with pytest.raises(WorkflowTransitionError, match="positive integer"):
        StepAttempt(step_id="s-1", attempt=0, succeeded=True)
    with pytest.raises(WorkflowTransitionError, match="positive integer"):
        StepAttempt(step_id="s-1", attempt=cast(int, True), succeeded=True)
    with pytest.raises(WorkflowTransitionError, match="must be a boolean"):
        StepAttempt(step_id="s-1", attempt=1, succeeded=cast(bool, "yes"))
    with pytest.raises(WorkflowTransitionError, match="error_code"):
        StepAttempt(step_id="s-1", attempt=1, succeeded=False, error_code="bad code")


def test_step_attempt_roundtrips_through_a_mapping() -> None:
    attempt = StepAttempt(step_id="s-1", attempt=2, succeeded=False, error_code="flaky")
    assert StepAttempt.from_dict(attempt.to_dict()) == attempt


def test_step_attempt_from_dict_requires_every_field() -> None:
    payload = StepAttempt(step_id="s-1", attempt=1, succeeded=True).to_dict()
    del payload["attempt"]
    with pytest.raises(WorkflowTransitionError, match="missing a field"):
        StepAttempt.from_dict(payload)


def test_transition_record_validates_its_fields() -> None:
    with pytest.raises(WorkflowTransitionError, match="sequence must be positive"):
        WorkflowTransitionRecord(
            sequence=0,
            transition_id="t-1",
            from_state="a",
            to_state="b",
            step_id="s-1",
            attempt=1,
        )
    with pytest.raises(WorkflowTransitionError, match="attempt must be positive"):
        WorkflowTransitionRecord(
            sequence=1,
            transition_id="t-1",
            from_state="a",
            to_state="b",
            step_id="s-1",
            attempt=0,
        )
    with pytest.raises(WorkflowTransitionError, match="to_state"):
        WorkflowTransitionRecord(
            sequence=1,
            transition_id="t-1",
            from_state="a",
            to_state="  ",
            step_id="s-1",
            attempt=1,
        )


def test_transition_record_roundtrips_through_a_mapping() -> None:
    record = WorkflowTransitionRecord(
        sequence=1,
        transition_id="t-1",
        from_state="a",
        to_state="b",
        step_id="s-1",
        attempt=1,
    )
    assert WorkflowTransitionRecord.from_dict(record.to_dict()) == record


def test_transition_record_from_dict_requires_every_field() -> None:
    payload = WorkflowTransitionRecord(
        sequence=1,
        transition_id="t-1",
        from_state="a",
        to_state="b",
        step_id="s-1",
        attempt=1,
    ).to_dict()
    del payload["to_state"]
    with pytest.raises(WorkflowTransitionError, match="missing a field"):
        WorkflowTransitionRecord.from_dict(payload)


def test_state_validates_replanning_attempts() -> None:
    with pytest.raises(WorkflowTransitionError, match="must be an integer"):
        _state(replanning_attempts=cast(int, True))
    with pytest.raises(WorkflowTransitionError, match="must be nonnegative"):
        _state(replanning_attempts=-1)


def test_state_rejects_non_approval_pending() -> None:
    with pytest.raises(WorkflowTransitionError, match="must be a WorkflowApproval"):
        _state(pending_approval=cast(Any, {"approval_id": "a"}))


def test_state_requires_approval_when_waiting() -> None:
    with pytest.raises(WorkflowTransitionError, match="require a pending approval"):
        _state(status=WorkflowStatus.WAITING_FOR_APPROVAL)


def test_state_exposes_terminal_predicates() -> None:
    assert _state(status=WorkflowStatus.COMPLETED).is_terminal is True
    assert _state(status=WorkflowStatus.FAILED).is_terminal is True
    assert _state(status=WorkflowStatus.CANCELLED).is_cancelled is True
    assert _state(status=WorkflowStatus.RUNNING).is_terminal is False
    assert _state(status=WorkflowStatus.RUNNING).is_cancelled is False


def test_state_rejects_unsafe_identifiers() -> None:
    with pytest.raises(WorkflowTransitionError, match="workflow_id"):
        _state(workflow_id="bad id")
    with pytest.raises(WorkflowTransitionError, match="definition_id"):
        _state(definition_id="")
    with pytest.raises(WorkflowTransitionError, match="definition_version"):
        _state(definition_version="  ")
    with pytest.raises(WorkflowTransitionError, match="current_state"):
        _state(current_state="bad state")
    with pytest.raises(WorkflowTransitionError, match="completed_step_id"):
        _state(completed_step_ids=("bad id",))


def test_state_roundtrips_through_json() -> None:
    state = _state(
        status=WorkflowStatus.WAITING_FOR_APPROVAL,
        pending_approval=_approval(),
        completed_step_ids=("step-0",),
        attempts=(StepAttempt(step_id="step-0", attempt=1, succeeded=True),),
        history=(
            WorkflowTransitionRecord(
                sequence=1,
                transition_id="t-0",
                from_state="start",
                to_state="middle",
                step_id="step-0",
                attempt=1,
            ),
        ),
        last_error="earlier",
    )
    document = json.loads(state.to_json())
    assert document["status"] == "waiting_for_approval"
    assert WorkflowState.from_json(state.to_json()) == state


def test_state_from_dict_rejects_non_mapping_approval() -> None:
    payload = dict(_state().to_dict())
    payload["pending_approval"] = ["not", "a", "mapping"]
    with pytest.raises(WorkflowTransitionError, match="approval is invalid"):
        WorkflowState.from_dict(payload)


def test_state_from_dict_rejects_bad_attempt_collections() -> None:
    payload = dict(_state().to_dict())
    payload["attempts"] = "step-1"
    with pytest.raises(WorkflowTransitionError, match="attempts are invalid"):
        WorkflowState.from_dict(payload)
    payload = dict(_state().to_dict())
    payload["history"] = {"a": 1}
    with pytest.raises(WorkflowTransitionError, match="history is invalid"):
        WorkflowState.from_dict(payload)


def test_state_from_dict_requires_core_fields() -> None:
    payload = dict(_state().to_dict())
    del payload["current_state"]
    with pytest.raises(WorkflowTransitionError, match="missing a field"):
        WorkflowState.from_dict(payload)


def test_state_from_dict_defaults_optional_collections() -> None:
    payload = cast("dict[str, object]", dict(_state().to_dict()))
    payload.pop("attempts")
    payload.pop("history")
    payload.pop("replanning_attempts")
    payload.pop("pending_approval")
    restored = WorkflowState.from_dict(payload)
    assert restored.attempts == ()
    assert restored.history == ()
    assert restored.replanning_attempts == 0
    assert restored.pending_approval is None


def test_state_from_json_requires_a_string() -> None:
    with pytest.raises(TypeError, match="must be a string"):
        WorkflowState.from_json(cast(str, 7))


def test_state_from_json_rejects_unparsable_text() -> None:
    with pytest.raises(WorkflowTransitionError, match="state is invalid"):
        WorkflowState.from_json("{not json")


def test_state_from_json_requires_an_object() -> None:
    with pytest.raises(WorkflowTransitionError, match="must be an object"):
        WorkflowState.from_json("[]")


def test_workflow_event_payload_renders_statuses() -> None:
    event = WorkflowEvent(
        event_type="workflow.transition",
        workflow_id="workflow-1",
        from_status=WorkflowStatus.RUNNING,
        to_status=WorkflowStatus.COMPLETED,
        current_state="done",
        transition_id="t-1",
        step_id="step-1",
        attempt=1,
        reason="done",
        approval=_approval(),
    )
    payload = event.payload
    assert payload["from_status"] == "running"
    assert payload["to_status"] == "completed"
    assert cast("dict[str, object]", payload["approval"])["approval_id"] == "approval-1"
    assert event.to_dict()["event_type"] == "workflow.transition"
    assert event.to_dict()["workflow_id"] == "workflow-1"


def test_workflow_event_payload_renders_missing_fields() -> None:
    event = WorkflowEvent(
        event_type="workflow.created",
        workflow_id="workflow-1",
        from_status=None,
        to_status=WorkflowStatus.CREATED,
        current_state="start",
    )
    payload = event.payload
    assert payload["from_status"] is None
    assert payload["transition_id"] is None
    assert payload["attempt"] is None
    assert payload["approval"] is None


def test_engine_requires_a_definition() -> None:
    with pytest.raises(TypeError, match="WorkflowDefinition"):
        WorkflowEngine(cast(WorkflowDefinition, object()))


def test_engine_rejects_multiple_callbacks() -> None:
    with pytest.raises(TypeError, match="only one event journal"):
        WorkflowEngine(_definition(), list().append, journal_callback=list().append)
    with pytest.raises(TypeError, match="only one event journal"):
        WorkflowEngine(_definition(), list().append, event_callback=list().append)


def test_engine_rejects_bad_retry_bounds() -> None:
    with pytest.raises(WorkflowTransitionError, match="positive integers"):
        WorkflowEngine(_definition(), max_attempts={"step-1": 0})
    with pytest.raises(WorkflowTransitionError, match="positive integers"):
        WorkflowEngine(_definition(), max_attempts={"step-1": cast(int, True)})
    with pytest.raises(WorkflowTransitionError, match="nonempty and bounded"):
        WorkflowEngine(_definition(), max_attempts={"": 1})


def test_engine_reports_missing_state() -> None:
    engine = WorkflowEngine(_definition())
    assert engine.state is None
    assert engine.current_state is None
    with pytest.raises(WorkflowTransitionError, match="has not been started"):
        engine.run()
    with pytest.raises(WorkflowTransitionError, match="has not been started"):
        engine.cancel()
    with pytest.raises(WorkflowTransitionError, match="has not been started"):
        engine.serialize()
    with pytest.raises(WorkflowTransitionError, match="has not been started"):
        engine.state_digest()
    with pytest.raises(WorkflowTransitionError, match="has not been started"):
        engine.snapshot()


def test_engine_exposes_its_definition() -> None:
    definition = _definition()
    assert WorkflowEngine(definition).definition is definition


def test_start_is_idempotent_for_the_same_workflow_id() -> None:
    engine = WorkflowEngine(_definition())
    first = engine.start("workflow-1")
    assert engine.start("workflow-1") is first


def test_start_restarts_for_a_different_workflow_id() -> None:
    engine = WorkflowEngine(_definition())
    first = engine.start("workflow-1")
    second = engine.start("workflow-2")
    assert second.workflow_id == "workflow-2"
    assert first is not second


def test_start_rejects_bad_replanning_attempts() -> None:
    engine = WorkflowEngine(_definition())
    with pytest.raises(WorkflowTransitionError, match="must be an integer"):
        engine.start("workflow-1", replanning_attempts=cast(int, True))
    with pytest.raises(WorkflowLimitExceeded, match="exceed the workflow limit"):
        engine.start("workflow-1", replanning_attempts=-1)
    with pytest.raises(WorkflowLimitExceeded, match="exceed the workflow limit"):
        engine.start("workflow-1", replanning_attempts=9)


def test_start_emits_a_created_event() -> None:
    events: list[WorkflowEvent] = []
    engine = WorkflowEngine(_definition(), events.append)
    engine.start("workflow-1")
    assert [event.event_type for event in events] == ["workflow.created"]
    assert events[0].from_status is None
    assert events[0].to_status is WorkflowStatus.CREATED


def test_run_requires_succeeded_with_step_id() -> None:
    engine = WorkflowEngine(_definition())
    state = engine.start("workflow-1")
    with pytest.raises(WorkflowTransitionError, match="succeeded must be supplied"):
        engine.run(state, step_id="step-1")


def test_run_infers_step_from_selected_transition() -> None:
    engine = WorkflowEngine(_definition())
    state = engine.start("workflow-1")
    advanced = engine.run(state, succeeded=True)
    assert advanced.current_state == "middle"
    assert advanced.completed_step_ids == ("step-1",)


def test_run_completes_when_no_transition_remains() -> None:
    engine = WorkflowEngine(_single_step_definition())
    state = engine.start("workflow-1")
    completed = engine.run(state)
    assert completed.status is WorkflowStatus.COMPLETED
    assert completed.current_state == "done"


def test_run_leaves_terminal_states_untouched() -> None:
    engine = WorkflowEngine(_single_step_definition())
    state = engine.start("workflow-1")
    cancelled = engine.cancel(state)
    assert engine.run(cancelled) == cancelled


def test_run_rejects_undeclared_transition() -> None:
    engine = WorkflowEngine(_definition())
    state = engine.start("workflow-1")
    with pytest.raises(WorkflowTransitionError, match="not declared from the current state"):
        engine.run(state, transition_id="transition-9")


def test_run_rejects_unbounded_transition_id() -> None:
    engine = WorkflowEngine(_definition())
    state = engine.start("workflow-1")
    with pytest.raises(WorkflowTransitionError, match="must be a string"):
        engine.run(state, transition_id=cast(str, 7))


def test_run_rejects_state_from_another_definition() -> None:
    engine = WorkflowEngine(_definition())
    with pytest.raises(WorkflowTransitionError, match="definition_id does not match"):
        engine.run(_state(definition_id="other"))
    with pytest.raises(WorkflowTransitionError, match="definition version does not match"):
        engine.run(_state(definition_version="9.9.9"))
    with pytest.raises(WorkflowTransitionError, match="current_state is not declared"):
        engine.run(_state(current_state="nowhere"))
    with pytest.raises(TypeError, match="must be a WorkflowState"):
        engine.run(cast(WorkflowState, "state"))


def test_engine_aliases_point_at_run() -> None:
    assert WorkflowEngine.advance is WorkflowEngine.run
    assert WorkflowEngine.execute is WorkflowEngine.run
    assert WorkflowEngine.step is WorkflowEngine.run
    assert WorkflowEngine.retry_step is WorkflowEngine.record_step_failure
    assert WorkflowEngine.reject is WorkflowEngine.reject_checkpoint
    assert WorkflowEngine.abort is WorkflowEngine.abort_checkpoint
    assert WorkflowEngine.replan is WorkflowEngine.request_replan
    assert WorkflowEngine.restore_state is WorkflowEngine.restore
    assert WorkflowEngine.resume_from is WorkflowEngine.restore


def test_record_step_result_returns_terminal_state_unchanged() -> None:
    engine = WorkflowEngine(_single_step_definition())
    state = engine.start("workflow-1")
    cancelled = engine.cancel(state)
    assert engine.record_step_result(cancelled, "step-1", True) == cancelled


def test_record_step_result_requires_approval_first() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    with pytest.raises(WorkflowApprovalError, match="before checkpoint approval"):
        engine.record_step_result(waiting, "step-1", True)


def test_record_step_result_cannot_skip_a_checkpoint_that_was_never_opened() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    with pytest.raises(WorkflowApprovalError, match="before checkpoint approval"):
        engine.record_step_result(state, "step-1", True)
    with pytest.raises(WorkflowApprovalError, match="before checkpoint approval"):
        engine.run(state, step_id="step-1", succeeded=True)


def test_record_step_result_applies_a_checkpoint_after_approval() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    approved = engine.approve_checkpoint(engine.run(engine.start("workflow-1")))
    completed = engine.record_step_result(approved, "step-1", True)
    assert completed.current_state == "done"


def test_record_step_result_rejects_mismatched_step() -> None:
    engine = WorkflowEngine(_definition())
    state = engine.start("workflow-1")
    with pytest.raises(WorkflowTransitionError, match="does not match the declared transition"):
        engine.record_step_result(state, "step-2", True)


def test_record_step_result_requires_a_boolean_outcome() -> None:
    engine = WorkflowEngine(_definition())
    state = engine.start("workflow-1")
    with pytest.raises(WorkflowTransitionError, match="must be a boolean"):
        engine.record_step_result(state, "step-1", cast(bool, "yes"))


def test_record_step_result_completes_when_no_transition_remains() -> None:
    engine = WorkflowEngine(_single_step_definition())
    state = engine.start("workflow-1")
    completed = engine.record_step_result(state, "step-1", True)
    assert completed.status is WorkflowStatus.COMPLETED


def test_record_step_result_fails_after_retry_bound_on_success() -> None:
    engine = WorkflowEngine(_definition(), max_attempts={"step-1": 1})
    state = _state(
        attempts=(StepAttempt(step_id="step-1", attempt=1, succeeded=False),),
    )
    exhausted = engine.record_step_result(state, "step-1", True)
    assert exhausted.status is WorkflowStatus.FAILED
    assert exhausted.last_error == "step_retry_bound_exhausted"


def test_record_step_result_fails_after_retry_bound_on_failure() -> None:
    engine = WorkflowEngine(_definition(), max_attempts={"step-1": 1})
    state = engine.start("workflow-1")
    exhausted = engine.record_step_result(state, "step-1", False)
    assert exhausted.status is WorkflowStatus.FAILED
    assert exhausted.last_error == "step_failed"


def test_record_step_result_records_custom_error_code() -> None:
    engine = WorkflowEngine(_definition(), max_attempts={"step-1": 3})
    state = engine.start("workflow-1")
    retried = engine.record_step_failure(state, "step-1", error_code="solver_diverged")
    assert retried.status is WorkflowStatus.RUNNING
    assert retried.last_error == "solver_diverged"
    assert retried.attempts[-1].error_code == "solver_diverged"
    assert retried.attempts[-1].succeeded is False


def test_record_step_success_advances_and_completes() -> None:
    engine = WorkflowEngine(_definition())
    state = engine.start("workflow-1")
    middle = engine.record_step_success(state, "step-1")
    assert middle.current_state == "middle"
    done = engine.record_step_success(middle, "step-2")
    assert done.status is WorkflowStatus.COMPLETED
    assert done.completed_step_ids == ("step-1", "step-2")
    assert done.last_error is None


def test_apply_transition_rejects_illegal_origin() -> None:
    engine = WorkflowEngine(_definition())
    state = _state(current_state="done")
    with pytest.raises(WorkflowTransitionError, match="not legal from the current state"):
        engine._apply_transition(
            state,
            _definition().transitions[0],
        )


def test_apply_transition_rejects_undeclared_step() -> None:
    definition = WorkflowDefinition(
        definition_id="workflow-def",
        version="1.0.0",
        name="Single step",
        initial_state="start",
        states=("start", "done"),
        transitions=(
            WorkflowTransition(
                transition_id="transition-1",
                from_state="start",
                to_state="done",
                step_id="step-1",
            ),
        ),
        available_step_ids=("step-1",),
        max_replanning_attempts=1,
    )
    engine = WorkflowEngine(definition)
    state = _state(current_state="start")
    transition = WorkflowTransition.model_construct(
        transition_id="ghost",
        from_state="start",
        to_state="middle",
        step_id="ghost-step",
        checkpoint_type=None,
    )
    with pytest.raises(WorkflowTransitionError, match="not declared by the workflow"):
        engine._apply_transition(state, transition)


def test_apply_transition_fails_after_retry_bound() -> None:
    engine = WorkflowEngine(_definition(), max_attempts={"step-1": 1})
    state = _state(
        status=WorkflowStatus.RUNNING,
        attempts=(StepAttempt(step_id="step-1", attempt=1, succeeded=True),),
    )
    failed = engine._apply_transition(state, _definition().transitions[0])
    assert failed.status is WorkflowStatus.FAILED
    assert failed.last_error == "step_retry_bound_exhausted"


def test_advance_emits_status_change_then_transition() -> None:
    events: list[WorkflowEvent] = []
    engine = WorkflowEngine(_definition(), events.append)
    state = engine.start("workflow-1")
    engine.run(state)
    assert [event.event_type for event in events] == [
        "workflow.created",
        "workflow.status_changed",
        "workflow.transition",
    ]


def test_advance_stops_at_a_pending_approval() -> None:
    events: list[WorkflowEvent] = []
    engine = WorkflowEngine(_single_step_definition(checkpoint=True), events.append)
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    assert waiting.status is WorkflowStatus.WAITING_FOR_APPROVAL
    assert any(event.event_type == "workflow.checkpoint" for event in events)
    assert engine.run(waiting) == waiting


def test_advance_invalidates_a_stale_approval() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    approved = engine.approve_checkpoint(waiting)
    stale = engine.request_replan(approved)
    assert stale.replanning_attempts == 1
    invalidated = engine.run(stale)
    assert invalidated.status is WorkflowStatus.WAITING_FOR_APPROVAL
    assert invalidated.pending_approval is not None
    assert invalidated.pending_approval.status is ApprovalStatus.INVALIDATED
    assert invalidated.last_error == "state_or_action_digest_changed"


def test_advance_returns_terminal_states_unchanged() -> None:
    engine = WorkflowEngine(_definition())
    state = _state(status=WorkflowStatus.FAILED, current_state="start")
    assert engine._advance(state) is state


def test_resume_returns_unapproved_pending_unchanged() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    assert engine.resume(waiting) == waiting


def test_resume_applies_an_approved_transition() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    approved = engine.approve_checkpoint(waiting)
    resumed = engine.resume(approved)
    assert resumed.status is WorkflowStatus.COMPLETED
    assert resumed.current_state == "done"
    assert resumed.pending_approval is None


def test_resume_invalidates_a_stale_approval() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    approved = engine.approve_checkpoint(waiting)
    tampered = _state(
        status=WorkflowStatus.WAITING_FOR_APPROVAL,
        current_state="start",
        replanning_attempts=1,
        pending_approval=approved.pending_approval,
    )
    resumed = engine.resume(tampered)
    assert resumed.pending_approval is not None
    assert resumed.pending_approval.status is ApprovalStatus.INVALIDATED


def test_resume_advances_when_no_approval_is_pending() -> None:
    engine = WorkflowEngine(_single_step_definition())
    state = engine.start("workflow-1")
    assert engine.resume(state).status is WorkflowStatus.COMPLETED


def test_record_step_result_invalidates_a_stale_approval() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    approved = engine.approve_checkpoint(waiting)
    tampered = _state(
        status=WorkflowStatus.WAITING_FOR_APPROVAL,
        current_state="start",
        replanning_attempts=1,
        pending_approval=approved.pending_approval,
    )
    result = engine.record_step_result(tampered, "step-1", True)
    assert result.pending_approval is not None
    assert result.pending_approval.status is ApprovalStatus.INVALIDATED


def test_record_step_result_consumes_an_approved_approval() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    approved = engine.approve_checkpoint(waiting)
    completed = engine.record_step_result(approved, "step-1", True)
    assert completed.status is WorkflowStatus.COMPLETED


def test_approve_requires_a_pending_checkpoint() -> None:
    engine = WorkflowEngine(_definition())
    state = engine.start("workflow-1")
    with pytest.raises(WorkflowApprovalError, match="no pending checkpoint"):
        engine.approve_checkpoint(state)
    with pytest.raises(WorkflowApprovalError, match="no pending checkpoint"):
        engine.reject_checkpoint(state)
    with pytest.raises(WorkflowApprovalError, match="no pending checkpoint"):
        engine.abort_checkpoint(state)


def test_approve_rejects_a_mismatched_approval_id() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    pending = waiting.pending_approval
    assert pending is not None
    with pytest.raises(WorkflowApprovalError, match="approval_id does not match"):
        engine.approve_checkpoint(waiting, "other-approval")
    with pytest.raises(WorkflowApprovalError, match="approval_id does not match"):
        engine.reject_checkpoint(waiting, "other-approval")
    assert engine.approve_checkpoint(waiting, pending.approval_id).pending_approval is not None


def test_approve_is_idempotent_once_approved() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    approved = engine.approve_checkpoint(waiting)
    assert engine.approve_checkpoint(approved) == approved


def test_approve_refuses_an_already_rejected_checkpoint() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    rejected = engine.reject_checkpoint(waiting)
    assert rejected.status is WorkflowStatus.FAILED
    revived = _state(
        status=WorkflowStatus.WAITING_FOR_APPROVAL,
        current_state="start",
        pending_approval=_replaced_approval(waiting, ApprovalStatus.REJECTED),
    )
    restored = engine.restore(revived)
    assert restored.status is WorkflowStatus.WAITING_FOR_APPROVAL
    with pytest.raises(WorkflowApprovalError, match="already rejected"):
        engine.approve_checkpoint(restored)
    with pytest.raises(WorkflowApprovalError, match="already rejected"):
        engine.reject_checkpoint(restored)


def test_approve_invalidates_on_digest_mismatch() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    invalidated = engine.approve_checkpoint(waiting, state_digest="c" * 64)
    assert invalidated.pending_approval is not None
    assert invalidated.pending_approval.status is ApprovalStatus.INVALIDATED
    assert invalidated.last_error == "state_or_action_digest_changed"


def test_approve_rejects_unsafe_supplied_digest() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    with pytest.raises(WorkflowTransitionError, match="state_digest"):
        engine.approve_checkpoint(waiting, state_digest="bad digest")
    with pytest.raises(WorkflowTransitionError, match="action_digest"):
        engine.approve_checkpoint(waiting, action_digest=cast(str, 7))


def test_approve_records_the_actor_and_time() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    approved = engine.approve_checkpoint(
        waiting,
        state_digest=waiting.pending_approval.state_digest if waiting.pending_approval else "",
        action_digest=waiting.pending_approval.action_digest if waiting.pending_approval else "",
        actor_id="reviewer",
        now=DECIDED_AT,
    )
    assert approved.pending_approval is not None
    assert approved.pending_approval.actor_id == "reviewer"
    assert approved.pending_approval.decided_at == DECIDED_AT
    assert approved.pending_approval.reason is None


def test_approve_rejects_unsafe_actor_and_naive_time() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    with pytest.raises(WorkflowTransitionError, match="actor_id"):
        engine.approve_checkpoint(waiting, actor_id="bad actor")
    with pytest.raises(WorkflowTransitionError, match="must include a timezone"):
        engine.approve_checkpoint(waiting, now=datetime(2026, 1, 1))


def test_approve_emits_a_resolution_event() -> None:
    events: list[WorkflowEvent] = []
    engine = WorkflowEngine(_single_step_definition(checkpoint=True), events.append)
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    events.clear()
    engine.approve_checkpoint(waiting)
    assert [event.event_type for event in events] == ["workflow.approval_resolved"]
    assert events[0].from_status is events[0].to_status


def test_reject_fails_the_workflow_with_a_reason() -> None:
    events: list[WorkflowEvent] = []
    engine = WorkflowEngine(_single_step_definition(checkpoint=True), events.append)
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    rejected = engine.reject_checkpoint(waiting, reason="geometry rejected")
    assert rejected.status is WorkflowStatus.FAILED
    assert rejected.last_error == "geometry rejected"
    assert rejected.pending_approval is None
    assert any(event.event_type == "workflow.approval_resolved" for event in events)


def test_abort_cancels_the_workflow() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    aborted = engine.abort_checkpoint(waiting, reason="operator aborted")
    assert aborted.status is WorkflowStatus.CANCELLED
    assert aborted.last_error == "operator aborted"
    assert aborted.pending_approval is None


def test_resolve_checkpoint_invalidates_on_digest_mismatch() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    invalidated = engine.resolve_checkpoint(waiting, None, "reject", action_digest="d" * 64)
    assert invalidated.pending_approval is not None
    assert invalidated.pending_approval.status is ApprovalStatus.INVALIDATED


def test_resolve_checkpoint_refuses_a_resolved_checkpoint() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    rejected = engine.reject_checkpoint(waiting)
    assert rejected.status is WorkflowStatus.FAILED
    pending = waiting.pending_approval
    assert pending is not None
    resolved = replace(
        pending,
        status=ApprovalStatus.REJECTED,
        actor_id="reviewer",
        decided_at=DECIDED_AT,
    )
    revived = _state(
        status=WorkflowStatus.WAITING_FOR_APPROVAL,
        current_state="start",
        pending_approval=resolved,
    )
    engine.restore(revived)
    with pytest.raises(WorkflowApprovalError, match="already rejected"):
        engine.reject_checkpoint(revived)
    with pytest.raises(WorkflowApprovalError, match="already rejected"):
        engine.approve_checkpoint(revived)


def test_resolve_checkpoint_rejects_a_mismatched_approval_id() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    with pytest.raises(WorkflowApprovalError, match="approval_id does not match"):
        engine.resolve_checkpoint(waiting, "other", "approve")
    with pytest.raises(WorkflowApprovalError, match="approval_id does not match"):
        engine.reject_checkpoint(waiting, "other")
    with pytest.raises(WorkflowApprovalError, match="approval_id does not match"):
        engine.abort_checkpoint(waiting, "other")


def test_decide_checkpoint_accepts_every_spelling() -> None:
    for spelling in ("approve", "approved"):
        fresh = WorkflowEngine(_single_step_definition(checkpoint=True))
        state = fresh.start("workflow-1")
        waiting = fresh.run(state)
        decided = fresh.decide_checkpoint(waiting, None, spelling)
        assert decided.status is WorkflowStatus.WAITING_FOR_APPROVAL
        assert fresh.resume(decided).status is WorkflowStatus.COMPLETED
    for spelling, expected in (
        ("reject", WorkflowStatus.FAILED),
        ("rejected", WorkflowStatus.FAILED),
        ("abort", WorkflowStatus.CANCELLED),
        ("aborted", WorkflowStatus.CANCELLED),
    ):
        fresh = WorkflowEngine(_single_step_definition(checkpoint=True))
        state = fresh.start("workflow-1")
        waiting = fresh.run(state)
        assert fresh.decide_checkpoint(waiting, None, spelling).status is expected


def test_decide_checkpoint_accepts_enum_decisions() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    assert (
        engine.decide_checkpoint(waiting, None, ApprovalStatus.REJECTED).status
        is WorkflowStatus.FAILED
    )


def test_decide_checkpoint_rejects_undecided_decisions() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    with pytest.raises(WorkflowApprovalError, match="must be approve, reject, or abort"):
        engine.decide_checkpoint(state, None, ApprovalStatus.PENDING)
    with pytest.raises(WorkflowApprovalError, match="must be approve, reject, or abort"):
        engine.decide_checkpoint(state, None, ApprovalStatus.PENDING.value)


def test_decide_checkpoint_rejects_unknown_decision_names() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    with pytest.raises(WorkflowTransitionError, match="approval status is invalid"):
        engine.decide_checkpoint(state, None, "maybe")


def test_decide_checkpoint_rejects_non_string_non_enum() -> None:
    engine = WorkflowEngine(_single_step_definition(checkpoint=True))
    state = engine.start("workflow-1")
    with pytest.raises(WorkflowTransitionError, match="approval status is invalid"):
        engine.decide_checkpoint(state, None, cast(Any, 7))


def test_cancel_is_idempotent_and_local() -> None:
    engine = WorkflowEngine(_definition())
    state = engine.start("workflow-1")
    cancelled = engine.cancel(state, reason="stopped")
    assert cancelled.status is WorkflowStatus.CANCELLED
    assert cancelled.last_error == "stopped"
    assert engine.cancel(cancelled) == cancelled


def test_cancel_leaves_completed_and_failed_states_alone() -> None:
    engine = WorkflowEngine(_single_step_definition())
    state = engine.start("workflow-1")
    completed = engine.run(state)
    assert engine.cancel(completed) == completed
    other = WorkflowEngine(_single_step_definition())
    started = other.start("workflow-1")
    failed = other.record_step_failure(started, "step-1", error_code="solver_diverged")
    failed = other._fail_state(failed, "hard failure", "step-1")
    assert failed.status is WorkflowStatus.FAILED
    assert engine.cancel(failed) == failed


def test_cancel_aborts_a_pending_approval() -> None:
    events: list[WorkflowEvent] = []
    engine = WorkflowEngine(_single_step_definition(checkpoint=True), events.append)
    state = engine.start("workflow-1")
    waiting = engine.run(state)
    cancelled = engine.cancel(waiting, reason="operator stopped")
    assert cancelled.status is WorkflowStatus.CANCELLED
    assert cancelled.pending_approval is not None
    assert cancelled.pending_approval.status is ApprovalStatus.ABORTED
    assert cancelled.pending_approval.reason == "operator stopped"
    assert cancelled.pending_approval.actor_id == "local-user"
    assert cancelled.pending_approval.decided_at is not None
    assert [event.event_type for event in events][-1] == "workflow.cancelled"


def test_request_replan_increments_and_clears_errors() -> None:
    events: list[WorkflowEvent] = []
    engine = WorkflowEngine(
        _definition(max_replanning_attempts=2),
        events.append,
        max_attempts={"step-1": 2},
    )
    state = engine.start("workflow-1")
    retried = engine.record_step_failure(state, "step-1", error_code="flaky")
    replanned = engine.request_replan(retried, reason="different approach")
    assert replanned.replanning_attempts == 1
    assert replanned.last_error is None
    assert events[-1].event_type == "workflow.replanning_requested"


def test_request_replan_refuses_past_the_limit() -> None:
    engine = WorkflowEngine(_definition(max_replanning_attempts=1))
    state = engine.start("workflow-1")
    once = engine.request_replan(state)
    with pytest.raises(WorkflowLimitExceeded, match="exhausted"):
        engine.request_replan(once)


def test_request_replan_returns_terminal_states_unchanged() -> None:
    engine = WorkflowEngine(_definition())
    cancelled = _state(status=WorkflowStatus.CANCELLED)
    assert engine.request_replan(cancelled) is cancelled


def test_serialize_and_snapshot_round_trip() -> None:
    engine = WorkflowEngine(_definition())
    state = engine.start("workflow-1")
    assert engine.serialize(state) == state.to_json()
    assert engine.snapshot(state) is state


def test_restore_accepts_every_state_representation() -> None:
    engine = WorkflowEngine(_definition())
    state = engine.start("workflow-1")
    assert engine.restore(state) is state
    assert engine.restore(state.to_json()) == state
    assert engine.restore(cast(Mapping[str, object], state.to_dict())) == state
    assert engine.state == state


def test_restore_rejects_unsupported_types() -> None:
    engine = WorkflowEngine(_definition())
    with pytest.raises(TypeError, match="WorkflowState, mapping, or JSON string"):
        engine.restore(cast(Any, 7))


def test_restore_rejects_mismatched_definition() -> None:
    engine = WorkflowEngine(_definition())
    with pytest.raises(WorkflowTransitionError, match="definition_id does not match"):
        engine.restore(_state(definition_id="other"))
    with pytest.raises(WorkflowTransitionError, match="definition version does not match"):
        engine.restore(_state(definition_version="9.9.9"))
    with pytest.raises(WorkflowTransitionError, match="current_state is not declared"):
        engine.restore(_state(current_state="nowhere"))


def test_restore_rejects_malformed_json() -> None:
    engine = WorkflowEngine(_definition())
    with pytest.raises(WorkflowTransitionError, match="state is invalid"):
        engine.restore("{")


def test_state_digest_tracks_meaningful_state_only() -> None:
    engine = WorkflowEngine(_definition())
    state = engine.start("workflow-1")
    baseline = engine.state_digest(state)
    assert baseline == engine._state_digest(state)
    advanced = engine.run(state)
    assert engine.state_digest(advanced) != baseline
    twin = WorkflowEngine(_definition()).start("workflow-1")
    assert engine.state_digest(twin) == baseline


def test_action_digest_binds_the_transition() -> None:
    engine = WorkflowEngine(_definition())
    first, second = _definition().transitions
    assert engine._action_digest(first, "s") != engine._action_digest(second, "s")
    assert engine._action_digest(first, "s1") != engine._action_digest(first, "s2")


def test_approval_matches_rejects_unknown_transitions() -> None:
    engine = WorkflowEngine(_definition())
    state = _state(current_state="start")
    approval = _approval(transition_id="ghost")
    assert engine._approval_matches(state, approval) is False


def test_approval_matches_rejects_wrong_origin_state() -> None:
    engine = WorkflowEngine(_definition())
    state = _state(current_state="done")
    approval = _approval(transition_id="transition-1")
    assert engine._approval_matches(state, approval) is False


def test_approval_matches_rejects_stale_digests() -> None:
    engine = WorkflowEngine(_definition())
    state = _state(current_state="start")
    approval = _approval(transition_id="transition-1", state_digest="e" * 64)
    assert engine._approval_matches(state, approval) is False
    approval = _approval(transition_id="transition-1", action_digest="f" * 64)
    assert engine._approval_matches(state, approval) is False


def test_approval_matches_accepts_a_fresh_pair() -> None:
    engine = WorkflowEngine(_definition())
    state = _state(current_state="start")
    state_digest = engine._state_digest(state)
    transition = _definition().transitions[0]
    approval = _approval(
        transition_id="transition-1",
        state_digest=state_digest,
        action_digest=engine._action_digest(transition, state_digest),
    )
    assert engine._approval_matches(state, approval) is True


def test_invalidate_approval_without_pending_is_a_noop() -> None:
    engine = WorkflowEngine(_definition())
    state = _state()
    assert engine._invalidate_approval(state, "reason") is state


def test_invalidate_approval_keeps_existing_actor_and_time() -> None:
    engine = WorkflowEngine(_definition())
    state = _state(
        status=WorkflowStatus.WAITING_FOR_APPROVAL,
        pending_approval=_approval(
            status=ApprovalStatus.INVALIDATED,
            actor_id="reviewer",
            decided_at=DECIDED_AT,
        ),
    )
    invalidated = engine._invalidate_approval(state, "stale")
    assert invalidated.pending_approval is not None
    assert invalidated.pending_approval.actor_id == "reviewer"
    assert invalidated.pending_approval.decided_at == DECIDED_AT
    assert invalidated.pending_approval.reason == "stale"
    assert invalidated.status is WorkflowStatus.WAITING_FOR_APPROVAL


def test_invalidate_approval_fills_in_actor_and_time() -> None:
    engine = WorkflowEngine(_definition())
    state = _state(
        status=WorkflowStatus.WAITING_FOR_APPROVAL,
        pending_approval=_approval(),
    )
    invalidated = engine._invalidate_approval(state, "stale")
    assert invalidated.pending_approval is not None
    assert invalidated.pending_approval.actor_id == "local-user"
    assert invalidated.pending_approval.decided_at is not None
    assert invalidated.last_error == "stale"


def test_invalidate_approval_emits_an_event() -> None:
    events: list[WorkflowEvent] = []
    engine = WorkflowEngine(_definition(), events.append)
    state = _state(status=WorkflowStatus.WAITING_FOR_APPROVAL, pending_approval=_approval())
    engine._invalidate_approval(state, "stale")
    assert events[-1].event_type == "workflow.approval_invalidated"


def test_make_approval_requires_a_checkpoint_type() -> None:
    engine = WorkflowEngine(_definition())
    state = _state(current_state="start")
    with pytest.raises(WorkflowTransitionError, match="does not declare a checkpoint type"):
        engine._make_approval(state, _definition().transitions[0])


def test_get_transition_reports_undeclared_and_unsafe_ids() -> None:
    engine = WorkflowEngine(_definition())
    assert engine._get_transition("transition-1").step_id == "step-1"
    with pytest.raises(WorkflowTransitionError, match="not declared by the workflow"):
        engine._get_transition("ghost")
    with pytest.raises(WorkflowTransitionError, match="must be a string"):
        engine._get_transition(cast(str, 7))


def test_select_transition_returns_none_without_outgoing() -> None:
    engine = WorkflowEngine(_definition())
    state = _state(current_state="done")
    assert engine._select_transition(state, None) is None


def test_outgoing_lists_declared_transitions() -> None:
    engine = WorkflowEngine(_definition())
    assert len(engine._outgoing("start")) == 1
    assert engine._outgoing("nowhere") == ()


def test_attempt_limit_defaults_to_one() -> None:
    engine = WorkflowEngine(_definition())
    assert engine._attempt_limit("step-1") == 1


def test_emit_requires_a_callable_or_append() -> None:
    with pytest.raises(TypeError, match="callable or expose append"):
        WorkflowEngine(_definition(), object()).start("workflow-1")


def test_emit_supports_two_argument_callables() -> None:
    received: list[tuple[str, dict[str, object]]] = []

    def journal(event_type: str, payload: dict[str, object]) -> None:
        received.append((event_type, payload))

    WorkflowEngine(_definition(), journal).start("workflow-1")
    assert received[0][0] == "workflow.created"
    assert received[0][1]["workflow_id"] == "workflow-1"


def test_emit_supports_three_argument_callables() -> None:
    received: list[tuple[str, str, dict[str, object]]] = []

    def journal(event_type: str, workflow_id: str, payload: dict[str, object]) -> None:
        received.append((event_type, workflow_id, payload))

    WorkflowEngine(_definition(), journal).start("workflow-1")
    assert received[0][0] == "workflow.created"
    assert received[0][1] == "workflow-1"


def test_emit_supports_var_positional_callables() -> None:
    received: list[tuple[object, ...]] = []

    def journal(*args: object) -> None:
        received.append(args)

    WorkflowEngine(_definition(), journal).start("workflow-1")
    assert received[0][0] == "workflow.created"


def test_emit_falls_back_for_uninspectable_callables() -> None:
    import functools

    received: list[WorkflowEvent] = []

    @functools.partial
    def journal(event: WorkflowEvent) -> None:
        received.append(event)

    engine = WorkflowEngine(_definition(), journal)
    engine.start("workflow-1")
    assert [event.event_type for event in received] == ["workflow.created"]


def test_emit_supports_append_only_journals() -> None:
    journal: list[WorkflowEvent] = []
    WorkflowEngine(_definition(), journal).start("workflow-1")
    assert [event.event_type for event in journal] == ["workflow.created"]


def test_normalize_now_requires_a_timezone() -> None:
    engine = WorkflowEngine(_definition())
    assert engine._normalize_now(None).tzinfo is UTC
    assert engine._normalize_now(DECIDED_AT) == DECIDED_AT
    with pytest.raises(WorkflowTransitionError, match="must include a timezone"):
        engine._normalize_now(datetime(2026, 1, 1))


def test_complete_if_terminal_keeps_terminal_states() -> None:
    engine = WorkflowEngine(_definition())
    for status in (
        WorkflowStatus.COMPLETED,
        WorkflowStatus.FAILED,
        WorkflowStatus.CANCELLED,
    ):
        state = _state(status=status)
        assert engine._complete_if_terminal(state) is state


def test_complete_if_terminal_completes_running_workflows() -> None:
    events: list[WorkflowEvent] = []
    engine = WorkflowEngine(_definition(), events.append)
    completed = engine._complete_if_terminal(_state())
    assert completed.status is WorkflowStatus.COMPLETED
    assert completed.last_error is None
    assert events[-1].event_type == "workflow.completed"


def test_record_failure_fails_immediately_past_the_bound() -> None:
    engine = WorkflowEngine(_definition(), max_attempts={"step-1": 1})
    state = _state(
        attempts=(StepAttempt(step_id="step-1", attempt=1, succeeded=False),),
    )
    failed = engine._record_failure(state, _definition().transitions[0], "flaky")
    assert failed.status is WorkflowStatus.FAILED
    assert failed.last_error == "step_retry_bound_exhausted"


def test_record_failure_fails_on_the_last_attempt() -> None:
    events: list[WorkflowEvent] = []
    engine = WorkflowEngine(_definition(), events.append, max_attempts={"step-1": 2})
    state = _state()
    retried = engine._record_failure(state, _definition().transitions[0], "flaky")
    assert retried.status is WorkflowStatus.RUNNING
    assert events[-1].event_type == "workflow.step_retry"
    final = engine._record_failure(retried, _definition().transitions[0], "flaky")
    assert final.status is WorkflowStatus.FAILED
    assert final.last_error == "flaky"


def test_record_failure_defaults_the_error_code() -> None:
    engine = WorkflowEngine(_definition(), max_attempts={"step-1": 1})
    failed = engine._record_failure(_state(), _definition().transitions[0], None)
    assert failed.last_error == "step_failed"
    assert failed.attempts[-1].error_code is None


def test_fail_state_emits_an_event_with_the_reason() -> None:
    events: list[WorkflowEvent] = []
    engine = WorkflowEngine(_definition(), events.append)
    failed = engine._fail_state(_state(), "solver diverged", "step-1")
    assert failed.status is WorkflowStatus.FAILED
    assert failed.last_error == "solver diverged"
    assert events[-1].event_type == "workflow.failed"
    assert events[-1].step_id == "step-1"
    assert events[-1].reason == "solver diverged"
