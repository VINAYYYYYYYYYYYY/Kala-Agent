//! The composition root's failure hierarchy.
//!
//! Every failure the Python composition root reports is one
//! [`ApplicationError`] carrying the Python class's `code` and its exact
//! message. The code is what the HTTP layer maps onto a status and what the
//! CLI reports, so it is preserved literally.
//!
//! [`ErrorKind`] is a native addition: it names the Python class family so a
//! transport can map statuses without rebuilding an `isinstance` chain. It
//! never changes a code or a message.

use std::fmt;

/// The Python exception class family of an application failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ErrorKind {
    /// `ApplicationError`.
    Application,
    /// `UnknownDomainError` (`ValueError`).
    UnknownDomain,
    /// `InvalidIdentifierError` (`ValueError`).
    InvalidIdentifier,
    /// `ReservedIdentifierError`.
    ReservedIdentifier,
    /// `InvalidParametersError` (`ValueError`).
    InvalidParameters,
    /// `UnknownRevisionError` (`LookupError`).
    UnknownRevision,
    /// `UnknownWorkflowError` (`LookupError`).
    UnknownWorkflow,
    /// `ProvenanceUnavailableError` (`LookupError`).
    ProvenanceUnavailable,
    /// `CheckpointNotPendingError` (`ValueError`).
    CheckpointNotPending,
    /// `CheckpointDecisionDeniedError` (`ValueError`).
    CheckpointDecisionDenied,
    /// `CheckpointDigestMismatchError`.
    CheckpointDigestMismatch,
    /// `RevisionCommitRejectedError`.
    RevisionCommitRejected,
    /// `RevisionForkRefusedError`.
    RevisionForkRefused,
    /// `RevisionHeadMovedError`.
    RevisionHeadMoved,
    /// `UnknownPreparedRevisionError` (`LookupError`).
    UnknownPreparedRevision,
    /// `CorruptKernelStateError` (`RuntimeError`).
    CorruptKernelState,
    /// `WorkflowFinishedError` (`ValueError`).
    WorkflowFinished,
    /// `ClosedApplicationError` (`RuntimeError`).
    ClosedApplication,
}

impl ErrorKind {
    /// The `code` class attribute the Python class declares.
    #[must_use]
    pub const fn default_code(self) -> &'static str {
        match self {
            Self::Application => "application_error",
            Self::UnknownDomain => "unknown_domain",
            Self::InvalidIdentifier => "invalid_identifier",
            Self::ReservedIdentifier => "reserved_identifier",
            Self::InvalidParameters => "invalid_parameters",
            Self::UnknownRevision => "unknown_revision",
            Self::UnknownWorkflow => "unknown_workflow",
            Self::ProvenanceUnavailable => "provenance_unavailable",
            Self::CheckpointNotPending => "checkpoint_not_pending",
            Self::CheckpointDecisionDenied => "checkpoint_denied",
            Self::CheckpointDigestMismatch => "checkpoint_digest_mismatch",
            Self::RevisionCommitRejected => "revision_commit_rejected",
            Self::RevisionForkRefused => "revision_fork_refused",
            Self::RevisionHeadMoved => "revision_head_moved",
            Self::UnknownPreparedRevision => "unknown_prepared_revision",
            Self::CorruptKernelState => "corrupt_kernel_state",
            Self::WorkflowFinished => "workflow_finished",
            Self::ClosedApplication => "application_closed",
        }
    }

    /// The Python exception class name this family maps to.
    #[must_use]
    pub const fn python_class_name(self) -> &'static str {
        match self {
            Self::Application => "ApplicationError",
            Self::UnknownDomain => "UnknownDomainError",
            Self::InvalidIdentifier => "InvalidIdentifierError",
            Self::ReservedIdentifier => "ReservedIdentifierError",
            Self::InvalidParameters => "InvalidParametersError",
            Self::UnknownRevision => "UnknownRevisionError",
            Self::UnknownWorkflow => "UnknownWorkflowError",
            Self::ProvenanceUnavailable => "ProvenanceUnavailableError",
            Self::CheckpointNotPending => "CheckpointNotPendingError",
            Self::CheckpointDecisionDenied => "CheckpointDecisionDeniedError",
            Self::CheckpointDigestMismatch => "CheckpointDigestMismatchError",
            Self::RevisionCommitRejected => "RevisionCommitRejectedError",
            Self::RevisionForkRefused => "RevisionForkRefusedError",
            Self::RevisionHeadMoved => "RevisionHeadMovedError",
            Self::UnknownPreparedRevision => "UnknownPreparedRevisionError",
            Self::CorruptKernelState => "CorruptKernelStateError",
            Self::WorkflowFinished => "WorkflowFinishedError",
            Self::ClosedApplication => "ClosedApplicationError",
        }
    }

    /// Whether the Python class is a `LookupError` (a missing record), which
    /// the transport reports as `404`.
    #[must_use]
    pub const fn is_lookup(self) -> bool {
        matches!(
            self,
            Self::UnknownRevision
                | Self::UnknownWorkflow
                | Self::ProvenanceUnavailable
                | Self::UnknownPreparedRevision
        )
    }

    /// Whether the Python class is a `ValueError` (a refused value), which the
    /// transport reports as `422` unless a more specific rule applies.
    #[must_use]
    pub const fn is_value(self) -> bool {
        matches!(
            self,
            Self::UnknownDomain
                | Self::InvalidIdentifier
                | Self::ReservedIdentifier
                | Self::InvalidParameters
                | Self::CheckpointNotPending
                | Self::CheckpointDecisionDenied
                | Self::CheckpointDigestMismatch
                | Self::RevisionCommitRejected
                | Self::RevisionForkRefused
                | Self::RevisionHeadMoved
                | Self::WorkflowFinished
        )
    }
}

