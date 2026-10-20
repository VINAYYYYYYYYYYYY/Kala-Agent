use kala_json::CanonicalJsonError;
use thiserror::Error;

/// A journal row that does not belong to the stored hash chain.
///
/// These errors are deliberately distinct from a database or validation error:
/// callers can treat every occurrence as a tampered or corrupt journal, not as
/// a transient failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JournalIntegrityError {
    /// Sequences are not exactly `1..=N`.
    #[error("event journal continuity is broken: expected sequence {expected} but found {actual}")]
    Continuity {
        /// Expected one-based sequence.
        expected: i64,
        /// Sequence actually stored.
        actual: i64,
    },

    /// Migration 1 to 2 found a missing sequence in a pre-chain journal.
    #[error("event journal is missing sequence {expected}; refusing to chain it")]
    BackfillGap {
        /// Expected one-based sequence.
        expected: i64,
    },

    /// A row's previous digest does not link to the prior row.
    #[error("event {sequence} does not link to the preceding event digest")]
    Link {
        /// Offending one-based sequence.
        sequence: i64,
    },

    /// A row's payload bytes do not match its stored payload digest.
    #[error("event {sequence} failed payload verification")]
    Payload {
        /// Offending one-based sequence.
        sequence: i64,
    },

    /// A pre-chain row failed payload verification during migration backfill.
    #[error("event {sequence} failed payload digest verification")]
    BackfillPayload {
        /// Offending one-based sequence.
        sequence: i64,
    },

    /// A row's recomputed event digest does not match its stored digest.
    #[error("event {sequence} failed chain verification")]
    EventDigest {
        /// Offending one-based sequence.
        sequence: i64,
    },

    /// A stored chain digest is not exactly 64 lowercase hexadecimal digits.
    ///
    /// Uppercase input is rejected before it can be converted to a typed
    /// digest; [`kala_json::Sha256Digest::from_hex`] alone accepts uppercase and
    /// is therefore not sufficient for this check.
    #[error("event {sequence} does not record a valid chain digest")]
    StoredDigest {
        /// Offending one-based sequence.
        sequence: i64,
    },

    /// A stored payload is not the canonical JSON object it claims to be.
    #[error("event {sequence} stored payload is not canonical")]
    PayloadNotCanonical {
        /// Offending one-based sequence.
        sequence: i64,
    },

    /// A stored identifier is outside the safe identifier domain.
    #[error("event {sequence} contains an invalid {field}")]
    Identifier {
        /// Offending one-based sequence.
        sequence: i64,
        /// Stored column/field name.
        field: &'static str,
    },

    /// A stored timestamp is not a timezone-aware ISO timestamp.
    #[error("event {sequence} contains an invalid timestamp")]
    Timestamp {
        /// Offending one-based sequence.
        sequence: i64,
    },
}

/// A persistence operation failure.
#[derive(Debug, Error)]
pub enum PersistenceError {
    /// The database was closed before this operation started.
    #[error("database is closed")]
    Closed,

    /// A close was attempted while a connection borrow is still active.
    #[error("database connection is still in use")]
    ConnectionInUse,

    /// Busy timeout outside SQLite's signed 32-bit millisecond range.
    #[error("busy_timeout_ms must be between 0 and 2147483647")]
    InvalidBusyTimeout {
        /// Rejected value.
        value: i64,
    },

    /// An identifier failed the shared safe identifier pattern.
    #[error("{field} must be a safe identifier")]
    InvalidIdentifier {
        /// Caller-facing field name.
        field: &'static str,
    },

    /// A timestamp has no timezone/offset.
    #[error("timestamp must include a timezone")]
    TimestampWithoutTimezone,

    /// A parsed timestamp is not representable as a Python `datetime`.
    #[error("timestamp is outside the supported range")]
    TimestampOutOfRange,

    /// A `chrono` timestamp carries a leap second, which Python `datetime`
    /// cannot represent. Writers must refuse it instead of inventing a time.
    #[error("timestamp contains a leap second that Python datetime cannot represent")]
    TimestampLeapSecond,

    /// A stored timestamp could not be parsed.
    #[error("stored timestamp is invalid")]
    StoredTimestamp,

    /// A sequence-like public argument was negative.
    #[error("{field} must be nonnegative")]
    NegativeSequence {
        /// Caller-facing field name.
        field: &'static str,
    },

    /// A read limit was not positive.
    #[error("limit must be positive")]
    InvalidLimit,

    /// The journal sequence cannot advance beyond SQLite's integer range.
    #[error("event sequence overflow")]
    SequenceOverflow,

    /// The database records a schema version this binary does not know.
    #[error("database schema is newer than this application")]
    SchemaNewer,

    /// The value could not be parsed, validated or canonicalized as JSON.
    #[error("value must be a valid JSON object")]
    InvalidJsonObject,

    /// The parsed JSON value is not an object.
    #[error("value must be a JSON object")]
    JsonObjectRequired,

    /// The canonical serializer refused a value accepted by the parser.
    #[error("value cannot be canonicalized as JSON")]
    NotCanonicalizable,

    /// A stored event payload is not canonical.
    #[error("stored event payload is not canonical")]
    StoredEventPayloadNotCanonical,

    /// A cleanly parsed event row does not match its payload digest.
    #[error("event digest verification failed")]
    EventDigestVerification,

    /// A stored snapshot payload is not canonical.
    #[error("stored snapshot JSON is not canonical")]
    StoredSnapshotNotCanonical,

    /// A cleanly parsed snapshot row does not match its digest.
    #[error("snapshot digest verification failed")]
    SnapshotDigestVerification,

    /// SQLite did not return the row id of a snapshot insert.
    #[error("snapshot insert did not produce a row identifier")]
    SnapshotRowIdMissing,

    /// A row in the append-only journal is invalid.
    #[error(transparent)]
    JournalIntegrity(#[from] JournalIntegrityError),

    /// Canonical JSON machinery failed.
    #[error(transparent)]
    CanonicalJson(#[from] CanonicalJsonError),

    /// SQLite failed.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    /// Filesystem preparation or permission setting failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
