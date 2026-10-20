"""The Studio's own hash-chained session journal and artifact store.

The Studio keeps a *separate* journal from the core ``KalaApplication``. The
core journal is unreachable from the Studio on purpose: the Studio server
never instantiates the composition root, so a design turn cannot quietly write
into the agent's own history. What the Studio reuses are two libraries, and
only two:

- :mod:`kala_agent.persistence.chain` supplies the chain itself:
  :func:`~kala_agent.persistence.chain.chain_digest` and
  :class:`~kala_agent.persistence.chain.EventChainFields`. The digest of a
  Studio event is therefore the same length-prefixed, domain-separated
  SHA-256 preimage the core journal uses, over the same covered fields in the
  same order, and a Studio chain digest is reproducible with the core
  function without any Studio-specific knowledge.
- :mod:`kala_agent.persistence.artifacts` supplies
  :class:`~kala_agent.persistence.artifacts.ContentAddressedArtifactStore`,
  which stores the STEP blob and the script JSON by content and returns the
  SHA-256 of the bytes.

The canonical JSON convention is not re-implemented here: payloads are
canonicalised with the shared
:data:`kala_agent.contracts.CanonicalJsonObject` contract, which is the very
validator ``kala_agent.persistence.database`` uses for the core journal, so
"canonical" means exactly the same bytes in both places (sorted keys, no
insignificant whitespace, ASCII-escaped, no NaN or Infinity).

The journal is a single SQLite file inside the caller-supplied Studio data
directory, opened in WAL mode and guarded by one re-entrant lock, so the
FastAPI thread pool may append, page, and verify concurrently without two
threads interleaving a statement. A write refuses to extend a chain that is
already broken, so a journal that has been edited outside the Studio fails
closed instead of growing a second, inconsistent history.

The chain is tamper-*evident*, not tamper-proof, exactly like the core chain:
anyone who can write the database can rewrite a whole suffix consistently.
The head digest returned by :meth:`StudioSessionJournal.verify` is the value
to anchor outside the database.

**When an edit is detected.** The chain is walked once, when the journal is
opened, and the verified head is cached from then on: the sequence and the
chain digest of the last event that verified. An append does not walk the
chain again. It re-reads only the newest stored row, re-verifies that one
row against the digests the row itself records, and checks that the row is
still the cached head. An append therefore costs one row read rather than
one walk, so a long session costs no more per event than a short one.

That gives a precise, deliberately narrow, detection guarantee:

- An edit of the **head** -- the newest event -- is detected by the next
  ``append``: the row no longer matches the cached head, the journal falls
  back to a full walk, and the append is refused with
  :class:`~kala_agent.persistence.chain.JournalIntegrityError`.
- An edit **in the middle** of the chain is detected by the next
  :meth:`StudioSessionJournal.verify` or by the next **open** of the
  journal, because only a full walk re-derives the digests of the events
  that follow the edited one. An append made between such an edit and the
  next full walk extends the chain as it stands; the break is reported as
  soon as the chain is walked in full.

A broken chain is never extended: the append fails closed either way. A
journal that is found broken when it is opened, or that fails a
verification, refuses every append until a later verification walks the
chain and finds it intact again.

Recording a design turn looks like this::

    digest = journal.put_artifact(step_bytes)
    journal.append(
        JournalEventType.ARTIFACT_STORED,
        {"kind": "step", "sha256": digest, "size_bytes": len(step_bytes)},
    )
    journal.append(JournalEventType.DESIGN_CHECKED, {"verified": True})

:meth:`StudioSessionJournal.put_artifact` stores the bytes and returns their
SHA-256 without writing an event of its own, so the caller decides which
event records the blob.
"""

from __future__ import annotations

import hashlib
import json
import os
import sqlite3
import sys
import threading
from collections.abc import Iterator, Mapping
from contextlib import contextmanager
from dataclasses import dataclass
from datetime import UTC, datetime
from enum import StrEnum
from pathlib import Path
from types import TracebackType
from typing import Final, Self, cast

from pydantic import TypeAdapter, ValidationError

