from __future__ import annotations

import ast
import json
from dataclasses import FrozenInstanceError
from datetime import UTC, datetime, timedelta
from decimal import Decimal
from pathlib import Path
from typing import Any

import pytest

from kala_agent.contracts import (
    ApprovalKind,
    PlanStep,
    RunBudget,
    WorkflowDefinition,
    WorkflowStatus,
    WorkflowTransition,
)
from kala_agent.kernel import (
    BudgetCounterError,
    BudgetExhaustedError,
    BudgetLedger,
    BudgetReservation,
    BudgetReservationMismatchError,
    BudgetReservationUnknownError,
    BudgetStateError,
    BudgetUsage,
    MemoryConflictError,
    MemoryManager,
    PlanRejectedError,
    PlanValidationCode,
    PlanValidationError,
    PlanValidator,
    WorkflowEngine,
    WorkflowTransitionError,
)
from kala_agent.persistence import Database, MemoryRecord, MemoryRepository

RECORDED_AT = datetime(2026, 1, 2, 3, 4, 5, tzinfo=UTC)
KERNEL_ROOT = Path(__file__).parents[2] / "src" / "kala_agent" / "kernel"


def _workflow(*, checkpoint: bool = False) -> WorkflowDefinition:
    transition = WorkflowTransition(
        transition_id="transition-1",
        from_state="start",
        to_state="done",
        step_id="step-1",
        checkpoint_type=ApprovalKind.PRE_COMMIT if checkpoint else None,
    )
    return WorkflowDefinition(
        definition_id="workflow-1",
        version="1.0.0",
        name="Test workflow",
        initial_state="start",
        states=("start", "done"),
        transitions=(transition,),
        available_step_ids=("step-1",),
        max_replanning_attempts=1,
    )


def _step(
    step_id: str,
    catalog_step_id: str = "step-1",
    *,
    depends_on: tuple[str, ...] = (),
    capabilities: tuple[str, ...] = (),
    max_attempts: int = 1,
) -> PlanStep:
    return PlanStep(
        step_id=step_id,
        catalog_step_id=catalog_step_id,
        intent="Perform a declared step.",
        required_capabilities=capabilities,
        depends_on=depends_on,
        max_attempts=max_attempts,
    )


def _memory_record(
    memory_id: str,
    content: str,
    *,
    source: str = "source-1",
    created_at: datetime = RECORDED_AT,
    expires_at: datetime | None = None,
) -> MemoryRecord:
    return MemoryRecord(
        memory_id=memory_id,
        content=content,
        source=source,
        provenance_id=f"provenance-{memory_id}",
        created_at=created_at,
        expires_at=expires_at,
    )


def test_budget_ledger_rejects_exhaustion_without_negative_counters() -> None:
    ledger = BudgetLedger(
        RunBudget(
            max_model_calls=1,
            max_tool_calls=1,
            max_total_tokens=10,
            max_artifact_bytes=10,
            max_duration_seconds=2.0,
            max_replanning_attempts=0,
        )
    )

    ledger.record_model_call(tokens=4, duration_seconds=0.5)
    with pytest.raises(BudgetExhaustedError) as error:
        ledger.record_model_call(tokens=1)

    assert error.value.dimension.value == "model_calls"
    assert ledger.usage.model_calls == 1
    assert ledger.available.model_calls == 0
    assert ledger.available.total_tokens == 6
    assert all(
        value >= 0
        for value in (
            ledger.available.model_calls,
            ledger.available.tool_calls,
            ledger.available.total_tokens,
            ledger.available.artifact_bytes,
            ledger.available.duration_seconds,
            ledger.available.replanning_attempts,
        )
    )


def test_budget_ledger_charge_is_atomic_and_reservations_commit_once() -> None:
    ledger = BudgetLedger(RunBudget(max_tool_calls=1, max_total_tokens=10))
    before = ledger.snapshot()

    with pytest.raises(BudgetExhaustedError):
        ledger.consume(tool_calls=1, total_tokens=11)

    assert ledger.snapshot() == before
    reservation = ledger.reserve(tool_calls=1, total_tokens=4)
    assert ledger.reserved.tool_calls == 1
    ledger.commit(reservation)
    assert ledger.usage.tool_calls == 1
    assert ledger.usage.total_tokens == 4
    assert ledger.reserved.tool_calls == 0


