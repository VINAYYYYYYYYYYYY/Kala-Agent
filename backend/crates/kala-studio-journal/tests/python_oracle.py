"""The live Python oracle for the native Studio session journal.

The script speaks JSON lines: one request object per line on stdin, one
response object per line on stdout.  It drives the repository's own
``kala_agent.studio.session_journal`` module, so the native crate and the
Python product write the same file and can be compared byte for byte.

Every response has the shape ``{"ok": true, "result": ...}`` or
``{"ok": false, "error": {"kind": ..., "class": ..., "message": ...}}``.  The
``kind`` is the coarse Python exception class the native error maps onto, so
the two implementations can be compared on failures as well as on successes.
"""

from __future__ import annotations

import base64
import json
import sqlite3
import sys
from datetime import datetime
from pathlib import Path
from typing import Any

from kala_agent.persistence.chain import JournalIntegrityError
from kala_agent.studio import session_journal as session_journal_module
from kala_agent.studio.session_journal import StudioSessionJournal

JOURNAL: StudioSessionJournal | None = None


def _load_payload(text: str) -> Any:
    """Turn the payload text into the mapping the module expects.

    The module's public API takes a mapping, so the oracle parses the text
    first. A text that is not JSON at all is refused through the module's own
    canonicalisation, so the failure the oracle reports is the failure the
    module reports for a value it cannot canonicalise.
    """
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        session_journal_module._canonical_json(text)
        raise


def _journal() -> StudioSessionJournal:
    if JOURNAL is None:
        raise RuntimeError("the studio session journal is closed")
    return JOURNAL


def _parse_timestamp(value: Any) -> datetime | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise TypeError("occurred_at must be a string")
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def _payload_canonical(payload: Any) -> str | None:
    if payload is None:
        return None
    return json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def _event(event: Any) -> dict[str, Any]:
    return {
        "sequence": event.sequence,
        "event_type": event.event_type,
        "aggregate_id": event.aggregate_id,
        "payload": event.payload,
        "payload_canonical": _payload_canonical(event.payload),
        "payload_json": event.payload_json,
        "digest": event.digest,
        "occurred_at": event.occurred_at,
        "previous_event_digest": event.previous_event_digest,
        "event_digest": event.event_digest,
    }


def _verification(verification: Any) -> dict[str, Any]:
    return {
        "ok": verification.ok,
        "event_count": verification.event_count,
        "head_digest": verification.head_digest,
        "first_bad_sequence": verification.first_bad_sequence,
    }


def _error_kind(error: BaseException) -> str:
    if isinstance(error, JournalIntegrityError):
        return "integrity"
    if isinstance(error, TypeError):
        return "type"
    if isinstance(error, FileNotFoundError):
        return "not_found"
    if isinstance(error, ValueError):
        return "value"
    if isinstance(error, RuntimeError):
        return "closed"
    return "other"