from kala_agent.contracts import CanonicalJsonObject, Identifier
from kala_agent.persistence.artifacts import ContentAddressedArtifactStore
from kala_agent.persistence.chain import (
    CHAIN_COLUMNS,
    GENESIS_PREVIOUS_DIGEST,
    EventChainFields,
    JournalIntegrityError,
    chain_digest,
    chain_fields_from_row,
    is_chain_digest,
)

__all__ = (
    "ARTIFACT_DIRECTORY_NAME",
    "DEFAULT_AGGREGATE_ID",
    "DEFAULT_BUSY_TIMEOUT_MS",
    "DEFAULT_MAX_ARTIFACT_BYTES",
    "DEFAULT_MAX_PAGE_SIZE",
    "DEFAULT_MAX_PAYLOAD_BYTES",
    "JOURNAL_FILE_NAME",
    "JournalEventType",
    "JournalVerification",
    "SessionEvent",
    "StudioSessionJournal",
)

JOURNAL_FILE_NAME: Final[str] = "studio-journal.sqlite3"
ARTIFACT_DIRECTORY_NAME: Final[str] = "artifacts"
DEFAULT_AGGREGATE_ID: Final[str] = "studio"
DEFAULT_MAX_PAYLOAD_BYTES: Final[int] = 256 * 1024
DEFAULT_MAX_ARTIFACT_BYTES: Final[int] = 64 * 1024 * 1024
DEFAULT_MAX_PAGE_SIZE: Final[int] = 500
DEFAULT_BUSY_TIMEOUT_MS: Final[int] = 5_000

_DATA_DIRECTORY_MODE: Final[int] = 0o700
_DATABASE_FILE_MODE: Final[int] = 0o600
_IS_WINDOWS: Final[bool] = sys.platform.startswith("win")

_IDENTIFIER_ADAPTER: TypeAdapter[str] = TypeAdapter(Identifier)
_CANONICAL_JSON_ADAPTER: TypeAdapter[str] = TypeAdapter(CanonicalJsonObject)

_CREATE_EVENTS_TABLE: Final[str] = """
    CREATE TABLE IF NOT EXISTS studio_events (
        sequence INTEGER PRIMARY KEY,
        event_type TEXT NOT NULL,
        aggregate_id TEXT NOT NULL,
        workflow_id TEXT,
        run_id TEXT,
        payload_json TEXT NOT NULL,
        digest TEXT NOT NULL,
        occurred_at TEXT NOT NULL,
        previous_event_digest TEXT NOT NULL,
        event_digest TEXT NOT NULL
    )
    """

_CREATE_EVENTS_INDEX: Final[str] = """
    CREATE INDEX IF NOT EXISTS studio_events_type_sequence_idx
    ON studio_events(event_type, sequence)
    """

_INSERT_EVENT: Final[str] = """
    INSERT INTO studio_events(
        sequence, event_type, aggregate_id, workflow_id, run_id, payload_json,
        digest, occurred_at, previous_event_digest, event_digest
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    """


class JournalEventType(StrEnum):
    """The event types the design-turn flow records in the Studio journal.

    The list is the one named by ADR-0009 section 5. The journal accepts any
    safe identifier as an event type, so a new event type does not require a
    change here; the enum exists so the API layer can name them without
    repeating string literals.
    """

    DESIGN_INTENT = "design.intent"
    DESIGN_COMPILED = "design.compiled"
    DESIGN_MEASURED = "design.measured"
    DESIGN_CHECKED = "design.checked"
    ARTIFACT_STORED = "artifact.stored"
    EXPORT_COMPLETED = "export.completed"
    CHAT_TURN = "chat.turn"


