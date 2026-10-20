"""A durable local journal of build submissions.

The journal is the client's own memory of what it sent. It records the
normalised script itself, the script digest, the canonical request digest, the
normalized bridge target, the request id, the build id, and the last known
status. It never records the bearer token, a URL with credentials, or artifact
bytes.

The request digest covers everything that changes the requested build —
calibration, timeout, density, summary, and the ordered procedure — so a retry
reuses a request id only when the whole semantic request is identical. The
bridge target binds that to one bridge URL and calibration, so a re-probed host
does not inherit an old job. A legacy entry written before those fields existed
has no exact identity; it is read for history but never reused for a new
submission.
"""

from __future__ import annotations

import json
import os
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Final

JOURNAL_FILENAME: Final[str] = "journal.jsonl"
MAX_ENTRIES: Final[int] = 200
MAX_DIGEST_CHARS: Final[int] = 64
MAX_ID_CHARS: Final[int] = 64
MAX_TARGET_CHARS: Final[int] = 512
MAX_SCRIPT_CHARS: Final[int] = 65_536


@dataclass(frozen=True)
class JournalEntry:
    """One recorded submission."""

    script_digest: str
    request_id: str
    build_id: str | None
    status: str
    summary: str
    request_digest: str | None = None
    bridge_target: str | None = None
    script: str | None = None

    def as_json(self) -> dict[str, object]:
        return {
            "script_digest": self.script_digest,
            "request_id": self.request_id,
            "build_id": self.build_id,
            "status": self.status,
            "summary": self.summary,
            "request_digest": self.request_digest,
            "bridge_target": self.bridge_target,
            "script": self.script,
        }


class Journal:
    """An append-only, bounded JSONL journal with atomic rewrites."""

    def __init__(self, path: Path) -> None:
        self.path = path

    def read(self) -> tuple[JournalEntry, ...]:
        """Return the recorded entries, newest last. Malformed lines are skipped."""
        try:
            text = self.path.read_text(encoding="utf-8")
        except FileNotFoundError:
            return ()
        except OSError:
            return ()
        entries: list[JournalEntry] = []
        for line in text.splitlines():
            entry = _parse(line)
            if entry is not None:
                entries.append(entry)
        return tuple(entries)

    def find_by_digest(self, script_digest: str) -> JournalEntry | None:
        """Return the most recent entry for a semantic script digest."""
        for entry in reversed(self.read()):
            if entry.script_digest == script_digest:
                return entry
        return None

    def find_exact(
        self,
        *,
        script_digest: str,
        request_digest: str,
        bridge_target: str,
    ) -> JournalEntry | None:
        """Return the newest entry with the exact request identity, if any."""
        for entry in reversed(self.read()):
            if (
                entry.script_digest == script_digest
                and entry.request_digest == request_digest
                and entry.bridge_target == bridge_target
            ):
                return entry
        return None

    def find_by_request(self, request_id: str) -> JournalEntry | None:
        """Return the entry for one request id."""
        for entry in reversed(self.read()):
            if entry.request_id == request_id:
                return entry
        return None

    def append(self, entry: JournalEntry) -> None:
        """Append one entry and compact to the bounded size."""
        entries = [*self.read(), entry][-MAX_ENTRIES:]
        self._rewrite(entries)

    def update(self, request_id: str, *, build_id: str | None, status: str) -> None:
        """Replace the newest entry for ``request_id`` with updated fields."""
        entries = list(self.read())
        for index in range(len(entries) - 1, -1, -1):
            if entries[index].request_id == request_id:
                entries[index] = JournalEntry(
                    script_digest=entries[index].script_digest,
                    request_id=request_id,
                    build_id=build_id if build_id is not None else entries[index].build_id,
                    status=status,
                    summary=entries[index].summary,
                    request_digest=entries[index].request_digest,
                    bridge_target=entries[index].bridge_target,
                    script=entries[index].script,
                )
                break
        else:
            return
        self._rewrite(entries[-MAX_ENTRIES:])

    def _rewrite(self, entries: list[JournalEntry]) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        payload = "".join(
            json.dumps(entry.as_json(), sort_keys=True, separators=(",", ":")) + "\n"
            for entry in entries
        )
        descriptor, temporary = tempfile.mkstemp(prefix=f".{self.path.name}.", dir=self.path.parent)
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
                handle.write(payload)
                handle.flush()
                os.fsync(handle.fileno())
            os.replace(temporary, self.path)
        except BaseException:
            try:
                os.unlink(temporary)
            except OSError:
                pass
            raise


def _parse(line: str) -> JournalEntry | None:
    try:
        payload = json.loads(line)
    except json.JSONDecodeError:
        return None
    if not isinstance(payload, dict):
        return None
    digest = payload.get("script_digest")
    request_id = payload.get("request_id")
    build_id = payload.get("build_id")
    status = payload.get("status")
    summary = payload.get("summary")
    request_digest = payload.get("request_digest")
    bridge_target = payload.get("bridge_target")
    script = payload.get("script")
    if not isinstance(digest, str) or not isinstance(request_id, str):
        return None
    if len(digest) > MAX_DIGEST_CHARS or len(request_id) > MAX_ID_CHARS:
        return None
    if build_id is not None and (not isinstance(build_id, str) or len(build_id) > MAX_ID_CHARS):
        return None
    if not isinstance(status, str) or len(status) > 32:
        return None
    if not isinstance(summary, str) or len(summary) > 200:
        return None
    if request_digest is not None and (
        not isinstance(request_digest, str) or len(request_digest) > MAX_DIGEST_CHARS
    ):
        return None
    if bridge_target is not None and (
        not isinstance(bridge_target, str) or len(bridge_target) > MAX_TARGET_CHARS
    ):
        return None
    if script is not None and (not isinstance(script, str) or len(script) > MAX_SCRIPT_CHARS):
        return None
    return JournalEntry(
        script_digest=digest,
        request_id=request_id,
        build_id=build_id,
        status=status,
        summary=summary,
        request_digest=request_digest,
        bridge_target=bridge_target,
        script=script,
    )


__all__ = (
    "JOURNAL_FILENAME",
    "MAX_ENTRIES",
    "MAX_SCRIPT_CHARS",
    "Journal",
    "JournalEntry",
)
