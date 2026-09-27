from __future__ import annotations

import hashlib
import os
import sqlite3
import threading
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest
from pydantic import ValidationError

from kala_agent import Settings
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
from kala_agent.persistence import (
    GENESIS_PREVIOUS_DIGEST,
    ApprovalRequestRepository,
    ContentAddressedArtifactStore,
    Database,
    DesignRevisionRepository,
    DesignSpecRepository,
    EventJournal,
    MemoryRecord,
    MemoryRepository,
    SnapshotStore,
    ToolCallRepository,
    ToolResultRepository,
)
from kala_agent.persistence.repositories import HeadMovedError

RECORDED_AT = datetime(2026, 1, 2, 3, 4, 5, tzinfo=UTC)
DIGEST_A = "a" * 64
DIGEST_B = "b" * 64


def _design_spec() -> DesignSpec:
    return DesignSpec(
        id="spec-1",
        name="Fixture design",
        summary="A persisted design.",
        parameter_schema_digest=DIGEST_A,
        attributes_json='{"z":2,"a":1}',
        requirement_ids=("requirement-1",),
        constraints=("Remain local-first.",),
    )


def _design_revision(sequence: int = 0) -> DesignRevision:
    return DesignRevision(
        revision_id=f"revision-{sequence}",
        design_id="design-1",
        sequence=sequence,
        spec=_design_spec(),
        provenance_id="provenance-1",
        created_at=RECORDED_AT + timedelta(minutes=sequence),
    )


def _approval(status: ApprovalStatus = ApprovalStatus.PENDING) -> ApprovalRequest:
    if status is ApprovalStatus.PENDING:
        return ApprovalRequest(
            request_id="approval-1",
            checkpoint_type=ApprovalKind.EGRESS,
            status=status,
            workflow_id="workflow-1",
            action_digest=DIGEST_A,
            state_digest=DIGEST_B,
            options=("approve", "deny"),
            created_at=RECORDED_AT,
        )
    return ApprovalRequest(
        request_id="approval-1",
        checkpoint_type=ApprovalKind.EGRESS,
        status=status,
        workflow_id="workflow-1",
        action_digest=DIGEST_A,
        state_digest=DIGEST_B,
        options=("approve", "deny"),
        selected_option_id="approve",
        created_at=RECORDED_AT,
        decided_at=RECORDED_AT + timedelta(minutes=1),
        actor_id="reviewer-1",
    )


def _tool_call() -> ToolCall:
    return ToolCall(
        call_id="call-1",
        tool_id="tool-1",
        tool_version="1.0.0",
        arguments_json='{"value":2,"enabled":true}',
        requested_capabilities=("compute.local",),
        side_effect_level=SideEffectLevel.READ,
        timeout_seconds=30.0,
        issued_at=RECORDED_AT,
    )


def _tool_result() -> ToolResult:
    return ToolResult(
        call_id="call-1",
        tool_id="tool-1",
        status=ToolRunStatus.SUCCEEDED,
        result_json='{"value":4}',
        started_at=RECORDED_AT,
        finished_at=RECORDED_AT + timedelta(milliseconds=25),
        duration_ms=25.0,
    )


