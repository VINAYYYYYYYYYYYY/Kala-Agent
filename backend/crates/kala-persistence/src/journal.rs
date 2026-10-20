//! The append-only event journal.
//!
//! Every append commits to the previous event digest under the frozen chain
//! domain `kala-agent.event-chain.v1.sha256`. A journal is fully verified when
//! it is opened and again before every read or `get`, so an edited, dropped or
//! reordered row is detected even when it is outside the caller's filters.

use chrono::{DateTime, Utc};
use kala_json::{
    CanonicalValue, ChainVerification as TypedChainVerification, EventChainFields,
    GENESIS_PREVIOUS_DIGEST, Sha256Digest, chain_digest,
};
use rusqlite::{Connection, Row, params};

use crate::canonical::{
    canonicalize_json_object, canonicalize_object_value, parse_json_object, parse_stored_digest,
    sha256_text,
};
use crate::database::Database;
use crate::error::{JournalIntegrityError, PersistenceError};
use crate::time::{timestamp_from_iso, timestamp_to_iso, truncate_to_micros};

/// The columns carrying every chain-covered event value.
pub const CHAIN_COLUMNS: &str = "sequence, event_type, aggregate_id, workflow_id, run_id, \
                                 payload_json, digest, occurred_at, previous_event_digest, \
                                 event_digest";

const INSERT_EVENT: &str = "
    INSERT INTO events(
        sequence, event_type, aggregate_id, workflow_id, run_id, payload_json,
        digest, occurred_at, previous_event_digest, event_digest
    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
    ";

/// A verified journal state, including the head digest to anchor elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainVerification {
    /// Number of verified events.
    pub event_count: u64,
    /// Last verified sequence, or `None` for an empty journal.
    pub head_sequence: Option<i64>,
    /// Last verified event digest, or `None` for an empty journal.
    pub head_event_digest: Option<Sha256Digest>,
}

impl ChainVerification {
    /// Convert the shared `kala-json` chain result where the domains overlap.
    pub fn from_typed(value: TypedChainVerification) -> Self {
        Self {
            event_count: value.event_count,
            head_sequence: value.head_sequence.map(|sequence| sequence as i64),
            head_event_digest: value.head_event_digest,
        }
    }
}

/// One verified journal event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// One-based sequence.
    pub sequence: i64,
    /// Event type identifier.
    pub event_type: String,
    /// Aggregate identifier.
    pub aggregate_id: String,
    /// Optional workflow identifier.
    pub workflow_id: Option<String>,
    /// Optional run identifier.
    pub run_id: Option<String>,
    /// Canonical payload JSON exactly as persisted.
    pub payload_json: String,
    /// SHA-256 of the persisted payload bytes.
    pub digest: Sha256Digest,
    /// Occurred-at timestamp in UTC.
    pub timestamp: DateTime<Utc>,
    /// Digest of the preceding event, or the genesis zero digest.
    pub previous_event_digest: Sha256Digest,
    /// Digest binding this event to the chain.
    pub event_digest: Sha256Digest,
}

impl Event {
    /// Parse the persisted payload, which verification already canonicalized.
    pub fn payload(&self) -> Result<CanonicalValue, PersistenceError> {
        parse_json_object(&self.payload_json)
    }
}

/// Optional values for one append.
#[derive(Debug, Clone, Default)]
pub struct AppendOptions {
    /// Optional workflow identifier.
    pub workflow_id: Option<String>,
    /// Optional run identifier.
    pub run_id: Option<String>,
    /// Explicit timestamp; defaults to the current UTC wall clock.
    pub occurred_at: Option<DateTime<Utc>>,
}

impl AppendOptions {
    /// Set the workflow identifier.
    pub fn with_workflow_id(mut self, workflow_id: impl Into<String>) -> Self {
        self.workflow_id = Some(workflow_id.into());
        self
    }

    /// Set the run identifier.
    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    /// Set an explicit timestamp.
    pub fn with_occurred_at(mut self, occurred_at: DateTime<Utc>) -> Self {
        self.occurred_at = Some(occurred_at);
        self
    }
}

