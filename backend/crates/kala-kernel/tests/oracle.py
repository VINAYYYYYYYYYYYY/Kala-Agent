"""Test-only live Python oracle for the native kernel differential tests.

Usage: python oracle.py <mode> < request.json > response.json

This script is never imported by product code. It executes the same operation
sequences against the repository's Python reference implementation and prints
a deterministic JSON document for byte/value comparison.
"""

from __future__ import annotations

import json
import sys
import traceback
from datetime import UTC, datetime
from typing import Any

from kala_agent.contracts import (
    ApprovalKind,
    ApprovalStatus,
    PlanStep,
    SideEffectLevel,
    WorkflowDefinition,
    WorkflowStatus,
    WorkflowTransition,
)
from kala_agent.kernel import (
    PlanValidationError,
    PlanValidator,
    PolicyEngine,
    PolicyRequest,
    WorkflowEngine,
    WorkflowState,
)


def _dt(value: str | None) -> datetime | None:
    if value is None:
        return None
    return datetime.fromisoformat(value)


def _error(exc: BaseException) -> dict[str, object]:
    return {"error": type(exc).__name__, "message": str(exc)}


def _request(payload: dict[str, object]) -> PolicyRequest:
    checkpoint = payload.get("checkpoint_type")
    return PolicyRequest(
        request_id=payload["request_id"],
        state_digest=payload["state_digest"],
        action_digest=payload["action_digest"],
        requested_capabilities=tuple(payload.get("requested_capabilities", ())),
        side_effect_level=SideEffectLevel(payload.get("side_effect_level", "none")),
        destructive=bool(payload.get("destructive", False)),
        egress=bool(payload.get("egress", False)),
        scope=payload.get("scope"),
        checkpoint_type=None if checkpoint is None else ApprovalKind(checkpoint),
    )


def _grant(payload: object):
    from kala_agent.kernel import CapabilityGrant

    if isinstance(payload, str):
        return CapabilityGrant(payload)
    return CapabilityGrant(
        capability=payload["capability"],
        scope=payload.get("scope"),
        expires_at=_dt(payload.get("expires_at")),
    )


def policy_mode(document: dict[str, object]) -> dict[str, object]:
    engine = PolicyEngine(
        [_grant(item) for item in document.get("grants", ())],
        now=_dt(document.get("now")),
        max_tracked_requests=document.get("max_tracked_requests", 512),
    )
    results: list[object] = []
    for operation in document.get("ops", ()):
        name = operation["op"]
        try:
            if name in {"decide", "request_approval"}:
                request = _request(operation["request"])
                now = _dt(operation.get("now"))
                if name == "decide":
                    result = engine.decide(request, now=now).as_dict()
                else:
                    result = engine.request_approval(request, now=now).as_dict()
            elif name in {"approve", "reject", "abort", "invalidate"}:
                target = (
                    _request(operation["request"])
                    if "request" in operation
                    else operation["request_id"]
                )
                kwargs: dict[str, Any] = {
                    "state_digest": operation.get("state_digest"),
                    "action_digest": operation.get("action_digest"),
                    "actor_id": operation.get("actor_id", "local-user"),
                    "now": _dt(operation.get("now")),
                }
                if name in {"reject", "abort"}:
                    kwargs["reason"] = operation.get("reason")
                result = getattr(engine, name)(target, **kwargs).as_dict()
            elif name == "get_approval":
                record = engine.get_approval(
                    operation["request_id"],
                    state_digest=operation.get("state_digest"),
                    action_digest=operation.get("action_digest"),
                )
                result = None if record is None else record.as_dict()
            elif name == "forget_tool_class":
                result = engine.forget_tool_class(
                    operation["action_digest"], scope=operation.get("scope")
                )
            elif name == "grant":
                result = engine.grant(_grant(operation["grant"])).as_dict()
            elif name == "revoke":
                result = engine.revoke(operation["capability"])
            elif name == "export_state":
                result = engine.export_state()
            elif name == "import_state":
                engine.import_state(operation["state"])
                result = None
            elif name == "clock":
                result = None if engine.clock is None else engine.clock.isoformat()
            elif name == "grants":
                result = sorted(engine.grants)
            elif name == "max_tracked_requests":
                result = engine.max_tracked_requests
            elif name == "tracked_request_ids":
                result = list(engine.tracked_request_ids)
            elif name == "approvals":
                result = [record.as_dict() for record in engine.approvals]
            elif name == "approved_tool_classes":
                result = list(engine.approved_tool_classes)
            else:
                raise AssertionError(f"unknown policy op {name}")
        except BaseException as exc:  # the oracle must report every Python class
            result = _error(exc)
        results.append(result)
    return {"results": results}


