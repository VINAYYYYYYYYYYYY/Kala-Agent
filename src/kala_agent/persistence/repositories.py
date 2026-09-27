from __future__ import annotations

import sqlite3
from datetime import UTC, datetime
from typing import ClassVar, Self, TypeVar

from pydantic import AwareDatetime, BaseModel, ConfigDict, model_validator

from kala_agent.contracts.models import (
    ApprovalRequest,
    CanonicalJsonObject,
    DesignRevision,
    DesignSpec,
    Identifier,
    NonEmptyText,
    ToolCall,
    ToolResult,
)
from kala_agent.persistence.database import (
    Database,
    _timestamp_to_iso,
    _validate_identifier,
)

ModelT = TypeVar("ModelT", bound=BaseModel)


class HeadMovedError(RuntimeError):
    """Raised when a design head is not the one a write expected.

    A compare-and-swap that has lost the race says so with this type, and the
    enclosing transaction is rolled back, so a caller never observes an orphan
    revision row for a head it did not extend. ``expected_sequence`` is the
    sequence the caller tried to write and ``actual_sequence`` is the sequence
    the head of the design holds.
    """

    def __init__(
        self,
        message: str,
        *,
        design_id: str,
        expected_head_revision_id: str | None,
        actual_head_revision_id: str | None,
        expected_sequence: int,
        actual_sequence: int,
    ) -> None:
        super().__init__(message)
        self.message = message
        self.design_id = design_id
        self.expected_head_revision_id = expected_head_revision_id
        self.actual_head_revision_id = actual_head_revision_id
        self.expected_sequence = expected_sequence
        self.actual_sequence = actual_sequence


class ApprovalAlreadyDecidedError(RuntimeError):
    """Raised when a write would change an approval that already carries a decision."""