/// Journal read filters, mirroring `EventJournal.read`.
#[derive(Debug, Clone)]
pub struct EventFilter {
    /// Only events with this type.
    pub event_type: Option<String>,
    /// Only events with this aggregate id.
    pub aggregate_id: Option<String>,
    /// Only events with this workflow id.
    pub workflow_id: Option<String>,
    /// Only events with this run id.
    pub run_id: Option<String>,
    /// Only events with a sequence strictly greater than this value.
    pub after_sequence: i64,
    /// Maximum number of events; `None` means every matching event.
    pub limit: Option<i64>,
}

impl Default for EventFilter {
    fn default() -> Self {
        Self {
            event_type: None,
            aggregate_id: None,
            workflow_id: None,
            run_id: None,
            after_sequence: 0,
            limit: Some(1_000),
        }
    }
}

/// The append-only journal, verified before every read.
pub struct EventJournal<'a> {
    database: &'a Database,
}

impl std::fmt::Debug for EventJournal<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EventJournal")
            .field("database", self.database)
            .finish()
    }
}

impl<'a> EventJournal<'a> {
    /// Open a journal and verify the whole stored chain immediately.
    pub fn new(database: &'a Database) -> Result<Self, PersistenceError> {
        let journal = Self { database };
        journal.verify_chain()?;
        Ok(journal)
    }

    /// Append one event from canonical JSON text and return it.
    pub fn append(
        &self,
        event_type: &str,
        aggregate_id: &str,
        payload_json: &str,
    ) -> Result<Event, PersistenceError> {
        self.append_with(
            event_type,
            aggregate_id,
            payload_json,
            &AppendOptions::default(),
        )
    }

    /// Append one event with explicit optional values.
    pub fn append_with(
        &self,
        event_type: &str,
        aggregate_id: &str,
        payload_json: &str,
        options: &AppendOptions,
    ) -> Result<Event, PersistenceError> {
        let canonical = canonicalize_json_object(payload_json)?;
        self.append_canonical(event_type, aggregate_id, canonical, options)
    }

    /// Append one event from a typed canonical value.
    pub fn append_value(
        &self,
        event_type: &str,
        aggregate_id: &str,
        payload: &CanonicalValue,
        options: &AppendOptions,
    ) -> Result<Event, PersistenceError> {
        let canonical = canonicalize_object_value(payload)?;
        self.append_canonical(event_type, aggregate_id, canonical, options)
    }

    fn append_canonical(
        &self,
        event_type: &str,
        aggregate_id: &str,
        payload_json: String,
        options: &AppendOptions,
    ) -> Result<Event, PersistenceError> {
        let event_type = validate_identifier(event_type, "event_type")?;
        let aggregate_id = validate_identifier(aggregate_id, "aggregate_id")?;
        let workflow_id = options
            .workflow_id
            .as_deref()
            .map(|value| validate_identifier(value, "workflow_id"))
            .transpose()?;
        let run_id = options
            .run_id
            .as_deref()
            .map(|value| validate_identifier(value, "run_id"))
            .transpose()?;
        let timestamp = truncate_to_micros(options.occurred_at.unwrap_or_else(Utc::now));
        let occurred_at = timestamp_to_iso(&timestamp)?;
        let payload_digest = sha256_text(&payload_json);

        self.database.transaction(|connection| {
            let (head_sequence, previous_event_digest) = verified_tail(connection)?;
            let sequence = match head_sequence {
                None => 1_i64,
                Some(head) => head
                    .checked_add(1)
                    .ok_or(PersistenceError::SequenceOverflow)?,
            };
            let fields = EventChainFields {
                sequence: u64::try_from(sequence)
                    .map_err(|_| PersistenceError::SequenceOverflow)?,
                event_type: event_type.clone(),
                aggregate_id: aggregate_id.clone(),
                workflow_id: workflow_id.clone(),
                run_id: run_id.clone(),
                occurred_at: occurred_at.clone(),
                payload_digest,
                previous_event_digest,
            };
            let event_digest = chain_digest(&fields);
            connection.execute(
                INSERT_EVENT,
                params![
                    sequence,
                    event_type,
                    aggregate_id,
                    workflow_id,
                    run_id,
                    payload_json,
                    payload_digest.to_hex(),
                    occurred_at,
                    previous_event_digest.to_hex(),
                    event_digest.to_hex(),
                ],
            )?;
            Ok(Event {
                sequence,
                event_type: event_type.clone(),
                aggregate_id: aggregate_id.clone(),
                workflow_id: workflow_id.clone(),
                run_id: run_id.clone(),
                payload_json: payload_json.clone(),
                digest: payload_digest,
                timestamp,
                previous_event_digest,
                event_digest,
            })
        })
    }

