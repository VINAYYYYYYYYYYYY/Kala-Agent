from __future__ import annotations

import threading
from collections.abc import Iterable, Iterator
from dataclasses import FrozenInstanceError
from datetime import UTC, datetime, timedelta
from typing import Any, cast

import pytest

from kala_agent.contracts import ApprovalKind, PlanStep, WorkflowDefinition, WorkflowTransition
from kala_agent.kernel import memory as memory_module
from kala_agent.kernel.memory import (
    MemoryAddResult,
    MemoryConflictError,
    MemoryEntry,
    MemoryError,
    MemoryManager,
    MemoryNotFoundError,
    MemoryPromotionError,
    MemoryStatus,
    MemoryValidationError,
    _is_expired,
    _now,
    _validated_record,
)
from kala_agent.kernel.planning import (
    NormalizedPlan,
    PlanRejectedError,
    PlanValidationCode,
    PlanValidationError,
    PlanValidator,
    _capability_set,
    _combine_capability_sets,
    _find_cycle,
    _step_sequence,
)
from kala_agent.persistence import Database, MemoryRecord, MemoryRepository

RECORDED_AT = datetime(2026, 1, 2, 3, 4, 5, tzinfo=UTC)


@pytest.fixture
def repository() -> Iterator[MemoryRepository]:
    with Database() as database:
        yield MemoryRepository(database)


def _record(
    memory_id: str,
    content: str,
    *,
    source: str = "source-1",
    created_at: datetime = RECORDED_AT,
    expires_at: datetime | None = None,
    provenance_id: str | None = None,
) -> MemoryRecord:
    return MemoryRecord(
        memory_id=memory_id,
        content=content,
        source=source,
        provenance_id=provenance_id,
        created_at=created_at,
        expires_at=expires_at,
    )


