from __future__ import annotations

import hashlib
import json
import sqlite3
from collections.abc import Mapping
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import cast

from kala_agent.persistence.chain import (
    CHAIN_COLUMNS,
    GENESIS_PREVIOUS_DIGEST,
    EventChainFields,
    JournalIntegrityError,
    chain_digest,
    chain_fields_from_row,
    is_chain_digest,
)
from kala_agent.persistence.database import (
    Database,
    JsonObject,
    _canonical_json_object,
    _timestamp_from_iso,
    _timestamp_to_iso,
    _validate_digest,
    _validate_identifier,
)

_INSERT_EVENT: str = """
    INSERT INTO events(
        sequence, event_type, aggregate_id, workflow_id, run_id, payload_json,
        digest, occurred_at, previous_event_digest, event_digest
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    """


@dataclass(frozen=True, slots=True)
class ChainVerification:
    """The verified state of a whole journal.

    ``head_event_digest`` is the value to anchor outside the database: a journal
    that was rewritten after the fact ends in a different head digest even when
    its own chain is internally consistent.
    """

    event_count: int
    head_sequence: int | None
    head_event_digest: str | None


@dataclass(frozen=True, slots=True)
class Event:
    sequence: int
    event_type: str
    aggregate_id: str
    workflow_id: str | None
    run_id: str | None
    payload_json: str
    digest: str
    timestamp: datetime
    previous_event_digest: str
    event_digest: str

    @property
    def payload(self) -> JsonObject:
        return cast(JsonObject, json.loads(self.payload_json))