@dataclass(frozen=True, slots=True)
class SessionEvent:
    """One journalled Studio event.

    ``digest`` is the SHA-256 of the canonical payload and ``event_digest``
    is the chain digest that binds the event to the one before it, which is
    the same split the core journal makes. ``occurred_at`` is the exact
    ISO-8601 UTC string that is part of the chain preimage, so an outside
    reader can recompute ``event_digest`` with
    :func:`~kala_agent.persistence.chain.chain_digest`.

    ``payload`` is ``None`` only when the stored text is not a JSON object,
    which can happen only if the file was edited outside the Studio; the raw
    text always remains available as ``payload_json``.
    """

    sequence: int
    event_type: str
    aggregate_id: str
    payload: Mapping[str, object] | None
    payload_json: str
    digest: str
    occurred_at: str
    previous_event_digest: str
    event_digest: str

    @property
    def timestamp(self) -> datetime:
        """Return the occurrence time as an aware UTC ``datetime``."""
        return _timestamp_from_iso(self.occurred_at)

    def as_dict(self) -> dict[str, object]:
        """Return a JSON-serialisable view of the event."""
        return {
            "sequence": self.sequence,
            "event_type": self.event_type,
            "aggregate_id": self.aggregate_id,
            "payload": None if self.payload is None else dict(self.payload),
            "payload_json": self.payload_json,
            "digest": self.digest,
            "occurred_at": self.occurred_at,
            "previous_event_digest": self.previous_event_digest,
            "event_digest": self.event_digest,
        }


@dataclass(frozen=True, slots=True)
class JournalVerification(Mapping[str, object]):
    """The outcome of a chain verification pass.

    The object is a mapping, so the API layer can hand it to the JSON
    encoder directly: ``verification["ok"]`` and ``dict(verification)`` both
    read the four documented keys.

    - ``ok`` is ``True`` only when every event verified.
    - ``event_count`` is the number of events the journal holds, verified or
      not, so the UI can show how much history exists.
    - ``head_digest`` is the chain digest of the last event that verified,
      which is the value to anchor outside the database. It is ``None`` for
      an empty journal and for a journal whose very first event is broken.
    - ``first_bad_sequence`` is the sequence of the first event that failed
      verification, or ``None`` when the chain is intact.
    """

    ok: bool
    event_count: int
    head_digest: str | None
    first_bad_sequence: int | None

    def as_dict(self) -> dict[str, object]:
        """Return the four documented keys as a plain dictionary."""
        return {
            "ok": self.ok,
            "event_count": self.event_count,
            "head_digest": self.head_digest,
            "first_bad_sequence": self.first_bad_sequence,
        }

    def __getitem__(self, key: str) -> object:
        try:
            return self.as_dict()[key]
        except KeyError:
            raise KeyError(key) from None

    def __iter__(self) -> Iterator[str]:
        return iter(self.as_dict())

    def __len__(self) -> int:
        return 4


@dataclass(frozen=True, slots=True)
class _ChainWalk:
    """The internal result of walking the stored chain."""

    event_count: int
    verified_count: int
    head_digest: str | None
    first_bad_sequence: int | None


