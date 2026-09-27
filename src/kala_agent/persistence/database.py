from __future__ import annotations

import hashlib
import os
import sqlite3
import threading
from collections.abc import Callable, Iterator, Mapping
from contextlib import contextmanager
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from types import TracebackType
from typing import Final, Self, TypeAlias

from pydantic import TypeAdapter, ValidationError

from kala_agent.config import Settings
from kala_agent.contracts.models import CanonicalJsonObject, Identifier, Sha256Digest
from kala_agent.persistence.chain import backfill_event_chain

_IDENTIFIER_ADAPTER: TypeAdapter[str] = TypeAdapter(Identifier)
_DIGEST_ADAPTER: TypeAdapter[str] = TypeAdapter(Sha256Digest)
_JSON_OBJECT_ADAPTER: TypeAdapter[str] = TypeAdapter(CanonicalJsonObject)

MigrationStep: TypeAlias = "str | Callable[[sqlite3.Connection], None]"

_APPEND_ONLY_UPDATE_TRIGGER: Final[str] = """
    CREATE TRIGGER events_prevent_update
    BEFORE UPDATE ON events
    BEGIN
        SELECT RAISE(ABORT, 'events are append-only');
    END
    """

_APPEND_ONLY_DELETE_TRIGGER: Final[str] = """
    CREATE TRIGGER events_prevent_delete
    BEFORE DELETE ON events
    BEGIN
        SELECT RAISE(ABORT, 'events are append-only');
    END
    """

_CHAINED_INSERT_TRIGGER: Final[str] = """
    CREATE TRIGGER events_require_event_chain
    BEFORE INSERT ON events
    WHEN length(NEW.previous_event_digest) <> 64
      OR length(NEW.event_digest) <> 64
      OR NEW.previous_event_digest <> lower(NEW.previous_event_digest)
      OR NEW.event_digest <> lower(NEW.event_digest)
    BEGIN
        SELECT RAISE(ABORT, 'events require a hash chain');
    END
    """


_BASE_EXECUTE: Final[Callable[..., sqlite3.Cursor]] = sqlite3.Connection.execute
_BASE_EXECUTEMANY: Final[Callable[..., sqlite3.Cursor]] = sqlite3.Connection.executemany


class _SerializedConnection(sqlite3.Connection):
    """A connection that serializes every statement on one writer lock.

    ``Database`` holds the lock across whole logical operations, so a
    multi-statement operation cannot interleave with another thread. This class
    closes the remaining escape hatch: code that only ever holds the connection
    still cannot interleave a statement with another thread's transaction.
    """

    lock: threading.RLock

    def execute(self, sql: str, parameters: object = (), /) -> sqlite3.Cursor:
        with self.lock:
            return _BASE_EXECUTE(self, sql, parameters)

    def executemany(self, sql: str, parameters: object, /) -> sqlite3.Cursor:
        with self.lock:
            return _BASE_EXECUTEMANY(self, sql, parameters)

    def executescript(self, sql_script: str, /) -> sqlite3.Cursor:
        with self.lock:
            return super().executescript(sql_script)

    def commit(self) -> None:
        with self.lock:
            super().commit()

    def rollback(self) -> None:
        with self.lock:
            super().rollback()


@dataclass(frozen=True, slots=True)
class SchemaMigration:
    """One ordered, versioned schema change.

    A step is either a statement applied to the open connection or a callable
    that receives it, which is how the one data backfill this schema needs is
    expressed without weakening the append-only triggers around it.
    """

    version: int
    name: str
    steps: tuple[MigrationStep, ...]


