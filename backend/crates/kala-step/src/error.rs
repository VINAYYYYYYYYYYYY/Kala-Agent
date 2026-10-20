//! Typed, sanitized STEP export failures.
//!
//! The Python source separates "OCP is missing" (503 `step_unavailable`),
//! "the dimensions are invalid" (`ValueError`, 422 `invalid_request`) and
//! "the writer could not finish" (500-class `StepExportError`). This build has
//! no optional kernel dependency, so only the latter two exist here; a service
//! adapter keeps its own unavailable branch for builds without this crate.

use std::fmt;

use kala_geometry::GeometryError;

/// A failed STEP export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepError {
    /// The primitive kind or one of its dimensions is not acceptable.
    ///
    /// The payload is the reviewed geometry validation error, whose
    /// `Display` text is byte-identical to the Python `ValueError` message.
    Invalid(GeometryError),
    /// The writer refused to produce a document.
    ///
    /// The message is sanitized and never echoes submitted values or file
    /// system paths.
    Export(String),
}

impl StepError {
    /// Return the sanitized, user-facing message.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Invalid(error) => error.message(),
            Self::Export(message) => message,
        }
    }
}

impl fmt::Display for StepError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for StepError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Invalid(error) => Some(error),
            Self::Export(_) => None,
        }
    }
}

impl From<GeometryError> for StepError {
    fn from(error: GeometryError) -> Self {
        Self::Invalid(error)
    }
}
