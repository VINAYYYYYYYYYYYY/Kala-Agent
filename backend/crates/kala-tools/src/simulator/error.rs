//! The simulator error hierarchy.
//!
//! The Python simulator module raises three public exception classes:
//!
//! ```text
//! SimulatorInputError(ValueError)
//! ToolWorkspaceError(ToolProtocolError, ValueError)
//! ```
//!
//! and it lets the `OverflowError` of a numeric conversion and the
//! `ValidationError` of the contract models escape. The Rust port collects
//! them in one enum, so a caller can match the failure without matching on a
//! message.

use kala_contracts::ContractError;

use crate::registry::ToolRegistryError;

/// Every failure raised by the simulator module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SimulatorError {
    /// `SimulatorInputError`: the arguments of a call are not usable.
    Input(String),
    /// `ToolWorkspaceError`: the workspace path is unusable.
    Workspace(String),
    /// `ValueError`: a value could not be converted.
    Value(String),
    /// `OverflowError`: an integer is too large to convert to a float.
    Overflow(String),
    /// `ValidationError`: the contract model refused a value.
    Contract(String),
    /// `JsonEnvelopeError`: a value could not be canonicalised.
    Json(String),
    /// A failure raised by the tool registry.
    Registry(ToolRegistryError),
}

impl SimulatorError {
    /// Build a `SimulatorInputError`.
    pub fn input(message: impl Into<String>) -> Self {
        Self::Input(message.into())
    }

    /// Build a `ToolWorkspaceError`.
    pub fn workspace(message: impl Into<String>) -> Self {
        Self::Workspace(message.into())
    }

    /// Build a `ValueError`.
    pub fn value(message: impl Into<String>) -> Self {
        Self::Value(message.into())
    }

    /// Build an `OverflowError`.
    pub fn overflow(message: impl Into<String>) -> Self {
        Self::Overflow(message.into())
    }

    /// Build a `ValidationError`.
    pub fn contract(error: &ContractError) -> Self {
        Self::Contract(error.to_string())
    }

    /// The name of the Python exception class the port would have raised.
    pub fn python_class_name(&self) -> &'static str {
        match self {
            Self::Input(_) => "SimulatorInputError",
            Self::Workspace(_) => "ToolWorkspaceError",
            Self::Value(_) => "ValueError",
            Self::Overflow(_) => "OverflowError",
            Self::Contract(_) => "ValidationError",
            Self::Json(_) => "JsonEnvelopeError",
            Self::Registry(error) => error.python_class_name(),
        }
    }
}

impl std::fmt::Display for SimulatorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input(message)
            | Self::Workspace(message)
            | Self::Value(message)
            | Self::Overflow(message)
            | Self::Contract(message)
            | Self::Json(message) => formatter.write_str(message),
            Self::Registry(error) => std::fmt::Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for SimulatorError {}

impl From<ToolRegistryError> for SimulatorError {
    fn from(error: ToolRegistryError) -> Self {
        Self::Registry(error)
    }
}