_STATE_MIGRATION: tuple[SchemaMigration, ...] = (
    SchemaMigration(
        1,
        "initial_persistence_schema",
        (
            """
            CREATE TABLE events (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT NOT NULL,
                aggregate_id TEXT NOT NULL,
                workflow_id TEXT,
                run_id TEXT,
                payload_json TEXT NOT NULL,
                digest TEXT NOT NULL,
                occurred_at TEXT NOT NULL
            )
            """,
            """
            CREATE INDEX events_type_sequence_idx
            ON events(event_type, sequence)
            """,
            """
            CREATE INDEX events_aggregate_sequence_idx
            ON events(aggregate_id, sequence)
            """,
            """
            CREATE INDEX events_workflow_sequence_idx
            ON events(workflow_id, sequence)
            """,
            """
            CREATE INDEX events_run_sequence_idx
            ON events(run_id, sequence)
            """,
            _APPEND_ONLY_UPDATE_TRIGGER,
            _APPEND_ONLY_DELETE_TRIGGER,
            """
            CREATE TABLE design_specs (
                id TEXT PRIMARY KEY,
                model_json TEXT NOT NULL
            )
            """,
            """
            CREATE TABLE design_revisions (
                revision_id TEXT PRIMARY KEY,
                design_id TEXT NOT NULL,
                sequence INTEGER NOT NULL,
                model_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE(design_id, sequence)
            )
            """,
            """
            CREATE INDEX design_revisions_design_sequence_idx
            ON design_revisions(design_id, sequence)
            """,
            """
            CREATE TABLE approval_requests (
                request_id TEXT PRIMARY KEY,
                workflow_id TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                model_json TEXT NOT NULL
            )
            """,
            """
            CREATE INDEX approval_requests_workflow_created_idx
            ON approval_requests(workflow_id, created_at, request_id)
            """,
            """
            CREATE TABLE tool_calls (
                call_id TEXT PRIMARY KEY,
                tool_id TEXT NOT NULL,
                issued_at TEXT NOT NULL,
                model_json TEXT NOT NULL
            )
            """,
            """
            CREATE INDEX tool_calls_tool_issued_idx
            ON tool_calls(tool_id, issued_at, call_id)
            """,
            """
            CREATE TABLE tool_results (
                call_id TEXT PRIMARY KEY,
                tool_id TEXT NOT NULL,
                status TEXT NOT NULL,
                model_json TEXT NOT NULL,
                FOREIGN KEY(call_id) REFERENCES tool_calls(call_id)
                    ON UPDATE RESTRICT ON DELETE RESTRICT
            )
            """,
            """
            CREATE INDEX tool_results_tool_status_idx
            ON tool_results(tool_id, status, call_id)
            """,
            """
            CREATE TABLE memory_entries (
                memory_id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                provenance_id TEXT,
                created_at TEXT NOT NULL,
                expires_at TEXT,
                model_json TEXT NOT NULL
            )
            """,
            """
            CREATE INDEX memory_entries_source_created_idx
            ON memory_entries(source, created_at, memory_id)
            """,
            """
            CREATE INDEX memory_entries_expiry_idx
            ON memory_entries(expires_at)
            """,
            """
            CREATE TABLE workflow_snapshots (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                snapshot_id TEXT NOT NULL UNIQUE,
                workflow_id TEXT NOT NULL,
                state_json TEXT NOT NULL,
                digest TEXT NOT NULL,
                created_at TEXT NOT NULL
            )
            """,
            """
            CREATE INDEX workflow_snapshots_workflow_sequence_idx
            ON workflow_snapshots(workflow_id, sequence)
            """,
        ),
    ),
    SchemaMigration(
        2,
        "journal_event_hash_chain",
        (
            "ALTER TABLE events ADD COLUMN previous_event_digest TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE events ADD COLUMN event_digest TEXT NOT NULL DEFAULT ''",
            "DROP TRIGGER IF EXISTS events_prevent_update",
            backfill_event_chain,
            _APPEND_ONLY_UPDATE_TRIGGER,
            _CHAINED_INSERT_TRIGGER,
        ),
    ),
)


def _validate_identifier(value: str, field_name: str = "identifier") -> str:
    try:
        return _IDENTIFIER_ADAPTER.validate_python(value)
    except ValidationError as error:
        raise ValueError(f"{field_name} must be a safe identifier") from error


def _validate_digest(value: str, field_name: str = "digest") -> str:
    try:
        return _DIGEST_ADAPTER.validate_python(value)
    except ValidationError as error:
        raise ValueError(f"{field_name} must be a SHA-256 digest") from error


def _canonical_json_object(value: str | Mapping[str, object]) -> str:
    try:
        return _JSON_OBJECT_ADAPTER.validate_python(value)
    except ValidationError as error:
        raise ValueError("value must be a canonical JSON object") from error


def _timestamp_to_iso(value: datetime) -> str:
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


