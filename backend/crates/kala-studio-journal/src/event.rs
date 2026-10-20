//! The journalled Studio event, the verification report and the event types.

use std::fmt;

use chrono::{DateTime, Utc};
use kala_json::CanonicalValue;
use kala_persistence::timestamp_from_iso;

use crate::error::StudioJournalError;

/// The event types the design-turn flow records in the Studio journal.
///
/// The list is the one named by ADR-0009 section 5. The journal accepts any
/// safe identifier as an event type, so a new event type does not require a
/// change here; the enum exists so the API layer can name them without
/// repeating string literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JournalEventType {
    /// The design intent was recorded.
    DesignIntent,
    /// The design compiled to geometry.
    DesignCompiled,
    /// The design was measured.
    DesignMeasured,
    /// The design was checked against its intent.
    DesignChecked,
    /// An artifact was stored.
    ArtifactStored,
    /// An export completed.
    ExportCompleted,
    /// A chat turn was recorded.
    ChatTurn,
}

impl JournalEventType {
    /// The event type as it is stored in the journal.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DesignIntent => "design.intent",
            Self::DesignCompiled => "design.compiled",
            Self::DesignMeasured => "design.measured",
            Self::DesignChecked => "design.checked",
            Self::ArtifactStored => "artifact.stored",
            Self::ExportCompleted => "export.completed",
            Self::ChatTurn => "chat.turn",
        }
    }

    /// Every event type the Studio records, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::DesignIntent,
        Self::DesignCompiled,
        Self::DesignMeasured,
        Self::DesignChecked,
        Self::ArtifactStored,
        Self::ExportCompleted,
        Self::ChatTurn,
    ];
}

impl AsRef<str> for JournalEventType {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for JournalEventType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One journalled Studio event.
///
/// `digest` is the SHA-256 of the canonical payload and `event_digest` is the
/// chain digest that binds the event to the one before it, which is the same
/// split the core journal makes. `occurred_at` is the exact ISO-8601 UTC string
/// that is part of the chain preimage, so an outside reader can recompute
/// `event_digest` with the core chain function.
///
/// `payload` is `None` only when the stored text is not a JSON object, which
/// can happen only if the file was edited outside the Studio; the raw text
/// always remains available as `payload_json`.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionEvent {
    /// The one-based journal sequence.
    pub sequence: i64,
    /// The event type identifier.
    pub event_type: String,
    /// The aggregate identifier.
    pub aggregate_id: String,
    /// The decoded payload, or `None` when the stored text is not an object.
    pub payload: Option<CanonicalValue>,
    /// The exact canonical text that was hashed.
    pub payload_json: String,
    /// The SHA-256 of the canonical payload bytes.
    pub digest: String,
    /// The exact ISO-8601 UTC string that is part of the chain preimage.
    pub occurred_at: String,
    /// The chain digest of the preceding event.
    pub previous_event_digest: String,
    /// The chain digest that binds this event to the chain.
    pub event_digest: String,
}

impl SessionEvent {
    /// Return the occurrence time as an aware UTC timestamp.
    ///
    /// The stored string is part of the chain preimage, so it is never
    /// rewritten; the parsed value is only a convenience for the API layer.
    pub fn timestamp(&self) -> Result<DateTime<Utc>, StudioJournalError> {
        Ok(timestamp_from_iso(&self.occurred_at)?)
    }
}

/// The outcome of a chain verification pass.
///
/// - `ok` is `true` only when every event verified.
/// - `event_count` is the number of events the journal holds, verified or not,
///   so the UI can show how much history exists.
/// - `head_digest` is the chain digest of the last event that verified, which
///   is the value to anchor outside the database. It is `None` for an empty
///   journal and for a journal whose very first event is broken.
/// - `first_bad_sequence` is the sequence of the first event that failed
///   verification, or `None` when the chain is intact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalVerification {
    /// Whether every stored event verified.
    pub ok: bool,
    /// The number of stored events, verified or not.
    pub event_count: i64,
    /// The chain digest of the last event that verified.
    pub head_digest: Option<String>,
    /// The sequence of the first event that failed verification.
    pub first_bad_sequence: Option<i64>,
}
