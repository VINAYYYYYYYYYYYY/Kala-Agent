//! Sanitized geometry validation errors.
//!
//! Python's geometry validators raise `ValueError` with a bounded message and
//! never echo the submitted value. The studio server maps `ValueError` to a
//! 422 `invalid_request` response whose `details` contains `str(error)`. This
//! type keeps that observable text while giving Rust callers a normal error
//! value.

use std::fmt;

/// A sanitized geometry request error.
///
/// `Display` produces exactly the message the Python source would have put in
/// `str(ValueError)`, so a service layer can forward it unchanged. The
/// submitted value is deliberately not included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeometryError {
    message: String,
}

impl GeometryError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Return the sanitized, user-facing message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for GeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for GeometryError {}