class Database:
    """One SQLite database, written by one serialized writer.

    A single connection is shared by every caller, and a single transaction on a
    shared connection cannot belong to two threads at once: a second thread that
    joins the first thread's transaction would have its own work committed, or
    rolled back, by the other thread. Every transaction and every connection
    access therefore runs under one reentrant per-database lock, so exactly one
    thread writes at a time and no acknowledged write is ever lost.
    """

    def __init__(
        self,
        path: Path | str = ":memory:",
        *,
        busy_timeout_ms: int = 5_000,
    ) -> None:
        if isinstance(busy_timeout_ms, bool) or not isinstance(busy_timeout_ms, int):
            raise TypeError("busy_timeout_ms must be an integer")
        if not 0 <= busy_timeout_ms <= 2_147_483_647:
            raise ValueError("busy_timeout_ms must be between 0 and 2147483647")
        if isinstance(path, str) and path == ":memory:":
            self.path: Path | str = path
            connection_target = path
        else:
            candidate = (path if isinstance(path, Path) else Path(path)).expanduser()
            candidate.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            os.chmod(candidate.parent, 0o700)
            self.path = candidate
            connection_target = str(candidate)
        self._busy_timeout_ms = busy_timeout_ms
        self._closed = False
        self._writer_lock = threading.RLock()
        self._connection = sqlite3.connect(
            connection_target,
            timeout=busy_timeout_ms / 1_000.0,
            isolation_level=None,
            check_same_thread=False,
            factory=_SerializedConnection,
        )
        self._connection.lock = self._writer_lock
        self._connection.row_factory = sqlite3.Row
        if not (isinstance(path, str) and path == ":memory:"):
            os.chmod(connection_target, 0o600)
        try:
            self._connection.execute("PRAGMA foreign_keys = ON")
            self._connection.execute("PRAGMA journal_mode = WAL")
            self.initialize()
        except BaseException:
            self._connection.close()
            self._closed = True
            raise

    @classmethod
    def open(
        cls,
        path: Path | str = ":memory:",
        *,
        busy_timeout_ms: int = 5_000,
    ) -> Self:
        return cls(path, busy_timeout_ms=busy_timeout_ms)

    @classmethod
    def from_settings(
        cls,
        settings: Settings,
        *,
        busy_timeout_ms: int = 5_000,
    ) -> Self:
        return cls(settings.database_path, busy_timeout_ms=busy_timeout_ms)

    @property
    def connection(self) -> sqlite3.Connection:
        """Return the shared connection, which serializes every statement it runs."""
        if self._closed:
            raise RuntimeError("database is closed")
        return self._connection

    @property
    def busy_timeout_ms(self) -> int:
        return self._busy_timeout_ms

    @property
    def closed(self) -> bool:
        return self._closed

    def initialize(self) -> None:
        """Apply every outstanding migration, atomically and in version order."""
        with self._writer_lock:
            connection = self.connection
            connection.execute(
                """
                CREATE TABLE IF NOT EXISTS schema_migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                )
                """
            )
            with self.transaction():
                applied_rows = connection.execute(
                    "SELECT version FROM schema_migrations ORDER BY version"
                ).fetchall()
                applied_versions = frozenset(int(row["version"]) for row in applied_rows)
                known_versions = frozenset(migration.version for migration in _STATE_MIGRATION)
                if applied_versions - known_versions:
                    raise RuntimeError("database schema is newer than this application")
                for migration in _STATE_MIGRATION:
                    if migration.version in applied_versions:
                        continue
                    self._apply_migration(connection, migration)
                    connection.execute(
                        "INSERT INTO schema_migrations(version, name, applied_at) VALUES (?, ?, ?)",
                        (
                            migration.version,
                            migration.name,
                            _timestamp_to_iso(datetime.now(UTC)),
                        ),
                    )

    @property
    def schema_version(self) -> int:
        with self._writer_lock:
            row = self.connection.execute(
                "SELECT MAX(version) AS version FROM schema_migrations"
            ).fetchone()
        if row is None or row["version"] is None:
            return 0
        return int(row["version"])

    @contextmanager
    def serialized(self) -> Iterator[sqlite3.Connection]:
        """Hold the writer lock so a read cannot interleave with another thread."""
        with self._writer_lock:
            yield self.connection

    @contextmanager
    def transaction(self) -> Iterator[sqlite3.Connection]:
        """Run a write transaction, or a savepoint when one is already open."""
        with self._writer_lock:
            connection = self.connection
            nested = connection.in_transaction
            connection.execute("SAVEPOINT kala_agent_persistence" if nested else "BEGIN IMMEDIATE")
            try:
                yield connection
                connection.execute(
                    "RELEASE SAVEPOINT kala_agent_persistence" if nested else "COMMIT"
                )
            except BaseException:
                if nested:
                    if connection.in_transaction:
                        connection.execute("ROLLBACK TO SAVEPOINT kala_agent_persistence")
                        connection.execute("RELEASE SAVEPOINT kala_agent_persistence")
                elif connection.in_transaction:
                    connection.execute("ROLLBACK")
                raise

    def close(self) -> None:
        with self._writer_lock:
            if self._closed:
                return
            if self._connection.in_transaction:
                self._connection.execute("ROLLBACK")
            self._connection.close()
            self._closed = True

    def _apply_migration(self, connection: sqlite3.Connection, migration: SchemaMigration) -> None:
        for step in migration.steps:
            if isinstance(step, str):
                connection.execute(step)
            else:
                step(connection)

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.close()