    /// Fetch one event by sequence, verifying the whole journal first.
    pub fn get(&self, sequence: i64) -> Result<Option<Event>, PersistenceError> {
        if sequence < 0 {
            return Err(PersistenceError::NegativeSequence { field: "sequence" });
        }
        self.database.serialized(|connection| {
            verify_chain_on(connection)?;
            let mut statement = connection.prepare(&format!(
                "SELECT {CHAIN_COLUMNS} FROM events WHERE sequence = ?1"
            ))?;
            let mut rows = statement.query(params![sequence])?;
            match rows.next()? {
                Some(row) => event_from_row(row).map(Some),
                None => Ok(None),
            }
        })
    }

    /// Read every event (up to the default 1,000 row limit).
    pub fn read(&self) -> Result<Vec<Event>, PersistenceError> {
        self.read_filtered(&EventFilter::default())
    }

    /// Read events matching a filter, verifying the whole journal first.
    pub fn read_filtered(&self, filter: &EventFilter) -> Result<Vec<Event>, PersistenceError> {
        let event_type = filter
            .event_type
            .as_deref()
            .map(|value| validate_identifier(value, "event_type"))
            .transpose()?;
        let aggregate_id = filter
            .aggregate_id
            .as_deref()
            .map(|value| validate_identifier(value, "aggregate_id"))
            .transpose()?;
        let workflow_id = filter
            .workflow_id
            .as_deref()
            .map(|value| validate_identifier(value, "workflow_id"))
            .transpose()?;
        let run_id = filter
            .run_id
            .as_deref()
            .map(|value| validate_identifier(value, "run_id"))
            .transpose()?;
        if filter.after_sequence < 0 {
            return Err(PersistenceError::NegativeSequence {
                field: "after_sequence",
            });
        }
        if let Some(limit) = filter.limit {
            if limit < 1 {
                return Err(PersistenceError::InvalidLimit);
            }
        }
        let limit = filter.limit.unwrap_or(-1);
        self.database.serialized(|connection| {
            verify_chain_on(connection)?;
            let mut statement = connection.prepare(&format!(
                "
                SELECT {CHAIN_COLUMNS}
                FROM events
                WHERE (:event_type IS NULL OR event_type = :event_type)
                  AND (:aggregate_id IS NULL OR aggregate_id = :aggregate_id)
                  AND (:workflow_id IS NULL OR workflow_id = :workflow_id)
                  AND (:run_id IS NULL OR run_id = :run_id)
                  AND sequence > :after_sequence
                ORDER BY sequence ASC
                LIMIT :limit
                "
            ))?;
            let mut rows = statement.query(params![
                event_type,
                aggregate_id,
                workflow_id,
                run_id,
                filter.after_sequence,
                limit
            ])?;
            let mut events = Vec::new();
            while let Some(row) = rows.next()? {
                events.push(event_from_row(row)?);
            }
            Ok(events)
        })
    }

    /// Verify sequence continuity and every chain digest, failing closed.
    pub fn verify_chain(&self) -> Result<ChainVerification, PersistenceError> {
        self.database.serialized(verify_chain_on)
    }
}