class StudioSessionJournal:
    """The Studio's own append-only, hash-chained session journal.

    The journal lives in a caller-supplied data directory and never touches
    the core Kala database. The file is a single SQLite database in WAL mode
    and every statement runs under one re-entrant lock, so the FastAPI
    thread pool may use one instance from several threads at once.

    A payload is stored as canonical JSON and is bounded in size; a page is
    bounded in length; an artifact is bounded by the artifact store. Nothing
    this class accepts can grow without a limit.

    The chain is walked once when the journal is opened and the verified
    head is cached from then on, so an append verifies only the newest
    stored row instead of the whole history. The detection guarantee that
    follows from that is stated in the module docstring.
    """

    def __init__(
        self,
        data_dir: Path,
        *,
        max_payload_bytes: int = DEFAULT_MAX_PAYLOAD_BYTES,
        max_artifact_bytes: int = DEFAULT_MAX_ARTIFACT_BYTES,
        max_page_size: int = DEFAULT_MAX_PAGE_SIZE,
        busy_timeout_ms: int = DEFAULT_BUSY_TIMEOUT_MS,
    ) -> None:
        _require_positive_int(max_payload_bytes, "max_payload_bytes")
        _require_positive_int(max_artifact_bytes, "max_artifact_bytes")
        _require_positive_int(max_page_size, "max_page_size")
        _require_nonnegative_int(busy_timeout_ms, "busy_timeout_ms")
        self._data_dir = data_dir.expanduser().resolve(strict=False)
        self._data_dir.mkdir(parents=True, exist_ok=True, mode=_DATA_DIRECTORY_MODE)
        _tighten_directory_mode(self._data_dir)
        self._database_path = self._data_dir / JOURNAL_FILE_NAME
        self._max_payload_bytes = max_payload_bytes
        self._max_page_size = max_page_size
        self._artifacts = ContentAddressedArtifactStore(
            self._data_dir / ARTIFACT_DIRECTORY_NAME,
            max_object_size=max_artifact_bytes,
        )
        self._lock = threading.RLock()
        self._closed = False
        self._head_sequence = 0
        self._head_digest: str | None = None
        self._first_bad_sequence: int | None = None
        self._connection = sqlite3.connect(
            str(self._database_path),
            timeout=busy_timeout_ms / 1_000.0,
            isolation_level=None,
            check_same_thread=False,
        )
        self._connection.row_factory = sqlite3.Row
        try:
            self._connection.execute("PRAGMA journal_mode = WAL")
            self._connection.execute("PRAGMA synchronous = FULL")
            self._connection.execute(_CREATE_EVENTS_TABLE)
            self._connection.execute(_CREATE_EVENTS_INDEX)
            self._load_chain_state()
        except BaseException:
            self._connection.close()
            self._closed = True
            raise
        _tighten_file_mode(self._database_path)

    # --- identity ---

    @property
    def data_dir(self) -> Path:
        """Return the Studio data directory that holds the journal."""
        return self._data_dir

    @property
    def database_path(self) -> Path:
        """Return the path of the Studio journal database file."""
        return self._database_path

    @property
    def artifact_root(self) -> Path:
        """Return the root of the Studio content-addressed artifact store."""
        return self._artifacts.root

    @property
    def max_payload_bytes(self) -> int:
        """Return the largest canonical payload the journal accepts."""
        return self._max_payload_bytes

    @property
    def max_page_size(self) -> int:
        """Return the largest page the journal will return."""
        return self._max_page_size

    # --- writing ---

    def append(
        self,
        event_type: str | JournalEventType,
        payload: Mapping[str, object],
        *,
        aggregate_id: str = DEFAULT_AGGREGATE_ID,
        occurred_at: datetime | None = None,
    ) -> SessionEvent:
        """Append one event and return it with the digests it committed to.

        The payload is canonicalised before it is stored, so the bytes that
        are hashed are the bytes that are kept. The sequence is taken from
        the verified chain under the write lock.

        The write does not walk the whole chain. It re-reads only the newest
        stored row, re-verifies that row against its own recorded digests,
        and checks that it is still the cached head. If it is not -- the head
        was edited, or another instance appended to the same file -- the
        journal falls back to a full walk and refuses the append when that
        walk finds the chain broken, with
        :class:`~kala_agent.persistence.chain.JournalIntegrityError`, instead
        of growing a second, inconsistent history.
        """
        if not isinstance(payload, Mapping):
            raise TypeError("event payload must be a mapping")
        normalized_event_type = _validate_identifier(str(event_type), "event_type")
        normalized_aggregate_id = _validate_identifier(aggregate_id, "aggregate_id")
        payload_json = _canonical_json(payload)
        payload_bytes = payload_json.encode("utf-8")
        if len(payload_bytes) > self._max_payload_bytes:
            raise ValueError("event payload exceeds the configured size limit")
        payload_digest = hashlib.sha256(payload_bytes).hexdigest()
        occurred_at_iso = _timestamp_to_iso(
            occurred_at if occurred_at is not None else datetime.now(UTC)
        )
        with self._write_transaction() as connection:
            sequence, previous_event_digest = self._resolve_append_head(connection)
            event_digest = chain_digest(
                EventChainFields(
                    sequence=sequence,
                    event_type=normalized_event_type,
                    aggregate_id=normalized_aggregate_id,
                    workflow_id=None,
                    run_id=None,
                    occurred_at=occurred_at_iso,
                    payload_digest=payload_digest,
                    previous_event_digest=previous_event_digest,
                )
            )
            connection.execute(
                _INSERT_EVENT,
                (
                    sequence,
                    normalized_event_type,
                    normalized_aggregate_id,
                    None,
                    None,
                    payload_json,
                    payload_digest,
                    occurred_at_iso,
                    previous_event_digest,
                    event_digest,
                ),
            )
            # The write lock is still held, so the cached head can move here
            # without another thread overwriting it.
            self._head_sequence = sequence
            self._head_digest = event_digest
            self._first_bad_sequence = None
        return SessionEvent(
            sequence=sequence,
            event_type=normalized_event_type,
            aggregate_id=normalized_aggregate_id,
            payload=_decode_payload(payload_json),
            payload_json=payload_json,
            digest=payload_digest,
            occurred_at=occurred_at_iso,
            previous_event_digest=previous_event_digest,
            event_digest=event_digest,
        )

    # --- reading ---

    def page(
        self,
        after_sequence: int = 0,
        limit: int = DEFAULT_MAX_PAGE_SIZE,
    ) -> tuple[SessionEvent, ...]:
        """Return at most ``limit`` events after ``after_sequence``.

        The page is bounded twice: ``limit`` is capped at the journal's
        ``max_page_size``, and the events come back in ascending sequence
        order. A page never raises because the chain is broken: the UI reads
        the history and asks :meth:`verify` separately whether the history
        still verifies.
        """
        _require_nonnegative_int(after_sequence, "after_sequence")
        _require_positive_int(limit, "limit")
        effective_limit = min(limit, self._max_page_size)
        with self._read_connection() as connection:
            rows = connection.execute(
                f"""
                SELECT {CHAIN_COLUMNS}
                FROM studio_events
                WHERE sequence > ?
                ORDER BY sequence ASC
                LIMIT ?
                """,
                (after_sequence, effective_limit),
            ).fetchall()
        return tuple(self._event_from_row(row) for row in rows)

    def get(self, sequence: int) -> SessionEvent | None:
        """Return the event with ``sequence``, or ``None`` when it is absent."""
        _require_nonnegative_int(sequence, "sequence")
        with self._read_connection() as connection:
            row = connection.execute(
                f"SELECT {CHAIN_COLUMNS} FROM studio_events WHERE sequence = ?",
                (sequence,),
            ).fetchone()
        return None if row is None else self._event_from_row(row)

    @property
    def head_digest(self) -> str | None:
        """Return the verified chain digest of the newest event, if any."""
        return self.verify().head_digest

    # --- verification ---

    def verify(self) -> JournalVerification:
        """Recompute the chain and report where it breaks, if it does.

        The whole journal is walked from the genesis event, so an edited,
        dropped, or reordered event is found by the first event whose
        recorded digest no longer matches its content or no longer links to
        its predecessor. The walk also refreshes the cached head, so an edit
        in the middle of the chain is noticed here and at the next open,
        and an edit of the head is noticed here and at the next append.
        """
        with self._read_connection() as connection:
            walk = self._walk_chain(connection)
            self._apply_walk(walk)
        return JournalVerification(
            ok=walk.first_bad_sequence is None,
            event_count=walk.event_count,
            head_digest=walk.head_digest,
            first_bad_sequence=walk.first_bad_sequence,
        )

    # --- artifacts ---

    def put_artifact(self, data: bytes) -> str:
        """Store ``data`` in the Studio artifact store and return its SHA-256.

        The caller records the returned digest in an ``artifact.stored``
        event, which is what ties the blob to the design turn that produced
        it. The store is content-addressed, so storing the same bytes twice
        returns the same digest and creates no second object.
        """
        if not isinstance(data, bytes):
            raise TypeError("artifact data must be bytes")
        return self._artifacts.put_bytes(data)

    def get_artifact(self, digest: str) -> bytes:
        """Return the bytes stored under ``digest``."""
        return self._artifacts.get_bytes(digest)

    def get_artifact_text(self, digest: str) -> str:
        """Return the bytes stored under ``digest`` decoded as UTF-8 text."""
        return self._artifacts.get_text(digest)

    def has_artifact(self, digest: str) -> bool:
        """Return whether an object with ``digest`` is stored."""
        return self._artifacts.contains(digest)

    # --- lifecycle ---

    def close(self) -> None:
        """Close the journal database. Closing twice is harmless."""
        with self._lock:
            if self._closed:
                return
            self._closed = True
            self._connection.close()

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.close()

    # --- internals ---

    @contextmanager
    def _read_connection(self) -> Iterator[sqlite3.Connection]:
        """Hold the lock so a read cannot interleave with another thread."""
        with self._lock:
            if self._closed:
                raise RuntimeError("the studio session journal is closed")
            yield self._connection

    @contextmanager
    def _write_transaction(self) -> Iterator[sqlite3.Connection]:
        """Run one write transaction under the journal lock."""
        with self._lock:
            if self._closed:
                raise RuntimeError("the studio session journal is closed")
            connection = self._connection
            connection.execute("BEGIN IMMEDIATE")
            try:
                yield connection
            except BaseException:
                if connection.in_transaction:
                    connection.execute("ROLLBACK")
                raise
            connection.execute("COMMIT")

    def _load_chain_state(self) -> None:
        """Walk the chain once and cache the verified head."""
        with self._read_connection() as connection:
            walk = self._walk_chain(connection)
            self._apply_walk(walk)

    def _apply_walk(self, walk: _ChainWalk) -> None:
        """Cache the verified head and the first break a full walk found."""
        self._head_sequence = walk.verified_count
        self._head_digest = walk.head_digest
        self._first_bad_sequence = walk.first_bad_sequence

    def _resolve_append_head(self, connection: sqlite3.Connection) -> tuple[int, str]:
        """Return the sequence and the previous digest the new event links to.

        The fast path trusts the cached head when the newest stored row is
        still exactly that row, so a steady stream of appends costs one row
        read each. Anything else -- an edited head, an append by another
        instance on the same file -- falls back to a full walk, which is also
        the only way to notice an edit in the middle of the chain.
        """
        if self._first_bad_sequence is not None:
            raise JournalIntegrityError(
                "the studio session journal is broken at sequence "
                f"{self._first_bad_sequence}; refusing to extend it"
            )
        if self._cached_head_matches(connection):
            if self._head_digest is None:
                return 1, GENESIS_PREVIOUS_DIGEST
            return self._head_sequence + 1, self._head_digest
        walk = self._walk_chain(connection)
        self._apply_walk(walk)
        if walk.first_bad_sequence is not None:
            raise JournalIntegrityError(
                "the studio session journal is broken at sequence "
                f"{walk.first_bad_sequence}; refusing to extend it"
            )
        previous_event_digest = (
            GENESIS_PREVIOUS_DIGEST if walk.head_digest is None else walk.head_digest
        )
        return walk.verified_count + 1, previous_event_digest

    def _cached_head_matches(self, connection: sqlite3.Connection) -> bool:
        """Return whether the newest stored row is the cached, verified head.

        The row is re-verified against the digests it records itself: its
        payload digest must match the bytes it stores, and its chain digest
        must match the chain digest recomputed over the covered fields it
        stores, including its stored previous digest.
        """
        row = connection.execute(
            f"SELECT {CHAIN_COLUMNS} FROM studio_events ORDER BY sequence DESC LIMIT 1"
        ).fetchone()
        if row is None:
            return self._head_sequence == 0 and self._head_digest is None
        if int(row["sequence"]) != self._head_sequence:
            return False
        if str(row["event_digest"]) != self._head_digest:
            return False
        return _row_is_self_consistent(row)

    def _walk_chain(self, connection: sqlite3.Connection) -> _ChainWalk:
        """Walk the stored chain and report the first event that fails."""
        rows = connection.execute(
            f"SELECT {CHAIN_COLUMNS} FROM studio_events ORDER BY sequence ASC"
        ).fetchall()
        expected_sequence = 1
        previous_event_digest = GENESIS_PREVIOUS_DIGEST
        verified_count = 0
        first_bad_sequence: int | None = None
        for row in rows:
            sequence = int(row["sequence"])
            if sequence != expected_sequence:
                first_bad_sequence = expected_sequence
                break
            if str(row["previous_event_digest"]) != previous_event_digest:
                first_bad_sequence = sequence
                break
            if not _row_is_self_consistent(row):
                first_bad_sequence = sequence
                break
            previous_event_digest = str(row["event_digest"])
            expected_sequence = sequence + 1
            verified_count += 1
        return _ChainWalk(
            event_count=len(rows),
            verified_count=verified_count,
            head_digest=None if verified_count == 0 else previous_event_digest,
            first_bad_sequence=first_bad_sequence,
        )

    def _event_from_row(self, row: sqlite3.Row) -> SessionEvent:
        payload_json = str(row["payload_json"])
        return SessionEvent(
            sequence=int(row["sequence"]),
            event_type=str(row["event_type"]),
            aggregate_id=str(row["aggregate_id"]),
            payload=_decode_payload(payload_json),
            payload_json=payload_json,
            digest=str(row["digest"]),
            occurred_at=str(row["occurred_at"]),
            previous_event_digest=str(row["previous_event_digest"]),
            event_digest=str(row["event_digest"]),
        )


