"""The hash chain that makes journal tampering detectable.

Every journal event stores an ``event_digest`` that commits to the event's own
immutable content and to the ``event_digest`` of the event before it, so the
journal is a chain from its genesis event rather than a set of independent rows.
Omitting an event, reordering events, or editing the content of any event all
break every digest that follows the edit, and the break is found by recomputing
the chain from the genesis event.

The chain is tamper-evident, not tamper-proof. Anyone who can write the database
can rewrite a whole suffix of the chain consistently, so the head digest is
meant to be anchored outside the database; ``EventJournal.verify_chain`` returns
it for exactly that purpose, and callers compare it against their own record.
"""

from __future__ import annotations

import hashlib
import sqlite3
from dataclasses import dataclass, replace
from typing import Final

CHAIN_ALGORITHM: Final[str] = "sha256"
CHAIN_DOMAIN: Final[str] = f"kala-agent.event-chain.v1.{CHAIN_ALGORITHM}"
GENESIS_PREVIOUS_DIGEST: Final[str] = "0" * 64
CHAIN_COLUMNS: Final[str] = (
    "sequence, event_type, aggregate_id, workflow_id, run_id,"
    " payload_json, digest, occurred_at, previous_event_digest, event_digest"
)
_DIGEST_LENGTH: Final[int] = 64
_LENGTH_PREFIX_BYTES: Final[int] = 8
_LOWERCASE_HEX: Final[frozenset[str]] = frozenset("0123456789abcdef")


def _length_prefixed(part: str) -> bytes:
    encoded = part.encode("utf-8")
    return len(encoded).to_bytes(_LENGTH_PREFIX_BYTES, "big") + encoded


_CHAIN_DOMAIN_PREFIX: Final[bytes] = _length_prefixed(CHAIN_DOMAIN)


class JournalIntegrityError(RuntimeError):
    """Raised when a stored row does not belong to the journal hash chain."""


@dataclass(frozen=True, slots=True)
class EventChainFields:
    """The immutable content that a single journal event digest commits to."""

    sequence: int
    event_type: str
    aggregate_id: str
    workflow_id: str | None
    run_id: str | None
    occurred_at: str
    payload_digest: str
    previous_event_digest: str

    def as_preimage_parts(self) -> tuple[str, ...]:
        """Return the covered values, with an absent optional id as an empty part."""
        return (
            str(self.sequence),
            self.event_type,
            self.aggregate_id,
            self.workflow_id or "",
            self.run_id or "",
            self.occurred_at,
            self.payload_digest,
            self.previous_event_digest,
        )


def chain_digest(fields: EventChainFields) -> str:
    """Return the digest that binds one journal event to the event before it."""
    digest = hashlib.sha256()
    update = digest.update
    update(_CHAIN_DOMAIN_PREFIX)
    for part in fields.as_preimage_parts():
        encoded = part.encode("utf-8")
        update(len(encoded).to_bytes(_LENGTH_PREFIX_BYTES, "big"))
        update(encoded)
    return digest.hexdigest()


def is_chain_digest(value: object) -> bool:
    """Return whether a stored value is a lowercase SHA-256 digest."""
    return (
        isinstance(value, str) and len(value) == _DIGEST_LENGTH and _LOWERCASE_HEX.issuperset(value)
    )


def chain_fields_from_row(row: sqlite3.Row) -> EventChainFields:
    """Read the covered fields out of a stored journal row."""
    return EventChainFields(
        sequence=int(row["sequence"]),
        event_type=str(row["event_type"]),
        aggregate_id=str(row["aggregate_id"]),
        workflow_id=None if row["workflow_id"] is None else str(row["workflow_id"]),
        run_id=None if row["run_id"] is None else str(row["run_id"]),
        occurred_at=str(row["occurred_at"]),
        payload_digest=str(row["digest"]),
        previous_event_digest=str(row["previous_event_digest"]),
    )


def backfill_event_chain(connection: sqlite3.Connection) -> None:
    """Chain every pre-existing event so a migrated journal keeps one history.

    The migration runs this between dropping and recreating the append-only
    update trigger, because writing the chain of a pre-chain journal is the one
    time the journal is ever rewritten. A journal that already fails its own
    payload digests, or that is missing sequences, is refused instead of being
    blessed, so a corrupt history is never promoted into a chained one.
    """
    rows = connection.execute(
        f"SELECT {CHAIN_COLUMNS} FROM events ORDER BY sequence ASC"
    ).fetchall()
    previous_event_digest = GENESIS_PREVIOUS_DIGEST
    for expected_sequence, row in enumerate(rows, start=1):
        sequence = int(row["sequence"])
        if sequence != expected_sequence:
            raise JournalIntegrityError(
                f"event journal is missing sequence {expected_sequence}; refusing to chain it"
            )
        payload_json = str(row["payload_json"])
        if hashlib.sha256(payload_json.encode("utf-8")).hexdigest() != str(row["digest"]):
            raise JournalIntegrityError(f"event {sequence} failed payload digest verification")
        fields = replace(chain_fields_from_row(row), previous_event_digest=previous_event_digest)
        event_digest = chain_digest(fields)
        connection.execute(
            """
            UPDATE events
            SET previous_event_digest = ?, event_digest = ?
            WHERE sequence = ?
            """,
            (previous_event_digest, event_digest, sequence),
        )
        previous_event_digest = event_digest