pub(crate) fn validate_identifier(
    value: &str,
    field: &'static str,
) -> Result<String, PersistenceError> {
    let normalized = python_strip(value);
    if is_safe_identifier(normalized) {
        Ok(normalized.to_owned())
    } else {
        Err(PersistenceError::InvalidIdentifier { field })
    }
}

/// CPython's `str.isspace()` set.
///
/// `str.strip()` uses this set, which is wider than Rust's Unicode
/// `char::is_whitespace`: it also includes the information separators
/// `U+001C..=U+001F`. The Python identifier normalizer strips with
/// `str.strip()`, so using Rust `.trim()` here would reject stored or appended
/// identifiers that the product accepts.
fn is_python_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000D}'
            | '\u{001C}'..='\u{001F}'
            | '\u{0020}'
            | '\u{0085}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

fn python_strip(value: &str) -> &str {
    value.trim_matches(is_python_whitespace)
}

fn is_safe_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 128 {
        return false;
    }
    if !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

struct RawEventRow {
    sequence: i64,
    event_type: String,
    aggregate_id: String,
    workflow_id: Option<String>,
    run_id: Option<String>,
    payload_json: String,
    digest: String,
    occurred_at: String,
    previous_event_digest: String,
    event_digest: String,
}

fn read_raw_event(row: &Row<'_>) -> Result<RawEventRow, PersistenceError> {
    Ok(RawEventRow {
        sequence: row.get("sequence")?,
        event_type: row.get("event_type")?,
        aggregate_id: row.get("aggregate_id")?,
        workflow_id: row.get("workflow_id")?,
        run_id: row.get("run_id")?,
        payload_json: row.get("payload_json")?,
        digest: row.get("digest")?,
        occurred_at: row.get("occurred_at")?,
        previous_event_digest: row.get("previous_event_digest")?,
        event_digest: row.get("event_digest")?,
    })
}

struct ValidatedEvent {
    sequence: i64,
    /// Raw stored identifier bytes. The chain commits to exactly these bytes,
    /// not to the whitespace-normalized API representation.
    event_type: String,
    aggregate_id: String,
    workflow_id: Option<String>,
    run_id: Option<String>,
    payload_json: String,
    payload_digest: Sha256Digest,
    occurred_at: String,
    timestamp: DateTime<Utc>,
    previous_event_digest: Sha256Digest,
    event_digest: Sha256Digest,
}

fn validate_event_row(raw: RawEventRow) -> Result<ValidatedEvent, JournalIntegrityError> {
    let sequence = raw.sequence;
    let canonical = canonicalize_json_object(&raw.payload_json)
        .map_err(|_| JournalIntegrityError::PayloadNotCanonical { sequence })?;
    if canonical != raw.payload_json {
        return Err(JournalIntegrityError::PayloadNotCanonical { sequence });
    }
    if !crate::canonical::is_lowercase_sha256_hex(&raw.digest) {
        return Err(JournalIntegrityError::Payload { sequence });
    }
    let payload_digest = sha256_text(&canonical);
    if payload_digest.to_hex() != raw.digest {
        return Err(JournalIntegrityError::Payload { sequence });
    }
    let previous_event_digest = parse_stored_digest(&raw.previous_event_digest)
        .ok_or(JournalIntegrityError::StoredDigest { sequence })?;
    let event_digest = parse_stored_digest(&raw.event_digest)
        .ok_or(JournalIntegrityError::StoredDigest { sequence })?;
    for (field, value) in [
        ("event_type", Some(raw.event_type.as_str())),
        ("aggregate_id", Some(raw.aggregate_id.as_str())),
        ("workflow_id", raw.workflow_id.as_deref()),
        ("run_id", raw.run_id.as_deref()),
    ] {
        if let Some(value) = value {
            if !is_safe_identifier(python_strip(value)) {
                return Err(JournalIntegrityError::Identifier { sequence, field });
            }
        }
    }
    let timestamp = timestamp_from_iso(&raw.occurred_at)
        .map_err(|_| JournalIntegrityError::Timestamp { sequence })?;
    Ok(ValidatedEvent {
        sequence,
        event_type: raw.event_type,
        aggregate_id: raw.aggregate_id,
        workflow_id: raw.workflow_id,
        run_id: raw.run_id,
        payload_json: raw.payload_json,
        payload_digest,
        occurred_at: raw.occurred_at,
        timestamp,
        previous_event_digest,
        event_digest,
    })
}