def test_plan_validator_returns_structured_cycle_unknown_and_capability_errors() -> None:
    definition = _workflow()
    validator = PlanValidator(
        definition,
        allowed_capabilities={"read"},
        max_steps=2,
        max_attempts=2,
    )

    cycle = validator.validate(
        (
            _step("first", depends_on=("second",)),
            _step("second", depends_on=("first",)),
        )
    )
    assert isinstance(cycle, PlanValidationError)
    assert cycle.code is PlanValidationCode.DEPENDENCY_CYCLE

    unknown = validator.validate((_step("first", catalog_step_id="not-in-catalog"),))
    assert isinstance(unknown, PlanValidationError)
    assert unknown.code is PlanValidationCode.UNKNOWN_CATALOG_STEP

    over_capability = validator.validate((_step("first", capabilities=("write",)),))
    assert isinstance(over_capability, PlanValidationError)
    assert over_capability.code is PlanValidationCode.CAPABILITY_NOT_GRANTED
    assert over_capability.as_dict()["code"] == "capability_not_granted"

    over_attempts = validator.validate((_step("first", max_attempts=3),))
    assert isinstance(over_attempts, PlanValidationError)
    assert over_attempts.code is PlanValidationCode.TOO_MANY_ATTEMPTS

    normalized = validator.validate((_step("first", capabilities=("read",)),))
    assert not isinstance(normalized, PlanValidationError)
    assert normalized.required_capabilities == ("read",)
    normalized_field = "workflow_definition_id"
    with pytest.raises(FrozenInstanceError):
        setattr(normalized, normalized_field, "changed")


def test_memory_manager_requires_explicit_promotion_and_detects_conflicts_and_expiry() -> None:
    with Database() as database:
        repository = MemoryRepository(database)
        manager = MemoryManager(repository)
        expiring = _memory_record(
            "memory-1",
            "A bounded note.",
            expires_at=RECORDED_AT + timedelta(minutes=1),
        )
        manager.add(expiring, now=RECORDED_AT)
        assert repository.list(now=RECORDED_AT) == ()
        assert manager.list(now=RECORDED_AT) == (expiring,)

        promoted = manager.promote("memory-1", now=RECORDED_AT)
        assert promoted.provenance_id == "provenance-memory-1"
        assert repository.get("memory-1", now=RECORDED_AT) == promoted

        with pytest.raises(MemoryConflictError):
            manager.add(
                _memory_record("memory-2", "A contradictory note."),
                now=RECORDED_AT,
            )

        assert manager.list(now=RECORDED_AT + timedelta(minutes=1)) == ()
        replacement = _memory_record(
            "memory-3",
            "A replacement after expiry.",
            expires_at=RECORDED_AT + timedelta(minutes=2),
        )
        manager.add(replacement, now=RECORDED_AT + timedelta(minutes=1))
        assert manager.get("memory-3", now=RECORDED_AT + timedelta(minutes=1)) == replacement


def test_workflow_stops_at_checkpoint_and_resumes_from_serialized_state() -> None:
    events: list[object] = []
    definition = _workflow(checkpoint=True)
    engine = WorkflowEngine(definition, events.append)
    created = engine.start("workflow-1")
    waiting = engine.run(created)

    assert waiting.status is WorkflowStatus.WAITING_FOR_APPROVAL
    assert waiting.pending_approval is not None
    assert waiting.current_state == "start"
    assert any(getattr(event, "event_type", "") == "workflow.checkpoint" for event in events)

    serialized = waiting.to_json()
    resumed_engine = WorkflowEngine(definition, events.append)
    restored = resumed_engine.restore(serialized)
    assert restored == waiting

    approved = resumed_engine.approve_checkpoint(restored)
    completed = resumed_engine.resume(approved)
    assert completed.status is WorkflowStatus.COMPLETED
    assert completed.current_state == "done"
    assert completed.history[0].transition_id == "transition-1"


def test_workflow_cancellation_and_bounded_retries_are_terminal_and_local() -> None:
    definition = _workflow()
    events: list[object] = []
    engine = WorkflowEngine(definition, events.append, max_attempts={"step-1": 2})
    state = engine.start("workflow-1")

    state = engine.run(state, step_id="step-1", succeeded=False)
    assert state.status is WorkflowStatus.RUNNING
    state = engine.run(state, step_id="step-1", succeeded=False)
    assert state.status is WorkflowStatus.FAILED
    assert state.last_error == "step_failed"

    retry_engine = WorkflowEngine(definition, events.append)
    cancellable = retry_engine.start("workflow-2")
    cancelled = retry_engine.cancel(cancellable, reason="user requested")
    assert cancelled.status is WorkflowStatus.CANCELLED
    assert retry_engine.run(cancelled) == cancelled

    with pytest.raises(WorkflowTransitionError):
        retry_engine.run(cancellable, transition_id="not-declared")


def test_kernel_imports_do_not_depend_on_domain_runtime_or_tool_packages() -> None:
    forbidden_prefixes = (
        "kala_agent.domains",
        "kala_agent.runtime",
        "kala_agent.tools",
    )
    for path in KERNEL_ROOT.glob("*.py"):
        tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                names = [alias.name for alias in node.names]
            elif isinstance(node, ast.ImportFrom):
                names = [node.module or ""]
            else:
                continue
            assert all(not name.startswith(forbidden_prefixes) for name in names), path


