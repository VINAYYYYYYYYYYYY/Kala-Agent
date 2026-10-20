//! Typed errors for the native memory manager.
//!
//! The Python source raises a small exception hierarchy out of
//! `kala_agent.kernel.memory`:
//!
//! ```text
//! MemoryError(RuntimeError)
//! ├── MemoryValidationError(MemoryError, ValueError)
//! ├── MemoryConflictError(MemoryError, ValueError)
//! ├── MemoryNotFoundError(MemoryError, KeyError)
//! └── MemoryPromotionError(MemoryError, ValueError)
//! ```
//!
//! Rust has no exception hierarchy, so the hierarchy is preserved as the
//! [`MemoryError`] variants plus two accessors that report the Python class
//! name and the Python `str()` rendering. Callers that need to distinguish
//! "this is a validation failure" from "this is a promotion failure" match on
//! the variant; callers that need the Python text read
//! [`MemoryError::message`].
//!
//! The Python module also lets three non-`MemoryError` exceptions escape:
//! `TypeError` for an argument of the wrong type, `ValueError` for an
//! argument of the right type but an unusable value, and Pydantic's
//! `ValidationError` for a contract value that fails re-validation. Those are
//! [`MemoryError::Type`], [`MemoryError::Value`] and
//! [`MemoryError::Contract`].

use std::fmt;

use kala_contracts::ContractError;
use kala_repositories::{MemoryRecord, RepositoryError};

/// The conflict raised when one source already holds different content.
///
/// This is the native counterpart of
/// `kala_agent.kernel.memory.MemoryConflictError`. It carries both records,
/// exactly like the Python exception, and renders the same message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryConflictError {
    record: MemoryRecord,
    conflicting: MemoryRecord,
}

impl MemoryConflictError {
    /// Build a conflict between the record that was offered and the record
    /// that already holds the source.
    pub fn new(record: MemoryRecord, conflicting: MemoryRecord) -> Self {
        Self {
            record,
            conflicting,
        }
    }

    /// The record that was offered.
    pub fn record(&self) -> &MemoryRecord {
        &self.record
    }

    /// The record that already holds the source.
    pub fn conflicting(&self) -> &MemoryRecord {
        &self.conflicting
    }

    /// The Python exception message.
    pub fn message(&self) -> String {
        format!(
            "memory source {} already contains different content",
            crate::repr::python_repr_str(self.record.source())
        )
    }
}

impl fmt::Display for MemoryConflictError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for MemoryConflictError {}

/// A memory manager failure.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    /// `MemoryError`, the base `RuntimeError` subclass.
    #[error("{0}")]
    Base(String),

    /// `MemoryValidationError`, also a `ValueError`.
    #[error("{0}")]
    Validation(String),

    /// `MemoryConflictError`, also a `ValueError`.
    ///
    /// The conflict is boxed so the error type stays small; the two records
    /// it carries are the bulk of its size.
    #[error("{0}")]
    Conflict(Box<MemoryConflictError>),

    /// `MemoryNotFoundError`, also a `KeyError`.
    ///
    /// The payload is the missing key. The Python exception renders it with
    /// `repr()` because `KeyError.__str__` does, so
    /// [`MemoryError::message`] applies the same rule.
    #[error("{0}")]
    NotFound(String),

    /// `MemoryPromotionError`, also a `ValueError`.
    #[error("{0}")]
    Promotion(String),

    /// Python `TypeError` for an argument of an unusable type.
    #[error("{0}")]
    Type(String),

    /// Python `ValueError` for an argument of a usable type but an unusable
    /// value.
    #[error("{0}")]
    Value(String),

    /// A Pydantic `ValidationError` raised by a nested contract validation.
    #[error("{0}")]
    Contract(#[from] ContractError),

    /// A failure from the accepted repository/persistence layer.
    #[error("{0}")]
    Repository(#[from] RepositoryError),
}

impl From<MemoryConflictError> for MemoryError {
    fn from(error: MemoryConflictError) -> Self {
        Self::Conflict(Box::new(error))
    }
}

impl MemoryError {
    /// The Python exception class this error corresponds to.
    pub fn python_class_name(&self) -> &'static str {
        match self {
            Self::Base(_) => "MemoryError",
            Self::Validation(_) => "MemoryValidationError",
            Self::Conflict(_) => "MemoryConflictError",
            Self::NotFound(_) => "MemoryNotFoundError",
            Self::Promotion(_) => "MemoryPromotionError",
            Self::Type(_) => "TypeError",
            Self::Value(_) => "ValueError",
            Self::Contract(_) => "ValidationError",
            Self::Repository(error) => match error {
                RepositoryError::InvalidIdentifier { .. } | RepositoryError::InvalidLimit => {
                    "ValueError"
                }
                _ => "RuntimeError",
            },
        }
    }

    /// The Python `str()` rendering of the exception.
    pub fn message(&self) -> String {
        match self {
            Self::Base(message)
            | Self::Validation(message)
            | Self::Promotion(message)
            | Self::Type(message)
            | Self::Value(message) => message.clone(),
            Self::Conflict(error) => error.message(),
            Self::NotFound(key) => crate::repr::python_repr_str(key),
            Self::Contract(error) => error.message().to_owned(),
            Self::Repository(error) => error.to_string(),
        }
    }

    /// The conflict, if this is one.
    pub fn as_conflict(&self) -> Option<&MemoryConflictError> {
        match self {
            Self::Conflict(error) => Some(error.as_ref()),
            _ => None,
        }
    }

    /// The repository error, if this is one.
    pub fn as_repository(&self) -> Option<&RepositoryError> {
        match self {
            Self::Repository(error) => Some(error),
            _ => None,
        }
    }
}
