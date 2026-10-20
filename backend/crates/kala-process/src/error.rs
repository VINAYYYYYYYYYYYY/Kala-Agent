//! Typed errors for the process, environment, workspace and sandbox layers.

use std::path::PathBuf;

use thiserror::Error;

use crate::compatibility::{CRASHED, INTERNAL_ERROR, ISOLATION_UNAVAILABLE, SANDBOX_UNAVAILABLE};
use crate::outcome::{Compatibility, CompatibilityStatus};

/// A relative workspace path was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkspaceError {
    /// The path was empty or whitespace-only.
    #[error("workspace paths must be nonempty strings")]
    Empty,
    /// A NUL byte or a backslash was present.
    #[error("workspace paths cannot contain null bytes or backslashes")]
    NullOrBackslash,
    /// The path exceeded the 1,024-character compatibility bound.
    #[error("workspace path exceeds the bounded length")]
    TooLong,
    /// An absolute path was supplied where a relative one is required.
    #[error("workspace paths must be relative")]
    Absolute,
    /// The path had no usable segments or more than 32 segments.
    #[error("workspace path has an unusable segment count")]
    SegmentCount,
    /// A segment was `.`, `..`, empty or outside the admitted character set.
    #[error("workspace path contains an unsafe segment")]
    UnsafeSegment,
    /// The base workspace is not an existing directory.
    #[error("workspace directory does not exist")]
    MissingDirectory,
    /// A segment was a symbolic link.
    #[error("workspace paths cannot traverse symlinks")]
    Symlink,
    /// Resolution escaped the base directory.
    #[error("workspace path escapes its run directory")]
    Escape,
    /// The resolved path exists but is not a directory.
    #[error("workspace path is not a directory")]
    NotADirectory,
    /// The filesystem could not be inspected.
    #[error("workspace path could not be inspected: {0}")]
    Io(String),
}

/// A child environment was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EnvironmentError {
    /// A key was empty, too long or outside `[A-Za-z_][A-Za-z0-9_]{0,63}`.
    #[error("environment keys must be simple names")]
    InvalidKey,
    /// A caller tried to replace a key owned by the supervisor.
    #[error("environment key {0} is protected by the executor")]
    ProtectedKey(String),
    /// A value contained a NUL byte.
    #[error("environment values must be null-free strings")]
    NullValue,
    /// Two keys differ only by case, which Windows cannot represent.
    #[error("environment keys must be unique without regard to case")]
    DuplicateKey,
    /// An extra key was not on the caller's explicit allow-list.
    #[error("environment key {0} is not permitted by the profile")]
    DisallowedKey(String),
    /// The workspace could not be made absolute.
    #[error("environment workspace is not usable: {0}")]
    Workspace(String),
}

/// The strict sandbox could not be built.
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

/// A sandbox probe payload or probe launch failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct SandboxProbeError(String);

impl SandboxProbeError {
    /// Create a probe error with a short, sanitised reason.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The sanitised detail.
    pub fn detail(&self) -> &str {
        &self.0
    }
}

/// A process could not be validated, launched, contained or reaped.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProcessError {
    /// The process specification is internally inconsistent or unusable.
    #[error("invalid process specification: {0}")]
    InvalidSpec(String),
    /// The working directory or a caller-supplied path was refused.
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    /// The child environment was refused.
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    /// The strict sandbox could not be prepared before launch.
    #[error("the sandbox could not be established: {0}")]
    SandboxSetup(#[from] SandboxSetupError),
    /// The executable could not be launched.
    #[error("the process could not be started ({}): {detail}", path.display())]
    Spawn {
        /// The explicit executable that was requested.
        path: PathBuf,
        /// A short, sanitised explanation.
        detail: String,
    },
    /// The platform containment boundary could not be established. No
    /// uncontrolled child was left running.
    #[error("process containment could not be established: {0}")]
    Containment(String),
    /// The post-fork bootstrap handshake failed before the worker ran.
    #[error("the process bootstrap failed: {0}")]
    Bootstrap(String),
    /// The platform wait operation failed.
    #[error("waiting for the process failed: {0}")]
    Wait(String),
    /// The child tree or its pipes could not be fully cleaned up within the
    /// termination deadline. The direct child was still reaped when possible.
    #[error("process cleanup was incomplete: {0}")]
    Cleanup(String),
    /// An operating-system operation failed.
    #[error("process I/O failed: {0}")]
    Io(String),
    /// An invariant this crate owns was violated.
    #[error("internal process error: {0}")]
    Internal(String),
}

impl ProcessError {
    /// A spawn failure carrying the explicit executable path and sanitised
    /// detail.
    pub fn spawn(path: impl Into<PathBuf>, detail: impl Into<String>) -> Self {
        Self::Spawn {
            path: path.into(),
            detail: detail.into(),
        }
    }

    /// The compatibility mapping a future tools executor can use.
    ///
    /// The low layer returns typed failures instead of pretending they are a
    /// successful dispatch. The mapping follows the Python executor's error
    /// classes where an equivalent exists:
    ///
    /// * sandbox preparation -> `sandbox_unavailable`;
    /// * executable/containment launch -> `isolation_unavailable`;
    /// * incomplete tree cleanup -> `tool_crashed`;
    /// * all input-validation failures -> `tool_error`.
    pub fn compatibility(&self) -> Compatibility {
        let (status, code) = match self {
            Self::SandboxSetup(_) => (CompatibilityStatus::Failed, SANDBOX_UNAVAILABLE),
            Self::Spawn { .. } | Self::Containment(_) | Self::Bootstrap(_) => {
                (CompatibilityStatus::Failed, ISOLATION_UNAVAILABLE)
            }
            Self::Cleanup(_) => (CompatibilityStatus::Failed, CRASHED),
            Self::InvalidSpec(_)
            | Self::Workspace(_)
            | Self::Environment(_)
            | Self::Wait(_)
            | Self::Io(_)
            | Self::Internal(_) => (CompatibilityStatus::Failed, INTERNAL_ERROR),
        };
        Compatibility {
            status,
            error_code: Some(code),
            message: self.to_string(),
        }
    }
}