def test_plan_validator_treats_an_explicit_empty_allow_list_as_deny_all() -> None:
    definition = _workflow()
    reading = _step("first", capabilities=("read",))
    plain = _step("first")

    unrestricted = PlanValidator(definition)
    assert unrestricted.allowed_capabilities is None
    assert unrestricted.unrestricted
    assert not isinstance(unrestricted.validate((reading,)), PlanValidationError)

    denied_all = PlanValidator(definition, allowed_capabilities=())
    assert denied_all.allowed_capabilities == frozenset()
    assert not denied_all.unrestricted

    empty = denied_all.validate((reading,))
    assert isinstance(empty, PlanValidationError)
    assert empty.code is PlanValidationCode.CAPABILITY_NOT_GRANTED
    assert empty.details == (("capability", "read"),)
    assert not isinstance(denied_all.validate((plain,)), PlanValidationError)

    narrowed = PlanValidator(definition, allowed_capabilities={"read"})
    assert narrowed.allowed_capabilities == frozenset({"read"})
    assert not isinstance(narrowed.validate((reading,)), PlanValidationError)

    intersected = PlanValidator(
        definition,
        allowed_capabilities={"read"},
        granted_capabilities={"write"},
    )
    assert intersected.allowed_capabilities == frozenset()
    disjoint = intersected.validate((reading,))
    assert isinstance(disjoint, PlanValidationError)
    assert disjoint.code is PlanValidationCode.CAPABILITY_NOT_GRANTED

    unioned = PlanValidator(
        definition, allowed_capabilities={"read"}, granted_capabilities={"read"}
    )
    assert unioned.allowed_capabilities == frozenset({"read"})
    assert not isinstance(unioned.validate((reading,)), PlanValidationError)

    per_call = PlanValidator(definition, allowed_capabilities={"read", "write"})
    assert isinstance(
        per_call.validate((reading,), granted_capabilities=()),
        PlanValidationError,
    )
    assert not isinstance(
        per_call.validate((reading,), granted_capabilities={"read"}),
        PlanValidationError,
    )
    assert not isinstance(
        per_call.validate((reading,), granted_capabilities=("read",), replanning_attempts=0),
        PlanValidationError,
    )
    assert isinstance(
        per_call.validate((reading,), replanning_attempts=2),
        PlanValidationError,
    )
    with pytest.raises(PlanRejectedError):
        per_call.validate_or_raise((reading,), granted_capabilities=())
    assert per_call.is_valid((reading,), granted_capabilities={"read"})


def test_budget_ledger_state_round_trip_restores_usage_and_reservations() -> None:
    ledger = BudgetLedger(RunBudget(max_tool_calls=4, max_total_tokens=100))
    committed = ledger.reserve(tool_calls=1, total_tokens=10)
    ledger.commit(committed, tool_calls=1, total_tokens=10)
    ledger.record_model_call(tokens=5, duration_seconds=0.25)
    outstanding = ledger.reserve(tool_calls=1, total_tokens=20)
    before = ledger.snapshot()

    state = ledger.export_state()
    assert json.loads(json.dumps(state)) == state

    restored = BudgetLedger()
    restored.import_state(state)

    assert restored.budget.max_tool_calls == 4
    assert restored.budget.max_total_tokens == 100
    assert restored.snapshot() == before
    assert restored.reservations() == (outstanding,)
    assert restored.reserved.total_tokens == 20

    with pytest.raises(BudgetReservationUnknownError):
        restored.commit(committed)
    with pytest.raises(BudgetReservationMismatchError):
        restored.commit(
            BudgetReservation(
                reservation_id=outstanding.reservation_id,
                usage=BudgetUsage(total_tokens=1),
            )
        )
    assert restored.snapshot() == before

    assert restored.release(outstanding).total_tokens == 0
    assert restored.reservations() == ()
    with pytest.raises(BudgetReservationUnknownError):
        restored.release(outstanding)

    replacement = restored.reserve(tool_calls=1, total_tokens=20)
    assert replacement.reservation_id != outstanding.reservation_id
    restored.commit(replacement)
    assert restored.usage.tool_calls == 2
    assert restored.usage.total_tokens == 35
    assert restored.usage.model_calls == 1
    assert restored.reserved == BudgetUsage()
    assert restored.available.tool_calls == 2
    assert restored.available.total_tokens == 65

    with pytest.raises(BudgetExhaustedError):
        restored.record_tool_call(count=3)
    assert restored.usage.tool_calls == 2


