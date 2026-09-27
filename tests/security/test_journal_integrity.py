"""Tamper evidence for the append-only event journal.

The journal is chained, so removing, reordering, or editing a stored event
breaks every digest from the edit onwards. These tests are written from the
position of an attacker who already has direct write access to the database
file, which is the only position from which the journal is protected at all:
they disable the append-only triggers the way a file-level attacker would, and
require that the tampering is still detected afterwards.
"""

from __future__ import annotations

import hashlib
import sqlite3
from collections.abc import Sequence
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

from kala_agent.persistence import (
    GENESIS_PREVIOUS_DIGEST,
    Database,
    EventChainFields,
    EventJournal,
    JournalIntegrityError,
    chain_digest,
)
from kala_agent.persistence.database import _STATE_MIGRATION, _timestamp_to_iso

RECORDED_AT = datetime(2026, 1, 2, 3, 4, 5, tzinfo=UTC)
TAMPERED_PAYLOAD = '{"a":1,"b":2}'
TAMPERED_PAYLOAD_DIGEST = hashlib.sha256(TAMPERED_PAYLOAD.encode("utf-8")).hexdigest()

TAMPERED_HEAD: tuple[tuple[str, str, tuple[object, ...], str], ...] = (
    (
        "sequence",
        "UPDATE events SET sequence = ? WHERE sequence = 3",
        (4,),
        "continuity is broken",
    ),
    (
        "event_type",
        "UPDATE events SET event_type = ? WHERE sequence = 3",
        ("step.tampered",),
        "failed chain verification",
    ),
    (
        "aggregate_id",
        "UPDATE events SET aggregate_id = ? WHERE sequence = 3",
        ("step-tampered",),
        "failed chain verification",
    ),
    (
        "workflow_id",
        "UPDATE events SET workflow_id = ? WHERE sequence = 3",
        ("workflow-tampered",),
        "failed chain verification",
    ),
    (
        "run_id",
        "UPDATE events SET run_id = ? WHERE sequence = 3",
        ("run-tampered",),
        "failed chain verification",
    ),
    (
        "occurred_at",
        "UPDATE events SET occurred_at = ? WHERE sequence = 3",
        ("2020-01-01T00:00:00.000000Z",),
        "failed chain verification",
    ),
    (
        "payload_digest",
        "UPDATE events SET payload_json = ?, digest = ? WHERE sequence = 3",
        (TAMPERED_PAYLOAD, TAMPERED_PAYLOAD_DIGEST),
        "failed chain verification",
    ),
    (
        "payload_without_digest",
        "UPDATE events SET payload_json = ? WHERE sequence = 3",
        (TAMPERED_PAYLOAD,),
        "failed payload verification",
    ),
    (
        "previous_event_digest",
        "UPDATE events SET previous_event_digest = ? WHERE sequence = 3",
        (GENESIS_PREVIOUS_DIGEST,),
        "does not link",
    ),
    (
        "event_digest",
        "UPDATE events SET event_digest = ? WHERE sequence = 3",
        ("0" * 64,),
        "failed chain verification",
    ),
    (
        "event_digest_format",
        "UPDATE events SET event_digest = ? WHERE sequence = 3",
        ("not-a-digest",),
        "does not record a valid chain digest",
    ),
)


def _seeded_journal(database_path: Path) -> tuple[Database, EventJournal]:
    database = Database(database_path)
    journal = EventJournal(database)
    for index in range(3):
        journal.append(
            f"step.{index}",
            f"step-{index}",
            {"index": index},
            workflow_id="workflow-1",
            run_id="run-1",
            timestamp=RECORDED_AT + timedelta(seconds=index),
        )
    return database, journal


def _tamper(
    database_path: Path | str, statements: Sequence[tuple[str, tuple[object, ...]]]
) -> None:
    with Database(database_path) as attacker:
        attacker.connection.execute("DROP TRIGGER events_prevent_update")
        attacker.connection.execute("DROP TRIGGER events_prevent_delete")
        for statement, parameters in statements:
            attacker.connection.execute(statement, parameters)


def test_journal_chains_every_event_to_its_predecessor(tmp_path: Path) -> None:
    database, journal = _seeded_journal(tmp_path / "journal.sqlite3")
    with database:
        events = journal.read()
        assert [event.sequence for event in events] == [1, 2, 3]
        assert events[0].previous_event_digest == GENESIS_PREVIOUS_DIGEST
        assert [event.previous_event_digest for event in events[1:]] == [
            event.event_digest for event in events[:-1]
        ]
        verification = journal.verify_chain()
        assert verification.event_count == 3
        assert verification.head_sequence == 3
        assert verification.head_event_digest == events[-1].event_digest
        assert journal.get(2) == events[1]


