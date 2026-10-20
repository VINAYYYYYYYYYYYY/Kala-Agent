//! Typed errors mirroring `kala_agent.tools.sandbox`.
//!
//! The Python module raises `SandboxSetupError` for a jail that could not be
//! built and `SandboxProbeError` for a probe payload that could not be read.
//! Both derive from `SandboxError`. The rendered text is byte-identical to the
//! Python message so refusal reasons survive the migration unchanged.

use thiserror::Error;

/// The base error of the sandbox layer.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SandboxError {
    /// The jail could not be built.
    #[error(transparent)]
    Setup(#[from] SandboxSetupError),
    /// A probe payload could not be read.
    #[error(transparent)]
    Probe(#[from] SandboxProbeError),
}

/// The strict jail could not be built.
///
/// The message is short and sanitised: it names the failing operation and the
/// operating-system reason, never a host path that could leak a user name.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct SandboxSetupError(String);

impl SandboxSetupError {
    /// Create a setup error with a short, sanitised reason.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The malformed-payload error used by [`crate::SandboxSpec::from_payload`].
    pub fn malformed() -> Self {
        Self("the sandbox specification is malformed".to_owned())
    }

    /// The sanitised detail.
    pub fn detail(&self) -> &str {
        &self.0
    }
}

/// A probe payload or probe launch failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct SandboxProbeError(String);

impl SandboxProbeError {
    /// Create a probe error with a short, sanitised reason.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The malformed-payload error used by the probe entry point.
    pub fn malformed() -> Self {
        Self("the probe payload is malformed".to_owned())
    }

    /// The sanitised detail.
    pub fn detail(&self) -> &str {
        &self.0
    }
}

/// The Python exception class name that a Rust failure maps onto.
///
/// The Python module reports the *class* of a failure in the probe refusal
/// text (`the sandbox probe could not start: OSError`). Rust has no
/// exception classes, so the failure kinds are mapped onto the Python class
/// names that the same failure would have produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// An operating-system failure (`OSError`).
    Os,
    /// A value failure (`ValueError`).
    Value,
    /// A sandbox setup failure (`SandboxSetupError`).
    Setup,
    /// An unexpected internal failure (`RuntimeError`).
    Runtime,
}

impl FailureClass {
    /// The Python exception class name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Os => "OSError",
            Self::Value => "ValueError",
            Self::Setup => "SandboxSetupError",
            Self::Runtime => "RuntimeError",
        }
    }
}

impl std::fmt::Display for FailureClass {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}