def test_database_initializes_schema_pragmas_and_reopens(tmp_path: Path) -> None:
    settings = Settings(data_dir=tmp_path / "configured")
    database_path = settings.database_path

    with Database.from_settings(settings, busy_timeout_ms=7_500) as database:
        assert database.schema_version == 2
        assert database.connection.execute("PRAGMA foreign_keys").fetchone()[0] == 1
        assert database.connection.execute("PRAGMA journal_mode").fetchone()[0] == "wal"
        assert database.connection.execute("PRAGMA busy_timeout").fetchone()[0] == 7_500
        migration = database.connection.execute(
            "SELECT version, name FROM schema_migrations"
        ).fetchall()
        assert [(row["version"], row["name"]) for row in migration] == [
            (1, "initial_persistence_schema"),
            (2, "journal_event_hash_chain"),
        ]
        tables = {
            str(row["name"])
            for row in database.connection.execute(
                "SELECT name FROM sqlite_master WHERE type = ?",
                ("table",),
            ).fetchall()
        }
        assert {
            "events",
            "design_specs",
            "design_revisions",
            "approval_requests",
            "tool_calls",
            "tool_results",
            "memory_entries",
            "workflow_snapshots",
        } <= tables
        triggers = {
            str(row["name"])
            for row in database.connection.execute(
                "SELECT name FROM sqlite_master WHERE type = ? AND tbl_name = ?",
                ("trigger", "events"),
            ).fetchall()
        }
        assert triggers == {
            "events_prevent_update",
            "events_prevent_delete",
            "events_require_event_chain",
        }
        assert [
            str(row["name"])
            for row in database.connection.execute("PRAGMA table_info(events)").fetchall()
        ] == [
            "sequence",
            "event_type",
            "aggregate_id",
            "workflow_id",
            "run_id",
            "payload_json",
            "digest",
            "occurred_at",
            "previous_event_digest",
            "event_digest",
        ]
        EventJournal(database).append("workflow.started", "workflow-1", {"z": 1, "a": 2})

    with Database(database_path) as reopened:
        events = EventJournal(reopened).read()
        assert [event.sequence for event in events] == [1]
        assert events[0].event_digest == EventJournal(reopened).verify_chain().head_event_digest
        assert reopened.schema_version == 2


def test_database_uses_private_directory_and_file_modes(tmp_path: Path) -> None:
    database_path = tmp_path / "private" / "kala.sqlite3"
    with Database(database_path):
        assert (tmp_path / "private").stat().st_mode & 0o777 == 0o700
        assert database_path.stat().st_mode & 0o777 == 0o600


def test_transaction_rolls_back_and_supports_atomic_nested_savepoints() -> None:
    with Database() as database:
        journal = EventJournal(database)
        with pytest.raises(RuntimeError, match="rollback"):
            with database.transaction():
                journal.append("workflow.started", "workflow-1", {"attempt": 1})
                raise RuntimeError("rollback")
        assert journal.read() == ()
        assert journal.append("workflow.started", "workflow-1", {"attempt": 2}).sequence == 1

        with database.transaction():
            journal.append("workflow.checked", "workflow-1", {"step": 1})
            with pytest.raises(RuntimeError, match="nested rollback"):
                with database.transaction():
                    journal.append("workflow.checked", "workflow-1", {"step": 2})
                    raise RuntimeError("nested rollback")
        assert [event.payload["step"] for event in journal.read(event_type="workflow.checked")] == [
            1
        ]


