//! Native Rust budget kernel: arbitrary-precision [`Decimal`] arithmetic plus
//! the run-budget ledger and usage accounting used by the Kala Agent
//! contracts.
//!
//! All types are pure Rust and never shell out to Python. The implementation
//! mirrors the CPython `decimal`/`kala_agent.kernel.budget` semantics frozen
//! in `backend/fixtures/corpus/v1/decimal_budget.json`.

pub mod compat;
pub mod counter;
pub mod decimal;
pub mod dimension;
pub mod duration;
pub mod errors;
mod json;
pub mod ledger;
pub mod run_budget;
mod unicode;
pub mod usage;

pub use compat::{DomainParameterError, clamp_unit_interval};
pub use counter::{Counter, CounterError};
pub use decimal::{
    Context, Decimal, DecimalError, DecimalParseError, PYTHON_DEFAULT_EMAX, PYTHON_DEFAULT_EMIN,
    PYTHON_DEFAULT_PRECISION, PYTHON_MAX_EMAX, PYTHON_MIN_ETINY, RoundingMode,
};
pub use dimension::{BudgetDimension, DimensionError};
pub use duration::{
    FloatParseError, MICROSECONDS_PER_SECOND, normalize_duration, python_float_parse,
};
pub use errors::{BudgetError, BudgetValue, ExhaustedBudget};
pub use ledger::{BUDGET_STATE_VERSION, BudgetLedger, BudgetReservation};
pub use run_budget::RunBudget;
pub use usage::{BudgetSnapshot, BudgetUsage, ModelCallCharge, ToolCallCharge, UsageDelta};