/// Normalize a stored identifier for the typed API after its raw bytes have
/// already been covered by the chain digest.
fn normalized_identifier(
    value: &str,
    sequence: i64,
    field: &'static str,
) -> Result<String, JournalIntegrityError> {
    let normalized = python_strip(value);
    if is_safe_identifier(normalized) {
        Ok(normalized.to_owned())
    } else {
        Err(JournalIntegrityError::Identifier { sequence, field })
    }
}

fn chain_fields(validated: &ValidatedEvent) -> Result<EventChainFields, JournalIntegrityError> {
    let sequence =
        u64::try_from(validated.sequence).map_err(|_| JournalIntegrityError::Continuity {
            expected: 1,
            actual: validated.sequence,
        })?;
    Ok(EventChainFields {
        sequence,
        event_type: validated.event_type.clone(),
        aggregate_id: validated.aggregate_id.clone(),
        workflow_id: validated.workflow_id.clone(),
        run_id: validated.run_id.clone(),
        occurred_at: validated.occurred_at.clone(),
        payload_digest: validated.payload_digest,
        previous_event_digest: validated.previous_event_digest,
    })
}

/// Compute the verified chain state for every stored row.
///
/// This walks the whole journal in sequence order and checks, for each row:
/// continuity, the raw lowercase chain digest strings, canonical payload
/// bytes, the payload digest, identifier shape, timestamp validity, previous
/// digest linkage and the recomputed event digest.
pub fn verify_chain_on(connection: &Connection) -> Result<ChainVerification, PersistenceError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {CHAIN_COLUMNS} FROM events ORDER BY sequence ASC"
    ))?;
    let mut rows = statement.query([])?;
    let mut expected_sequence: i64 = 1;
    let mut previous_event_digest = GENESIS_PREVIOUS_DIGEST;
    while let Some(row) = rows.next()? {
        let raw = read_raw_event(row)?;
        if raw.sequence != expected_sequence {
            return Err(JournalIntegrityError::Continuity {
                expected: expected_sequence,
                actual: raw.sequence,
            }
            .into());
        }
        let validated = validate_event_row(raw)?;
        if validated.previous_event_digest != previous_event_digest {
            return Err(JournalIntegrityError::Link {
                sequence: validated.sequence,
            }
            .into());
        }
        let fields = chain_fields(&validated)?;
        let digest = chain_digest(&fields);
        if digest != validated.event_digest {
            return Err(JournalIntegrityError::EventDigest {
                sequence: validated.sequence,
            }
            .into());
        }
        previous_event_digest = digest;
        expected_sequence += 1;
    }
    let event_count = (expected_sequence - 1) as u64;
    Ok(ChainVerification {
        event_count,
        head_sequence: if event_count == 0 {
            None
        } else {
            Some(expected_sequence - 1)
        },
        head_event_digest: if event_count == 0 {
            None
        } else {
            Some(previous_event_digest)
        },
    })
}

fn verified_tail(connection: &Connection) -> Result<(Option<i64>, Sha256Digest), PersistenceError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {CHAIN_COLUMNS} FROM events ORDER BY sequence DESC LIMIT 1"
    ))?;
    let mut rows = statement.query([])?;
    let Some(row) = rows.next()? else {
        return Ok((None, GENESIS_PREVIOUS_DIGEST));
    };
    let raw = read_raw_event(row)?;
    let validated = validate_event_row(raw)?;
    let fields = chain_fields(&validated)?;
    if chain_digest(&fields) != validated.event_digest {
        return Err(JournalIntegrityError::EventDigest {
            sequence: validated.sequence,
        }
        .into());
    }
    Ok((Some(validated.sequence), validated.event_digest))
}

