//! The executor error hierarchy.
//!
//! The Python module declares three exception classes of its own:
//!
//! ```text
//! ToolExecutorError(RuntimeError)
//! ToolAuthorityError(ToolExecutorError)
//! ToolIsolationError(ToolExecutorError)
//! ```
//!
//! and it also raises the protocol layer's `ToolWorkspaceError` when a path
//! escapes the run directory. Rust has no exception hierarchy, so the classes
//! are the variants of one error type. [`ToolExecutorError::python_class_name`]
//! reports the class the Python implementation would have raised and
//! [`std::fmt::Display`] reproduces `str(error)`, so a differential can
//! compare the class and the message of a refusal.

use crate::protocol::ToolProtocolError;

/// A refusal raised by the tool executor.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolExecutorError {
    /// `ToolExecutorError`: a configuration or an internal failure.
    Executor(String),
    /// `ToolAuthorityError`: a tool asked for authority the executor does not
    /// have.
    Authority(String),
    /// `ToolIsolationError`: the requested isolation could not be established.
    Isolation(String),
    /// `ToolWorkspaceError`: a path escaped the run directory.
    Workspace(ToolProtocolError),
}

impl ToolExecutorError {
    /// The Python exception class the port reports.
    pub fn python_class_name(&self) -> &'static str {
        match self {
            Self::Executor(_) => "ToolExecutorError",
            Self::Authority(_) => "ToolAuthorityError",
            Self::Isolation(_) => "ToolIsolationError",
            Self::Workspace(_) => "ToolWorkspaceError",
        }
    }

    /// The message of the refusal.
    pub fn message(&self) -> &str {
        match self {
            Self::Executor(message) | Self::Authority(message) | Self::Isolation(message) => {
                message
            }
            Self::Workspace(error) => error.message(),
        }
    }

    /// A `ToolExecutorError`.
    pub fn executor(message: impl Into<String>) -> Self {
        Self::Executor(message.into())
    }

    /// A `ToolAuthorityError`.
    pub fn authority(message: impl Into<String>) -> Self {
        Self::Authority(message.into())
    }

    /// A `ToolIsolationError`.
    pub fn isolation(message: impl Into<String>) -> Self {
        Self::Isolation(message.into())
    }

    /// A `ToolWorkspaceError` with a plain message.
    pub fn workspace(message: impl Into<String>) -> Self {
        Self::Workspace(ToolProtocolError::workspace(message.into()))
    }

    /// The protocol error of a workspace refusal, when this is one.
    pub fn protocol_error(&self) -> Option<&ToolProtocolError> {
        match self {
            Self::Workspace(error) => Some(error),
            _ => None,
        }
    }
}

impl std::fmt::Display for ToolExecutorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for ToolExecutorError {}

/// The result of an executor operation.
pub type ExecutorResult<T> = Result<T, ToolExecutorError>;
