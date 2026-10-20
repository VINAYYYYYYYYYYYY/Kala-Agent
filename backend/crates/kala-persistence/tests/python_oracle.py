#!/usr/bin/env python3
"""Test-only Python oracle for the native persistence crate.

This script is spawned by the Rust integration tests. It imports the live
product implementation from ``src/kala_agent`` and is never used by the
library at production time.

Modes:
  * ``canonical``: stdin is a JSON array of source strings; stdout is a JSON
    array of ``{"ok": true, "output": ...}`` or ``{"ok": false, "error": ...}``
    from ``kala_agent.contracts.models._canonical_json_object``.
  * ``timestamps``: stdin is a JSON array of stored timestamp strings; stdout
    is a JSON array of the live ``_timestamp_from_iso`` result normalized back
    through ``_timestamp_to_iso``, or its error.
  * ``create``: arguments are a database path. Builds a fresh database with
    fixed-clock events and a snapshot and prints the logical record.
  * ``append``: arguments are a database path. Reopens it, verifies the chain,
    appends one fixed event and one snapshot, then prints the appended record
    plus the verification head.
  * ``inspect``: arguments are a database path. Reopens it, fully verifies the
    chain, and prints every event and the latest snapshot.
"""

from __future__ import annotations

import json
import sys
from datetime import UTC, datetime, timedelta
from pathlib import Path

from kala_agent.persistence.database import (
    Database,
    SnapshotStore,
    _timestamp_from_iso,
    _timestamp_to_iso,
)
from kala_agent.persistence.journal import EventJournal

BASE = datetime(2026, 1, 1, tzinfo=UTC)


def _timestamp(value: datetime) -> str:
    return value.isoformat(timespec="microseconds").replace("+00:00", "Z")


def _event_record(event: object) -> dict[str, object]:
    return {
        "sequence": event.sequence,
        "event_type": event.event_type,
        "aggregate_id": event.aggregate_id,
        "workflow_id": event.workflow_id,
        "run_id": event.run_id,
        "payload_json": event.payload_json,
        "digest": event.digest,
        "occurred_at": _timestamp(event.timestamp),
        "previous_event_digest": event.previous_event_digest,
        "event_digest": event.event_digest,
    }


def _snapshot_record(snapshot: object) -> dict[str, object]:
    return {
        "sequence": snapshot.sequence,
        "snapshot_id": snapshot.snapshot_id,
        "workflow_id": snapshot.workflow_id,
        "state_json": snapshot.state_json,
        "digest": snapshot.digest,
        "created_at": _timestamp(snapshot.created_at),
    }


def _verification_record(journal: EventJournal) -> dict[str, object]:
    verification = journal.verify_chain()
    return {
        "event_count": verification.event_count,
        "head_sequence": verification.head_sequence,
        "head_event_digest": verification.head_event_digest,
    }


def mode_canonical() -> None:
    from kala_agent.contracts.models import _canonical_json_object

    sources = json.load(sys.stdin)
    results: list[dict[str, object]] = []
    for source in sources:
        try:
            results.append({"ok": True, "output": _canonical_json_object(source)})
        except Exception as error:  # noqa: BLE001 - oracle reports the message
            results.append({"ok": False, "error": f"{type(error).__name__}: {error}"})
    json.dump(results, sys.stdout, ensure_ascii=True, separators=(",", ":"))
    sys.stdout.flush()


def mode_timestamps() -> None:
    sources = json.load(sys.stdin)
    results: list[dict[str, object]] = []
    for source in sources:
        try:
            parsed = _timestamp_from_iso(source)
            results.append({"ok": True, "output": _timestamp_to_iso(parsed)})
        except Exception as error:  # noqa: BLE001 - oracle reports the message
            results.append({"ok": False, "error": f"{type(error).__name__}: {error}"})
    json.dump(results, sys.stdout, ensure_ascii=True, separators=(",", ":"))
    sys.stdout.flush()


def mode_create(path: str) -> None:
    database_path = Path(path)
    if database_path.exists():
        database_path.unlink()
    definitions = [
        ("design.created", "design-oracle", {"big_integer": 2**70, "name": "Oracle"}),
        (
            "workflow.started",
            "flow-oracle",
            {"definition_id": "mechanical.reference", "offline": True},
        ),
    ]
    with Database(database_path) as database:
        journal = EventJournal(database)
        events = []
        for index, (event_type, aggregate_id, payload) in enumerate(definitions):
            event = journal.append(
                event_type,
                aggregate_id,
                payload,
                workflow_id="flow-oracle" if index else None,
                run_id="run-oracle" if index else None,
                timestamp=BASE + timedelta(seconds=index),
            )
            events.append(_event_record(event))
        snapshot = SnapshotStore(database).save(
            "snapshot-oracle-000001",
            "flow-oracle",
            {"state": "created", "steps": ["one", "two"]},
            created_at=BASE + timedelta(seconds=10),
        )
        record = {
            "events": events,
            "snapshot": _snapshot_record(snapshot),
            "verification": _verification_record(journal),
        }
    json.dump(record, sys.stdout, ensure_ascii=True, separators=(",", ":"))


def mode_append(path: str) -> None:
    deep: object = 0
    for _ in range(64):
        deep = [deep]
    payload = {
        "source": "python",
        "large_string": "a" * 33_000,
        "large_array": list(range(2_100)),
        "deep": deep,
        "unicode": "κ 😀",
    }
    with Database(path) as database:
        journal = EventJournal(database)
        event = journal.append(
            "workflow.appended",
            "flow-oracle",
            payload,
            workflow_id="flow-oracle",
            run_id="run-oracle",
            timestamp=BASE + timedelta(seconds=30),
        )
        snapshot = SnapshotStore(database).save(
            "snapshot-oracle-000002",
            "flow-oracle",
            {"state": "appended"},
            created_at=BASE + timedelta(seconds=31),
        )
        record = {
            "event": _event_record(event),
            "snapshot": _snapshot_record(snapshot),
            "verification": _verification_record(journal),
        }
    json.dump(record, sys.stdout, ensure_ascii=True, separators=(",", ":"))


def mode_inspect(path: str, workflow_id: str) -> None:
    with Database(path) as database:
        journal = EventJournal(database)
        record = {
            "events": [_event_record(event) for event in journal.read(limit=None)],
            "verification": _verification_record(journal),
        }
        latest = SnapshotStore(database).latest(workflow_id)
        record["snapshot"] = None if latest is None else _snapshot_record(latest)
    json.dump(record, sys.stdout, ensure_ascii=True, separators=(",", ":"))


def main() -> None:
    mode = sys.argv[1]
    if mode == "canonical":
        mode_canonical()
    elif mode == "timestamps":
        mode_timestamps()
    elif mode == "create":
        mode_create(sys.argv[2])
    elif mode == "append":
        mode_append(sys.argv[2])
    elif mode == "inspect":
        mode_inspect(sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else "flow-oracle")
    else:
        raise SystemExit(f"unknown mode {mode!r}")


if __name__ == "__main__":
    main()
