from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime
from enum import StrEnum
from threading import RLock

from kala_agent.persistence import MemoryRecord, MemoryRepository


class MemoryError(RuntimeError):
    pass


class MemoryValidationError(MemoryError, ValueError):
    pass


class MemoryConflictError(MemoryError, ValueError):
    def __init__(self, record: MemoryRecord, conflicting: MemoryRecord) -> None:
        self.record = record
        self.conflicting = conflicting
        super().__init__(f"memory source {record.source!r} already contains different content")


class MemoryNotFoundError(MemoryError, KeyError):
    pass


class MemoryPromotionError(MemoryError, ValueError):
    pass


class MemoryStatus(StrEnum):
    ADDED = "added"
    DURABLE = "durable"
    DUPLICATE = "duplicate"
    CONFLICT = "conflict"


@dataclass(frozen=True, slots=True)
class MemoryAddResult:
    status: MemoryStatus
    record: MemoryRecord
    existing: MemoryRecord | None = None
    conflicting: MemoryRecord | None = None

    @property
    def memory_id(self) -> str:
        return self.record.memory_id

    @property
    def content(self) -> str:
        return self.record.content

    @property
    def source(self) -> str:
        return self.record.source

    @property
    def provenance_id(self) -> str | None:
        return self.record.provenance_id

    @property
    def accepted(self) -> bool:
        return self.status in {MemoryStatus.ADDED, MemoryStatus.DURABLE, MemoryStatus.DUPLICATE}

    def as_dict(self) -> dict[str, object]:
        return {
            "status": self.status.value,
            "record": self.record.model_dump(mode="json"),
            "existing": None if self.existing is None else self.existing.model_dump(mode="json"),
            "conflicting": (
                None if self.conflicting is None else self.conflicting.model_dump(mode="json")
            ),
        }


@dataclass(frozen=True, slots=True)
class MemoryEntry:
    record: MemoryRecord
    durable: bool = False


def _now(value: datetime | None) -> datetime:
    if value is None:
        return datetime.now(UTC)
    if value.tzinfo is None or value.utcoffset() is None:
        raise ValueError("now must include a timezone")
    return value.astimezone(UTC)


def _is_expired(record: MemoryRecord, now: datetime) -> bool:
    return record.expires_at is not None and record.expires_at <= now


def _validated_record(record: object) -> MemoryRecord:
    if not isinstance(record, MemoryRecord):
        raise MemoryValidationError("record must be a MemoryRecord")
    try:
        return MemoryRecord.model_validate(record.model_dump(mode="python"))
    except Exception as error:
        if isinstance(error, MemoryValidationError):
            raise
        raise MemoryValidationError("record failed validation") from error


