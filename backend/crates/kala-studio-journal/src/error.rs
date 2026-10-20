//! Errors raised by the native Studio session journal.
//!
//! The Python module raises `TypeError`, `ValueError`, `RuntimeError`,
//! `FileNotFoundError` and
//! `kala_agent.persistence.chain.JournalIntegrityError`. Every message below
//! is the Python message verbatim, so a caller that logs the error sees the
//! same text in both implementations. The *kind* is carried by the variant
//! instead of by the exception class.

use kala_artifacts::ArtifactError;
use kala_json::CanonicalJsonError;
use kala_persistence::PersistenceError;
use thiserror::Error;

/// A Studio session journal failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StudioJournalError {
    /// The journal was closed before the operation started.
    ///
    /// Python raises `RuntimeError("the studio session journal is closed")`.
    #[error("the studio session journal is closed")]
    Closed,

    /// The event payload was not a mapping.
    ///
    /// Python raises `TypeError("event payload must be a mapping")` before it
    /// validates the identifier or canonicalises anything.
    #[error("event payload must be a mapping")]
    PayloadNotAMapping,

    /// An event type or aggregate id was outside the safe identifier domain.
    ///
    /// Python raises `ValueError(f"{field} must be a safe identifier")`.
    #[error("{field} must be a safe identifier")]
    InvalidIdentifier {
        /// The rejected field name.
        field: &'static str,
    },

    /// The payload could not be canonicalised into a JSON object.
    ///
    /// Python raises `ValueError("value must be a canonical JSON object")`.
    #[error("value must be a canonical JSON object")]
    InvalidPayload,

    /// The canonical payload was larger than the configured limit.
    ///
    /// Python raises `ValueError("event payload exceeds the configured size limit")`.
    #[error("event payload exceeds the configured size limit")]
    PayloadTooLarge,

    /// A limit or bound was not positive.
    ///
    /// Python raises `ValueError(f"{field} must be positive")`.
    #[error("{field} must be positive")]
    NotPositive {
        /// The rejected field name.
        field: &'static str,
    },

    /// A sequence-like argument was negative.
    ///
    /// Python raises `ValueError(f"{field} must be nonnegative")`.
    #[error("{field} must be nonnegative")]
    NegativeValue {
        /// The rejected field name.
        field: &'static str,
    },

    /// The chain is broken and must not be extended.
    ///
    /// Python raises
    /// `JournalIntegrityError("the studio session journal is broken at sequence
    /// N; refusing to extend it")`.
    #[error("the studio session journal is broken at sequence {sequence}; refusing to extend it")]
    Integrity {
        /// The first sequence that failed verification.
        sequence: i64,
    },

    /// An artifact store failure.
    #[error(transparent)]
    Artifact(#[from] ArtifactError),

    /// The payload could not be parsed as canonical JSON.
    #[error(transparent)]
    Json(#[from] CanonicalJsonError),

    /// A persistence-domain failure, for example an invalid timestamp.
    #[error(transparent)]
    Persistence(#[from] PersistenceError),

    /// The journal database could not be read or written.
    #[error(transparent)]
    Database(#[from] rusqlite::Error),

    /// A filesystem operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl StudioJournalError {
    /// Return the Python-compatible error kind of this failure.
    ///
    /// The kinds are the coarse classes the Python module distinguishes:
    /// `type`, `value`, `integrity`, `not_found` and `closed`. They let an
    /// interoperability test compare a native failure with a Python failure
    /// without depending on the exception class hierarchy.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Closed => ErrorKind::Closed,
            Self::PayloadNotAMapping => ErrorKind::Type,
            Self::InvalidIdentifier { .. }
            | Self::InvalidPayload
            | Self::PayloadTooLarge
            | Self::NotPositive { .. }
            | Self::NegativeValue { .. } => ErrorKind::Value,
            Self::Integrity { .. } => ErrorKind::Integrity,
            Self::Artifact(ArtifactError::NotFound(_)) => ErrorKind::NotFound,
            Self::Artifact(ArtifactError::TooLarge) => ErrorKind::Value,
            Self::Artifact(ArtifactError::InvalidDigest) => ErrorKind::Value,
            Self::Artifact(ArtifactError::InvalidUtf8(_)) => ErrorKind::Value,
            Self::Artifact(_) => ErrorKind::Other,
            Self::Json(_) | Self::Persistence(_) => ErrorKind::Value,
            Self::Database(_) | Self::Io(_) => ErrorKind::Other,
        }
    }
}

/// The coarse class of a journal failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The payload was not a mapping (`TypeError` in Python).
    Type,
    /// The value was rejected by a validation rule (`ValueError` in Python).
    Value,
    /// The hash chain is broken (`JournalIntegrityError` in Python).
    Integrity,
    /// The object does not exist (`FileNotFoundError` in Python).
    NotFound,
    /// The journal was closed (`RuntimeError` in Python).
    Closed,
    /// Any other failure.
    Other,
}

impl ErrorKind {
    /// Return the Python exception class name this kind corresponds to.
    pub fn python_class(self) -> &'static str {
        match self {
            Self::Type => "TypeError",
            Self::Value => "ValueError",
            Self::Integrity => "JournalIntegrityError",
            Self::NotFound => "FileNotFoundError",
            Self::Closed => "RuntimeError",
            Self::Other => "Exception",
        }
    }
}