def _transition(payload: dict[str, object]) -> WorkflowTransition:
    return WorkflowTransition(
        transition_id=payload["transition_id"],
        from_state=payload["from_state"],
        to_state=payload["to_state"],
        step_id=payload["step_id"],
        checkpoint_type=payload.get("checkpoint_type"),
    )


def _definition(payload: dict[str, object]) -> WorkflowDefinition:
    return WorkflowDefinition(
        definition_id=payload["definition_id"],
        version=payload["version"],
        name=payload["name"],
        description=payload.get("description"),
        initial_state=payload["initial_state"],
        states=tuple(payload["states"]),
        transitions=tuple(_transition(item) for item in payload.get("transitions", ())),
        available_step_ids=tuple(payload["available_step_ids"]),
        max_replanning_attempts=payload.get("max_replanning_attempts", 1),
    )


def _workflow_state(payload: object) -> WorkflowState:
    if isinstance(payload, str):
        return WorkflowState.from_json(payload)
    return WorkflowState.from_dict(payload)


def workflow_mode(document: dict[str, object]) -> dict[str, object]:
    definition = _definition(document["definition"])
    events: list[dict[str, object]] = []

    def journal(event) -> None:
        events.append(event.to_dict())

    clock = _dt(document.get("clock"))
    if clock is None:
        engine = WorkflowEngine(
            definition,
            journal,
            max_attempts={
                key: value for key, value in document.get("max_attempts", {}).items()
            },
        )
    else:
        # The Python engine has no injectable clock; `_normalize_now` is patched
        # through a subclass for deterministic checkpoint/cancel timestamps.
        class FixedClockWorkflowEngine(WorkflowEngine):
            def _normalize_now(self, value):  # noqa: ANN001
                return clock if value is None else super()._normalize_now(value)

        engine = FixedClockWorkflowEngine(
            definition,
            journal,
            max_attempts={
                key: value for key, value in document.get("max_attempts", {}).items()
            },
        )

    results: list[object] = []
    event_cursor = 0
    for operation in document.get("ops", ()):
        name = operation["op"]
        try:
            state = (
                _workflow_state(operation["state"])
                if "state" in operation and operation["state"] is not None
                else None
            )
            if name == "start":
                result_state = engine.start(
                    operation.get("workflow_id", "workflow-1"),
                    replanning_attempts=operation.get("replanning_attempts", 0),
                )
            elif name == "run":
                result_state = engine.run(
                    state,
                    transition_id=operation.get("transition_id"),
                    step_id=operation.get("step_id"),
                    succeeded=operation.get("succeeded"),
                )
            elif name == "record_step_result":
                result_state = engine.record_step_result(
                    state if state is not None else engine.state,
                    operation["step_id"],
                    operation["succeeded"],
                    transition_id=operation.get("transition_id"),
                    error_code=operation.get("error_code"),
                )
            elif name == "record_step_success":
                result_state = engine.record_step_success(
                    state if state is not None else engine.state,
                    operation["step_id"],
                    transition_id=operation.get("transition_id"),
                )
            elif name == "record_step_failure":
                result_state = engine.record_step_failure(
                    state if state is not None else engine.state,
                    operation["step_id"],
                    error_code=operation.get("error_code", "step_failed"),
                    transition_id=operation.get("transition_id"),
                )
            elif name == "resume":
                result_state = engine.resume(state)
            elif name == "approve_checkpoint":
                result_state = engine.approve_checkpoint(
                    state,
                    operation.get("approval_id"),
                    state_digest=operation.get("state_digest"),
                    action_digest=operation.get("action_digest"),
                    actor_id=operation.get("actor_id", "local-user"),
                    now=_dt(operation.get("now")),
                )
            elif name == "reject_checkpoint":
                result_state = engine.reject_checkpoint(
                    state,
                    operation.get("approval_id"),
                    state_digest=operation.get("state_digest"),
                    action_digest=operation.get("action_digest"),
                    actor_id=operation.get("actor_id", "local-user"),
                    reason=operation.get("reason", "checkpoint rejected"),
                    now=_dt(operation.get("now")),
                )
            elif name == "abort_checkpoint":
                result_state = engine.abort_checkpoint(
                    state,
                    operation.get("approval_id"),
                    state_digest=operation.get("state_digest"),
                    action_digest=operation.get("action_digest"),
                    actor_id=operation.get("actor_id", "local-user"),
                    reason=operation.get("reason", "checkpoint aborted"),
                    now=_dt(operation.get("now")),
                )
            elif name == "resolve_checkpoint":
                result_state = engine.resolve_checkpoint(
                    state,
                    operation.get("approval_id"),
                    operation.get("decision", "approved"),
                    state_digest=operation.get("state_digest"),
                    action_digest=operation.get("action_digest"),
                    actor_id=operation.get("actor_id", "local-user"),
                    reason=operation.get("reason", "checkpoint rejected"),
                    now=_dt(operation.get("now")),
                )
            elif name == "decide_checkpoint":
                result_state = engine.decide_checkpoint(
                    state,
                    operation.get("approval_id"),
                    operation.get("decision", "approved"),
                    state_digest=operation.get("state_digest"),
                    action_digest=operation.get("action_digest"),
                    actor_id=operation.get("actor_id", "local-user"),
                    reason=operation.get("reason", "checkpoint rejected"),
                    now=_dt(operation.get("now")),
                )
            elif name == "cancel":
                result_state = engine.cancel(
                    state, reason=operation.get("reason", "workflow cancelled")
                )
            elif name == "request_replan":
                result_state = engine.request_replan(
                    state, reason=operation.get("reason", "replan requested")
                )
            elif name == "serialize":
                result = engine.serialize(state)
                results.append(
                    {
                        "result": result,
                        "state": engine.state.to_dict() if engine.state else None,
                    }
                )
                event_cursor = _append_events(events, event_cursor, results)
                continue
            elif name == "snapshot":
                result_state = engine.snapshot(state)
            elif name == "state_digest":
                result = engine.state_digest(state)
                results.append(
                    {
                        "result": result,
                        "state": engine.state.to_dict() if engine.state else None,
                    }
                )
                event_cursor = _append_events(events, event_cursor, results)
                continue
            elif name == "restore":
                result_state = engine.restore(operation["state"])
            elif name == "state":
                result_state = engine.state
            elif name == "set_state":
                engine._state = _workflow_state(operation["state"])  # oracle only
                result_state = engine.state
            else:
                raise AssertionError(f"unknown workflow op {name}")
        except BaseException as exc:
            result = _error(exc)
            results.append(
                {
                    "result": result,
                    "state": engine.state.to_dict() if engine.state else None,
                    "state_json": engine.state.to_json() if engine.state else None,
                }
            )
            event_cursor = _append_events(events, event_cursor, results)
            continue
        results.append(
            {
                "result": result_state.to_dict() if result_state is not None else None,
                "result_json": result_state.to_json() if result_state is not None else None,
                "state": engine.state.to_dict() if engine.state else None,
                "state_json": engine.state.to_json() if engine.state else None,
            }
        )
        event_cursor = _append_events(events, event_cursor, results)
    return {"results": results, "events": events}