def test_concurrent_transactions_serialize_one_writer_and_lose_no_acknowledged_event() -> None:
    thread_count = 8
    appends_per_thread = 40
    with Database() as database:
        journal = EventJournal(database)
        barrier = threading.Barrier(thread_count)
        guard = threading.Lock()
        failures: list[BaseException] = []
        acknowledged: list[tuple[int, int, int]] = []
        active_transactions = 0
        peak_transactions = 0

        def writer(thread_index: int) -> None:
            nonlocal active_transactions, peak_transactions
            barrier.wait()
            for append_index in range(appends_per_thread):
                try:
                    with database.transaction():
                        with guard:
                            active_transactions += 1
                            peak_transactions = max(peak_transactions, active_transactions)
                        event = journal.append(
                            "step.started",
                            f"step-{thread_index}",
                            {"thread": thread_index, "append": append_index},
                        )
                        with guard:
                            active_transactions -= 1
                    with guard:
                        acknowledged.append((thread_index, append_index, event.sequence))
                except BaseException as error:
                    with guard:
                        failures.append(error)
                    return

        threads = [
            threading.Thread(target=writer, args=(thread_index,), name=f"writer-{thread_index}")
            for thread_index in range(thread_count)
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()

        assert failures == []
        assert peak_transactions == 1
        assert not database.connection.in_transaction
        assert len(acknowledged) == thread_count * appends_per_thread
        assert sorted(
            (thread_index, append_index) for thread_index, append_index, _ in acknowledged
        ) == [
            (thread_index, append_index)
            for thread_index in range(thread_count)
            for append_index in range(appends_per_thread)
        ]
        sequences_by_thread: dict[int, list[int]] = {}
        for thread_index, _, sequence in acknowledged:
            sequences_by_thread.setdefault(thread_index, []).append(sequence)
        for sequences in sequences_by_thread.values():
            assert sequences == sorted(sequences)
            assert len(set(sequences)) == len(sequences)

        events = journal.read(limit=None)
        assert len(events) == thread_count * appends_per_thread
        assert [event.sequence for event in events] == list(
            range(1, thread_count * appends_per_thread + 1)
        )
        assert journal.verify_chain().head_event_digest == events[-1].event_digest
        for event in events:
            thread_index = int(event.aggregate_id.removeprefix("step-"))
            assert event.payload["thread"] == thread_index
            assert 0 <= int(str(event.payload["append"])) < appends_per_thread
            assert event.aggregate_id == f"step-{thread_index}"


def test_event_journal_orders_filters_and_verifies_canonical_payloads() -> None:
    with Database() as database:
        journal = EventJournal(database)
        first = journal.append(
            "step.started",
            "step-1",
            '{"z":2,"a":1}',
            workflow_id="workflow-1",
            run_id="run-1",
            timestamp=RECORDED_AT,
        )
        second = journal.append(
            "step.finished",
            "step-1",
            {"result": "ok"},
            workflow_id="workflow-1",
            run_id="run-1",
            timestamp=RECORDED_AT + timedelta(seconds=1),
        )
        third = journal.append(
            "step.finished",
            "step-2",
            {"result": "skipped"},
            workflow_id="workflow-2",
            run_id="run-2",
            timestamp=RECORDED_AT + timedelta(seconds=2),
        )

        assert [event.sequence for event in journal.read()] == [1, 2, 3]
        assert first.payload_json == '{"a":1,"z":2}'
        assert first.payload == {"a": 1, "z": 2}
        assert first.digest == hashlib.sha256(first.payload_json.encode("utf-8")).hexdigest()
        assert first.timestamp == RECORDED_AT
        assert first.previous_event_digest == GENESIS_PREVIOUS_DIGEST
        assert second.previous_event_digest == first.event_digest
        assert third.previous_event_digest == second.event_digest
        verification = journal.verify_chain()
        assert verification.event_count == 3
        assert verification.head_sequence == 3
        assert verification.head_event_digest == third.event_digest
        assert [event.sequence for event in journal.read(event_type="step.finished")] == [2, 3]
        assert [
            event.sequence
            for event in journal.read(
                workflow_id="workflow-1",
                run_id="run-1",
                aggregate_id="step-1",
                after_sequence=1,
            )
        ] == [2]
        assert journal.get(2) == second
        assert journal.get(third.sequence + 1) is None


def test_event_journal_rejects_invalid_input_and_enforces_append_only() -> None:
    with Database() as database:
        journal = EventJournal(database)
        event = journal.append("step.started", "step-1", {"value": 1})

        with pytest.raises(ValueError, match="event_type"):
            journal.append("bad type;DROP TABLE events", "step-1", {})
        with pytest.raises(ValueError, match="workflow_id"):
            journal.append("step.started", "step-1", {}, workflow_id="../workflow")
        with pytest.raises(ValueError, match="JSON object"):
            journal.append("step.started", "step-1", "[]")
        with pytest.raises(ValueError, match="JSON object"):
            journal.append("step.started", "step-1", {"unsupported": object()})

        with pytest.raises(sqlite3.DatabaseError, match="append-only"):
            database.connection.execute(
                "UPDATE events SET event_type = ? WHERE sequence = ?",
                ("changed", event.sequence),
            )
        with pytest.raises(sqlite3.DatabaseError, match="append-only"):
            database.connection.execute(
                "DELETE FROM events WHERE sequence = ?",
                (event.sequence,),
            )
        with pytest.raises(sqlite3.DatabaseError, match="hash chain"):
            database.connection.execute(
                """
                INSERT INTO events(
                    event_type, aggregate_id, workflow_id, run_id, payload_json,
                    digest, occurred_at
                ) VALUES (?, ?, NULL, NULL, ?, ?, ?)
                """,
                (
                    "step.started",
                    "step-1",
                    "{}",
                    hashlib.sha256(b"{}").hexdigest(),
                    "2026-01-02T03:04:05.000000Z",
                ),
            )
        assert journal.get(event.sequence) == event


def test_artifact_store_is_atomic_content_addressed_and_size_limited(tmp_path: Path) -> None:
    settings = Settings(data_dir=tmp_path / "configured")
    store = ContentAddressedArtifactStore.from_settings(settings, max_object_size=32)
    digest = store.put_text("hello κόσμε")
    expected_digest = hashlib.sha256("hello κόσμε".encode()).hexdigest()

    assert digest == expected_digest
    assert store.get_text(digest) == "hello κόσμε"
    assert store.get_bytes(digest) == "hello κόσμε".encode()
    assert store.get_size(digest) == len("hello κόσμε".encode())
    assert store.contains(digest)
    assert store.put_bytes("hello κόσμε".encode()) == digest
    assert store.key_for(digest) == f"objects/{digest[:2]}/{digest}"
    assert ".." not in Path(store.key_for(digest)).parts

    with pytest.raises(ValueError, match="size limit"):
        store.put_bytes(b"x" * 33)
    with pytest.raises(ValueError, match="SHA-256"):
        store.get_bytes("../outside")
    with pytest.raises(FileNotFoundError):
        store.get_bytes(DIGEST_A)


def test_artifact_publication_failure_leaves_no_partial_object(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store = ContentAddressedArtifactStore(tmp_path / "artifacts", max_object_size=32)
    data = b"atomic artifact"
    digest = hashlib.sha256(data).hexdigest()

    def fail_link(source: str, destination: str, **options: object) -> None:
        raise OSError("publication failed")

    monkeypatch.setattr(os, "link", fail_link)
    with pytest.raises(OSError, match="publication failed"):
        store.put_bytes(data)
    assert not store.contains(digest)
    assert not tuple(store.root.rglob("*.tmp"))

    monkeypatch.undo()
    assert store.put_bytes(data) == digest
    assert store.get_bytes(digest) == data


def test_artifact_store_rejects_traversal_tampering_and_symlinks(tmp_path: Path) -> None:
    store = ContentAddressedArtifactStore(tmp_path / "artifacts", max_object_size=32)
    data = b"safe artifact"
    digest = hashlib.sha256(data).hexdigest()
    store.put_bytes(data)
    target = store.root / "objects" / digest[:2] / digest

    target.write_bytes(b"tampered")
    with pytest.raises(ValueError, match="digest verification"):
        store.get_bytes(digest)
    with pytest.raises(ValueError, match="digest verification"):
        store.put_bytes(data)

    target.unlink()
    outside = tmp_path / "outside.bin"
    outside.write_bytes(data)
    target.symlink_to(outside)
    with pytest.raises(ValueError, match="symlink"):
        store.get_bytes(digest)
    with pytest.raises(ValueError, match="symlink"):
        store.put_bytes(data)

    other_root = tmp_path / "other-artifacts"
    other_store = ContentAddressedArtifactStore(other_root, max_object_size=32)
    outside_directory = tmp_path / "outside-directory"
    outside_directory.mkdir()
    bucket_digest = hashlib.sha256(b"bucket").hexdigest()
    outside_object = outside_directory / bucket_digest
    outside_object.write_bytes(b"bucket")
    (other_store.root / "objects" / bucket_digest[:2]).symlink_to(
        outside_directory,
        target_is_directory=True,
    )
    with pytest.raises(ValueError, match="symlink"):
        other_store.get_bytes(bucket_digest)

    linked_root = tmp_path / "linked-root"
    linked_root.symlink_to(outside_directory, target_is_directory=True)
    with pytest.raises(ValueError, match="root cannot be a symlink"):
        ContentAddressedArtifactStore(linked_root, max_object_size=32)


def test_typed_repositories_round_trip_pydantic_contracts(tmp_path: Path) -> None:
    with Database(tmp_path / "repository.sqlite3") as database:
        spec_repository = DesignSpecRepository(database)
        revision_repository = DesignRevisionRepository(database)
        approval_repository = ApprovalRequestRepository(database)
        call_repository = ToolCallRepository(database)
        result_repository = ToolResultRepository(database)

        spec = _design_spec()
        revision = _design_revision()
        approval = _approval()
        call = _tool_call()
        result = _tool_result()
        spec_repository.save(spec)
        revision_repository.commit_revision(revision, expected_head_revision_id=None)
        approval_repository.save(approval)
        call_repository.save(call)
        result_repository.save(result)

        assert spec_repository.get(spec.id) == spec
        assert spec_repository.list() == (spec,)
        assert revision_repository.get(revision.revision_id) == revision
        assert revision_repository.list_for_design(revision.design_id) == (revision,)
        assert revision_repository.latest_for_design(revision.design_id) == revision
        assert approval_repository.get(approval.request_id) == approval
        assert approval_repository.list_for_workflow(approval.workflow_id) == (approval,)
        assert call_repository.get(call.call_id) == call
        assert call_repository.list_for_tool(call.tool_id) == (call,)
        assert result_repository.get(result.call_id) == result

        approved = _approval(ApprovalStatus.APPROVED)
        approval_repository.save(approved)
        assert approval_repository.get(approved.request_id) == approved


def test_memory_repository_uses_validated_ids_provenance_and_expiry(tmp_path: Path) -> None:
    with Database(tmp_path / "memory.sqlite3") as database:
        repository = MemoryRepository(database)
        record = MemoryRecord(
            memory_id="memory-1",
            content="Remember the local database location.",
            source="conversation-1",
            provenance_id="provenance-1",
            metadata_json='{"priority":"high","labels":["local","db"]}',
            created_at=RECORDED_AT,
            expires_at=RECORDED_AT + timedelta(hours=1),
        )
        repository.put(record)

        assert repository.get(record.memory_id, now=RECORDED_AT) == record
        assert repository.list(source=record.source, now=RECORDED_AT) == (record,)
        assert repository.get(record.memory_id, now=record.expires_at) is None
        assert (
            repository.get(
                record.memory_id,
                now=record.expires_at,
                include_expired=True,
            )
            == record
        )
        assert repository.list(now=record.expires_at) == ()
        assert repository.list(now=record.expires_at, include_expired=True) == (record,)
        assert repository.delete(record.memory_id)
        assert repository.get(record.memory_id, include_expired=True) is None

    with pytest.raises(ValidationError):
        MemoryRecord(
            memory_id="../memory",
            content="Unsafe.",
            source="source-1",
            created_at=RECORDED_AT,
        )
    with pytest.raises(ValidationError):
        MemoryRecord(
            memory_id="memory-2",
            content="Unsafe provenance.",
            source="source-1",
            provenance_id="../provenance",
            created_at=RECORDED_AT,
        )
    with pytest.raises(ValidationError, match="later"):
        MemoryRecord(
            memory_id="memory-3",
            content="Already expired.",
            source="source-1",
            created_at=RECORDED_AT,
            expires_at=RECORDED_AT,
        )


def test_repositories_reject_inconsistent_related_records_atomically() -> None:
    with Database() as database:
        revision_repository = DesignRevisionRepository(database)
        revision_repository.commit_revision(_design_revision(0), expected_head_revision_id=None)
        conflicting_revision = DesignRevision(
            revision_id="revision-conflict",
            design_id="design-1",
            sequence=0,
            spec=_design_spec(),
            created_at=RECORDED_AT,
        )
        with pytest.raises(HeadMovedError) as excinfo:
            revision_repository.commit_revision(
                conflicting_revision,
                expected_head_revision_id=None,
            )
        assert excinfo.value.expected_head_revision_id is None
        assert excinfo.value.actual_head_revision_id == "revision-0"
        assert excinfo.value.actual_sequence == 0
        assert revision_repository.get(conflicting_revision.revision_id) is None
        assert revision_repository.latest_for_design("design-1") == _design_revision(0)

        with pytest.raises(HeadMovedError) as excinfo:
            revision_repository.commit_revision(
                _design_revision(0).model_copy(update={"revision_id": "revision-forked"}),
                expected_head_revision_id="revision-9",
            )
        assert excinfo.value.expected_head_revision_id == "revision-9"
        assert revision_repository.get("revision-forked") is None

        second_design = DesignRevision(
            revision_id="revision-2",
            design_id="design-2",
            sequence=0,
            spec=_design_spec(),
            created_at=RECORDED_AT,
        )
        revision_repository.commit_revision(second_design, expected_head_revision_id=None)
        assert revision_repository.list_for_design(second_design.design_id) == (second_design,)

        result_repository = ToolResultRepository(database)
        with pytest.raises(ValueError, match="existing tool call"):
            result_repository.save(_tool_result())


def test_the_design_revisions_table_refuses_two_rows_at_one_sequence() -> None:
    with Database() as database:
        first = _design_revision(0)
        with database.transaction() as connection:
            connection.execute(
                """
                INSERT INTO design_specs(id, model_json) VALUES (?, ?)
                """,
                (first.spec.id, first.spec.model_dump_json()),
            )
            connection.execute(
                """
                INSERT INTO design_revisions(
                    revision_id, design_id, sequence, model_json, created_at
                ) VALUES (?, ?, ?, ?, ?)
                """,
                (
                    first.revision_id,
                    first.design_id,
                    first.sequence,
                    first.model_dump_json(),
                    RECORDED_AT.isoformat(),
                ),
            )
        with pytest.raises(sqlite3.IntegrityError):
            with database.transaction() as connection:
                connection.execute(
                    """
                    INSERT INTO design_revisions(
                        revision_id, design_id, sequence, model_json, created_at
                    ) VALUES (?, ?, ?, ?, ?)
                    """,
                    (
                        "revision-duplicate-sequence",
                        first.design_id,
                        first.sequence,
                        _design_revision(1).model_dump_json(),
                        RECORDED_AT.isoformat(),
                    ),
                )
        assert DesignRevisionRepository(database).latest_for_design("design-1") == first


def test_snapshot_save_and_restore_validates_canonical_json(tmp_path: Path) -> None:
    with Database(tmp_path / "snapshot.sqlite3") as database:
        snapshots = SnapshotStore(database)
        first = snapshots.save(
            "snapshot-1",
            "workflow-1",
            '{"z":2,"a":1}',
            created_at=RECORDED_AT,
        )
        snapshots.save(
            "snapshot-2",
            "workflow-1",
            {"state": "running", "step": 2},
            created_at=RECORDED_AT + timedelta(minutes=1),
        )

        assert first.state_json == '{"a":1,"z":2}'
        assert first.digest == hashlib.sha256(first.state_json.encode("utf-8")).hexdigest()
        assert snapshots.restore(first.snapshot_id) == '{"a":1,"z":2}'
        assert snapshots.restore_latest("workflow-1") == '{"state":"running","step":2}'
        assert snapshots.latest("workflow-1") == snapshots.get("snapshot-2")
        assert snapshots.restore("missing-snapshot") is None
        assert snapshots.restore_latest("missing-workflow") is None

        with pytest.raises(ValueError, match="JSON object"):
            snapshots.save("snapshot-3", "workflow-1", "[]")
        with pytest.raises(ValueError, match="workflow_id"):
            snapshots.save("snapshot-3", "../workflow", {})
        with pytest.raises(sqlite3.IntegrityError):
            snapshots.save("snapshot-1", "workflow-1", {})


def test_database_rejects_invalid_configuration_and_closed_access(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="busy_timeout_ms"):
        Database(busy_timeout_ms=-1)
    with pytest.raises(TypeError, match="busy_timeout_ms"):
        Database(busy_timeout_ms=True)

    database = Database(tmp_path / "closed.sqlite3")
    database.close()
    assert database.closed
    database.close()
    with pytest.raises(RuntimeError, match="closed"):
        _ = database.connection


def test_the_revision_repository_offers_no_unconditional_write_path() -> None:
    with Database() as database:
        repository = DesignRevisionRepository(database)
        assert not hasattr(repository, "save")
        assert callable(repository.commit_revision)
        with pytest.raises(TypeError, match="DesignRevision"):
            repository.commit_revision(
                "not-a-revision",  # type: ignore[arg-type]
                expected_head_revision_id=None,
            )
