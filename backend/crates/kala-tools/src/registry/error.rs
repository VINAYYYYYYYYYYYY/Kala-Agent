//! The registry error hierarchy.
//!
//! Python raises seven public exception classes from
//! `kala_agent.tools.registry` plus their shared base class:
//!
//! ```text
//! ToolRegistryError(RuntimeError)
//! ├── ToolRegistrationError(ToolRegistryError, ValueError)
//! │   ├── DuplicateToolRegistrationError(ToolRegistrationError)
//! │   └── SchemaDigestMismatchError(ToolRegistrationError)
//! ├── UnknownToolError(ToolRegistryError)
//! │   └── ToolVersionNotFoundError(UnknownToolError)
//! └── ToolCapabilityError(ToolRegistryError)
//! ```
//!
//! Rust has no exception hierarchy, so the classes are the variants of one
//! enum. Every variant keeps the exact Python message text and
//! [`ToolRegistryError::python_class_name`] reports the class the Python
//! implementation would have raised, so a differential oracle can compare the
//! class name as well as the message.
//!
//! [`ToolCapabilityError`] additionally carries the `missing_capabilities`
//! attribute of the Python class: the sorted tuple of the capabilities the
//! tool needs and the grant does not contain.

use std::fmt;

/// Every failure raised by the tool registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolRegistryError {
    /// `ToolRegistryError`: a registry level failure with no more specific
    /// class.
    Registry(String),
    /// `ToolRegistrationError`: a registration the registry refuses.
    Registration(String),
    /// `DuplicateToolRegistrationError`: a `(tool_id, version)` pair that is
    /// already registered.
    Duplicate(String),
    /// `SchemaDigestMismatchError`: a schema whose digest does not match the
    /// digest the descriptor declares.
    SchemaDigestMismatch(String),
    /// `UnknownToolError`: a tool that is not registered.
    UnknownTool(String),
    /// `ToolVersionNotFoundError`: a registered tool without the requested
    /// version.
    VersionNotFound(String),
    /// `ToolCapabilityError`: a call that is missing a granted capability.
    Capability {
        /// The message of the Python exception.
        message: String,
        /// The `missing_capabilities` attribute: the sorted capability names.
        missing_capabilities: Vec<String>,
    },
    /// `ValueError`: a version segment that CPython's `int()` cannot parse.
    ///
    /// The Python implementation builds a version ordering key with
    /// `int(segment)`. A segment of characters that are digits for
    /// `str.isdigit()` but not for `str.isdecimal()` — a superscript, for
    /// example — makes `int()` raise a `ValueError`, which is neither a
    /// `ToolRegistryError` nor a `ToolRegistrationError`.
    Value(String),
    /// `JsonEnvelopeError`: a protocol failure raised while the registry
    /// canonicalises a schema or a descriptor.
    ///
    /// The Python implementation computes the schema digests with
    /// `canonical_json_digest`, which raises a `JsonEnvelopeError` when a
    /// value cannot be canonicalised.
    Json(String),
}

impl ToolRegistryError {
    /// Build a `ToolRegistryError`.
    pub fn registry(message: impl Into<String>) -> Self {
        Self::Registry(message.into())
    }

    /// Build a `ToolRegistrationError`.
    pub fn registration(message: impl Into<String>) -> Self {
        Self::Registration(message.into())
    }

    /// Build a `DuplicateToolRegistrationError`.
    pub fn duplicate(message: impl Into<String>) -> Self {
        Self::Duplicate(message.into())
    }

    /// Build a `SchemaDigestMismatchError`.
    pub fn schema_digest_mismatch(message: impl Into<String>) -> Self {
        Self::SchemaDigestMismatch(message.into())
    }

    /// Build an `UnknownToolError`.
    pub fn unknown_tool(message: impl Into<String>) -> Self {
        Self::UnknownTool(message.into())
    }

    /// Build a `ToolVersionNotFoundError`.
    pub fn version_not_found(message: impl Into<String>) -> Self {
        Self::VersionNotFound(message.into())
    }

    /// Build a `ToolCapabilityError`.
    pub fn capability(message: impl Into<String>, missing_capabilities: Vec<String>) -> Self {
        Self::Capability {
            message: message.into(),
            missing_capabilities,
        }
    }

    /// Build a `ValueError`.
    pub fn value(message: impl Into<String>) -> Self {
        Self::Value(message.into())
    }

    /// Build a `JsonEnvelopeError`.
    pub fn json(message: impl Into<String>) -> Self {
        Self::Json(message.into())
    }

    /// The name of the Python exception class the port would have raised.
    pub const fn python_class_name(&self) -> &'static str {
        match self {
            Self::Registry(_) => "ToolRegistryError",
            Self::Registration(_) => "ToolRegistrationError",
            Self::Duplicate(_) => "DuplicateToolRegistrationError",
            Self::SchemaDigestMismatch(_) => "SchemaDigestMismatchError",
            Self::UnknownTool(_) => "UnknownToolError",
            Self::VersionNotFound(_) => "ToolVersionNotFoundError",
            Self::Capability { .. } => "ToolCapabilityError",
            Self::Value(_) => "ValueError",
            Self::Json(_) => "JsonEnvelopeError",
        }
    }

    /// The `missing_capabilities` attribute of a `ToolCapabilityError`.
    pub fn missing_capabilities(&self) -> Option<&[String]> {
        match self {
            Self::Capability {
                missing_capabilities,
                ..
            } => Some(missing_capabilities),
            _ => None,
        }
    }

    /// Whether the failure is a `ToolRegistrationError` or one of its
    /// subclasses.
    pub fn is_registration(&self) -> bool {
        matches!(
            self,
            Self::Registration(_) | Self::Duplicate(_) | Self::SchemaDigestMismatch(_)
        )
    }

    /// Whether the failure is an `UnknownToolError` or one of its subclasses.
    pub fn is_unknown_tool(&self) -> bool {
        matches!(self, Self::UnknownTool(_) | Self::VersionNotFound(_))
    }
}

impl fmt::Display for ToolRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registry(message)
            | Self::Registration(message)
            | Self::Duplicate(message)
            | Self::SchemaDigestMismatch(message)
            | Self::UnknownTool(message)
            | Self::VersionNotFound(message)
            | Self::Value(message)
            | Self::Json(message) => formatter.write_str(message),
            Self::Capability { message, .. } => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ToolRegistryError {}
