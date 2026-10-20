//! Budget errors mirroring the Python kernel exception hierarchy.

use crate::counter::{Counter, CounterError};
use crate::decimal::{Decimal, DecimalError, DecimalParseError};
use crate::dimension::{BudgetDimension, DimensionError};
use std::fmt;

/// A numeric value carried by an exhaustion error.
#[derive(Clone, Debug, PartialEq)]
pub enum BudgetValue {
    /// An arbitrary-precision integer counter.
    Integer(Counter),
    /// A floating-point duration.
    Float(f64),
    /// An arbitrary-precision decimal cost.
    Decimal(Decimal),
}

impl fmt::Display for BudgetValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(value) => formatter.write_str(&value.to_decimal_string()),
            Self::Float(value) => formatter.write_str(&Decimal::python_float_repr(*value)),
            Self::Decimal(value) => formatter.write_str(&value.to_python_string()),
        }
    }
}

/// Details of a budget exhaustion, mirroring `BudgetExhaustedError`.
#[derive(Clone, Debug, PartialEq)]
pub struct ExhaustedBudget {
    /// The exhausted dimension.
    pub dimension: BudgetDimension,
    /// The prospective value for the dimension.
    pub requested: BudgetValue,
    /// The remaining budget for the dimension.
    pub remaining: BudgetValue,
    /// The configured limit for the dimension.
    pub limit: BudgetValue,
}

impl fmt::Display for ExhaustedBudget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "budget exhausted for {}: requested {}, remaining {}, limit {}",
            self.dimension.as_str(),
            self.requested,
            self.remaining,
            self.limit
        )
    }
}

/// Errors produced by the budget kernel.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum BudgetError {
    /// Invalid persisted budget state (`BudgetStateError`).
    #[error("budget state is invalid: {0}")]
    State(String),
    /// Invalid constructor or charge input.
    #[error("{0}")]
    Validation(String),
    /// A counter subtraction would become negative (`BudgetCounterError`).
    #[error("budget counters cannot become negative")]
    CounterUnderflow,
    /// A reservation id was not registered (`BudgetReservationUnknownError`).
    #[error("unknown reservation: {id}")]
    ReservationUnknown {
        /// The unknown reservation id.
        id: String,
    },
    /// The supplied reservation usage does not match the ledger.
    #[error("reservation usage does not match the ledger")]
    ReservationMismatch {
        /// The reservation id being committed or released.
        id: String,
    },
    /// The same reservation id appeared twice while importing state.
    #[error("budget state is invalid: duplicate reservation {id}")]
    DuplicateReservation {
        /// The duplicated reservation id.
        id: String,
    },
    /// The persisted state version is not supported.
    #[error("budget state is invalid: unsupported version {version}")]
    UnsupportedVersion {
        /// The rejected version.
        version: String,
    },
    /// A decimal arithmetic error escaped into the budget domain.
    #[error(transparent)]
    Decimal(#[from] DecimalError),
    /// A decimal literal could not be parsed.
    #[error(transparent)]
    DecimalParse(#[from] DecimalParseError),
    /// A counter input was invalid or negative.
    #[error(transparent)]
    Counter(#[from] CounterError),
    /// A duration could not be represented on the floating-point range.
    #[error("duration is outside the representable range")]
    DurationRange,
    /// A dimension name was invalid.
    #[error(transparent)]
    Dimension(#[from] DimensionError),
    /// A charge would exceed the configured limits (`BudgetExhaustedError`).
    #[error("{0}")]
    Exhausted(Box<ExhaustedBudget>),
}

impl BudgetError {
    /// The exhausted dimension, when this error is an exhaustion error.
    #[must_use]
    pub fn exhausted_dimension(&self) -> Option<BudgetDimension> {
        match self {
            Self::Exhausted(details) => Some(details.dimension),
            _ => None,
        }
    }

    /// The Python exception class name a service wrapper needs to preserve.
    #[must_use]
    pub fn contract_error_name(&self) -> &'static str {
        match self {
            Self::State(_) => "BudgetStateError",
            Self::Validation(_) => "BudgetError",
            Self::CounterUnderflow => "BudgetCounterError",
            Self::ReservationUnknown { .. } => "BudgetReservationUnknownError",
            Self::ReservationMismatch { .. } => "BudgetReservationMismatchError",
            Self::DuplicateReservation { .. } | Self::UnsupportedVersion { .. } => {
                "BudgetStateError"
            }
            Self::Exhausted(details) => match details.dimension {
                BudgetDimension::ModelCalls => "ModelCallBudgetExhaustedError",
                BudgetDimension::ToolCalls => "ToolCallBudgetExhaustedError",
                BudgetDimension::TotalTokens => "TokenBudgetExhaustedError",
                BudgetDimension::ArtifactBytes => "ArtifactBudgetExhaustedError",
                BudgetDimension::DurationSeconds => "DurationBudgetExhaustedError",
                BudgetDimension::Cost => "CostBudgetExhaustedError",
                BudgetDimension::ReplanningAttempts => "ReplanningBudgetExhaustedError",
            },
            Self::Decimal(DecimalError::Overflow) => "Overflow",
            Self::Decimal(_) => "InvalidOperation",
            Self::DecimalParse(_) => "ValueError",
            Self::DurationRange => "OverflowError",
            Self::Counter(_) => "ValueError",
            Self::Dimension(_) => "ValueError",
        }
    }
}

impl From<ExhaustedBudget> for BudgetError {
    fn from(details: ExhaustedBudget) -> Self {
        Self::Exhausted(Box::new(details))
    }
}