def _validate_identifier(value: str, field_name: str) -> str:
    try:
        return _IDENTIFIER_ADAPTER.validate_python(value)
    except ValidationError as error:
        raise ValueError(f"{field_name} must be a safe identifier") from error


def _canonical_json(payload: Mapping[str, object]) -> str:
    try:
        return _CANONICAL_JSON_ADAPTER.validate_python(_to_jsonable(payload))
    except ValidationError as error:
        raise ValueError("value must be a canonical JSON object") from error


def _to_jsonable(value: object) -> object:
    """Convert nested mappings and sequences into plain JSON containers."""
    if isinstance(value, Mapping):
        return {str(key): _to_jsonable(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_to_jsonable(item) for item in value]
    return value


def _decode_payload(payload_json: str) -> Mapping[str, object] | None:
    """Return the decoded payload, or ``None`` when the text is not an object."""
    try:
        decoded = json.loads(payload_json)
    except (json.JSONDecodeError, RecursionError, ValueError):
        return None
    if not isinstance(decoded, dict):
        return None
    return cast("Mapping[str, object]", decoded)


def _row_is_self_consistent(row: sqlite3.Row) -> bool:
    """Return whether one stored row verifies against its own recorded digests.

    The row's payload digest must be the SHA-256 of the payload text it
    stores, and its chain digest must be the chain digest recomputed over
    the covered fields the row itself records, including the previous
    digest it stores. A row that passes is a well-formed event; whether it
    belongs to *this* chain is decided by the event before it, which is
    what the link to the stored previous digest commits to.
    """
    payload_json = str(row["payload_json"])
    if hashlib.sha256(payload_json.encode("utf-8")).hexdigest() != str(row["digest"]):
        return False
    stored_event_digest = row["event_digest"]
    if not is_chain_digest(stored_event_digest):
        return False
    return chain_digest(chain_fields_from_row(row)) == str(stored_event_digest)


def _timestamp_to_iso(value: datetime) -> str:
    """Return the UTC ISO-8601 form the core journal stores, to the microsecond."""
    if value.tzinfo is None or value.utcoffset() is None:
        raise ValueError("timestamp must include a timezone")
    return value.astimezone(UTC).isoformat(timespec="microseconds").replace("+00:00", "Z")


def _timestamp_from_iso(value: str) -> datetime:
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError("stored timestamp is invalid") from error
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        raise ValueError("stored timestamp is invalid")
    return parsed.astimezone(UTC)


def _require_positive_int(value: int, field_name: str) -> None:
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError(f"{field_name} must be an integer")
    if value < 1:
        raise ValueError(f"{field_name} must be positive")


def _require_nonnegative_int(value: int, field_name: str) -> None:
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError(f"{field_name} must be an integer")
    if value < 0:
        raise ValueError(f"{field_name} must be nonnegative")


def _tighten_directory_mode(path: Path) -> None:
    """Set owner-only permissions on ``path`` where the platform supports it."""
    if _IS_WINDOWS:
        return
    try:
        os.chmod(path, _DATA_DIRECTORY_MODE)
    except OSError:
        return


def _tighten_file_mode(path: Path) -> None:
    """Set owner-only permissions on ``path`` where the platform supports it."""
    if _IS_WINDOWS:
        return
    try:
        os.chmod(path, _DATABASE_FILE_MODE)
    except OSError:
        return
