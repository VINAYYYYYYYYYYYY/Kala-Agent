#!/usr/bin/env python3
"""Test-only Python oracle for the native typed repositories.

This script imports the live product implementation from ``src/kala_agent`` and
is spawned by the Rust integration tests. It is never used by the native
library at production time.

Modes:
  * ``seed <path>``: create a fresh database with representative specs,
    revisions, approvals, calls, results and memory records through the live
    Python repositories, then print every raw row and wire model.
  * ``seed-large <path>``: create a fresh database with one memory record whose
    metadata is 80 levels deep, contains a 1,000-digit integer and is larger
    than 1 MiB.
  * ``inspect <path>``: reopen any schema-v2 database, read every repository
    through the live Python implementation, re-serialize every model and report
    whether the bytes match the stored ``model_json``.
"""

from __future__ import annotations

import json
import sys
from datetime import UTC, datetime, timedelta
from pathlib import Path

from kala_agent.contracts import (
    ApprovalKind,
    ApprovalRequest,
    ApprovalStatus,
    DesignRevision,
    DesignSpec,
    SideEffectLevel,
    ToolCall,
    ToolResult,
    ToolRunStatus,
)
from kala_agent.persistence.database import Database
from kala_agent.persistence.repositories import (
    ApprovalRequestRepository,
    DesignRevisionRepository,
    DesignSpecRepository,
    MemoryRecord,
    MemoryRepository,
    ToolCallRepository,
    ToolResultRepository,
)

NOW = datetime(2026, 1, 2, 3, 4, 5, tzinfo=UTC)
DIGEST_A = "a" * 64
DIGEST_B = "b" * 64


def _spec() -> DesignSpec:
    return DesignSpec(
        id="spec-python",
        name="Café ☕ design",
        summary='A "quoted" \\ summary.',
        parameter_schema_digest=DIGEST_A,
        attributes_json='{"z":2,"a":{"nested":[1,true,null,"κ"]}}',
        requirement_ids=("requirement-2", "requirement-1"),
        constraints=("Remain local-first.", "Preserve data."),
    )


def _revision(sequence: int, spec: DesignSpec) -> DesignRevision:
    return DesignRevision(
        revision_id=f"revision-python-{sequence}",
        design_id="design-python",
        sequence=sequence,
        spec=spec,
        provenance_id="provenance-python",
        created_at=NOW + timedelta(minutes=sequence, microseconds=123_456),
    )


def _approval(status: ApprovalStatus) -> ApprovalRequest:
    base = {
        "request_id": "approval-python",
        "checkpoint_type": ApprovalKind.EGRESS,
        "workflow_id": "workflow-python",
        "action_digest": DIGEST_A,
        "state_digest": DIGEST_B,
        "options": ("approve", "deny"),
        "created_at": NOW,
    }
    if status is ApprovalStatus.PENDING:
        return ApprovalRequest(status=status, **base)
    return ApprovalRequest(
        status=status,
        selected_option_id="approve",
        decided_at=NOW + timedelta(minutes=1),
        actor_id="reviewer-python",
        **base,
    )


def _call() -> ToolCall:
    return ToolCall(
        call_id="call-python",
        tool_id="tool-python",
        tool_version="1.0.0",
        arguments_json='{"value":2,"enabled":true}',
        requested_capabilities=("compute.local",),
        side_effect_level=SideEffectLevel.READ,
        timeout_seconds=30.0,
        issued_at=NOW,
    )


def _result() -> ToolResult:
    return ToolResult(
        call_id="call-python",
        tool_id="tool-python",
        status=ToolRunStatus.SUCCEEDED,
        result_json='{"value":4}',
        started_at=NOW,
        finished_at=NOW + timedelta(milliseconds=25),
        duration_ms=25.0,
    )


def _memory(memory_id: str, *, expired: bool = False, offset: bool = False) -> MemoryRecord:
    created = NOW
    if offset:
        created = datetime.fromisoformat("2026-01-02T08:34:05+05:30")
    expires = created + timedelta(hours=1)
    if expired:
        created = NOW - timedelta(hours=2)
        expires = created + timedelta(hours=1)
    return MemoryRecord(
        memory_id=memory_id,
        content="  Remember the local database location.  ",
        source="conversation-python",
        provenance_id="provenance-python",
        metadata_json='{"priority":"high","labels":["local","db"],"nested":{"κ":"☕"}}',
        created_at=created,
        expires_at=expires,
    )


