//! Budget dimension identifiers matching `kala_agent.kernel.budget.BudgetDimension`.

use std::fmt;
use std::str::FromStr;

/// Error raised when a dimension name is not a known budget dimension.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("'{name}' is not a valid BudgetDimension")]
pub struct DimensionError {
    /// The rejected name.
    pub name: String,
}

/// One of the seven accounted budget dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BudgetDimension {
    /// Model calls.
    ModelCalls,
    /// Tool calls.
    ToolCalls,
    /// Total tokens.
    TotalTokens,
    /// Artifact bytes.
    ArtifactBytes,
    /// Duration seconds.
    DurationSeconds,
    /// Cost (arbitrary-precision decimal).
    Cost,
    /// Replanning attempts.
    ReplanningAttempts,
}

impl BudgetDimension {
    /// Every dimension in the exhaustion-priority order used by the kernel.
    pub const ALL: [Self; 7] = [
        Self::ModelCalls,
        Self::ToolCalls,
        Self::TotalTokens,
        Self::ArtifactBytes,
        Self::DurationSeconds,
        Self::Cost,
        Self::ReplanningAttempts,
    ];

    /// Python `BudgetDimension` value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ModelCalls => "model_calls",
            Self::ToolCalls => "tool_calls",
            Self::TotalTokens => "total_tokens",
            Self::ArtifactBytes => "artifact_bytes",
            Self::DurationSeconds => "duration_seconds",
            Self::Cost => "cost",
            Self::ReplanningAttempts => "replanning_attempts",
        }
    }

    /// The `BudgetUsage` attribute name (same as [`Self::as_str`]).
    #[must_use]
    pub const fn field_name(self) -> &'static str {
        self.as_str()
    }
}

impl fmt::Display for BudgetDimension {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for BudgetDimension {
    type Err = DimensionError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "model_calls" => Ok(Self::ModelCalls),
            "tool_calls" => Ok(Self::ToolCalls),
            "total_tokens" => Ok(Self::TotalTokens),
            "artifact_bytes" => Ok(Self::ArtifactBytes),
            "duration_seconds" => Ok(Self::DurationSeconds),
            "cost" => Ok(Self::Cost),
            "replanning_attempts" => Ok(Self::ReplanningAttempts),
            _ => Err(DimensionError {
                name: name.to_owned(),
            }),
        }
    }
}
