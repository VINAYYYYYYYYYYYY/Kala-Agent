//! The protocol error hierarchy.
//!
//! Python raises four public exception classes from `kala_agent.tools.protocol`
//! plus their shared base class:
//!
//! ```text
//! ToolProtocolError(RuntimeError)
//! ├── JsonEnvelopeError(ToolProtocolError, ValueError)
//! ├── ToolEnvelopeError(ToolProtocolError, ValueError)
//! ├── ToolWorkspaceError(ToolProtocolError, ValueError)
//! └── SchemaValidationError(ToolProtocolError, ValueError)
//! ```
//!
//! Rust has no exception hierarchy, so the five classes are the variants of one
//! enum. Every variant keeps the exact Python message text, and
//! [`ToolProtocolError::python_class_name`] reports the class the Python
//! implementation would have raised so a differential oracle can compare the
//! class name as well as the message.
//!
//! [`SchemaValidationError`] additionally carries the `reason` and `path`
//! attributes of the Python class. Its `str()` value is `"{path}: {reason}"`,
//! which is what [`std::fmt::Display`] reproduces.

use std::fmt;

/// Every failure raised by the tool protocol layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolProtocolError {
    /// `ToolProtocolError`: a protocol level failure with no more specific
    /// class.
    Protocol(String),
    /// `JsonEnvelopeError`: a malformed or oversized JSON envelope.
    JsonEnvelope(String),
    /// `ToolEnvelopeError`: an invalid request or response envelope field.
    Envelope(String),
    /// `ToolWorkspaceError`: an unusable workspace path.
    Workspace(String),
    /// `SchemaValidationError`: a value that does not satisfy a schema.
    Schema {
        /// The `reason` attribute: the message without the path prefix.
        reason: String,
        /// The `path` attribute: the JSON path of the offending value.
        path: String,
    },
}

impl ToolProtocolError {
    /// Build a `ToolProtocolError`.
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::Protocol(message.into())
    }

    /// Build a `JsonEnvelopeError`.
    pub fn json_envelope(message: impl Into<String>) -> Self {
        Self::JsonEnvelope(message.into())
    }

    /// Build a `ToolEnvelopeError`.
    pub fn envelope(message: impl Into<String>) -> Self {
        Self::Envelope(message.into())
    }

    /// Build a `ToolWorkspaceError`.
    pub fn workspace(message: impl Into<String>) -> Self {
        Self::Workspace(message.into())
    }

    /// Build a `SchemaValidationError` with its reason and path.
    pub fn schema(reason: impl Into<String>, path: impl Into<String>) -> Self {
        Self::Schema {
            reason: reason.into(),
            path: path.into(),
        }
    }

    /// The name of the Python exception class the port would have raised.
    pub const fn python_class_name(&self) -> &'static str {
        match self {
            Self::Protocol(_) => "ToolProtocolError",
            Self::JsonEnvelope(_) => "JsonEnvelopeError",
            Self::Envelope(_) => "ToolEnvelopeError",
            Self::Workspace(_) => "ToolWorkspaceError",
            Self::Schema { .. } => "SchemaValidationError",
        }
    }

    /// The message without any path prefix.
    ///
    /// For `SchemaValidationError` this is the `reason` attribute; for every
    /// other class it is the whole message.
    pub fn message(&self) -> &str {
        match self {
            Self::Protocol(message)
            | Self::JsonEnvelope(message)
            | Self::Envelope(message)
            | Self::Workspace(message) => message,
            Self::Schema { reason, .. } => reason,
        }
    }

    /// The `path` attribute of a `SchemaValidationError`.
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::Schema { path, .. } => Some(path),
            _ => None,
        }
    }

    /// The `reason` attribute of a `SchemaValidationError`.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Schema { reason, .. } => Some(reason),
            _ => None,
        }
    }

    /// Whether this is a `ToolProtocolError` but not one of its subclasses.
    pub fn is_protocol(&self) -> bool {
        matches!(self, Self::Protocol(_))
    }

    /// Whether this is a `JsonEnvelopeError`.
    pub fn is_json_envelope(&self) -> bool {
        matches!(self, Self::JsonEnvelope(_))
    }

    /// Whether this is a `ToolEnvelopeError`.
    pub fn is_envelope(&self) -> bool {
        matches!(self, Self::Envelope(_))
    }

    /// Whether this is a `ToolWorkspaceError`.
    pub fn is_workspace(&self) -> bool {
        matches!(self, Self::Workspace(_))
    }

    /// Whether this is a `SchemaValidationError`.
    pub fn is_schema(&self) -> bool {
        matches!(self, Self::Schema { .. })
    }
}

impl fmt::Display for ToolProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(message)
            | Self::JsonEnvelope(message)
            | Self::Envelope(message)
            | Self::Workspace(message) => formatter.write_str(message),
            Self::Schema { reason, path } => write!(formatter, "{path}: {reason}"),
        }
    }
}

impl std::error::Error for ToolProtocolError {}

impl From<std::io::Error> for ToolProtocolError {
    /// An input or output failure while reading or writing a stream.
    ///
    /// The Python implementation lets the `OSError` propagate unchanged; the
    /// Rust port reports it as a protocol failure with the same message.
    fn from(error: std::io::Error) -> Self {
        Self::Protocol(error.to_string())
    }
}
