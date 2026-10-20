#!/usr/bin/env python3
"""Test-only oracle for deep persisted JSON and identifier whitespace.

This script is spawned only by the Rust integration tests. It imports the live
product implementation from ``src/kala_agent`` and never participates in the
native library's production path.

Modes:
  * ``canonical``: stdin is a JSON array of source strings; stdout is a JSON
    array of ``{"ok": true, "output": ..., "sha256": ..., "length": ...}`` or
    ``{"ok": false, "error": ...}`` from
    ``kala_agent.contracts.models._canonical_json_object``.
  * ``identifiers``: stdin is a JSON array of strings; stdout is the live
    ``_validate_identifier`` normalized value or its error.
  * ``create-deep <path> <depth> [tag]``: creates a fresh database with one
    deep event and one deep snapshot, then prints both records and the verified
    chain head.
  * ``append-deep <path> <depth> [tag]``: reopens and verifies a database,
    appends one deep event and snapshot, then prints the appended records and
    the new verified head.
  * ``inspect-deep <path> [workflow_id]``: reopens, fully verifies, and prints
    every event (canonical payload bytes, digests) and the latest snapshot.

Deep sources are built iteratively as strings before canonicalization; the
oracle never constructs a deep Python object graph itself. Errors are returned
verbatim so a host-specific CPython decoder/encoder recursion limit is visible
to the caller instead of being interpreted as a native success.
"""

from __future__ import annotations

import hashlib
import json
import sys
from datetime import UTC, datetime, timedelta
from pathlib import Path

from kala_agent.contracts.models import _canonical_json_object
from kala_agent.persistence.database import (
    Database,
    SnapshotStore,
    _timestamp_to_iso,
    _validate_identifier,
)
from kala_agent.persistence.journal import EventJournal

BASE = datetime(2026, 1, 1, tzinfo=UTC)

# A mixed leaf: supplementary/BMP Unicode, a large integer, negative zero,
# null. The backslash-u sequences are part of the JSON source text.
LEAF = '{"big":1180591620717411303424,"f":-0.0,"n":null,"u":"\\u03ba \\ud83d\\ude00"}'


def deep_source(depth: int, tag: str) -> str:
    """Build an alternating array/object chain iteratively."""
    prefix = '{"tag":' + json.dumps(tag, ensure_ascii=True) + ',"x":'
    closers: list[str] = []
    for level in range(depth):
        if level % 2 == 0:
            prefix += "["
            closers.append("]")
        else:
            prefix += '{"k":'
            closers.append("}")
    prefix += LEAF
    prefix += "".join(reversed(closers))
    prefix += "}"
    return prefix


def event_record(event: object) -> dict[str, object]:
    return {
        "sequence": event.sequence,
        "event_type": event.event_type,
        "aggregate_id": event.aggregate_id,
        "workflow_id": event.workflow_id,
        "run_id": event.run_id,
        "payload_json": event.payload_json,
        "digest": event.digest,
        "occurred_at": _timestamp_to_iso(event.timestamp),
        "previous_event_digest": event.previous_event_digest,
        "event_digest": event.event_digest,
    }


def snapshot_record(snapshot: object) -> dict[str, object]:
    return {
        "sequence": snapshot.sequence,
        "snapshot_id": snapshot.snapshot_id,
        "workflow_id": snapshot.workflow_id,
        "state_json": snapshot.state_json,
        "digest": snapshot.digest,
        "created_at": _timestamp_to_iso(snapshot.created_at),
    }


def verification_record(journal: EventJournal) -> dict[str, object]:
    verification = journal.verify_chain()
    return {
        "event_count": verification.event_count,
        "head_sequence": verification.head_sequence,
        "head_event_digest": verification.head_event_digest,
    }


def mode_canonical() -> None:
    sources = json.load(sys.stdin)
    results: list[dict[str, object]] = []
    for source in sources:
        try:
            output = _canonical_json_object(source)
            results.append(
                {
                    "ok": True,
                    "output": output,
                    "sha256": hashlib.sha256(output.encode("utf-8")).hexdigest(),
                    "length": len(output),
                }
            )
        except Exception as error:  # noqa: BLE001 - oracle reports the message
            results.append({"ok": False, "error": f"{type(error).__name__}: {error}"})
    json.dump(results, sys.stdout, ensure_ascii=True, separators=(",", ":"))
    sys.stdout.flush()


def mode_identifiers() -> None:
    values = json.load(sys.stdin)
    results: list[dict[str, object]] = []
    for value in values:
        try:
            results.append(
                {"ok": True, "output": _validate_identifier(value, "event_type")}
            )
        except Exception as error:  # noqa: BLE001 - oracle reports the message
            results.append({"ok": False, "error": f"{type(error).__name__}: {error}"})
    json.dump(results, sys.stdout, ensure_ascii=True, separators=(",", ":"))
    sys.stdout.flush()