class EventJournal:
    """The append-only event journal, chained and verified before it is read.

    An event is only ever inserted, and it is inserted with a digest that binds
    it to the event before it. The whole chain is verified when a journal is
    built and again before every read, so an edited, dropped, or reordered event
    makes a read raise instead of returning the rows it managed to load. An
    append additionally verifies the event it extends, so a write cannot extend
    a chain that is already broken at its head.
    """

    def __init__(self, database: Database) -> None:
        self._database = database
        self.verify_chain()

    def append(
        self,
        event_type: str,
        aggregate_id: str,
        payload: str | Mapping[str, object],
        *,
        workflow_id: str | None = None,
        run_id: str | None = None,
        timestamp: datetime | None = None,
    ) -> Event:
        """Append one event and return it with the chain digests it committed to.

        The sequence is taken from the journal tail while the writer lock is
        held, so a concurrent append can neither reuse nor skip a sequence.
        """
        normalized_event_type = _validate_identifier(event_type, "event_type")
        normalized_aggregate_id = _validate_identifier(aggregate_id, "aggregate_id")
        normalized_workflow_id = (
            None if workflow_id is None else _validate_identifier(workflow_id, "workflow_id")
        )
        normalized_run_id = None if run_id is None else _validate_identifier(run_id, "run_id")
        payload_json = _canonical_json_object(payload)
        payload_digest = hashlib.sha256(payload_json.encode("utf-8")).hexdigest()
        occurred_at = timestamp if timestamp is not None else datetime.now(UTC)
        occurred_at_iso = _timestamp_to_iso(occurred_at)
        with self._database.transaction() as connection:
            previous_sequence, previous_event_digest = self._verified_tail(connection)
            sequence = 1 if previous_sequence is None else previous_sequence + 1
            event_digest = chain_digest(
                EventChainFields(
                    sequence=sequence,
                    event_type=normalized_event_type,
                    aggregate_id=normalized_aggregate_id,
                    workflow_id=normalized_workflow_id,
                    run_id=normalized_run_id,
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
                    normalized_workflow_id,
                    normalized_run_id,
                    payload_json,
                    payload_digest,
                    occurred_at_iso,
                    previous_event_digest,
                    event_digest,
                ),
            )
        return Event(
            sequence=sequence,
            event_type=normalized_event_type,
            aggregate_id=normalized_aggregate_id,
            workflow_id=normalized_workflow_id,
            run_id=normalized_run_id,
            payload_json=payload_json,
            digest=payload_digest,
            timestamp=occurred_at.astimezone(UTC),
            previous_event_digest=previous_event_digest,
            event_digest=event_digest,
        )

    def get(self, sequence: int) -> Event | None:
        if isinstance(sequence, bool) or not isinstance(sequence, int):
            raise TypeError("sequence must be an integer")
        if sequence < 0:
            raise ValueError("sequence must be nonnegative")
        self.verify_chain()
        with self._database.serialized() as connection:
            row = connection.execute(
                f"SELECT {CHAIN_COLUMNS} FROM events WHERE sequence = ?",
                (sequence,),
            ).fetchone()
        return None if row is None else self._event_from_row(row)

    def read(
        self,
        *,
        event_type: str | None = None,
        aggregate_id: str | None = None,
        workflow_id: str | None = None,
        run_id: str | None = None,
        after_sequence: int = 0,
        limit: int | None = 1_000,
    ) -> tuple[Event, ...]:
        normalized_event_type = (
            None if event_type is None else _validate_identifier(event_type, "event_type")
        )
        normalized_aggregate_id = (
            None if aggregate_id is None else _validate_identifier(aggregate_id, "aggregate_id")
        )
        normalized_workflow_id = (
            None if workflow_id is None else _validate_identifier(workflow_id, "workflow_id")
        )
        normalized_run_id = None if run_id is None else _validate_identifier(run_id, "run_id")
        if isinstance(after_sequence, bool) or not isinstance(after_sequence, int):
            raise TypeError("after_sequence must be an integer")
        if after_sequence < 0:
            raise ValueError("after_sequence must be nonnegative")
        if limit is not None:
            if isinstance(limit, bool) or not isinstance(limit, int):
                raise TypeError("limit must be an integer or None")
            if limit < 1:
                raise ValueError("limit must be positive")
        self.verify_chain()
        with self._database.serialized() as connection:
            rows = connection.execute(
                f"""
                SELECT {CHAIN_COLUMNS}
                FROM events
                WHERE (:event_type IS NULL OR event_type = :event_type)
                  AND (:aggregate_id IS NULL OR aggregate_id = :aggregate_id)
                  AND (:workflow_id IS NULL OR workflow_id = :workflow_id)
                  AND (:run_id IS NULL OR run_id = :run_id)
                  AND sequence > :after_sequence
                ORDER BY sequence ASC
                LIMIT :limit
                """,
                {
                    "event_type": normalized_event_type,
                    "aggregate_id": normalized_aggregate_id,
                    "workflow_id": normalized_workflow_id,
                    "run_id": normalized_run_id,
                    "after_sequence": after_sequence,
                    "limit": -1 if limit is None else limit,
                },
            ).fetchall()
        return tuple(self._event_from_row(row) for row in rows)

    def verify_chain(self) -> ChainVerification:
        """Verify sequence continuity and every chain digest, failing closed.

        The whole journal is walked from the genesis event, so a row that is
        edited, omitted, or reordered is found by the first event whose recorded
        digest no longer matches its content or no longer links to its
        predecessor, whatever that event is.
        """
        with self._database.serialized() as connection:
            expected_sequence = 1
            previous_event_digest = GENESIS_PREVIOUS_DIGEST
            rows = connection.execute(f"SELECT {CHAIN_COLUMNS} FROM events ORDER BY sequence ASC")
            for row in rows:
                sequence = int(row["sequence"])
                if sequence != expected_sequence:
                    raise JournalIntegrityError(
                        "event journal continuity is broken: expected sequence "
                        f"{expected_sequence} but found {sequence}"
                    )
                fields = chain_fields_from_row(row)
                if fields.previous_event_digest != previous_event_digest:
                    raise JournalIntegrityError(
                        f"event {sequence} does not link to the preceding event digest"
                    )
                payload_json = str(row["payload_json"])
                if (
                    hashlib.sha256(payload_json.encode("utf-8")).hexdigest()
                    != fields.payload_digest
                ):
                    raise JournalIntegrityError(f"event {sequence} failed payload verification")
                event_digest = _stored_chain_digest(row["event_digest"], sequence)
                if chain_digest(fields) != event_digest:
                    raise JournalIntegrityError(f"event {sequence} failed chain verification")
                previous_event_digest = event_digest
                expected_sequence += 1
        event_count = expected_sequence - 1
        return ChainVerification(
            event_count=event_count,
            head_sequence=None if event_count == 0 else event_count,
            head_event_digest=None if event_count == 0 else previous_event_digest,
        )

    def _verified_tail(self, connection: sqlite3.Connection) -> tuple[int | None, str]:
        row = connection.execute(
            f"SELECT {CHAIN_COLUMNS} FROM events ORDER BY sequence DESC LIMIT 1"
        ).fetchone()
        if row is None:
            return None, GENESIS_PREVIOUS_DIGEST
        sequence = int(row["sequence"])
        fields = chain_fields_from_row(row)
        payload_json = str(row["payload_json"])
        if hashlib.sha256(payload_json.encode("utf-8")).hexdigest() != fields.payload_digest:
            raise JournalIntegrityError(f"event {sequence} failed payload verification")
        event_digest = _stored_chain_digest(row["event_digest"], sequence)
        if chain_digest(fields) != event_digest:
            raise JournalIntegrityError(f"event {sequence} failed chain verification")
        return sequence, event_digest

    def _event_from_row(self, row: sqlite3.Row) -> Event:
        payload_json = _canonical_json_object(str(row["payload_json"]))
        if payload_json != row["payload_json"]:
            raise ValueError("stored event payload is not canonical")
        sequence = int(row["sequence"])
        digest = _validate_digest(str(row["digest"]), "event digest")
        calculated_digest = hashlib.sha256(payload_json.encode("utf-8")).hexdigest()
        if digest != calculated_digest:
            raise ValueError("event digest verification failed")
        event_digest = _stored_chain_digest(row["event_digest"], sequence)
        previous_event_digest = _stored_chain_digest(row["previous_event_digest"], sequence)
        if chain_digest(chain_fields_from_row(row)) != event_digest:
            raise JournalIntegrityError(f"event {sequence} failed chain verification")
        return Event(
            sequence=sequence,
            event_type=_validate_identifier(str(row["event_type"]), "event_type"),
            aggregate_id=_validate_identifier(str(row["aggregate_id"]), "aggregate_id"),
            workflow_id=(
                None
                if row["workflow_id"] is None
                else _validate_identifier(str(row["workflow_id"]), "workflow_id")
            ),
            run_id=(
                None
                if row["run_id"] is None
                else _validate_identifier(str(row["run_id"]), "run_id")
            ),
            payload_json=payload_json,
            digest=digest,
            timestamp=_timestamp_from_iso(str(row["occurred_at"])),
            previous_event_digest=previous_event_digest,
            event_digest=event_digest,
        )


def _stored_chain_digest(value: object, sequence: int) -> str:
    if not is_chain_digest(value):
        raise JournalIntegrityError(f"event {sequence} does not record a valid chain digest")
    return cast(str, value)