def _definition(
    *,
    max_replanning_attempts: int = 1,
    available: tuple[str, ...] = ("step-1", "step-2", "step-3"),
) -> WorkflowDefinition:
    return WorkflowDefinition(
        definition_id="workflow-1",
        version="1.0.0",
        name="Test workflow",
        initial_state="start",
        states=("start", "done"),
        transitions=(
            WorkflowTransition(
                transition_id="transition-1",
                from_state="start",
                to_state="done",
                step_id="step-1",
                checkpoint_type=None,
            ),
        ),
        available_step_ids=available,
        max_replanning_attempts=max_replanning_attempts,
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


def test_now_defaults_to_current_utc() -> None:
    generated = _now(None)
    assert generated.tzinfo is UTC
    assert _now(RECORDED_AT) == RECORDED_AT


def test_now_normalizes_offset_to_utc() -> None:
    from datetime import timezone

    offset = datetime(2026, 1, 2, 5, 4, 5, tzinfo=timezone(timedelta(hours=2)))
    assert _now(offset) == RECORDED_AT


def test_now_rejects_naive_datetime() -> None:
    naive = datetime(2026, 1, 2, 3, 4, 5)
    with pytest.raises(ValueError, match="timezone"):
        _now(naive)


def test_is_expired_compares_expiry() -> None:
    created = RECORDED_AT - timedelta(minutes=2)
    assert _is_expired(_record("m", "c"), RECORDED_AT) is False
    assert (
        _is_expired(
            _record("m", "c", created_at=created, expires_at=RECORDED_AT + timedelta(seconds=1)),
            RECORDED_AT,
        )
        is False
    )
    assert (
        _is_expired(
            _record("m", "c", created_at=created, expires_at=RECORDED_AT),
            RECORDED_AT,
        )
        is True
    )
    assert (
        _is_expired(
            _record("m", "c", created_at=created, expires_at=RECORDED_AT - timedelta(seconds=1)),
            RECORDED_AT,
        )
        is True
    )


def test_validated_record_rejects_non_records() -> None:
    with pytest.raises(MemoryValidationError, match="MemoryRecord"):
        _validated_record({"memory_id": "m"})


def test_validated_record_reports_contract_failures() -> None:
    record = _record("m", "c")
    broken = record.model_copy(update={"expires_at": RECORDED_AT - timedelta(seconds=1)})
    with pytest.raises(MemoryValidationError) as error:
        _validated_record(broken)
    assert isinstance(error.value.__cause__, Exception)
    assert "failed validation" in str(error.value)


def test_memory_add_result_exposes_record_projection() -> None:
    record = _record("m-1", "content", provenance_id="p-1")
    result = MemoryAddResult(status=MemoryStatus.ADDED, record=record)
    assert result.memory_id == "m-1"
    assert result.content == "content"
    assert result.source == "source-1"
    assert result.provenance_id == "p-1"
    assert result.accepted is True
    assert result.existing is None
    assert result.conflicting is None
    payload = result.as_dict()
    assert payload["status"] == "added"
    assert payload["existing"] is None
    assert payload["conflicting"] is None


def test_memory_add_result_rejects_conflict() -> None:
    record = _record("m-1", "content")
    result = MemoryAddResult(
        status=MemoryStatus.CONFLICT,
        record=record,
        conflicting=_record("m-0", "other"),
    )
    assert result.accepted is False
    payload = result.as_dict()
    assert payload["existing"] is None
    assert cast("dict[str, object]", payload["conflicting"])["content"] == "other"


def test_memory_add_result_duplicate_is_accepted() -> None:
    record = _record("m-1", "content")
    result = MemoryAddResult(
        status=MemoryStatus.DUPLICATE,
        record=record,
        existing=record,
    )
    assert result.accepted is True
    assert cast("dict[str, object]", result.as_dict()["existing"])["memory_id"] == "m-1"


def test_memory_entry_is_frozen() -> None:
    entry = MemoryEntry(record=_record("m-1", "content"))
    assert entry.durable is False
    frozen_field = "durable"
    with pytest.raises(FrozenInstanceError):
        setattr(entry, frozen_field, True)


def test_memory_error_hierarchy() -> None:
    assert issubclass(MemoryValidationError, MemoryError)
    assert issubclass(MemoryValidationError, ValueError)
    assert issubclass(MemoryConflictError, MemoryError)
    assert issubclass(MemoryNotFoundError, KeyError)
    assert issubclass(MemoryPromotionError, MemoryError)


def test_memory_conflict_error_carries_both_records() -> None:
    record = _record("m-1", "content")
    conflicting = _record("m-0", "other")
    error = MemoryConflictError(record, conflicting)
    assert error.record is record
    assert error.conflicting is conflicting
    assert "source-1" in str(error)


def test_memory_manager_rejects_bad_configuration(repository: MemoryRepository) -> None:
    with pytest.raises(TypeError, match="MemoryRepository"):
        MemoryManager(cast(MemoryRepository, object()))
    with pytest.raises(TypeError, match="max_working_records"):
        MemoryManager(repository, max_working_records=cast(int, True))
    with pytest.raises(TypeError, match="max_working_records"):
        MemoryManager(repository, max_working_records=cast(int, "5"))
    with pytest.raises(ValueError, match="max_working_records"):
        MemoryManager(repository, max_working_records=0)


def test_memory_manager_exposes_configuration(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository, max_working_records=7)
    assert manager.repository is repository
    assert manager.max_working_records == 7


def test_try_add_reports_duplicate_against_durable(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    record = _record("m-1", "shared")
    manager.promote(record, now=RECORDED_AT)
    result = manager.try_add(_record("m-2", "shared"), now=RECORDED_AT)
    assert result.status is MemoryStatus.DUPLICATE
    assert result.existing is not None
    assert result.existing.memory_id == "m-1"
    assert result.accepted is True


def test_try_add_reports_conflict_against_durable(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    manager.promote(_record("m-1", "first"), now=RECORDED_AT)
    result = manager.try_add(_record("m-2", "second"), now=RECORDED_AT)
    assert result.status is MemoryStatus.CONFLICT
    assert result.conflicting is not None
    assert result.conflicting.memory_id == "m-1"
    assert result.accepted is False


def test_try_add_ignores_expired_durable_records(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    manager.promote(
        _record("m-1", "first", expires_at=RECORDED_AT + timedelta(minutes=1)),
        now=RECORDED_AT,
    )
    result = manager.try_add(
        _record("m-2", "second"),
        now=RECORDED_AT + timedelta(minutes=2),
    )
    assert result.status is MemoryStatus.ADDED


def test_try_add_reports_conflict_against_working(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    manager.add(_record("m-1", "first"), now=RECORDED_AT)
    result = manager.try_add(_record("m-2", "second"), now=RECORDED_AT)
    assert result.status is MemoryStatus.CONFLICT
    assert result.conflicting is not None
    assert result.conflicting.memory_id == "m-1"


def test_try_add_reports_duplicate_against_working(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    manager.add(_record("m-1", "shared"), now=RECORDED_AT)
    result = manager.try_add(_record("m-2", "shared"), now=RECORDED_AT)
    assert result.status is MemoryStatus.DUPLICATE
    assert result.existing is not None
    assert result.existing.memory_id == "m-1"


def test_try_add_ignores_expired_working_records(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    manager.add(
        _record("m-1", "first", expires_at=RECORDED_AT + timedelta(minutes=1)),
        now=RECORDED_AT,
    )
    result = manager.try_add(
        _record("m-2", "second"),
        now=RECORDED_AT + timedelta(minutes=2),
    )
    assert result.status is MemoryStatus.ADDED


def test_try_add_detects_same_id_with_different_content(
    repository: MemoryRepository,
) -> None:
    manager = MemoryManager(repository, max_working_records=8)
    first = _record("m-1", "first", source="source-a")
    assert manager.try_add(first, now=RECORDED_AT).status is MemoryStatus.ADDED
    conflicting = _record("m-1", "different", source="source-b")
    result = manager.try_add(conflicting, now=RECORDED_AT)
    assert result.status is MemoryStatus.CONFLICT
    assert result.conflicting is not None
    assert result.conflicting.content == "first"


def test_try_add_returns_existing_for_same_id_same_content(
    repository: MemoryRepository,
) -> None:
    manager = MemoryManager(repository, max_working_records=8)
    record = _record("m-1", "shared", source="source-a")
    manager.try_add(record, now=RECORDED_AT)
    again = _record("m-1", "shared", source="source-b")
    result = manager.try_add(again, now=RECORDED_AT)
    assert result.status is MemoryStatus.DUPLICATE
    assert result.record is result.existing
    assert result.record.source == "source-a"


def test_try_add_evicts_expired_records_at_capacity(
    repository: MemoryRepository,
) -> None:
    manager = MemoryManager(repository, max_working_records=2)
    manager.add(
        _record("m-1", "first", expires_at=RECORDED_AT + timedelta(minutes=1)),
        now=RECORDED_AT,
    )
    manager.add(_record("m-2", "second", source="source-2"), now=RECORDED_AT)
    later = RECORDED_AT + timedelta(minutes=2)
    result = manager.try_add(_record("m-3", "third", source="source-3"), now=later)
    assert result.status is MemoryStatus.ADDED
    assert [item.memory_id for item in manager.list(now=later)] == ["m-2", "m-3"]


def test_try_add_refuses_to_exceed_capacity(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository, max_working_records=1)
    manager.add(_record("m-1", "first"), now=RECORDED_AT)
    with pytest.raises(MemoryPromotionError, match="limit exceeded"):
        manager.try_add(_record("m-2", "second", source="source-2"), now=RECORDED_AT)


def test_add_raises_on_conflict(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    manager.add(_record("m-1", "first"), now=RECORDED_AT)
    with pytest.raises(MemoryConflictError):
        manager.add(_record("m-2", "second"), now=RECORDED_AT)


def test_add_promotes_when_requested(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    record = _record("m-1", "content")
    manager.add(record, promote=True, now=RECORDED_AT)
    assert manager.list_durable(now=RECORDED_AT) == (record,)


def test_add_promotes_when_durable_flag_set(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    record = _record("m-1", "content")
    manager.add(record, durable=True, now=RECORDED_AT)
    assert manager.list_durable(now=RECORDED_AT) == (record,)


def test_add_record_alias_matches_add(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    assert MemoryManager.add_record is MemoryManager.add
    stored = manager.add_record(_record("m-1", "content"), now=RECORDED_AT)
    assert manager.get("m-1", now=RECORDED_AT) == stored


def test_promote_by_id_from_working_memory(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    manager.add(_record("m-1", "content"), now=RECORDED_AT)
    promoted = manager.promote("m-1", now=RECORDED_AT)
    assert manager.list_durable(now=RECORDED_AT) == (promoted,)
    assert MemoryManager.promote_record is MemoryManager.promote


def test_promote_by_id_from_durable_repository(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    other = MemoryManager(repository)
    original = _record("m-1", "content")
    other.promote(original, now=RECORDED_AT)
    manager._working.clear()
    promoted = manager.promote("m-1", now=RECORDED_AT)
    assert promoted == original


def test_promote_unknown_id_raises_not_found(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    with pytest.raises(MemoryNotFoundError):
        manager.promote("absent", now=RECORDED_AT)


def test_promote_rejects_unsupported_type(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    with pytest.raises(TypeError, match="MemoryRecord or memory_id"):
        manager.promote(cast("MemoryRecord | str", 7), now=RECORDED_AT)


def test_promote_imports_working_record_not_in_memory(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository, max_working_records=4)
    record = _record("m-1", "content", source="source-a")
    promoted = manager.promote(record, now=RECORDED_AT)
    assert promoted == record
    assert manager.list_durable(now=RECORDED_AT) == (record,)


def test_promote_reports_conflict_against_durable(repository: MemoryRepository) -> None:
    other = MemoryManager(repository)
    manager = MemoryManager(repository, max_working_records=4)
    other.promote(_record("m-1", "first"), now=RECORDED_AT)
    manager._working["m-2"] = MemoryEntry(record=_record("m-2", "second"))
    with pytest.raises(MemoryConflictError) as error:
        manager.promote("m-2", now=RECORDED_AT)
    assert error.value.conflicting is not None
    assert error.value.conflicting.memory_id == "m-1"


def test_promote_detects_conflict_with_missing_conflicting_record(
    repository: MemoryRepository, monkeypatch: pytest.MonkeyPatch
) -> None:
    manager = MemoryManager(repository, max_working_records=4)
    record = _record("m-1", "first")

    def fake_try_add(
        _self: MemoryManager,
        candidate: MemoryRecord,
        *,
        now: datetime | None = None,
    ) -> MemoryAddResult:
        return MemoryAddResult(status=MemoryStatus.CONFLICT, record=candidate, conflicting=None)

    monkeypatch.setattr(memory_module.MemoryManager, "try_add", fake_try_add)
    with pytest.raises(MemoryConflictError) as error:
        manager.promote(record, now=RECORDED_AT)
    assert error.value.conflicting == record


def test_promote_reports_conflict_against_other_working_records(
    repository: MemoryRepository,
) -> None:
    manager = MemoryManager(repository, max_working_records=4)
    candidate = _record("m-1", "first", created_at=RECORDED_AT)
    rival = _record("m-2", "second", created_at=RECORDED_AT)
    manager._working["m-1"] = MemoryEntry(record=candidate)
    manager._working["m-2"] = MemoryEntry(record=rival)
    with pytest.raises(MemoryConflictError) as error:
        manager.promote("m-1", now=RECORDED_AT)
    assert error.value.conflicting is not None
    assert error.value.conflicting.memory_id == "m-2"


def test_promote_applies_provenance_override(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    manager.add(_record("m-1", "content"), now=RECORDED_AT)
    promoted = manager.promote("m-1", provenance_id="p-42", now=RECORDED_AT)
    assert promoted.provenance_id == "p-42"


def test_promote_requires_provenance_when_configured(
    repository: MemoryRepository,
) -> None:
    manager = MemoryManager(repository, require_provenance_on_promotion=True)
    with pytest.raises(MemoryPromotionError, match="requires provenance"):
        manager.promote(_record("m-1", "content", provenance_id=None), now=RECORDED_AT)


def test_get_rejects_bad_identifiers(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    with pytest.raises(ValueError, match="memory_id"):
        manager.get("   ")
    with pytest.raises(ValueError, match="memory_id"):
        manager.get(cast(str, 7))


def test_get_hides_expired_working_unless_requested(
    repository: MemoryRepository,
) -> None:
    manager = MemoryManager(repository)
    record = _record("m-1", "content", expires_at=RECORDED_AT + timedelta(minutes=1))
    manager.add(record, now=RECORDED_AT)
    later = RECORDED_AT + timedelta(minutes=2)
    assert manager.get("m-1", now=later) is None
    assert manager.get("m-1", now=later, include_expired=True) == record


def test_get_falls_back_to_repository(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    record = _record("m-1", "content")
    manager.promote(record, now=RECORDED_AT)
    manager._working.clear()
    assert manager.get("m-1", now=RECORDED_AT) == record


def test_list_validates_limit(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    with pytest.raises(TypeError, match="limit"):
        manager.list(limit=cast(int, True))
    with pytest.raises(ValueError, match="limit"):
        manager.list(limit=0)


def test_list_filters_by_source_and_truncates(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    first = _record("m-1", "first", created_at=RECORDED_AT)
    second = _record(
        "m-2", "second", source="source-2", created_at=RECORDED_AT + timedelta(seconds=1)
    )
    manager.add(first, now=RECORDED_AT)
    manager.add(second, now=RECORDED_AT)
    assert [item.memory_id for item in manager.list(now=RECORDED_AT)] == ["m-1", "m-2"]
    assert [item.memory_id for item in manager.list(source="source-2", now=RECORDED_AT)] == ["m-2"]
    assert [item.memory_id for item in manager.list(now=RECORDED_AT, limit=1)] == ["m-1"]


def test_list_skips_expired_and_supports_durable_only(
    repository: MemoryRepository,
) -> None:
    manager = MemoryManager(repository)
    expiring = _record("m-1", "first", expires_at=RECORDED_AT + timedelta(minutes=1))
    durable = _record("m-2", "second", source="source-2")
    manager.add(expiring, now=RECORDED_AT)
    manager.add(durable, promote=True, now=RECORDED_AT)
    later = RECORDED_AT + timedelta(minutes=2)
    assert [item.memory_id for item in manager.list(now=later)] == ["m-2"]
    assert [item.memory_id for item in manager.list_durable(now=RECORDED_AT)] == ["m-2"]


def test_list_orders_by_creation_then_id(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    later = _record("m-2", "second", source="source-2", created_at=RECORDED_AT)
    earlier = _record("m-1", "first", created_at=RECORDED_AT - timedelta(seconds=1))
    manager.add(later, now=RECORDED_AT)
    manager.add(earlier, now=RECORDED_AT)
    assert [item.memory_id for item in manager.list(now=RECORDED_AT)] == ["m-1", "m-2"]


def test_list_active_matches_list(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    record = _record("m-1", "content")
    manager.add(record, now=RECORDED_AT)
    assert manager.list_active(now=RECORDED_AT) == manager.list(now=RECORDED_AT)
    assert manager.list_active(now=RECORDED_AT, include_working=False, durable_only=True) == ()


def test_list_working_excludes_durable_and_expired(
    repository: MemoryRepository,
) -> None:
    manager = MemoryManager(repository)
    durable = _record("m-1", "first", source="source-1")
    working = _record("m-2", "second", source="source-2")
    expiring = _record(
        "m-3", "third", source="source-3", expires_at=RECORDED_AT + timedelta(minutes=1)
    )
    manager.add(durable, promote=True, now=RECORDED_AT)
    manager.add(working, now=RECORDED_AT)
    manager.add(expiring, now=RECORDED_AT)
    assert [item.memory_id for item in manager.list_working(now=RECORDED_AT)] == ["m-2", "m-3"]
    assert [
        item.memory_id for item in manager.list_working(source="source-1", now=RECORDED_AT)
    ] == []
    later = RECORDED_AT + timedelta(minutes=2)
    assert [item.memory_id for item in manager.list_working(now=later)] == ["m-2"]


def test_list_working_honours_limit(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository, max_working_records=4)
    manager.add(_record("m-1", "first"), now=RECORDED_AT)
    manager.add(_record("m-2", "second", source="source-2"), now=RECORDED_AT)
    assert len(manager.list_working(now=RECORDED_AT, limit=1)) == 1


def test_detect_conflicts_sorts_and_filters(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository, max_working_records=8)
    working_first = _record("m-1", "first", created_at=RECORDED_AT)
    durable_second = _record("m-2", "second", created_at=RECORDED_AT - timedelta(seconds=1))
    manager.add(working_first, now=RECORDED_AT)
    repository.put(durable_second)
    conflicts = manager.detect_conflicts(
        _record("m-3", "third", source="source-1"), now=RECORDED_AT
    )
    assert [item.memory_id for item in conflicts] == ["m-2", "m-1"]
    assert manager.detect_conflicts(_record("m-4", "other", source="other"), now=RECORDED_AT) == ()


def test_detect_conflicts_ignores_expired_entries(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    manager.add(
        _record("m-1", "first", expires_at=RECORDED_AT + timedelta(minutes=1)),
        now=RECORDED_AT,
    )
    later = RECORDED_AT + timedelta(minutes=2)
    assert manager.detect_conflicts(_record("m-2", "second"), now=later) == ()


def test_discard_working_only_removes_working(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    manager.add(_record("m-1", "content"), now=RECORDED_AT)
    assert manager.discard_working("m-1") is True
    assert manager.discard_working("m-1") is False
    manager.add(_record("m-2", "content"), promote=True, now=RECORDED_AT)
    assert manager.discard_working("m-2") is False
    assert manager.discard_working("absent") is False


def test_delete_removes_working_and_durable(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    manager.add(_record("m-1", "content"), promote=True, now=RECORDED_AT)
    assert manager.delete("m-1") is True
    assert manager.get("m-1", now=RECORDED_AT) is None
    assert manager.delete("absent") is False


def test_active_durable_records_hides_expired(repository: MemoryRepository) -> None:
    manager = MemoryManager(repository)
    record = _record("m-1", "content", expires_at=RECORDED_AT + timedelta(minutes=1))
    manager.promote(record, now=RECORDED_AT)
    later = RECORDED_AT + timedelta(minutes=2)
    assert manager._active_durable_records(None, later) == ()


def test_memory_manager_serializes_concurrent_adds(
    repository: MemoryRepository,
) -> None:
    manager = MemoryManager(repository, max_working_records=200)
    errors: list[BaseException] = []

    def worker(index: int) -> None:
        try:
            manager.add(
                _record(f"m-{index}", f"content-{index}", source=f"source-{index}"),
                now=RECORDED_AT,
            )
        except BaseException as error:
            errors.append(error)

    threads = [threading.Thread(target=worker, args=(index,)) for index in range(10)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    assert errors == []
    assert len(manager.list(now=RECORDED_AT)) == 10


def test_normalized_plan_rejects_blank_definition_id() -> None:
    with pytest.raises(ValueError, match="workflow_definition_id"):
        NormalizedPlan(workflow_definition_id="  ", steps=(_step("a"),))


def test_normalized_plan_rejects_empty_steps() -> None:
    with pytest.raises(ValueError, match="cannot be empty"):
        NormalizedPlan(workflow_definition_id="w-1", steps=())


def test_normalized_plan_rejects_non_integer_attempts() -> None:
    with pytest.raises(TypeError, match="max_attempts"):
        NormalizedPlan(
            workflow_definition_id="w-1",
            steps=(_step("a"),),
            max_attempts=cast(int, True),
        )


def test_normalized_plan_rejects_non_positive_attempts() -> None:
    with pytest.raises(ValueError, match="max_attempts"):
        NormalizedPlan(workflow_definition_id="w-1", steps=(_step("a"),), max_attempts=0)


def test_normalized_plan_rejects_non_plan_steps() -> None:
    with pytest.raises(TypeError, match="PlanStep"):
        NormalizedPlan(
            workflow_definition_id="w-1",
            steps=cast("tuple[PlanStep, ...]", (object(),)),
        )


def test_normalized_plan_exposes_sequence_protocol() -> None:
    steps = (_step("a"), _step("b", catalog_step_id="step-2"))
    plan = NormalizedPlan(workflow_definition_id="w-1", steps=steps)
    assert plan.step_ids == ("a", "b")
    assert plan.capabilities == ()
    assert len(plan) == 2
    assert plan[0] is steps[0]
    assert list(plan) == list(steps)


def test_plan_validation_error_renders_step_context() -> None:
    without_step = PlanValidationError(code=PlanValidationCode.EMPTY_PLAN, message="empty")
    assert str(without_step) == "empty"
    with_step = PlanValidationError(
        code=PlanValidationCode.DUPLICATE_STEP_ID,
        message="duplicate",
        step_id="s-1",
        details=(("count", "2"),),
    )
    assert str(with_step) == "duplicate (s-1)"
    assert with_step.reason is PlanValidationCode.DUPLICATE_STEP_ID
    assert with_step.field == "s-1"
    assert without_step.field is None
    assert with_step.as_dict() == {
        "code": "duplicate_step_id",
        "message": "duplicate",
        "step_id": "s-1",
        "details": {"count": "2"},
    }


def test_plan_rejected_error_wraps_validation_error() -> None:
    error = PlanValidationError(code=PlanValidationCode.EMPTY_PLAN, message="empty")
    rejected = PlanRejectedError(error)
    assert rejected.error is error
    assert str(rejected) == "empty"


def test_plan_step_sequence_rejects_non_sequences() -> None:
    assert _step_sequence("abc") is None
    assert _step_sequence(b"abc") is None
    assert _step_sequence(bytearray(b"abc")) is None
    assert _step_sequence(cast(Any, 7)) is None
    assert _step_sequence((1, 2)) == (1, 2)
    assert _step_sequence([1, 2]) == (1, 2)


def test_capability_set_rejects_non_strings() -> None:
    with pytest.raises(TypeError, match="strings"):
        _capability_set(cast("Iterable[str]", (7,)))


def test_capability_set_rejects_blank_entries() -> None:
    with pytest.raises(ValueError, match="cannot be empty"):
        _capability_set(("read", "  "))


def test_capability_set_strips_entries() -> None:
    assert _capability_set((" read ", "write")) == frozenset({"read", "write"})


def test_combine_capability_sets_intersects() -> None:
    assert _combine_capability_sets(None, None) is None
    assert _combine_capability_sets({"read"}, None) == frozenset({"read"})
    assert _combine_capability_sets({"read", "write"}, {"read"}) == frozenset({"read"})


def test_find_cycle_detects_long_cycle() -> None:
    steps = (
        _step("a", depends_on=("c",)),
        _step("b", catalog_step_id="step-2", depends_on=("a",)),
        _step("c", catalog_step_id="step-3", depends_on=("b",)),
    )
    cycle = _find_cycle(steps)
    assert cycle is not None
    assert set(cycle) == {"a", "b", "c"}


def test_find_cycle_returns_none_for_acyclic_plan() -> None:
    steps = (
        _step("a"),
        _step("b", catalog_step_id="step-2", depends_on=("a",)),
    )
    assert _find_cycle(steps) is None
    assert _find_cycle(()) is None


def test_find_cycle_ignores_dependencies_outside_the_graph() -> None:
    steps = (_step("a", depends_on=("absent",)),)
    assert _find_cycle(steps) is None


def test_plan_validator_rejects_non_definition() -> None:
    with pytest.raises(TypeError, match="WorkflowDefinition"):
        PlanValidator(cast(WorkflowDefinition, object()))


def test_plan_validator_validates_max_steps() -> None:
    definition = _definition()
    with pytest.raises(TypeError, match="max_steps"):
        PlanValidator(definition, max_steps=cast(int, True))
    with pytest.raises(ValueError, match="max_steps"):
        PlanValidator(definition, max_steps=0)


def test_plan_validator_validates_max_attempts() -> None:
    definition = _definition()
    with pytest.raises(TypeError, match="max_attempts"):
        PlanValidator(definition, max_attempts=cast(int, "2"))
    with pytest.raises(ValueError, match="max_attempts"):
        PlanValidator(definition, max_attempts=0)


def test_plan_validator_validates_max_total_attempts() -> None:
    definition = _definition()
    with pytest.raises(TypeError, match="max_total_attempts"):
        PlanValidator(definition, max_total_attempts=cast("int | None", True))
    with pytest.raises(ValueError, match="max_total_attempts"):
        PlanValidator(definition, max_total_attempts=0)


def test_plan_validator_exposes_capability_configuration() -> None:
    definition = _definition()
    unrestricted = PlanValidator(definition)
    assert unrestricted.unrestricted is True
    assert unrestricted.allowed_capabilities is None
    assert unrestricted.definition is definition
    assert PlanValidator.granted_capabilities is PlanValidator.allowed_capabilities
    restricted = PlanValidator(definition, allowed_capabilities={"read", "write"})
    assert restricted.unrestricted is False
    assert restricted.allowed_capabilities == frozenset({"read", "write"})
    both = PlanValidator(
        definition, allowed_capabilities={"read", "write"}, granted_capabilities={"read"}
    )
    assert both.allowed_capabilities == frozenset({"read"})


def test_plan_validate_rejects_non_sequence() -> None:
    result = PlanValidator(_definition()).validate(cast(Any, "not a plan"))
    assert isinstance(result, PlanValidationError)
    assert result.code is PlanValidationCode.INVALID_STEP


def test_plan_validate_rejects_empty_plan() -> None:
    result = PlanValidator(_definition()).validate(())
    assert isinstance(result, PlanValidationError)
    assert result.code is PlanValidationCode.EMPTY_PLAN


def test_plan_validate_rejects_too_many_steps() -> None:
    validator = PlanValidator(_definition(), max_steps=1)
    result = validator.validate((_step("a"), _step("b", catalog_step_id="step-2")))
    assert isinstance(result, PlanValidationError)
    assert result.code is PlanValidationCode.TOO_MANY_STEPS
    assert result.details == (("count", "2"), ("maximum", "1"))
    assert "maximum is 1" in result.message


def test_plan_validate_rejects_bad_replanning_attempts() -> None:
    validator = PlanValidator(_definition())
    not_integer = validator.validate((_step("a"),), replanning_attempts=cast(int, True))
    assert isinstance(not_integer, PlanValidationError)
    assert not_integer.code is PlanValidationCode.REPLANNING_LIMIT
    negative = validator.validate((_step("a"),), replanning_attempts=-1)
    assert isinstance(negative, PlanValidationError)
    assert negative.code is PlanValidationCode.REPLANNING_LIMIT
    over = validator.validate((_step("a"),), replanning_attempts=5)
    assert isinstance(over, PlanValidationError)
    assert over.code is PlanValidationCode.REPLANNING_LIMIT


def test_plan_validate_rejects_non_plan_step_entries() -> None:
    validator = PlanValidator(_definition())
    result = validator.validate(cast(Any, (object(),)))
    assert isinstance(result, PlanValidationError)
    assert result.code is PlanValidationCode.INVALID_STEP


def test_plan_validate_rejects_self_dependency() -> None:
    validator = PlanValidator(_definition())
    unvalidated = PlanStep.model_construct(
        step_id="a",
        catalog_step_id="step-1",
        intent="Perform a declared step.",
        parameters_json="{}",
        input_artifact_ids=(),
        required_capabilities=(),
        depends_on=("a",),
        estimated_duration_seconds=None,
        max_attempts=1,
    )
    result = validator.validate((unvalidated,))
    assert isinstance(result, PlanValidationError)
    assert result.code is PlanValidationCode.SELF_DEPENDENCY
    assert result.step_id == "a"


def test_plan_validate_rejects_invalid_step_contract() -> None:
    validator = PlanValidator(_definition())
    unvalidated = PlanStep.model_construct(
        step_id="a",
        catalog_step_id=" ",
        intent="Perform a declared step.",
        parameters_json="{}",
        input_artifact_ids=(),
        required_capabilities=(),
        depends_on=(),
        estimated_duration_seconds=None,
        max_attempts=1,
    )
    result = validator.validate((unvalidated,))
    assert isinstance(result, PlanValidationError)
    assert result.code is PlanValidationCode.INVALID_STEP
    assert result.step_id == "a"
    assert result.details[0][0] == "reason"


def test_plan_validate_rejects_duplicate_step_ids() -> None:
    validator = PlanValidator(_definition())
    result = validator.validate((_step("a"), _step("a")))
    assert isinstance(result, PlanValidationError)
    assert result.code is PlanValidationCode.DUPLICATE_STEP_ID
    assert result.step_id == "a"


def test_plan_validate_rejects_unknown_catalog_step() -> None:
    validator = PlanValidator(_definition())
    result = validator.validate((_step("a", catalog_step_id="absent"),))
    assert isinstance(result, PlanValidationError)
    assert result.code is PlanValidationCode.UNKNOWN_CATALOG_STEP
    assert result.details == (("catalog_step_id", "absent"),)


def test_plan_validate_rejects_non_positive_attempts() -> None:
    validator = PlanValidator(_definition(), max_attempts=3)
    result = validator.validate((_step("a", max_attempts=0),))
    assert isinstance(result, PlanValidationError)
    assert result.code is PlanValidationCode.INVALID_ATTEMPTS


def test_plan_validate_rejects_excessive_attempts() -> None:
    validator = PlanValidator(_definition(), max_attempts=1)
    result = validator.validate((_step("a", max_attempts=2),))
    assert isinstance(result, PlanValidationError)
    assert result.code is PlanValidationCode.TOO_MANY_ATTEMPTS
    assert result.details == (("requested", "2"), ("maximum", "1"))


def test_plan_validate_rejects_unknown_dependency() -> None:
    validator = PlanValidator(_definition())
    result = validator.validate((_step("a", depends_on=("absent",)),))
    assert isinstance(result, PlanValidationError)
    assert result.code is PlanValidationCode.UNKNOWN_DEPENDENCY
    assert result.details == (("dependency", "absent"),)


def test_plan_validate_rejects_dependency_cycle() -> None:
    validator = PlanValidator(_definition())
    steps = (
        _step("a", depends_on=("b",)),
        _step("b", catalog_step_id="step-2", depends_on=("a",)),
    )
    result = validator.validate(steps)
    assert isinstance(result, PlanValidationError)
    assert result.code is PlanValidationCode.DEPENDENCY_CYCLE
    assert result.step_id in {"a", "b"}
    assert result.details[0][0] == "cycle"


def test_plan_validate_rejects_missing_capabilities() -> None:
    validator = PlanValidator(_definition(), allowed_capabilities={"read"})
    result = validator.validate((_step("a", capabilities=("read", "write")),))
    assert isinstance(result, PlanValidationError)
    assert result.code is PlanValidationCode.CAPABILITY_NOT_GRANTED
    assert result.details == (("capability", "write"),)


def test_plan_validate_rejects_aggregate_attempt_overflow() -> None:
    validator = PlanValidator(_definition(), max_attempts=2, max_total_attempts=2)
    steps = (_step("a", max_attempts=2), _step("b", catalog_step_id="step-2", max_attempts=2))
    result = validator.validate(steps)
    assert isinstance(result, PlanValidationError)
    assert result.code is PlanValidationCode.TOO_MANY_ATTEMPTS
    assert result.details == (("requested", "4"), ("maximum", "2"))


def test_plan_validate_narrows_capabilities_per_call() -> None:
    validator = PlanValidator(_definition(), allowed_capabilities={"read", "write"})
    accepted = validator.validate(
        (_step("a", capabilities=("read",)),),
        allowed_capabilities={"read", "write"},
    )
    assert isinstance(accepted, NormalizedPlan)
    rejected = validator.validate(
        (_step("a", capabilities=("read",)),),
        granted_capabilities={"write"},
    )
    assert isinstance(rejected, PlanValidationError)
    assert rejected.code is PlanValidationCode.CAPABILITY_NOT_GRANTED


def test_plan_validate_uses_call_capabilities_when_unrestricted() -> None:
    validator = PlanValidator(_definition())
    accepted = validator.validate(
        (_step("a", capabilities=("read",)),),
        allowed_capabilities={"read"},
    )
    assert isinstance(accepted, NormalizedPlan)
    rejected = validator.validate(
        (_step("a", capabilities=("write",)),),
        allowed_capabilities={"read"},
    )
    assert isinstance(rejected, PlanValidationError)
    assert rejected.code is PlanValidationCode.CAPABILITY_NOT_GRANTED


def test_plan_validate_succeeds_and_reports_capabilities() -> None:
    validator = PlanValidator(_definition(), allowed_capabilities={"read", "write"})
    steps = (
        _step("a", capabilities=("write",)),
        _step("b", catalog_step_id="step-2", capabilities=("read",)),
    )
    plan = validator.validate(steps)
    assert isinstance(plan, NormalizedPlan)
    assert plan.workflow_definition_id == "workflow-1"
    assert plan.step_ids == ("a", "b")
    assert plan.required_capabilities == ("read", "write")
    assert plan.max_attempts == 1


def test_plan_validate_or_raise_and_is_valid() -> None:
    validator = PlanValidator(_definition())
    assert validator.is_valid((_step("a"),)) is True
    assert validator.is_valid(()) is False
    with pytest.raises(PlanRejectedError) as error:
        validator.validate_or_raise(())
    assert error.value.error.code is PlanValidationCode.EMPTY_PLAN
    assert validator.validate_or_raise((_step("a"),)).step_ids == ("a",)


def test_plan_selected_capabilities_prefers_constructor_when_unset() -> None:
    validator = PlanValidator(_definition(), allowed_capabilities={"read"})
    selected = validator._selected_capabilities(None, None)
    assert selected == frozenset({"read"})
    assert validator._selected_capabilities({"read", "write"}, None) == frozenset({"read"})


def test_checkpoint_kind_is_optional_in_workflow_definition() -> None:
    transition = WorkflowTransition(
        transition_id="transition-1",
        from_state="start",
        to_state="done",
        step_id="step-1",
        checkpoint_type=ApprovalKind.PRE_COMMIT,
    )
    definition = WorkflowDefinition(
        definition_id="workflow-1",
        version="1.0.0",
        name="Checkpointed",
        initial_state="start",
        states=("start", "done"),
        transitions=(transition,),
        available_step_ids=("step-1",),
        max_replanning_attempts=0,
    )
    validator = PlanValidator(definition)
    plan = validator.validate((_step("a"),))
    assert isinstance(plan, NormalizedPlan)