def _handle(request: dict[str, Any]) -> Any:
    global JOURNAL
    operation = request["op"]
    if operation == "open":
        data_dir = Path(request["data_dir"])
        keywords: dict[str, Any] = {}
        for name in ("max_payload_bytes", "max_artifact_bytes", "max_page_size", "busy_timeout_ms"):
            if name in request and request[name] is not None:
                keywords[name] = int(request[name])
        JOURNAL = StudioSessionJournal(data_dir, **keywords)
        return {
            "data_dir": str(JOURNAL.data_dir),
            "database_path": str(JOURNAL.database_path),
            "artifact_root": str(JOURNAL.artifact_root),
            "max_payload_bytes": JOURNAL.max_payload_bytes,
            "max_page_size": JOURNAL.max_page_size,
        }
    if operation == "close":
        if JOURNAL is not None:
            JOURNAL.close()
            JOURNAL = None
        return {"closed": True}
    if operation == "append":
        payload_text = request["payload_text"]
        payload = _load_payload(payload_text)
        keywords: dict[str, Any] = {}
        if request.get("aggregate_id") is not None:
            keywords["aggregate_id"] = request["aggregate_id"]
        occurred_at = _parse_timestamp(request.get("occurred_at"))
        if occurred_at is not None:
            keywords["occurred_at"] = occurred_at
        event = _journal().append(request["event_type"], payload, **keywords)
        return _event(event)
    if operation == "page":
        events = _journal().page(
            int(request.get("after_sequence", 0)), int(request.get("limit", 500))
        )
        return [_event(event) for event in events]
    if operation == "get":
        event = _journal().get(int(request["sequence"]))
        return None if event is None else _event(event)
    if operation == "verify":
        return _verification(_journal().verify())
    if operation == "head_digest":
        return _journal().head_digest
    if operation == "put_artifact":
        data = base64.b64decode(request["data_base64"])
        return _journal().put_artifact(data)
    if operation == "get_artifact":
        return {
            "base64": base64.b64encode(_journal().get_artifact(request["digest"])).decode("ascii")
        }
    if operation == "get_artifact_text":
        return {"text": _journal().get_artifact_text(request["digest"])}
    if operation == "has_artifact":
        return _journal().has_artifact(request["digest"])
    if operation == "schema":
        # The stored schema, so the two implementations can be compared on the
        # shape of the file they write.
        connection = sqlite3.connect(str(_journal().database_path))
        try:
            rows = connection.execute(
                "SELECT type, name, sql FROM sqlite_master ORDER BY type, name"
            ).fetchall()
        finally:
            connection.close()
        return [list(row) for row in rows]
    if operation == "pragmas":
        connection = sqlite3.connect(str(_journal().database_path))
        try:
            return {
                "journal_mode": connection.execute("PRAGMA journal_mode").fetchone()[0],
                "user_version": connection.execute("PRAGMA user_version").fetchone()[0],
            }
        finally:
            connection.close()
    if operation == "diagnostics":
        # The Python journal has no work counters; the native side reports
        # them, so the oracle reports the fields it can see.
        return {}
    if operation == "rows":
        # The stored rows, column by column, so the two implementations can be
        # compared on the bytes the file actually holds.
        connection = sqlite3.connect(str(_journal().database_path))
        try:
            connection.row_factory = sqlite3.Row
            rows = connection.execute(
                "SELECT sequence, event_type, aggregate_id, workflow_id, run_id,"
                " payload_json, digest, occurred_at, previous_event_digest, event_digest"
                " FROM studio_events ORDER BY sequence ASC"
            ).fetchall()
        finally:
            connection.close()
        return [dict(row) for row in rows]
    if operation == "append_many":
        count = int(request["count"])
        events = []
        for index in range(count):
            payload_text = request["payload_text"].replace("{{index}}", str(index))
            event = _journal().append(
                request["event_type"],
                _load_payload(payload_text),
                occurred_at=_parse_timestamp(request.get("occurred_at")),
            )
            events.append(_event(event))
        return events
    if operation == "tamper_payload":
        _execute(
            "UPDATE studio_events SET payload_json = ? WHERE sequence = ?",
            (request["payload_json"], int(request["sequence"])),
        )
        return {"tampered": True}
    if operation == "tamper_digest":
        _execute(
            "UPDATE studio_events SET event_digest = ? WHERE sequence = ?",
            (request["event_digest"], int(request["sequence"])),
        )
        return {"tampered": True}
    if operation == "drop":
        _execute(
            "DELETE FROM studio_events WHERE sequence = ?",
            (int(request["sequence"]),),
        )
        return {"tampered": True}
    raise ValueError(f"unknown operation: {operation}")


def _execute(statement: str, parameters: tuple[Any, ...]) -> None:
    connection = sqlite3.connect(str(_journal().database_path))
    try:
        connection.execute(statement, parameters)
        connection.commit()
    finally:
        connection.close()


def main() -> int:
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
            response = {"ok": True, "result": _handle(request)}
        except BaseException as error:  # noqa: BLE001 - the oracle reports every failure
            response = {
                "ok": False,
                "error": {
                    "kind": _error_kind(error),
                    "class": type(error).__name__,
                    "message": str(error),
                },
            }
        sys.stdout.write(json.dumps(response, sort_keys=True) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