/// One failure reported by the composition root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationError {
    kind: ErrorKind,
    code: &'static str,
    message: String,
}

impl ApplicationError {
    /// Build a failure of one class family with its declared code.
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: kind.default_code(),
            message: message.into(),
        }
    }

    /// Build the base `ApplicationError`.
    #[must_use]
    pub fn application(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Application, message)
    }

    /// The Python exception class family.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The Python `code` class attribute.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }

    /// The exception message (`str(error)`).
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ApplicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ApplicationError {}

impl From<kala_persistence::PersistenceError> for ApplicationError {
    fn from(error: kala_persistence::PersistenceError) -> Self {
        Self::application(error.to_string())
    }
}

impl From<kala_repositories::RepositoryError> for ApplicationError {
    fn from(error: kala_repositories::RepositoryError) -> Self {
        Self::application(error.to_string())
    }
}

impl From<kala_artifacts::ArtifactError> for ApplicationError {
    fn from(error: kala_artifacts::ArtifactError) -> Self {
        Self::application(error.to_string())
    }
}

/// Convenience constructor macro for one class family.
macro_rules! constructors {
    ($($name:ident => $kind:ident),* $(,)?) => {
        impl ApplicationError {
            $(
                #[doc = concat!("Build the Python `", stringify!($kind), "`-family failure.")]
                #[must_use]
                pub fn $name(message: impl Into<String>) -> Self {
                    Self::new(ErrorKind::$kind, message)
                }
            )*
        }
    };
}

constructors! {
    unknown_domain => UnknownDomain,
    invalid_identifier => InvalidIdentifier,
    reserved_identifier => ReservedIdentifier,
    invalid_parameters => InvalidParameters,
    unknown_revision => UnknownRevision,
    unknown_workflow => UnknownWorkflow,
    provenance_unavailable => ProvenanceUnavailable,
    checkpoint_not_pending => CheckpointNotPending,
    checkpoint_decision_denied => CheckpointDecisionDenied,
    checkpoint_digest_mismatch => CheckpointDigestMismatch,
    revision_commit_rejected => RevisionCommitRejected,
    revision_fork_refused => RevisionForkRefused,
    revision_head_moved => RevisionHeadMoved,
    unknown_prepared_revision => UnknownPreparedRevision,
    corrupt_kernel_state => CorruptKernelState,
    workflow_finished => WorkflowFinished,
    closed_application => ClosedApplication,
}

/// Shorthand for a fallible application operation.
pub type ApplicationResult<T> = Result<T, ApplicationError>;