def test_fractional_duration_reservations_release_in_any_order() -> None:
    ledger = BudgetLedger(RunBudget(max_duration_seconds=10.0))
    held = [ledger.reserve(duration_seconds=value) for value in (0.1, 0.2, 0.3, 1e-7)]
    for reservation in (held[1], held[3], held[0], held[2]):
        ledger.release(reservation)
    assert ledger.reserved == BudgetUsage()
    assert ledger.reserved.duration_seconds == 0.0

    first = ledger.reserve(duration_seconds=0.1)
    second = ledger.reserve(duration_seconds=0.2)
    state = ledger.export_state()
    restored = BudgetLedger()
    restored.import_state(state)
    restored.release(second)
    restored.release(first)
    assert restored.reserved == BudgetUsage()
    assert BudgetUsage(duration_seconds=0.1) + BudgetUsage(duration_seconds=0.2) == BudgetUsage(
        duration_seconds=0.3
    )


def test_budget_reserve_and_commit_are_fail_closed_and_never_go_negative() -> None:
    ledger = BudgetLedger(RunBudget(max_tool_calls=1, max_total_tokens=10))
    reservation = ledger.reserve(tool_calls=1, total_tokens=4)
    snapshot = ledger.snapshot()

    with pytest.raises(BudgetExhaustedError):
        ledger.commit(reservation, tool_calls=1, total_tokens=99)
    assert ledger.snapshot() == snapshot
    assert ledger.reservations() == (reservation,)

    with pytest.raises(BudgetExhaustedError):
        ledger.reserve(tool_calls=1)
    assert ledger.snapshot() == snapshot

    with pytest.raises(BudgetCounterError):
        BudgetUsage(tool_calls=1) - BudgetUsage(tool_calls=2)
    with pytest.raises(BudgetCounterError):
        BudgetUsage() - BudgetUsage(cost=Decimal("0.5"))

    assert ledger.commit(reservation).total_tokens == 4
    assert ledger.reserved == BudgetUsage()
    with pytest.raises(BudgetReservationUnknownError):
        ledger.commit(reservation)
    with pytest.raises(BudgetReservationUnknownError):
        ledger.release(reservation)

    for usage in (ledger.usage, ledger.reserved, ledger.available):
        assert all(
            value >= 0
            for value in (
                usage.model_calls,
                usage.tool_calls,
                usage.total_tokens,
                usage.artifact_bytes,
                usage.duration_seconds,
                usage.replanning_attempts,
            )
        )
        assert usage.cost >= 0


def test_budget_state_import_rejects_malformed_and_inconsistent_state() -> None:
    ledger = BudgetLedger(RunBudget(max_tool_calls=2, max_total_tokens=50))
    reservation = ledger.reserve(tool_calls=1, total_tokens=10)
    valid: dict[str, Any] = ledger.export_state()

    malformed: tuple[tuple[str, Any], ...] = (
        ("version", {**valid, "version": 99}),
        ("version-type", {**valid, "version": "1"}),
        ("unknown-key", {**valid, "unexpected": True}),
        ("budget-shape", {**valid, "budget": "none"}),
        ("budget-key", {**valid, "budget": {**valid["budget"], "extra": 1}}),
        ("budget-negative", {**valid, "budget": {**valid["budget"], "max_tool_calls": -1}}),
        ("usage-shape", {**valid, "usage": []}),
        ("usage-negative", {**valid, "usage": {**valid["usage"], "tool_calls": -1}}),
        ("usage-unknown", {**valid, "usage": {**valid["usage"], "extra": 1}}),
        ("usage-type", {**valid, "usage": {**valid["usage"], "cost": "free"}}),
        (
            "reserved-mismatch",
            {**valid, "reserved": {**valid["reserved"], "tool_calls": 5}},
        ),
        (
            "reservation-key",
            {**valid, "reservations": [{"reservation_id": "r1", "usage": {}, "extra": 1}]},
        ),
        (
            "reservation-id",
            {**valid, "reservations": [{"reservation_id": "", "usage": {}}]},
        ),
        (
            "reservation-duplicate",
            {**valid, "reservations": [valid["reservations"][0], valid["reservations"][0]]},
        ),
        ("next-reservation", {**valid, "next_reservation": 0}),
        ("next-reservation-type", {**valid, "next_reservation": "1"}),
        ("not-a-mapping", []),
    )
    for label, payload in malformed:
        with pytest.raises(BudgetStateError):
            BudgetLedger().import_state(payload)
        assert ledger.export_state() == valid, label

    empty = BudgetLedger()
    empty.import_state(valid)
    assert empty.reservations() == (reservation,)

    fresh = empty.reserve(tool_calls=1)
    assert fresh.reservation_id != reservation.reservation_id
    assert empty.reserved.tool_calls == 2