def test_empty_journal_verifies_without_a_head(tmp_path: Path) -> None:
    with Database(tmp_path / "empty.sqlite3") as database:
        verification = EventJournal(database).verify_chain()
        assert verification.event_count == 0
        assert verification.head_sequence is None
        assert verification.head_event_digest is None


@pytest.mark.parametrize(
    ("field", "statement", "parameters", "message"),
    TAMPERED_HEAD,
    ids=[case[0] for case in TAMPERED_HEAD],
)
def test_every_covered_field_is_bound_by_the_chain(
    tmp_path: Path,
    field: str,
    statement: str,
    parameters: tuple[object, ...],
    message: str,
) -> None:
    database, journal = _seeded_journal(tmp_path / f"tampered-{field}.sqlite3")
    with database:
        _tamper(database.path, ((statement, parameters),))
        with pytest.raises(JournalIntegrityError, match=message):
            journal.verify_chain()
        with pytest.raises(JournalIntegrityError, match=message):
            journal.read()
        with pytest.raises(JournalIntegrityError, match=message):
            journal.get(3)


def test_an_omitted_event_is_detected(tmp_path: Path) -> None:
    database, journal = _seeded_journal(tmp_path / "omitted.sqlite3")
    with database:
        _tamper(database.path, (("DELETE FROM events WHERE sequence = ?", (2,)),))
        with pytest.raises(JournalIntegrityError, match="continuity is broken"):
            journal.read()
        with pytest.raises(JournalIntegrityError, match="continuity is broken"):
            journal.verify_chain()


def test_a_reordered_history_is_detected(tmp_path: Path) -> None:
    database, journal = _seeded_journal(tmp_path / "reordered.sqlite3")
    with database:
        _swap_adjacent_events(database.path, 2, 3)
        with pytest.raises(JournalIntegrityError, match="event 2 failed chain verification"):
            journal.read()


def _swap_adjacent_events(database_path: Path | str, first: int, second: int) -> None:
    swapped_columns = (
        "event_type",
        "aggregate_id",
        "payload_json",
        "digest",
        "occurred_at",
    )
    assignments = ", ".join(
        f"{column} = (SELECT {column} FROM swapped WHERE sequence = ?)"
        for column in swapped_columns
    )
    with Database(database_path) as attacker:
        attacker.connection.execute("DROP TRIGGER events_prevent_update")
        attacker.connection.execute("DROP TRIGGER events_prevent_delete")
        attacker.connection.execute(
            f"CREATE TABLE swapped AS SELECT sequence, {', '.join(swapped_columns)}"
            " FROM events WHERE sequence IN (?, ?)",
            (first, second),
        )
        for source, target in ((second, first), (first, second)):
            attacker.connection.execute(
                f"UPDATE events SET {assignments} WHERE sequence = ?",
                (*(source for _ in swapped_columns), target),
            )
        attacker.connection.execute("DROP TABLE swapped")


def test_appending_to_a_tampered_journal_fails_closed(tmp_path: Path) -> None:
    database, journal = _seeded_journal(tmp_path / "append-tampered.sqlite3")
    with database:
        _tamper(
            database.path,
            (
                (
                    "UPDATE events SET occurred_at = ? WHERE sequence = 3",
                    ("2020-01-01T00:00:00.000000Z",),
                ),
            ),
        )
        with pytest.raises(JournalIntegrityError, match="event 3 failed chain verification"):
            journal.append("step.3", "step-3", {"index": 3})
        with pytest.raises(JournalIntegrityError):
            journal.verify_chain()


def test_startup_refuses_a_journal_that_was_tampered_while_closed(tmp_path: Path) -> None:
    database_path = tmp_path / "startup.sqlite3"
    database, journal = _seeded_journal(database_path)
    expected_head = journal.verify_chain().head_event_digest
    database.close()
    assert database.closed

    with Database(database_path) as reopened:
        assert reopened.schema_version == 2
        _tamper(database_path, (("UPDATE events SET run_id = ? WHERE sequence = 1", ("run-x",)),))
        with pytest.raises(JournalIntegrityError, match="event 1 failed chain verification"):
            EventJournal(reopened)
    assert expected_head is not None