def _reset(path: Path) -> None:
    for suffix in ("", "-wal", "-shm"):
        candidate = Path(f"{path}{suffix}")
        if candidate.exists():
            candidate.unlink()


def _rows(database: Database) -> dict[str, list[dict[str, object]]]:
    queries = {
        "design_specs": "SELECT id, model_json FROM design_specs ORDER BY id",
        "design_revisions": (
            "SELECT revision_id, design_id, sequence, created_at, model_json "
            "FROM design_revisions ORDER BY design_id, sequence"
        ),
        "approval_requests": (
            "SELECT request_id, workflow_id, status, created_at, model_json "
            "FROM approval_requests ORDER BY request_id"
        ),
        "tool_calls": (
            "SELECT call_id, tool_id, issued_at, model_json "
            "FROM tool_calls ORDER BY call_id"
        ),
        "tool_results": (
            "SELECT call_id, tool_id, status, model_json "
            "FROM tool_results ORDER BY call_id"
        ),
        "memory_entries": (
            "SELECT memory_id, source, provenance_id, created_at, expires_at, model_json "
            "FROM memory_entries ORDER BY memory_id"
        ),
    }
    with database.serialized() as connection:
        return {
            table: [dict(row) for row in connection.execute(sql).fetchall()]
            for table, sql in queries.items()
        }


def _wire(database: Database) -> dict[str, list[str]]:
    specs = DesignSpecRepository(database)
    revisions = DesignRevisionRepository(database)
    approvals = ApprovalRequestRepository(database)
    calls = ToolCallRepository(database)
    results = ToolResultRepository(database)
    memory = MemoryRepository(database)

    with database.serialized() as connection:
        spec_ids = [str(row[0]) for row in connection.execute(
            "SELECT id FROM design_specs ORDER BY id"
        ).fetchall()]
        design_ids = [str(row[0]) for row in connection.execute(
            "SELECT DISTINCT design_id FROM design_revisions ORDER BY design_id"
        ).fetchall()]
        workflow_ids = [str(row[0]) for row in connection.execute(
            "SELECT DISTINCT workflow_id FROM approval_requests ORDER BY workflow_id"
        ).fetchall()]
        tool_ids = [str(row[0]) for row in connection.execute(
            "SELECT DISTINCT tool_id FROM tool_calls ORDER BY tool_id"
        ).fetchall()]
        call_ids = [str(row[0]) for row in connection.execute(
            "SELECT call_id FROM tool_calls ORDER BY call_id"
        ).fetchall()]
        memory_ids = [str(row[0]) for row in connection.execute(
            "SELECT memory_id FROM memory_entries ORDER BY memory_id"
        ).fetchall()]

    return {
        "specs": [specs.get(spec_id).model_dump_json() for spec_id in spec_ids],
        "revisions": [
            revision.model_dump_json()
            for design_id in design_ids
            for revision in revisions.list_for_design(design_id)
        ],
        "approvals": [
            approval.model_dump_json()
            for workflow_id in workflow_ids
            for approval in approvals.list_for_workflow(workflow_id)
        ],
        "calls": [
            call.model_dump_json()
            for tool_id in tool_ids
            for call in calls.list_for_tool(tool_id)
        ],
        "results": [
            result.model_dump_json()
            for call_id in call_ids
            if (result := results.get(call_id)) is not None
        ],
        "memories": [
            record.model_dump_json()
            for record in memory.list(now=NOW, include_expired=True)
        ],
        "active_memories": [
            record.model_dump_json() for record in memory.list(now=NOW)
        ],
        "memory_ids": memory_ids,
    }


