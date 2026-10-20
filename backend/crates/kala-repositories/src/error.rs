//! Typed repository errors.
//!
//! The Python source raises three distinct public error shapes from the
//! repository layer and otherwise lets Pydantic, SQLite or the identifier
//! validator surface their own errors. The native equivalent keeps the two
//! repository-specific errors as dedicated types and wraps the rest in this
//! enum so callers can still tell validation, stored-data corruption,
//! persistence/SQLite and finality failures apart.

use kala_contracts::ContractError;
use kala_persistence::PersistenceError;
use num_bigint::BigInt;
use thiserror::Error;

/// Raised when a design head is not the one a write expected.
///
/// A compare-and-swap that lost the race says so with this type, and the
/// enclosing transaction is rolled back, so a caller never observes an orphan
/// revision row for a head it did not extend.
///
/// The fields mirror every attribute of the Python `HeadMovedError`:
/// `message`, `design_id`, `expected_head_revision_id`,
/// `actual_head_revision_id`, `expected_sequence` and `actual_sequence`.
/// Sequences are arbitrary-precision because the Python contract accepts
/// arbitrary nonnegative `int` values even though SQLite rows are signed
/// 64-bit integers.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct HeadMovedError {
    /// The exact message Python raises for this outcome.
    pub message: String,
    /// The design whose head moved.
    pub design_id: String,
    /// The normalized head the caller prepared against, if any.
    pub expected_head_revision_id: Option<String>,
    /// The head actually stored when the compare-and-swap ran, if any.
    pub actual_head_revision_id: Option<String>,
    /// The sequence the caller attempted to write.
    pub expected_sequence: BigInt,
    /// The sequence the stored head holds (`0` when the design is unborn).
    pub actual_sequence: BigInt,
}

impl HeadMovedError {
    /// The design whose head moved.
    pub fn design_id(&self) -> &str {
        &self.design_id
    }

    /// The normalized head the caller prepared against.
    pub fn expected_head_revision_id(&self) -> Option<&str> {
        self.expected_head_revision_id.as_deref()
    }

    /// The head actually stored when the compare-and-swap ran.
    pub fn actual_head_revision_id(&self) -> Option<&str> {
        self.actual_head_revision_id.as_deref()
    }

    /// The sequence the caller attempted to write.
    pub fn expected_sequence(&self) -> &BigInt {
        &self.expected_sequence
    }

    /// The sequence the stored head holds.
    pub fn actual_sequence(&self) -> &BigInt {
        &self.actual_sequence
    }
}

/// Raised when a write would change an approval that already carries a
/// decision.
///
/// The only accepted write to a decided approval is a byte-identical no-op,
/// exactly as in the Python source's conditional UPSERT.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("approval {request_id} is already decided and cannot change")]
pub struct ApprovalAlreadyDecidedError {
    /// The approval that is already final.
    pub request_id: String,
}

impl ApprovalAlreadyDecidedError {
    /// The approval that is already final.
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}

/// A repository operation failure.
#[derive(Debug, Error)]
pub enum RepositoryError {
    /// A caller-supplied identifier failed the shared safe-identifier domain.
    #[error("{field} must be a safe identifier")]
    InvalidIdentifier {
        /// Caller-facing field name.
        field: &'static str,
    },

    /// A list limit was not positive.
    #[error("limit must be positive")]
    InvalidLimit,

    /// A typed model could not be transported to its persisted JSON form.
    #[error(transparent)]
    Validation(#[from] ContractError),

    /// A stored `model_json` value failed contract validation.
    ///
    /// This is deliberately distinct from [`Self::Validation`]: it identifies
    /// tampered or otherwise unreadable stored data rather than a caller's
    /// model value.
    #[error("stored {model} model_json is invalid: {source}")]
    StoredModel {
        /// Contract model name (`DesignSpec`, `MemoryRecord`, ...).
        model: &'static str,
        /// The underlying contract validation failure.
        #[source]
        source: ContractError,
    },

    /// A tool result referenced a call that does not exist.
    #[error("tool result requires an existing tool call")]
    ToolResultMissingCall,

    /// A tool result's `tool_id` did not match the stored call.
    #[error("tool result tool_id must match its tool call")]
    ToolResultToolMismatch,

    /// A revision sequence does not fit SQLite's signed 64-bit `INTEGER`.
    #[error("revision sequence is outside SQLite's signed integer range")]
    SequenceOutOfRange,

    /// A timestamp is outside the range the persistence columns can hold.
    #[error("timestamp is outside the supported range")]
    TimestampOutOfRange,

    /// The design head moved before the revision could be written.
    #[error(transparent)]
    HeadMoved(#[from] Box<HeadMovedError>),

    /// The approval is already decided and the requested change is not a
    /// byte-identical no-op.
    #[error(transparent)]
    ApprovalAlreadyDecided(#[from] ApprovalAlreadyDecidedError),

    /// The accepted persistence layer or SQLite failed.
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
}

impl From<rusqlite::Error> for RepositoryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Persistence(PersistenceError::from(error))
    }
}

impl RepositoryError {
    /// The head-moved conflict, if this is one.
    pub fn as_head_moved(&self) -> Option<&HeadMovedError> {
        match self {
            Self::HeadMoved(error) => Some(error.as_ref()),
            _ => None,
        }
    }

    /// The approval-finality conflict, if this is one.
    pub fn as_approval_already_decided(&self) -> Option<&ApprovalAlreadyDecidedError> {
        match self {
            Self::ApprovalAlreadyDecided(error) => Some(error),
            _ => None,
        }
    }

    /// The accepted persistence error, if this is one.
    pub fn as_persistence(&self) -> Option<&PersistenceError> {
        match self {
            Self::Persistence(error) => Some(error),
            _ => None,
        }
    }

    /// The SQLite failure, if this is one.
    pub fn as_sqlite(&self) -> Option<&rusqlite::Error> {
        match self {
            Self::Persistence(PersistenceError::Sqlite(error)) => Some(error),
            _ => None,
        }
    }
}
