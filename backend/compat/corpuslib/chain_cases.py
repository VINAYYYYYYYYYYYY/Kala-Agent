"""Event-chain and schema-v2 SQLite fixtures.

The database is built by the live ``Database``, ``EventJournal``,
``SnapshotStore``, and ``DesignSpecRepository`` classes. Migration row
timestamps are the one non-deterministic value SQLite schema creation writes,
and are stabilized to a fixed value inside the fixture after migration; the
normalization is declared in this file's output and repeated in the corpus
README.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Final

from corpuslib.common import sha256_bytes, sha256_text

FIXED_MIGRATION_APPLIED_AT: Final[str] = "2026-01-01T00:00:00.000000Z"
SCHEMA_FILE_NAME: Final[str] = "schema_v2.sqlite3"


def _chain_preimage_cases() -> list[dict[str, object]]:
    from kala_agent.persistence.chain import (
        GENESIS_PREVIOUS_DIGEST,
        EventChainFields,
        chain_digest,
    )

    definitions: list[tuple[str, dict[str, object]]] = [
        (
            "workflow_absent",
            {
                "sequence": 1,
                "event_type": "design.created",
                "aggregate_id": "design-corpus",
                "workflow_id": None,
                "run_id": None,
                "occurred_at": "2026-01-01T00:00:00.000000Z",
                "payload_digest": "0" * 64,
                "previous_event_digest": GENESIS_PREVIOUS_DIGEST,
            },
        ),
        (
            "workflow_empty_collapses_with_absent",
            {
                "sequence": 1,
                "event_type": "design.created",
                "aggregate_id": "design-corpus",
                "workflow_id": "",
                "run_id": "",
                "occurred_at": "2026-01-01T00:00:00.000000Z",
                "payload_digest": "0" * 64,
                "previous_event_digest": GENESIS_PREVIOUS_DIGEST,
            },
        ),
        (
            "workflow_and_run_present",
            {
                "sequence": 2,
                "event_type": "workflow.started",
                "aggregate_id": "flow-corpus",
                "workflow_id": "flow-corpus",
                "run_id": "run-1",
                "occurred_at": "2026-01-01T00:00:01.000000Z",
                "payload_digest": "a" * 64,
                "previous_event_digest": "b" * 64,
            },
        ),
        (
            "unicode_parts",
            {
                "sequence": 7,
                "event_type": "事件.记录",
                "aggregate_id": "设计-😀",
                "workflow_id": "流程",
                "run_id": "运行",
                "occurred_at": "2026-01-01T00:00:07.000000Z",
                "payload_digest": "c" * 64,
                "previous_event_digest": "d" * 64,
            },
        ),
        (
            "long_parts_exercise_eight_byte_length_prefixes",
            {
                "sequence": 9,
                "event_type": "e" * 300,
                "aggregate_id": "g" * 300,
                "workflow_id": "w" * 300,
                "run_id": "r" * 300,
                "occurred_at": "2026-01-01T00:00:09.000000Z",
                "payload_digest": "e" * 64,
                "previous_event_digest": "f" * 64,
            },
        ),
    ]
    cases: list[dict[str, object]] = []
    for case_id, values in definitions:
        fields = EventChainFields(**values)  # type: ignore[arg-type]
        cases.append(
            {
                "id": case_id,
                "fields": values,
                "as_preimage_parts": list(fields.as_preimage_parts()),
                "chain_digest": chain_digest(fields),
            }
        )
    return cases


def _stabilize_migration_timestamps(path: Path) -> None:
    connection = sqlite3.connect(path)
    try:
        connection.execute(
            "UPDATE schema_migrations SET applied_at = ?",
            (FIXED_MIGRATION_APPLIED_AT,),
        )
        connection.commit()
    finally:
        connection.close()


def build_database(path: Path) -> dict[str, object]:
    """Create the schema-v2 database fixture and return its logical record."""
    from datetime import UTC, datetime, timedelta

    from kala_agent.contracts import DesignSpec
    from kala_agent.persistence.database import Database, SnapshotStore
    from kala_agent.persistence.journal import EventJournal
    from kala_agent.persistence.repositories import DesignSpecRepository

    if path.exists():
        path.unlink()

    base = datetime(2026, 1, 1, tzinfo=UTC)
    event_definitions: list[dict[str, object]] = [
        {
            "event_type": "design.created",
            "aggregate_id": "design-corpus",
            "payload": {
                "big_integer": 2**70,
                "name": "Corpus design",
                "negative_zero": -0.0,
                "ratio": 1e-05,
                "unicode": "κ 😀",
            },
            "workflow_id": None,
            "run_id": None,
        },
        {
            "event_type": "domain.evaluated",
            "aggregate_id": "design-corpus",
            "payload": {
                "blocked_step_id": None,
                "completed_step_ids": ["step.one", "step.two"],
                "evaluation_count": 2,
                "passed": True,
                "score": 0.910823,
            },
            "workflow_id": None,
            "run_id": None,
        },
        {
            "event_type": "workflow.started",
            "aggregate_id": "flow-corpus",
            "payload": {
                "definition_id": "mechanical.reference",
                "definition_version": "0.1.0",
                "offline": True,
                "provider": "local",
                "step_ids": ["step.one", "step.two"],
            },
            "workflow_id": "flow-corpus",
            "run_id": None,
        },
        {
            "event_type": "tool.called",
            "aggregate_id": "flow-corpus",
            "payload": {
                "artifact_bytes": 0,
                "call_id": "call-corpus-1",
                "nested": {"a": [1, 2.0, None], "b": False},
                "tool_id": "reference.simulator",
            },
            "workflow_id": "flow-corpus",
            "run_id": "run-1",
        },
        {
            "event_type": "workflow.completed",
            "aggregate_id": "flow-corpus",
            "payload": {
                "revision_id": "rev-corpus-design-r0",
                "status": "completed",
            },
            "workflow_id": "flow-corpus",
            "run_id": "run-1",
        },
    ]

    database = Database(path)
    try:
        journal = EventJournal(database)
        events: list[dict[str, object]] = []
        for index, definition in enumerate(event_definitions):
            event = journal.append(
                str(definition["event_type"]),
                str(definition["aggregate_id"]),
                definition["payload"],  # type: ignore[arg-type]
                workflow_id=definition["workflow_id"],  # type: ignore[arg-type]
                run_id=definition["run_id"],  # type: ignore[arg-type]
                timestamp=base + timedelta(seconds=index),
            )
            events.append(
                {
                    "sequence": event.sequence,
                    "event_type": event.event_type,
                    "aggregate_id": event.aggregate_id,
                    "workflow_id": event.workflow_id,
                    "run_id": event.run_id,
                    "payload_json": event.payload_json,
                    "digest": event.digest,
                    "occurred_at": event.timestamp.isoformat(timespec="microseconds").replace(
                        "+00:00", "Z"
                    ),
                    "previous_event_digest": event.previous_event_digest,
                    "event_digest": event.event_digest,
                }
            )

        snapshot = SnapshotStore(database).save(
            "snapshot-flow-corpus-000001",
            "flow-corpus",
            {
                "completed_step_ids": ["step.one"],
                "sequence": 1,
                "status": "running",
                "workflow_id": "flow-corpus",
            },
            created_at=base + timedelta(seconds=10),
        )
        DesignSpecRepository(database).save(
            DesignSpec(
                id="design-corpus",
                name="Corpus design",
                summary="Fixture design specification",
                attributes_json='{"domain":"mechanical","nested":{"b":2,"a":1}}',
                requirement_ids=("req-2", "req-1"),
                constraints=("c1",),
            )
        )
        schema_version = database.schema_version
        journal_verification = journal.verify_chain()
        with database.serialized() as connection:
            journal_mode = str(connection.execute("PRAGMA journal_mode").fetchone()[0])
            foreign_keys = int(connection.execute("PRAGMA foreign_keys").fetchone()[0])
    finally:
        database.close()

    _stabilize_migration_timestamps(path)

    connection = sqlite3.connect(path)
    connection.row_factory = sqlite3.Row
    try:
        master_rows = [
            {
                "type": str(row["type"]),
                "name": str(row["name"]),
                "tbl_name": str(row["tbl_name"]),
                "sql": None if row["sql"] is None else str(row["sql"]),
                "sql_normalized": None
                if row["sql"] is None
                else " ".join(str(row["sql"]).split()),
            }
            for row in connection.execute(
                "SELECT type, name, tbl_name, sql FROM sqlite_master"
                " ORDER BY type, name, tbl_name"
            )
        ]
        migration_rows = [
            {
                "version": int(row["version"]),
                "name": str(row["name"]),
                "applied_at": str(row["applied_at"]),
            }
            for row in connection.execute(
                "SELECT version, name, applied_at FROM schema_migrations ORDER BY version"
            )
        ]
        trigger_rows = [
            {"name": str(row["name"]), "sql_normalized": " ".join(str(row["sql"]).split())}
            for row in connection.execute(
                "SELECT name, sql FROM sqlite_master WHERE type = 'trigger' ORDER BY name"
            )
        ]
        table_names = [
            str(row["name"])
            for row in connection.execute(
                "SELECT name FROM sqlite_master"
                " WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
            )
        ]
        table_counts = {
            name: int(connection.execute(f'SELECT COUNT(*) FROM "{name}"').fetchone()[0])
            for name in table_names
        }
        sqlite_sequence_rows = [
            {"name": str(row["name"]), "seq": int(row["seq"])}
            for row in connection.execute("SELECT name, seq FROM sqlite_sequence ORDER BY name")
        ]
        event_rows = [
            dict(row)
            for row in connection.execute(
                "SELECT sequence, event_type, aggregate_id, workflow_id, run_id,"
                " payload_json, digest, occurred_at, previous_event_digest, event_digest"
                " FROM events ORDER BY sequence"
            )
        ]
        snapshot_rows = [
            dict(row)
            for row in connection.execute(
                "SELECT sequence, snapshot_id, workflow_id, state_json, digest, created_at"
                " FROM workflow_snapshots ORDER BY sequence"
            )
        ]
        spec_rows = [
            dict(row)
            for row in connection.execute("SELECT id, model_json FROM design_specs ORDER BY id")
        ]
    finally:
        connection.close()

    schema_entries = [
        [row["type"], row["name"], row["tbl_name"], row["sql_normalized"]]
        for row in master_rows
    ]
    schema_digest = sha256_text(
        __import__("json").dumps(
            schema_entries, ensure_ascii=True, separators=(",", ":"), sort_keys=True
        )
    )

    data = path.read_bytes()
    try:
        path.chmod(0o644)
    except OSError:
        pass
    return {
        "database_file": SCHEMA_FILE_NAME,
        "database_size_bytes": len(data),
        "database_sha256": sha256_bytes(data),
        "database_sha256_role": (
            "informational: SQLite file bytes are not asserted stable across SQLite "
            "library versions; logical rows and digests below are the contract"
        ),
        "sqlite_library_version_at_generation": sqlite3.sqlite_version,
        "schema_version": schema_version,
        "journal_mode": journal_mode,
        "foreign_keys": foreign_keys,
        "foreign_keys_note": (
            "SQLite foreign_keys is a per-connection PRAGMA set by Database, not "
            "a property stored in the file; a reader must enable it itself"
        ),
        "schema_entries": master_rows,
        "schema_digest": schema_digest,
        "table_count": sum(1 for row in master_rows if row["type"] == "table"),
        "schema_digest_definition": (
            "sha256 of sorted [type,name,tbl_name,whitespace-normalized sql] entries"
        ),
        "triggers": trigger_rows,
        "migrations": migration_rows,
        "table_counts": table_counts,
        "sqlite_sequence_rows": sqlite_sequence_rows,
        "events": events,
        "stored_event_rows": event_rows,
        "snapshot": {
            "sequence": snapshot.sequence,
            "snapshot_id": snapshot.snapshot_id,
            "workflow_id": snapshot.workflow_id,
            "state_json": snapshot.state_json,
            "digest": snapshot.digest,
            "created_at": snapshot.created_at.isoformat(timespec="microseconds").replace(
                "+00:00", "Z"
            ),
        },
        "stored_snapshot_rows": snapshot_rows,
        "stored_design_spec_rows": spec_rows,
        "verified_event_count": journal_verification.event_count,
        "verified_head_sequence": journal_verification.head_sequence,
        "verified_head_event_digest": journal_verification.head_event_digest,
        "normalizations": [
            {
                "field": "schema_migrations.applied_at",
                "value": FIXED_MIGRATION_APPLIED_AT,
                "reason": (
                    "Database.initialize writes the wall clock at migration time; the "
                    "fixture replaces both rows after migration so the file is stable"
                ),
            }
        ],
    }


def build(database_path: Path) -> dict[str, object]:
    from kala_agent.persistence.chain import (
        CHAIN_ALGORITHM,
        CHAIN_DOMAIN,
        GENESIS_PREVIOUS_DIGEST,
    )

    database = build_database(database_path)
    return {
        "corpus_schema_version": 1,
        "kind": "event_chain",
        "chain": {
            "algorithm": CHAIN_ALGORITHM,
            "domain": CHAIN_DOMAIN,
            "genesis_previous_digest": GENESIS_PREVIOUS_DIGEST,
            "length_prefix_bytes": 8,
            "length_prefix_byte_order": "big",
            "covers_domain": True,
            "covered_parts": [
                "sequence",
                "event_type",
                "aggregate_id",
                "workflow_id or empty",
                "run_id or empty",
                "occurred_at",
                "payload_digest",
                "previous_event_digest",
            ],
            "payload_digest_definition": "sha256(utf-8 bytes of payload_json)",
        },
        "preimage_cases": _chain_preimage_cases(),
        "database": database,
        "verification_recipe": {
            "python": (
                "Database(path); EventJournal(database).verify_chain(); compare the "
                "returned head digest with verified_head_event_digest"
            ),
            "python_version_note": (
                "The fixture was created by CPython 3.14.8 with sqlite3 3.54.0; "
                "the logical row values are the portable contract"
            ),
        },
    }


__all__ = ("SCHEMA_FILE_NAME", "build", "build_database")