def mode_seed(path: Path) -> None:
    _reset(path)
    with Database(path) as database:
        spec = _spec()
        DesignSpecRepository(database).save(spec)
        revisions = DesignRevisionRepository(database)
        first = _revision(0, spec)
        revisions.commit_revision(first, expected_head_revision_id=None)
        revisions.commit_revision(
            _revision(1, spec), expected_head_revision_id=first.revision_id
        )
        ApprovalRequestRepository(database).save(_approval(ApprovalStatus.PENDING))
        ApprovalRequestRepository(database).save(_approval(ApprovalStatus.APPROVED))
        ToolCallRepository(database).save(_call())
        ToolResultRepository(database).save(_result())
        memory = MemoryRepository(database)
        memory.put(_memory("memory-python-active"))
        memory.put(_memory("memory-python-offset", offset=True))
        memory.put(_memory("memory-python-expired", expired=True))
        payload = {
            "rows": _rows(database),
            "wire": _wire(database),
        }
    json.dump(payload, sys.stdout, ensure_ascii=True, separators=(",", ":"))


def mode_seed_large(path: Path) -> None:
    _reset(path)
    deep: object = 0
    for _ in range(80):
        deep = [deep]
    metadata = {
        "deep": deep,
        "big_integer": int("7" * 1_000),
        "large_text": "a" * 1_100_000,
    }
    record = MemoryRecord(
        memory_id="memory-python-large",
        content="large metadata",
        source="conversation-python",
        metadata_json=metadata,
        created_at=NOW,
        expires_at=NOW + timedelta(hours=1),
    )
    with Database(path) as database:
        MemoryRepository(database).put(record)
        payload = {
            "rows": _rows(database),
            "wire": _wire(database),
        }
    json.dump(payload, sys.stdout, ensure_ascii=True, separators=(",", ":"))


def mode_inspect(path: Path) -> None:
    with Database(path) as database:
        payload = {
            "rows": _rows(database),
            "wire": _wire(database),
        }
        # Every model read through the live repositories must re-serialize to
        # exactly the stored bytes. The natural primary key identifies each
        # row in its table.
        stored_specs = {row["id"]: row["model_json"] for row in payload["rows"]["design_specs"]}
        stored_revisions = {
            row["revision_id"]: row["model_json"]
            for row in payload["rows"]["design_revisions"]
        }
        stored_approvals = {
            row["request_id"]: row["model_json"]
            for row in payload["rows"]["approval_requests"]
        }
        stored_calls = {
            row["call_id"]: row["model_json"] for row in payload["rows"]["tool_calls"]
        }
        stored_results = {
            row["call_id"]: row["model_json"] for row in payload["rows"]["tool_results"]
        }
        stored_memory = {
            row["memory_id"]: row["model_json"]
            for row in payload["rows"]["memory_entries"]
        }
        revisions = DesignRevisionRepository(database)
        approvals = ApprovalRequestRepository(database)
        calls = ToolCallRepository(database)
        results = ToolResultRepository(database)
        memory = MemoryRepository(database)
        checks = []
        for spec_id, model_json in stored_specs.items():
            checks.append(
                DesignSpecRepository(database).get(spec_id).model_dump_json() == model_json
            )
        for revision_id, model_json in stored_revisions.items():
            checks.append(
                revisions.get(revision_id).model_dump_json() == model_json
            )
        for request_id, model_json in stored_approvals.items():
            checks.append(approvals.get(request_id).model_dump_json() == model_json)
        for call_id, model_json in stored_calls.items():
            checks.append(calls.get(call_id).model_dump_json() == model_json)
        for call_id, model_json in stored_results.items():
            checks.append(results.get(call_id).model_dump_json() == model_json)
        for memory_id, model_json in stored_memory.items():
            checks.append(
                memory.get(memory_id, now=NOW, include_expired=True).model_dump_json()
                == model_json
            )
        payload["raw_round_trip"] = all(checks)
        payload["raw_round_trip_count"] = len(checks)
    json.dump(payload, sys.stdout, ensure_ascii=True, separators=(",", ":"))


def main() -> None:
    mode = sys.argv[1]
    path = Path(sys.argv[2])
    if mode == "seed":
        mode_seed(path)
    elif mode == "seed-large":
        mode_seed_large(path)
    elif mode == "inspect":
        mode_inspect(path)
    else:
        raise SystemExit(f"unknown mode {mode}")


if __name__ == "__main__":
    main()