def test_a_rewritten_history_keeps_a_verifiable_chain_but_a_different_head(
    tmp_path: Path,
) -> None:
    database, journal = _seeded_journal(tmp_path / "rewritten.sqlite3")
    with database:
        original_head = journal.verify_chain().head_event_digest
        with Database(database.path) as attacker:
            attacker.connection.execute("DROP TRIGGER events_prevent_update")
            attacker.connection.execute(
                "UPDATE events SET payload_json = ?, digest = ? WHERE sequence = 2",
                (TAMPERED_PAYLOAD, TAMPERED_PAYLOAD_DIGEST),
            )
            _rechain(attacker.connection)

        rewritten = EventJournal(database)
        verification = rewritten.verify_chain()
        assert verification.event_count == 3
        assert verification.head_event_digest != original_head
        edited = rewritten.get(2)
        assert edited is not None
        assert edited.payload == {"a": 1, "b": 2}


def _rechain(connection: sqlite3.Connection) -> None:
    rows = connection.execute(
        """
        SELECT sequence, event_type, aggregate_id, workflow_id, run_id, digest, occurred_at
        FROM events
        ORDER BY sequence ASC
        """
    ).fetchall()
    previous_event_digest = GENESIS_PREVIOUS_DIGEST
    for row in rows:
        event_digest = chain_digest(
            EventChainFields(
                sequence=int(row["sequence"]),
                event_type=str(row["event_type"]),
                aggregate_id=str(row["aggregate_id"]),
                workflow_id=None if row["workflow_id"] is None else str(row["workflow_id"]),
                run_id=None if row["run_id"] is None else str(row["run_id"]),
                occurred_at=str(row["occurred_at"]),
                payload_digest=str(row["digest"]),
                previous_event_digest=previous_event_digest,
            )
        )
        connection.execute(
            """
            UPDATE events
            SET previous_event_digest = ?, event_digest = ?
            WHERE sequence = ?
            """,
            (previous_event_digest, event_digest, int(row["sequence"])),
        )
        previous_event_digest = event_digest


def test_migration_chains_a_pre_existing_journal_and_refuses_a_corrupt_one(
    tmp_path: Path,
) -> None:
    healthy_path = tmp_path / "pre-chain.sqlite3"
    _build_pre_chain_journal(healthy_path)
    with Database(healthy_path) as database:
        assert database.schema_version == 2
        journal = EventJournal(database)
        events = journal.read()
        assert [event.sequence for event in events] == [1, 2, 3]
        assert [event.payload for event in events] == [{"a": 1}, {"a": 2}, {"a": 3}]
        assert events[0].previous_event_digest == GENESIS_PREVIOUS_DIGEST
        assert [event.previous_event_digest for event in events[1:]] == [
            event.event_digest for event in events[:-1]
        ]
        appended = journal.append("step.migrated", "step-3", {"a": 4})
        assert appended.previous_event_digest == events[-1].event_digest
        assert journal.verify_chain().head_event_digest == appended.event_digest

    corrupt_path = tmp_path / "pre-chain-corrupt.sqlite3"
    _build_pre_chain_journal(corrupt_path, corrupt=True)
    with pytest.raises(JournalIntegrityError, match="failed payload digest verification"):
        Database(corrupt_path)
    with sqlite3.connect(corrupt_path) as connection:
        assert connection.execute("SELECT MAX(version) FROM schema_migrations").fetchone()[0] == 1
        assert [row[1] for row in connection.execute("PRAGMA table_info(events)").fetchall()] == [
            "sequence",
            "event_type",
            "aggregate_id",
            "workflow_id",
            "run_id",
            "payload_json",
            "digest",
            "occurred_at",
        ]


def _build_pre_chain_journal(database_path: Path, *, corrupt: bool = False) -> None:
    connection = sqlite3.connect(database_path, isolation_level=None)
    try:
        connection.execute(
            """
            CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL
            )
            """
        )
        for step in _STATE_MIGRATION[0].steps:
            assert isinstance(step, str)
            connection.execute(step)
        connection.execute(
            "INSERT INTO schema_migrations(version, name, applied_at) VALUES (1, ?, ?)",
            ("initial_persistence_schema", _timestamp_to_iso(RECORDED_AT)),
        )
        for index in range(3):
            payload = f'{{"a":{index + 1}}}'
            connection.execute(
                """
                INSERT INTO events(
                    event_type, aggregate_id, workflow_id, run_id, payload_json,
                    digest, occurred_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    f"step.{index}",
                    f"step-{index}",
                    "workflow-1",
                    "run-1",
                    payload,
                    hashlib.sha256(payload.encode("utf-8")).hexdigest(),
                    _timestamp_to_iso(RECORDED_AT + timedelta(seconds=index)),
                ),
            )
        if corrupt:
            connection.execute("DROP TRIGGER events_prevent_update")
            connection.execute(
                "UPDATE events SET payload_json = ? WHERE sequence = 2",
                ('{"a":99}',),
            )
    finally:
        connection.close()
