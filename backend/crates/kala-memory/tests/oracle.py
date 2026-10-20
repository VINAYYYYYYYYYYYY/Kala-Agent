"""Test-only live Python oracle for the native ``kala-memory`` differential.

Usage: ``python oracle.py < request.json > response.json``

This script is never imported by product code. It executes the same operation
sequences against the repository's Python reference implementation
(``kala_agent.kernel.memory``) and prints one canonical JSON document for
byte comparison against the native implementation.

The request and the response are both ``{"scenarios": [...]}`` documents, so
one interpreter start serves a whole batch of scenarios.

The error projection is deliberately narrower than Python's own rendering:

* the exception class is ``type(exc).__name__``;
* the message is ``str(exc)`` for every exception except Pydantic's
  ``ValidationError``, whose multi-line rendering is reduced to the first
  issue's message with Pydantic's ``"Value error, "`` prefix removed. The
  native crate reports the same single message through its contract error
  type, so the two sides can be compared byte for byte.
"""

from __future__ import annotations

import json
import sys
from datetime import datetime
from typing import Any

from pydantic import ValidationError

from kala_agent.kernel.memory import MemoryManager
from kala_agent.persistence import Database, MemoryRecord, MemoryRepository

PYDANTIC_VALUE_ERROR_PREFIX = "Value error, "


def _dt(value: str | None) -> datetime | None:
    if value is None:
        return None
    return datetime.fromisoformat(value)


def _record(payload: dict[str, Any]) -> MemoryRecord:
    return MemoryRecord.model_validate(payload)


def _dump(record: MemoryRecord) -> dict[str, Any]:
    return record.model_dump(mode="json")


def _error(exc: BaseException) -> dict[str, Any]:
    if isinstance(exc, ValidationError):
        first = exc.errors()[0]
        message = str(first.get("msg", ""))
        if message.startswith(PYDANTIC_VALUE_ERROR_PREFIX):
            message = message[len(PYDANTIC_VALUE_ERROR_PREFIX) :]
        return {"error": {"class": "ValidationError", "message": message}}
    return {"error": {"class": type(exc).__name__, "message": str(exc)}}


def _list_options(operation: dict[str, Any]) -> dict[str, Any]:
    return {
        "source": operation.get("source"),
        "now": _dt(operation.get("now")),
        "include_working": bool(operation.get("include_working", True)),
        "durable_only": bool(operation.get("durable_only", False)),
        "limit": int(operation.get("limit", 1000)),
    }


def _run_op(
    manager: MemoryManager,
    repository: MemoryRepository,
    operation: dict[str, Any],
) -> Any:
    name = operation["op"]
    if name == "add":
        result = manager.add(
            _record(operation["record"]),
            durable=bool(operation.get("durable", False)),
            promote=bool(operation.get("promote", False)),
            now=_dt(operation.get("now")),
        )
        return _dump(result)
    if name == "try_add":
        result = manager.try_add(
            _record(operation["record"]),
            now=_dt(operation.get("now")),
        )
        return result.as_dict()
    if name == "promote":
        target = operation["target"]
        if target["kind"] == "record":
            candidate: Any = _record(target["record"])
        else:
            candidate = target["id"]
        result = manager.promote(
            candidate,
            provenance_id=operation.get("provenance_id"),
            now=_dt(operation.get("now")),
        )
        return _dump(result)
    if name == "get":
        result = manager.get(
            operation["memory_id"],
            now=_dt(operation.get("now")),
            include_expired=bool(operation.get("include_expired", False)),
        )
        return None if result is None else _dump(result)
    if name == "list":
        return [_dump(item) for item in manager.list(**_list_options(operation))]
    if name == "list_active":
        return [_dump(item) for item in manager.list_active(**_list_options(operation))]
    if name == "list_durable":
        return [
            _dump(item)
            for item in manager.list_durable(
                source=operation.get("source"),
                now=_dt(operation.get("now")),
                limit=int(operation.get("limit", 1000)),
            )
        ]
    if name == "list_working":
        return [
            _dump(item)
            for item in manager.list_working(
                source=operation.get("source"),
                now=_dt(operation.get("now")),
                limit=int(operation.get("limit", 1000)),
            )
        ]
    if name == "detect_conflicts":
        result = manager.detect_conflicts(
            _record(operation["record"]),
            now=_dt(operation.get("now")),
        )
        return [_dump(item) for item in result]
    if name == "discard_working":
        return manager.discard_working(operation["memory_id"])
    if name == "delete":
        return manager.delete(operation["memory_id"])
    if name == "repository_put":
        repository.put(_record(operation["record"]))
        return None
    raise ValueError(f"unknown memory op {name}")


def run(document: dict[str, Any]) -> dict[str, Any]:
    operations = list(document.get("ops", ()))
    with Database() as database:
        repository = MemoryRepository(database)
        try:
            manager = MemoryManager(
                repository,
                max_working_records=document.get("max_working_records", 1000),
                require_provenance_on_promotion=document.get(
                    "require_provenance_on_promotion", False
                ),
            )
        except BaseException as error:  # noqa: BLE001 - the failure is reported
            return {"results": [_error(error) for _ in operations]}
        results: list[Any] = []
        for operation in operations:
            try:
                results.append(_run_op(manager, repository, operation))
            except BaseException as error:  # noqa: BLE001 - the failure is reported
                results.append(_error(error))
        return {"results": results}


def main() -> int:
    payload = json.loads(sys.stdin.read())
    responses = [run(scenario) for scenario in payload.get("scenarios", ())]
    sys.stdout.write(
        json.dumps(
            {"scenarios": responses},
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=True,
        )
    )
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