fn event_from_row(row: &Row<'_>) -> Result<Event, PersistenceError> {
    let raw = read_raw_event(row)?;
    let validated = validate_event_row(raw)?;
    let fields = chain_fields(&validated)?;
    if chain_digest(&fields) != validated.event_digest {
        return Err(PersistenceError::EventDigestVerification);
    }
    Ok(Event {
        sequence: validated.sequence,
        event_type: normalized_identifier(&validated.event_type, validated.sequence, "event_type")?,
        aggregate_id: normalized_identifier(
            &validated.aggregate_id,
            validated.sequence,
            "aggregate_id",
        )?,
        workflow_id: validated
            .workflow_id
            .as_deref()
            .map(|value| normalized_identifier(value, validated.sequence, "workflow_id"))
            .transpose()?,
        run_id: validated
            .run_id
            .as_deref()
            .map(|value| normalized_identifier(value, validated.sequence, "run_id"))
            .transpose()?,
        payload_json: validated.payload_json,
        digest: validated.payload_digest,
        timestamp: validated.timestamp,
        previous_event_digest: validated.previous_event_digest,
        event_digest: validated.event_digest,
    })
}

/// Chain every pre-existing event during migration 1 to 2.
///
/// Only contiguous rows whose payload digest verifies are rewritten; a
/// corrupt history is refused so it is never promoted into a chained one.
pub(crate) fn backfill_event_chain(connection: &Connection) -> Result<(), PersistenceError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {CHAIN_COLUMNS} FROM events ORDER BY sequence ASC"
    ))?;
    let mut rows = statement.query([])?;
    let mut expected_sequence: i64 = 1;
    let mut previous_event_digest = GENESIS_PREVIOUS_DIGEST;
    let mut updates: Vec<(i64, Sha256Digest, Sha256Digest)> = Vec::new();
    while let Some(row) = rows.next()? {
        let raw = read_raw_event(row)?;
        if raw.sequence != expected_sequence {
            return Err(JournalIntegrityError::BackfillGap {
                expected: expected_sequence,
            }
            .into());
        }
        if !crate::canonical::is_lowercase_sha256_hex(&raw.digest) {
            return Err(JournalIntegrityError::BackfillPayload {
                sequence: raw.sequence,
            }
            .into());
        }
        let payload_digest = sha256_text(&raw.payload_json);
        if payload_digest.to_hex() != raw.digest {
            return Err(JournalIntegrityError::BackfillPayload {
                sequence: raw.sequence,
            }
            .into());
        }
        let sequence =
            u64::try_from(raw.sequence).map_err(|_| JournalIntegrityError::BackfillGap {
                expected: expected_sequence,
            })?;
        let fields = EventChainFields {
            sequence,
            event_type: raw.event_type,
            aggregate_id: raw.aggregate_id,
            workflow_id: raw.workflow_id,
            run_id: raw.run_id,
            occurred_at: raw.occurred_at,
            payload_digest,
            previous_event_digest,
        };
        let event_digest = chain_digest(&fields);
        updates.push((raw.sequence, previous_event_digest, event_digest));
        previous_event_digest = event_digest;
        expected_sequence += 1;
    }
    drop(rows);
    drop(statement);
    for (sequence, previous, digest) in updates {
        connection.execute(
            "UPDATE events SET previous_event_digest = ?1, event_digest = ?2 WHERE sequence = ?3",
            params![previous.to_hex(), digest.to_hex(), sequence],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_pattern_matches_python() {
        assert!(is_safe_identifier("step.started"));
        assert!(is_safe_identifier("A1._:-"));
        assert!(!is_safe_identifier(""));
        assert!(!is_safe_identifier("-leading"));
        assert!(!is_safe_identifier("bad type"));
        assert!(!is_safe_identifier("../escape"));
        assert!(!is_safe_identifier(&"a".repeat(129)));
        assert!(is_safe_identifier(&"a".repeat(128)));
    }
}