class MemoryRecord(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid", frozen=True)

    memory_id: Identifier
    content: NonEmptyText
    source: Identifier
    provenance_id: Identifier | None = None
    metadata_json: CanonicalJsonObject = "{}"
    created_at: AwareDatetime
    expires_at: AwareDatetime | None = None

    @model_validator(mode="after")
    def validate_expiry(self) -> Self:
        if self.expires_at is not None and self.expires_at <= self.created_at:
            raise ValueError("expires_at must be later than created_at")
        return self


def _serialize(model: BaseModel) -> str:
    return model.model_dump_json()


def _deserialize(model_type: type[ModelT], model_json: str) -> ModelT:
    return model_type.model_validate_json(model_json)


def _validate_limit(limit: int) -> int:
    if isinstance(limit, bool) or not isinstance(limit, int):
        raise TypeError("limit must be an integer")
    if limit < 1:
        raise ValueError("limit must be positive")
    return limit


class DesignSpecRepository:
    def __init__(self, database: Database) -> None:
        self._database = database

    def save(self, spec: DesignSpec) -> None:
        if not isinstance(spec, DesignSpec):
            raise TypeError("spec must be a DesignSpec")
        with self._database.transaction() as connection:
            connection.execute(
                """
                INSERT INTO design_specs(id, model_json) VALUES (?, ?)
                ON CONFLICT(id) DO UPDATE SET model_json = excluded.model_json
                """,
                (spec.id, _serialize(spec)),
            )

    def get(self, spec_id: str) -> DesignSpec | None:
        normalized_spec_id = _validate_identifier(spec_id, "spec_id")
        with self._database.serialized() as connection:
            row = connection.execute(
                "SELECT model_json FROM design_specs WHERE id = ?",
                (normalized_spec_id,),
            ).fetchone()
        return None if row is None else _deserialize(DesignSpec, str(row["model_json"]))

    def list(self, *, limit: int = 1_000) -> tuple[DesignSpec, ...]:
        normalized_limit = _validate_limit(limit)
        with self._database.serialized() as connection:
            rows = connection.execute(
                "SELECT model_json FROM design_specs ORDER BY id LIMIT ?",
                (normalized_limit,),
            ).fetchall()
        return tuple(_deserialize(DesignSpec, str(row["model_json"])) for row in rows)


class DesignRevisionRepository:
    """The append-only revision store, whose only write path is a head swap.

    There is deliberately no unconditional save. A revision row names the design
    it extends and the sequence it occupies, and writing one without first
    proving the head is still the one the writer prepared against is what forges
    a fork or an orphan. :meth:`commit_revision` is therefore the only way in,
    and the insertion it performs is private to this class.
    """

    def __init__(self, database: Database) -> None:
        self._database = database

    def commit_revision(
        self,
        revision: DesignRevision,
        *,
        expected_head_revision_id: str | None,
    ) -> DesignRevision:
        """Append one revision onto the head the caller prepared against.

        This is the only way a revision reaches the store, and the expected head
        is a required argument rather than an assumption, so a caller cannot
        write without stating what it believes the head is. The head read, the
        sequence assignment, and the insert run in one immediate transaction, so
        two writers that prepared against the same head cannot both extend it:
        the loser is told the head moved and its transaction is rolled back
        before anything is written, which is why a failed compare-and-swap never
        leaves a revision row behind.
        """
        if not isinstance(revision, DesignRevision):
            raise TypeError("revision must be a DesignRevision")
        expected_head = (
            None
            if expected_head_revision_id is None
            else _validate_identifier(expected_head_revision_id, "expected_head_revision_id")
        )
        with self._database.transaction() as connection:
            head = self._head_locked(connection, revision.design_id)
            actual_head = None if head is None else str(head["revision_id"])
            if actual_head != expected_head:
                raise HeadMovedError(
                    f"the head of design {revision.design_id} moved before the revision was written",
                    design_id=revision.design_id,
                    expected_head_revision_id=expected_head,
                    actual_head_revision_id=actual_head,
                    expected_sequence=revision.sequence,
                    actual_sequence=0 if head is None else int(head["sequence"]),
                )
            sequence = 0 if head is None else int(head["sequence"]) + 1
            if revision.sequence != sequence:
                raise HeadMovedError(
                    f"revision {revision.revision_id} does not extend the head of design "
                    f"{revision.design_id}",
                    design_id=revision.design_id,
                    expected_head_revision_id=expected_head,
                    actual_head_revision_id=actual_head,
                    expected_sequence=revision.sequence,
                    actual_sequence=0 if head is None else int(head["sequence"]),
                )
            self._insert(connection, revision)
        return revision

    def get(self, revision_id: str) -> DesignRevision | None:
        normalized_revision_id = _validate_identifier(revision_id, "revision_id")
        with self._database.serialized() as connection:
            row = connection.execute(
                "SELECT model_json FROM design_revisions WHERE revision_id = ?",
                (normalized_revision_id,),
            ).fetchone()
        return None if row is None else _deserialize(DesignRevision, str(row["model_json"]))

    def list_for_design(
        self,
        design_id: str,
        *,
        limit: int = 1_000,
    ) -> tuple[DesignRevision, ...]:
        normalized_design_id = _validate_identifier(design_id, "design_id")
        normalized_limit = _validate_limit(limit)
        with self._database.serialized() as connection:
            rows = connection.execute(
                """
            SELECT model_json
            FROM design_revisions
            WHERE design_id = ?
            ORDER BY sequence ASC
            LIMIT ?
            """,
                (normalized_design_id, normalized_limit),
            ).fetchall()
        return tuple(_deserialize(DesignRevision, str(row["model_json"])) for row in rows)

    def latest_for_design(self, design_id: str) -> DesignRevision | None:
        normalized_design_id = _validate_identifier(design_id, "design_id")
        with self._database.serialized() as connection:
            row = connection.execute(
                """
            SELECT model_json
            FROM design_revisions
            WHERE design_id = ?
            ORDER BY sequence DESC
            LIMIT 1
            """,
                (normalized_design_id,),
            ).fetchone()
        return None if row is None else _deserialize(DesignRevision, str(row["model_json"]))

    @staticmethod
    def _head_locked(
        connection: sqlite3.Connection,
        design_id: str,
    ) -> sqlite3.Row | None:
        row: sqlite3.Row | None = connection.execute(
            """
            SELECT revision_id, sequence
            FROM design_revisions
            WHERE design_id = ?
            ORDER BY sequence DESC
            LIMIT 1
            """,
            (design_id,),
        ).fetchone()
        return row

    @staticmethod
    def _insert(connection: sqlite3.Connection, revision: DesignRevision) -> None:
        connection.execute(
            """
            INSERT INTO design_specs(id, model_json) VALUES (?, ?)
            ON CONFLICT(id) DO NOTHING
            """,
            (revision.spec.id, _serialize(revision.spec)),
        )
        connection.execute(
            """
            INSERT INTO design_revisions(
                revision_id, design_id, sequence, model_json, created_at
            ) VALUES (?, ?, ?, ?, ?)
            """,
            (
                revision.revision_id,
                revision.design_id,
                revision.sequence,
                _serialize(revision),
                _timestamp_to_iso(revision.created_at),
            ),
        )


class ApprovalRequestRepository:
    def __init__(self, database: Database) -> None:
        self._database = database

    def save(self, request: ApprovalRequest) -> None:
        """Insert an approval, or update one that is still pending.

        A decided approval is final: rewriting it with the same record is a
        no-op, and any other write, including one that would reopen it as
        pending, is refused.
        """
        if not isinstance(request, ApprovalRequest):
            raise TypeError("request must be an ApprovalRequest")
        with self._database.transaction() as connection:
            cursor = connection.execute(
                """
                INSERT INTO approval_requests(
                    request_id, workflow_id, status, created_at, model_json
                ) VALUES (?, ?, ?, ?, ?)
                ON CONFLICT(request_id) DO UPDATE SET
                    workflow_id = excluded.workflow_id,
                    status = excluded.status,
                    created_at = excluded.created_at,
                    model_json = excluded.model_json
                WHERE approval_requests.status = 'pending'
                    OR approval_requests.model_json = excluded.model_json
                """,
                (
                    request.request_id,
                    request.workflow_id,
                    request.status.value,
                    _timestamp_to_iso(request.created_at),
                    _serialize(request),
                ),
            )
            if cursor.rowcount == 0:
                raise ApprovalAlreadyDecidedError(
                    f"approval {request.request_id} is already decided and cannot change"
                )

    def get(self, request_id: str) -> ApprovalRequest | None:
        normalized_request_id = _validate_identifier(request_id, "request_id")
        with self._database.serialized() as connection:
            row = connection.execute(
                "SELECT model_json FROM approval_requests WHERE request_id = ?",
                (normalized_request_id,),
            ).fetchone()
        return None if row is None else _deserialize(ApprovalRequest, str(row["model_json"]))

    def list_for_workflow(
        self,
        workflow_id: str,
        *,
        limit: int = 1_000,
    ) -> tuple[ApprovalRequest, ...]:
        normalized_workflow_id = _validate_identifier(workflow_id, "workflow_id")
        normalized_limit = _validate_limit(limit)
        with self._database.serialized() as connection:
            rows = connection.execute(
                """
            SELECT model_json
            FROM approval_requests
            WHERE workflow_id = ?
            ORDER BY created_at ASC, request_id ASC
            LIMIT ?
            """,
                (normalized_workflow_id, normalized_limit),
            ).fetchall()
        return tuple(_deserialize(ApprovalRequest, str(row["model_json"])) for row in rows)


class ToolCallRepository:
    def __init__(self, database: Database) -> None:
        self._database = database

    def save(self, call: ToolCall) -> None:
        if not isinstance(call, ToolCall):
            raise TypeError("call must be a ToolCall")
        with self._database.transaction() as connection:
            connection.execute(
                """
                INSERT INTO tool_calls(call_id, tool_id, issued_at, model_json)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(call_id) DO UPDATE SET
                    tool_id = excluded.tool_id,
                    issued_at = excluded.issued_at,
                    model_json = excluded.model_json
                """,
                (
                    call.call_id,
                    call.tool_id,
                    _timestamp_to_iso(call.issued_at),
                    _serialize(call),
                ),
            )

    def get(self, call_id: str) -> ToolCall | None:
        normalized_call_id = _validate_identifier(call_id, "call_id")
        with self._database.serialized() as connection:
            row = connection.execute(
                "SELECT model_json FROM tool_calls WHERE call_id = ?",
                (normalized_call_id,),
            ).fetchone()
        return None if row is None else _deserialize(ToolCall, str(row["model_json"]))

    def list_for_tool(
        self,
        tool_id: str,
        *,
        limit: int = 1_000,
    ) -> tuple[ToolCall, ...]:
        normalized_tool_id = _validate_identifier(tool_id, "tool_id")
        normalized_limit = _validate_limit(limit)
        with self._database.serialized() as connection:
            rows = connection.execute(
                """
            SELECT model_json
            FROM tool_calls
            WHERE tool_id = ?
            ORDER BY issued_at ASC, call_id ASC
            LIMIT ?
            """,
                (normalized_tool_id, normalized_limit),
            ).fetchall()
        return tuple(_deserialize(ToolCall, str(row["model_json"])) for row in rows)


class ToolResultRepository:
    def __init__(self, database: Database) -> None:
        self._database = database

    def save(self, result: ToolResult) -> None:
        if not isinstance(result, ToolResult):
            raise TypeError("result must be a ToolResult")
        with self._database.transaction() as connection:
            call_row = connection.execute(
                "SELECT tool_id FROM tool_calls WHERE call_id = ?",
                (result.call_id,),
            ).fetchone()
            if call_row is None:
                raise ValueError("tool result requires an existing tool call")
            if str(call_row["tool_id"]) != result.tool_id:
                raise ValueError("tool result tool_id must match its tool call")
            connection.execute(
                """
                INSERT INTO tool_results(call_id, tool_id, status, model_json)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(call_id) DO UPDATE SET
                    tool_id = excluded.tool_id,
                    status = excluded.status,
                    model_json = excluded.model_json
                """,
                (
                    result.call_id,
                    result.tool_id,
                    result.status.value,
                    _serialize(result),
                ),
            )

    def get(self, call_id: str) -> ToolResult | None:
        normalized_call_id = _validate_identifier(call_id, "call_id")
        with self._database.serialized() as connection:
            row = connection.execute(
                "SELECT model_json FROM tool_results WHERE call_id = ?",
                (normalized_call_id,),
            ).fetchone()
        return None if row is None else _deserialize(ToolResult, str(row["model_json"]))


class MemoryRepository:
    def __init__(self, database: Database) -> None:
        self._database = database

    def put(self, record: MemoryRecord) -> None:
        if not isinstance(record, MemoryRecord):
            raise TypeError("record must be a MemoryRecord")
        with self._database.transaction() as connection:
            connection.execute(
                """
                INSERT INTO memory_entries(
                    memory_id, source, provenance_id, created_at, expires_at, model_json
                ) VALUES (?, ?, ?, ?, ?, ?)
                ON CONFLICT(memory_id) DO UPDATE SET
                    source = excluded.source,
                    provenance_id = excluded.provenance_id,
                    created_at = excluded.created_at,
                    expires_at = excluded.expires_at,
                    model_json = excluded.model_json
                """,
                (
                    record.memory_id,
                    record.source,
                    record.provenance_id,
                    _timestamp_to_iso(record.created_at),
                    None if record.expires_at is None else _timestamp_to_iso(record.expires_at),
                    _serialize(record),
                ),
            )

    def get(
        self,
        memory_id: str,
        *,
        now: datetime | None = None,
        include_expired: bool = False,
    ) -> MemoryRecord | None:
        normalized_memory_id = _validate_identifier(memory_id, "memory_id")
        if include_expired:
            with self._database.serialized() as connection:
                row = connection.execute(
                    "SELECT model_json FROM memory_entries WHERE memory_id = ?",
                    (normalized_memory_id,),
                ).fetchone()
        else:
            current = _timestamp_to_iso(now if now is not None else datetime.now(UTC))
            with self._database.serialized() as connection:
                row = connection.execute(
                    """
                    SELECT model_json
                    FROM memory_entries
                    WHERE memory_id = ?
                      AND (expires_at IS NULL OR expires_at > ?)
                    """,
                    (normalized_memory_id, current),
                ).fetchone()
        return None if row is None else _deserialize(MemoryRecord, str(row["model_json"]))

    def list(
        self,
        *,
        source: str | None = None,
        now: datetime | None = None,
        include_expired: bool = False,
        limit: int = 1_000,
    ) -> tuple[MemoryRecord, ...]:
        normalized_source = None if source is None else _validate_identifier(source, "source")
        normalized_limit = _validate_limit(limit)
        current = _timestamp_to_iso(now if now is not None else datetime.now(UTC))
        if include_expired:
            with self._database.serialized() as connection:
                rows = connection.execute(
                    """
                    SELECT model_json
                    FROM memory_entries
                    WHERE (? IS NULL OR source = ?)
                    ORDER BY created_at ASC, memory_id ASC
                    LIMIT ?
                    """,
                    (normalized_source, normalized_source, normalized_limit),
                ).fetchall()
        else:
            with self._database.serialized() as connection:
                rows = connection.execute(
                    """
                    SELECT model_json
                    FROM memory_entries
                    WHERE (? IS NULL OR source = ?)
                      AND (expires_at IS NULL OR expires_at > ?)
                    ORDER BY created_at ASC, memory_id ASC
                    LIMIT ?
                    """,
                    (
                        normalized_source,
                        normalized_source,
                        current,
                        normalized_limit,
                    ),
                ).fetchall()
        return tuple(_deserialize(MemoryRecord, str(row["model_json"])) for row in rows)

    def delete(self, memory_id: str) -> bool:
        normalized_memory_id = _validate_identifier(memory_id, "memory_id")
        with self._database.transaction() as connection:
            cursor = connection.execute(
                "DELETE FROM memory_entries WHERE memory_id = ?",
                (normalized_memory_id,),
            )
        return cursor.rowcount == 1