class MemoryManager:
    def __init__(
        self,
        repository: MemoryRepository,
        *,
        max_working_records: int = 1_000,
        require_provenance_on_promotion: bool = False,
    ) -> None:
        if not isinstance(repository, MemoryRepository):
            raise TypeError("repository must be a MemoryRepository")
        if isinstance(max_working_records, bool) or not isinstance(max_working_records, int):
            raise TypeError("max_working_records must be an integer")
        if max_working_records < 1:
            raise ValueError("max_working_records must be positive")
        self._repository = repository
        self._max_working_records = max_working_records
        self._require_provenance_on_promotion = require_provenance_on_promotion
        self._working: dict[str, MemoryEntry] = {}
        self._lock = RLock()

    @property
    def repository(self) -> MemoryRepository:
        return self._repository

    @property
    def max_working_records(self) -> int:
        return self._max_working_records

    def add(
        self,
        record: MemoryRecord,
        *,
        durable: bool = False,
        promote: bool = False,
        now: datetime | None = None,
    ) -> MemoryRecord:
        result = self.try_add(record, now=now)
        if result.status is MemoryStatus.CONFLICT:
            assert result.conflicting is not None
            raise MemoryConflictError(result.record, result.conflicting)
        target = result.existing if result.existing is not None else result.record
        if durable or promote:
            return self.promote(target, now=now)
        return target

    add_record = add

    def try_add(
        self,
        record: MemoryRecord,
        *,
        now: datetime | None = None,
    ) -> MemoryAddResult:
        normalized = _validated_record(record)
        current_time = _now(now)
        with self._lock:
            durable_records = self._active_durable_records(normalized.source, current_time)
            conflicting = next(
                (item for item in durable_records if item.content != normalized.content),
                None,
            )
            if conflicting is not None:
                return MemoryAddResult(
                    status=MemoryStatus.CONFLICT,
                    record=normalized,
                    conflicting=conflicting,
                )
            duplicate = next(
                (item for item in durable_records if item.content == normalized.content),
                None,
            )
            if duplicate is not None:
                return MemoryAddResult(
                    status=MemoryStatus.DUPLICATE,
                    record=normalized,
                    existing=duplicate,
                )

            working_conflict = next(
                (
                    item.record
                    for item in self._working.values()
                    if item.record.source == normalized.source
                    and not _is_expired(item.record, current_time)
                    and item.record.content != normalized.content
                ),
                None,
            )
            if working_conflict is not None:
                return MemoryAddResult(
                    status=MemoryStatus.CONFLICT,
                    record=normalized,
                    conflicting=working_conflict,
                )
            working_duplicate = next(
                (
                    item.record
                    for item in self._working.values()
                    if item.record.source == normalized.source
                    and not _is_expired(item.record, current_time)
                    and item.record.content == normalized.content
                ),
                None,
            )
            if working_duplicate is not None:
                return MemoryAddResult(
                    status=MemoryStatus.DUPLICATE,
                    record=normalized,
                    existing=working_duplicate,
                )

            existing_id = self._working.get(normalized.memory_id)
            if existing_id is not None:
                if existing_id.record.content != normalized.content:
                    return MemoryAddResult(
                        status=MemoryStatus.CONFLICT,
                        record=normalized,
                        conflicting=existing_id.record,
                    )
                return MemoryAddResult(
                    status=MemoryStatus.DUPLICATE,
                    record=existing_id.record,
                    existing=existing_id.record,
                )
            if len(self._working) >= self._max_working_records:
                expired_ids = [
                    memory_id
                    for memory_id, entry in self._working.items()
                    if _is_expired(entry.record, current_time)
                ]
                for memory_id in expired_ids:
                    del self._working[memory_id]
            if len(self._working) >= self._max_working_records:
                raise MemoryPromotionError("working memory limit exceeded")
            self._working[normalized.memory_id] = MemoryEntry(record=normalized)
            return MemoryAddResult(status=MemoryStatus.ADDED, record=normalized)

    def promote(
        self,
        record_or_id: MemoryRecord | str,
        *,
        provenance_id: str | None = None,
        now: datetime | None = None,
    ) -> MemoryRecord:
        current_time = _now(now)
        with self._lock:
            candidate: MemoryRecord | None
            if isinstance(record_or_id, MemoryRecord):
                candidate = _validated_record(record_or_id)
                if candidate.memory_id not in self._working:
                    candidate_result = self.try_add(candidate, now=current_time)
                    if candidate_result.status is MemoryStatus.CONFLICT:
                        conflicting = candidate_result.conflicting
                        if conflicting is None:
                            raise MemoryConflictError(candidate, candidate)
                        raise MemoryConflictError(candidate, conflicting)
                else:
                    candidate = self._working[candidate.memory_id].record
            elif isinstance(record_or_id, str):
                working = self._working.get(record_or_id)
                candidate = None if working is None else working.record
                if candidate is None:
                    candidate = self._repository.get(
                        record_or_id,
                        now=current_time,
                        include_expired=True,
                    )
            else:
                raise TypeError("record_or_id must be a MemoryRecord or memory_id")
            if candidate is None:
                raise MemoryNotFoundError(record_or_id)
            if provenance_id is not None:
                candidate = MemoryRecord.model_validate(
                    candidate.model_copy(update={"provenance_id": provenance_id}).model_dump(
                        mode="python"
                    )
                )
            if self._require_provenance_on_promotion and candidate.provenance_id is None:
                raise MemoryPromotionError("durable memory requires provenance")

            durable_records = self._active_durable_records(candidate.source, current_time)
            conflicting = next(
                (item for item in durable_records if item.content != candidate.content),
                None,
            )
            if conflicting is not None:
                raise MemoryConflictError(candidate, conflicting)
            working_conflict = next(
                (
                    item.record
                    for item in self._working.values()
                    if item.record.memory_id != candidate.memory_id
                    and item.record.source == candidate.source
                    and not _is_expired(item.record, current_time)
                    and item.record.content != candidate.content
                ),
                None,
            )
            if working_conflict is not None:
                raise MemoryConflictError(candidate, working_conflict)

            self._repository.put(candidate)
            self._working[candidate.memory_id] = MemoryEntry(record=candidate, durable=True)
            return candidate

    promote_record = promote

    def get(
        self,
        memory_id: str,
        *,
        now: datetime | None = None,
        include_expired: bool = False,
    ) -> MemoryRecord | None:
        if not isinstance(memory_id, str) or not memory_id.strip():
            raise ValueError("memory_id must be nonempty")
        current_time = _now(now)
        with self._lock:
            working = self._working.get(memory_id)
            if working is not None:
                if include_expired or not _is_expired(working.record, current_time):
                    return working.record
            return self._repository.get(
                memory_id,
                now=current_time,
                include_expired=include_expired,
            )

    def list(
        self,
        *,
        source: str | None = None,
        now: datetime | None = None,
        include_working: bool = True,
        durable_only: bool = False,
        limit: int = 1_000,
    ) -> tuple[MemoryRecord, ...]:
        current_time = _now(now)
        if isinstance(limit, bool) or not isinstance(limit, int):
            raise TypeError("limit must be an integer")
        if limit < 1:
            raise ValueError("limit must be positive")
        with self._lock:
            records: dict[str, MemoryRecord] = {}
            if not durable_only:
                for entry in self._working.values():
                    if source is not None and entry.record.source != source:
                        continue
                    if _is_expired(entry.record, current_time):
                        continue
                    records[entry.record.memory_id] = entry.record
            for record in self._active_durable_records(source, current_time):
                records[record.memory_id] = record
            ordered = sorted(records.values(), key=lambda item: (item.created_at, item.memory_id))
            return tuple(ordered[:limit])

    def list_active(
        self,
        *,
        source: str | None = None,
        now: datetime | None = None,
        include_working: bool = True,
        durable_only: bool = False,
        limit: int = 1_000,
    ) -> tuple[MemoryRecord, ...]:
        return self.list(
            source=source,
            now=now,
            include_working=include_working,
            durable_only=durable_only,
            limit=limit,
        )

    def list_durable(
        self,
        *,
        source: str | None = None,
        now: datetime | None = None,
        limit: int = 1_000,
    ) -> tuple[MemoryRecord, ...]:
        return self.list(
            source=source,
            now=now,
            include_working=False,
            durable_only=True,
            limit=limit,
        )

    def list_working(
        self,
        *,
        source: str | None = None,
        now: datetime | None = None,
        limit: int = 1_000,
    ) -> tuple[MemoryRecord, ...]:
        current_time = _now(now)
        with self._lock:
            records = [
                entry.record
                for entry in self._working.values()
                if not entry.durable
                and (source is None or entry.record.source == source)
                and not _is_expired(entry.record, current_time)
            ]
            records.sort(key=lambda item: (item.created_at, item.memory_id))
            return tuple(records[:limit])

    def detect_conflicts(
        self,
        record: MemoryRecord,
        *,
        now: datetime | None = None,
    ) -> tuple[MemoryRecord, ...]:
        normalized = _validated_record(record)
        current_time = _now(now)
        with self._lock:
            candidates = list(self._active_durable_records(normalized.source, current_time))
            candidates.extend(
                entry.record
                for entry in self._working.values()
                if entry.record.source == normalized.source
                and not _is_expired(entry.record, current_time)
            )
            return tuple(
                sorted(
                    {
                        item.memory_id: item
                        for item in candidates
                        if item.content != normalized.content
                    }.values(),
                    key=lambda item: (item.created_at, item.memory_id),
                )
            )

    def discard_working(self, memory_id: str) -> bool:
        with self._lock:
            entry = self._working.get(memory_id)
            if entry is None or entry.durable:
                return False
            del self._working[memory_id]
            return True

    def delete(self, memory_id: str) -> bool:
        with self._lock:
            self._working.pop(memory_id, None)
            return self._repository.delete(memory_id)

    def _active_durable_records(
        self,
        source: str | None,
        now: datetime,
    ) -> tuple[MemoryRecord, ...]:
        return tuple(
            record
            for record in self._repository.list(source=source, now=now, include_expired=False)
            if not _is_expired(record, now)
        )


__all__ = (
    "MemoryAddResult",
    "MemoryConflictError",
    "MemoryEntry",
    "MemoryError",
    "MemoryManager",
    "MemoryNotFoundError",
    "MemoryPromotionError",
    "MemoryRecord",
    "MemoryStatus",
    "MemoryValidationError",
)
