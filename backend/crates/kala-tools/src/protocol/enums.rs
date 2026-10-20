//! The protocol enumerations.
//!
//! Python declares both classes as `StrEnum`, so the member value *is* the
//! serialized form: `str(ToolErrorCode.TIMEOUT) == "timeout"` and
//! `ToolEnvelopeKind.REQUEST.value == "request"`. The Rust enums therefore
//! expose [`as_str`](ToolErrorCode::as_str) as the single source of truth and
//! parse with the exact Python member values.

use kala_contracts::ToolRunStatus;

/// The kind of envelope carried on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolEnvelopeKind {
    /// A request envelope.
    Request,
    /// A response envelope.
    Response,
}

impl ToolEnvelopeKind {
    /// Every accepted kind, in Python declaration order.
    pub const ALL: &'static [Self] = &[Self::Request, Self::Response];

    /// The Python string value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Response => "response",
        }
    }

    /// Parse an exact Python enum value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "request" => Some(Self::Request),
            "response" => Some(Self::Response),
            _ => None,
        }
    }
}

impl std::fmt::Display for ToolEnvelopeKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The error codes a tool run can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolErrorCode {
    /// The tool is not registered.
    UnknownTool,
    /// The requested tool version is not registered.
    VersionNotFound,
    /// The registered descriptor does not match the request.
    DescriptorMismatch,
    /// The registered schema digest does not match the request.
    SchemaDigestMismatch,
    /// The arguments do not satisfy the input schema.
    InvalidArguments,
    /// The result does not satisfy the output schema.
    InvalidResult,
    /// A capability was not granted.
    CapabilityDenied,
    /// A capability is forbidden by policy.
    CapabilityForbidden,
    /// Network access was denied.
    NetworkDenied,
    /// Egress was denied.
    EgressDenied,
    /// The run needs an approval that was not granted.
    ApprovalRequired,
    /// The declared side effect does not match the observed one.
    SideEffectMismatch,
    /// The tool requires an isolation profile.
    IsolationRequired,
    /// The requested isolation profile is unavailable.
    IsolationUnavailable,
    /// The run exceeded its deadline.
    Timeout,
    /// The run was cancelled.
    Cancelled,
    /// The worker crashed.
    Crashed,
    /// The worker violated the protocol.
    ProtocolError,
    /// The output exceeded the declared budget.
    OutputTooLarge,
    /// The tool touched a path outside its workspace.
    WorkspaceViolation,
    /// The produced artifact was rejected.
    ArtifactRejected,
    /// The run exhausted its budget.
    BudgetExhausted,
    /// The worker pool is exhausted.
    WorkerCapacityExhausted,
    /// The sandbox is unavailable.
    SandboxUnavailable,
    /// The tool failed with an internal error.
    InternalError,
}

impl ToolErrorCode {
    /// Every accepted code, in Python declaration order.
    pub const ALL: &'static [Self] = &[
        Self::UnknownTool,
        Self::VersionNotFound,
        Self::DescriptorMismatch,
        Self::SchemaDigestMismatch,
        Self::InvalidArguments,
        Self::InvalidResult,
        Self::CapabilityDenied,
        Self::CapabilityForbidden,
        Self::NetworkDenied,
        Self::EgressDenied,
        Self::ApprovalRequired,
        Self::SideEffectMismatch,
        Self::IsolationRequired,
        Self::IsolationUnavailable,
        Self::Timeout,
        Self::Cancelled,
        Self::Crashed,
        Self::ProtocolError,
        Self::OutputTooLarge,
        Self::WorkspaceViolation,
        Self::ArtifactRejected,
        Self::BudgetExhausted,
        Self::WorkerCapacityExhausted,
        Self::SandboxUnavailable,
        Self::InternalError,
    ];

    /// The Python string value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownTool => "unknown_tool",
            Self::VersionNotFound => "version_not_found",
            Self::DescriptorMismatch => "descriptor_mismatch",
            Self::SchemaDigestMismatch => "schema_digest_mismatch",
            Self::InvalidArguments => "invalid_arguments",
            Self::InvalidResult => "invalid_result",
            Self::CapabilityDenied => "capability_denied",
            Self::CapabilityForbidden => "capability_forbidden",
            Self::NetworkDenied => "network_denied",
            Self::EgressDenied => "egress_denied",
            Self::ApprovalRequired => "approval_required",
            Self::SideEffectMismatch => "side_effect_mismatch",
            Self::IsolationRequired => "isolation_required",
            Self::IsolationUnavailable => "isolation_unavailable",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::Crashed => "tool_crashed",
            Self::ProtocolError => "protocol_error",
            Self::OutputTooLarge => "output_too_large",
            Self::WorkspaceViolation => "workspace_violation",
            Self::ArtifactRejected => "artifact_rejected",
            Self::BudgetExhausted => "budget_exhausted",
            Self::WorkerCapacityExhausted => "worker_capacity_exhausted",
            Self::SandboxUnavailable => "sandbox_unavailable",
            Self::InternalError => "tool_error",
        }
    }

    /// Parse an exact Python enum value.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|code| code.as_str() == value)
    }
}

impl std::fmt::Display for ToolErrorCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Whether a run status is one of the failure statuses.
///
/// The Python module keeps this set in `_FAILURE_STATUSES` and uses it for
/// the "failed tool runs require an error code" and "failed tool runs cannot
/// carry a result payload" rules.
pub const fn is_failure_status(status: ToolRunStatus) -> bool {
    matches!(
        status,
        ToolRunStatus::Failed | ToolRunStatus::Denied | ToolRunStatus::TimedOut
    )
}