def mode_create_deep(path: str, depth: int, tag: str) -> None:
    database_path = Path(path)
    if database_path.exists():
        database_path.unlink()
    with Database(database_path) as database:
        journal = EventJournal(database)
        event = journal.append(
            "domain.deep.python",
            "deep-python",
            deep_source(depth, tag),
            workflow_id="flow-deep",
            run_id="run-deep",
            timestamp=BASE,
        )
        snapshot = SnapshotStore(database).save(
            "snapshot-deep-python-000001",
            "flow-deep",
            deep_source(max(1, depth // 2), tag + ".snapshot"),
            created_at=BASE + timedelta(seconds=1),
        )
        record = {
            "event": event_record(event),
            "snapshot": snapshot_record(snapshot),
            "verification": verification_record(journal),
        }
    json.dump(record, sys.stdout, ensure_ascii=True, separators=(",", ":"))
    sys.stdout.flush()


def mode_append_deep(path: str, depth: int, tag: str) -> None:
    with Database(Path(path)) as database:
        journal = EventJournal(database)
        event = journal.append(
            "domain.deep.python",
            "deep-python",
            deep_source(depth, tag),
            workflow_id="flow-deep",
            run_id="run-deep",
            timestamp=BASE + timedelta(seconds=30),
        )
        snapshot = SnapshotStore(database).save(
            "snapshot-deep-python-000002",
            "flow-deep",
            deep_source(max(1, depth // 2), tag + ".snapshot"),
            created_at=BASE + timedelta(seconds=31),
        )
        record = {
            "event": event_record(event),
            "snapshot": snapshot_record(snapshot),
            "verification": verification_record(journal),
        }
    json.dump(record, sys.stdout, ensure_ascii=True, separators=(",", ":"))
    sys.stdout.flush()


def mode_inspect_deep(path: str, workflow_id: str) -> None:
    with Database(Path(path)) as database:
        journal = EventJournal(database)
        record = {
            "events": [event_record(event) for event in journal.read(limit=None)],
            "verification": verification_record(journal),
        }
        latest = SnapshotStore(database).latest(workflow_id)
        record["snapshot"] = None if latest is None else snapshot_record(latest)
    json.dump(record, sys.stdout, ensure_ascii=True, separators=(",", ":"))
    sys.stdout.flush()


def mode_padded_row(path: str) -> None:
    """Create a row whose raw ids carry Python-only strip whitespace.

    This is deliberately an attacker-style direct SQLite edit: the journal
    append API normalizes identifiers. The chain digest is recomputed over the
    raw padded bytes, so a correct reader must accept the row and expose the
    normalized identifiers through its typed view.
    """
    import sqlite3

    from kala_agent.persistence.chain import EventChainFields, chain_digest

    database_path = Path(path)
    if database_path.exists():
        database_path.unlink()
    with Database(database_path) as database:
        EventJournal(database).append(
            "step.0",
            "step-0",
            {"index": 0},
            workflow_id="flow-deep",
            run_id="run-deep",
            timestamp=BASE,
        )
    pad = "\x1c"
    with sqlite3.connect(database_path) as connection:
        connection.execute("DROP TRIGGER events_prevent_update")
        connection.execute("DROP TRIGGER events_prevent_delete")
        connection.execute(
            "UPDATE events SET event_type = ?, aggregate_id = ?, "
            "workflow_id = ?, run_id = ? WHERE sequence = 1",
            (
                pad + "step.0" + pad,
                pad + "step-0" + pad,
                pad + "flow-deep" + pad,
                pad + "run-deep" + pad,
            ),
        )
        row = connection.execute(
            "SELECT sequence, event_type, aggregate_id, workflow_id, run_id, "
            "digest, occurred_at, previous_event_digest FROM events "
            "WHERE sequence = 1"
        ).fetchone()
        fields = EventChainFields(
            sequence=row[0],
            event_type=row[1],
            aggregate_id=row[2],
            workflow_id=row[3],
            run_id=row[4],
            occurred_at=row[6],
            payload_digest=row[5],
            previous_event_digest=row[7],
        )
        connection.execute(
            "UPDATE events SET event_digest = ? WHERE sequence = 1",
            (chain_digest(fields),),
        )
        connection.commit()
    with Database(database_path) as database:
        journal = EventJournal(database)
        record = {
            "events": [event_record(event) for event in journal.read(limit=None)],
            "verification": verification_record(journal),
        }
    json.dump(record, sys.stdout, ensure_ascii=True, separators=(",", ":"))
    sys.stdout.flush()


def main() -> None:
    mode = sys.argv[1]
    if mode == "canonical":
        mode_canonical()
    elif mode == "identifiers":
        mode_identifiers()
    elif mode == "create-deep":
        mode_create_deep(
            sys.argv[2],
            int(sys.argv[3]),
            sys.argv[4] if len(sys.argv) > 4 else "python",
        )
    elif mode == "append-deep":
        mode_append_deep(
            sys.argv[2],
            int(sys.argv[3]),
            sys.argv[4] if len(sys.argv) > 4 else "python",
        )
    elif mode == "inspect-deep":
        mode_inspect_deep(
            sys.argv[2],
            sys.argv[3] if len(sys.argv) > 3 else "flow-deep",
        )
    elif mode == "padded-row":
        mode_padded_row(sys.argv[2])
    else:
        raise SystemExit(f"unknown mode {mode!r}")


if __name__ == "__main__":
    main()
