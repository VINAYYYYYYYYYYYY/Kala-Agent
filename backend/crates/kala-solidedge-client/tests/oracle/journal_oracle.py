"""Live Python oracle for the journal record format.

The journal is the client's own memory of what it sent, and either client
must be able to read the other's records. This oracle writes a journal with
the Python ``Journal`` class and reads one back, so the Rust side can prove
that the record bytes and the record meaning agree.

Usage::

    python -I journal_oracle.py write <path>
    python -I journal_oracle.py read <path>
"""

from __future__ import annotations

import json
import os
import sys


def _repository_root() -> str:
    if len(sys.argv) > 3:
        return os.path.abspath(sys.argv[3])
    here = os.path.dirname(os.path.abspath(__file__))
    return os.path.abspath(os.path.join(here, "..", "..", "..", "..", ".."))


def _prepare(root: str) -> None:
    for part in (
        "src",
        os.path.join("adapters", "solidedge", "src"),
        os.path.join("adapters", "solidedge-bridge", "src"),
    ):
        path = os.path.join(root, part)
        if path not in sys.path:
            sys.path.insert(0, path)


def _write(path: str) -> int:
    from pathlib import Path

    from kala_agent_solidedge.journal import Journal, JournalEntry

    journal = Journal(Path(path))
    journal.append(
        JournalEntry(
            script_digest="a" * 64,
            request_id="req-1",
            build_id="build-1",
            status="pending",
            summary="a plate",
            request_digest="b" * 64,
            bridge_target="http://127.0.0.1:8797|cal-1",
            script=json.dumps({"summary": "plate"}, sort_keys=True, separators=(",", ":")),
        )
    )
    journal.append(
        JournalEntry(
            script_digest="c" * 64,
            request_id="req-2",
            build_id=None,
            status="succeeded",
            summary="",
        )
    )
    journal.update("req-1", build_id="build-2", status="succeeded")
    return 0


def _read(path: str) -> int:
    from pathlib import Path

    from kala_agent_solidedge.journal import Journal

    journal = Journal(Path(path))
    entries = [entry.as_json() for entry in journal.read()]
    json.dump(entries, sys.stdout, sort_keys=True, separators=(",", ":"))
    sys.stdout.write("\n")
    return 0


def main() -> int:
    if len(sys.argv) < 3:
        print("usage: journal_oracle.py write|read <path> [repository-root]", file=sys.stderr)
        return 2
    mode, path = sys.argv[1], sys.argv[2]
    _prepare(_repository_root())
    if mode == "write":
        return _write(path)
    if mode == "read":
        return _read(path)
    print(f"unknown mode {mode!r}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