def _append_events(
    events: list[dict[str, object]], cursor: int, results: list[object]
) -> int:
    results[-1]["events_since"] = events[cursor:]
    return len(events)


def planning_mode(document: dict[str, object]) -> dict[str, object]:
    definition = _definition(document["definition"])
    validator = PlanValidator(
        definition,
        allowed_capabilities=document.get("allowed_capabilities"),
        granted_capabilities=document.get("granted_capabilities"),
        max_steps=document.get("max_steps", 256),
        max_attempts=document.get("max_attempts", 1),
        max_total_attempts=document.get("max_total_attempts"),
    )
    results: list[object] = []
    for case in document.get("cases", ()):
        try:
            steps = [
                PlanStep.model_validate(item) if not isinstance(item, PlanStep) else item
                for item in case["steps"]
            ]
            outcome = validator.validate(
                steps,
                allowed_capabilities=case.get("allowed_capabilities"),
                granted_capabilities=case.get("granted_capabilities"),
                replanning_attempts=case.get("replanning_attempts", 0),
            )
        except BaseException as exc:
            results.append(_error(exc))
            continue
        if isinstance(outcome, PlanValidationError):
            results.append(outcome.as_dict())
        else:
            results.append(
                {
                    "plan": {
                        "workflow_definition_id": outcome.workflow_definition_id,
                        "steps": [step.model_dump(mode="json") for step in outcome.steps],
                        "required_capabilities": list(outcome.required_capabilities),
                        "max_attempts": outcome.max_attempts,
                    }
                }
            )
    return {"results": results}


def main() -> None:
    mode = sys.argv[1]
    document = json.load(sys.stdin)
    try:
        if mode == "policy":
            output = policy_mode(document)
        elif mode == "workflow":
            output = workflow_mode(document)
        elif mode == "planning":
            output = planning_mode(document)
        else:
            raise AssertionError(f"unknown mode {mode}")
    except BaseException:
        traceback.print_exc()
        raise
    json.dump(output, sys.stdout, ensure_ascii=True, sort_keys=True, separators=(",", ":"))


if __name__ == "__main__":
    main()