JsonObject: TypeAlias = dict[str, object]


@dataclass(frozen=True, slots=True)
class WorkflowSnapshot:
    sequence: int
    snapshot_id: str
    workflow_id: str
    state_json: str
    digest: str
    created_at: datetime


class SnapshotStore:
    def __init__(self, database: Database) -> None:
        self._database = database

    def save(
        self,
        snapshot_id: str,
        workflow_id: str,
        state: str | Mapping[str, object],
        *,
        created_at: datetime | None = None,
    ) -> WorkflowSnapshot:
        normalized_snapshot_id = _validate_identifier(snapshot_id, "snapshot_id")
        normalized_workflow_id = _validate_identifier(workflow_id, "workflow_id")
        state_json = _canonical_json_object(state)
        digest = hashlib.sha256(state_json.encode("utf-8")).hexdigest()
        timestamp = created_at if created_at is not None else datetime.now(UTC)
        timestamp_iso = _timestamp_to_iso(timestamp)
        with self._database.transaction() as connection:
            cursor = connection.execute(
                """
                INSERT INTO workflow_snapshots(
                    snapshot_id, workflow_id, state_json, digest, created_at
                ) VALUES (?, ?, ?, ?, ?)
                """,
                (
                    normalized_snapshot_id,
                    normalized_workflow_id,
                    state_json,
                    digest,
                    timestamp_iso,
                ),
            )
            last_row_id = cursor.lastrowid
            if last_row_id is None:
                raise RuntimeError("snapshot insert did not produce a row identifier")
            sequence = int(last_row_id)
        return WorkflowSnapshot(
            sequence=sequence,
            snapshot_id=normalized_snapshot_id,
            workflow_id=normalized_workflow_id,
            state_json=state_json,
            digest=digest,
            created_at=timestamp.astimezone(UTC),
        )

    def get(self, snapshot_id: str) -> WorkflowSnapshot | None:
        normalized_snapshot_id = _validate_identifier(snapshot_id, "snapshot_id")
        with self._database.serialized() as connection:
            row = connection.execute(
                """
                SELECT sequence, snapshot_id, workflow_id, state_json, digest, created_at
                FROM workflow_snapshots
                WHERE snapshot_id = ?
                """,
                (normalized_snapshot_id,),
            ).fetchone()
        return None if row is None else self._snapshot_from_row(row)

    def latest(self, workflow_id: str) -> WorkflowSnapshot | None:
        normalized_workflow_id = _validate_identifier(workflow_id, "workflow_id")
        with self._database.serialized() as connection:
            row = connection.execute(
                """
                SELECT sequence, snapshot_id, workflow_id, state_json, digest, created_at
                FROM workflow_snapshots
                WHERE workflow_id = ?
                ORDER BY sequence DESC
                LIMIT 1
                """,
                (normalized_workflow_id,),
            ).fetchone()
        return None if row is None else self._snapshot_from_row(row)

    def restore(self, snapshot_id: str) -> str | None:
        snapshot = self.get(snapshot_id)
        return None if snapshot is None else snapshot.state_json

    def restore_latest(self, workflow_id: str) -> str | None:
        snapshot = self.latest(workflow_id)
        return None if snapshot is None else snapshot.state_json

    def _snapshot_from_row(self, row: sqlite3.Row) -> WorkflowSnapshot:
        state_json = _canonical_json_object(str(row["state_json"]))
        if state_json != row["state_json"]:
            raise ValueError("stored snapshot JSON is not canonical")
        digest = hashlib.sha256(state_json.encode("utf-8")).hexdigest()
        if digest != row["digest"]:
            raise ValueError("snapshot digest verification failed")
        return WorkflowSnapshot(
            sequence=int(row["sequence"]),
            snapshot_id=_validate_identifier(str(row["snapshot_id"]), "snapshot_id"),
            workflow_id=_validate_identifier(str(row["workflow_id"]), "workflow_id"),
            state_json=state_json,
            digest=str(row["digest"]),
            created_at=_timestamp_from_iso(str(row["created_at"])),
        )
