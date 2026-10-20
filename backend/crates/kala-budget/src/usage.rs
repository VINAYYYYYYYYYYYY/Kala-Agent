//! Budget usage accounting: `BudgetUsage`, `BudgetSnapshot` and charge deltas.

use crate::counter::Counter;
use crate::decimal::Decimal;
use crate::dimension::BudgetDimension;
use crate::duration::{duration_microseconds, normalize_duration, seconds_from_microseconds};
use crate::errors::{BudgetError, BudgetValue, ExhaustedBudget};
use crate::json::{counter_value, decimal_string_value, float_value};
use crate::run_budget::RunBudget;
use num_bigint::BigInt;
use serde_json::{Map, Value};

/// The seven budget counters, mirroring `BudgetUsage`.
///
/// Integer fields are arbitrary precision and never wrap; durations are held
/// on the same whole-microsecond float grid as Python; `cost` preserves the
/// exact decimal coefficient/exponent supplied by callers.
#[derive(Clone, Debug)]
pub struct BudgetUsage {
    model_calls: Counter,
    tool_calls: Counter,
    total_tokens: Counter,
    artifact_bytes: Counter,
    duration_seconds: f64,
    cost: Decimal,
    replanning_attempts: Counter,
}

impl Default for BudgetUsage {
    fn default() -> Self {
        Self {
            model_calls: Counter::zero(),
            tool_calls: Counter::zero(),
            total_tokens: Counter::zero(),
            artifact_bytes: Counter::zero(),
            duration_seconds: 0.0,
            cost: Decimal::zero(),
            replanning_attempts: Counter::zero(),
        }
    }
}

impl PartialEq for BudgetUsage {
    fn eq(&self, other: &Self) -> bool {
        self.model_calls == other.model_calls
            && self.tool_calls == other.tool_calls
            && self.total_tokens == other.total_tokens
            && self.artifact_bytes == other.artifact_bytes
            && self.duration_seconds == other.duration_seconds
            && self.cost == other.cost
            && self.replanning_attempts == other.replanning_attempts
    }
}

impl Eq for BudgetUsage {}

impl BudgetUsage {
    /// A zero usage value with cost `Decimal("0")`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the model-call counter.
    #[must_use]
    pub fn with_model_calls(mut self, value: impl Into<Counter>) -> Self {
        self.model_calls = value.into();
        self
    }

    /// Set the tool-call counter.
    #[must_use]
    pub fn with_tool_calls(mut self, value: impl Into<Counter>) -> Self {
        self.tool_calls = value.into();
        self
    }

    /// Set the total-token counter.
    #[must_use]
    pub fn with_total_tokens(mut self, value: impl Into<Counter>) -> Self {
        self.total_tokens = value.into();
        self
    }

    /// Set the artifact-byte counter.
    #[must_use]
    pub fn with_artifact_bytes(mut self, value: impl Into<Counter>) -> Self {
        self.artifact_bytes = value.into();
        self
    }

    /// Set the replanning-attempt counter.
    #[must_use]
    pub fn with_replanning_attempts(mut self, value: impl Into<Counter>) -> Self {
        self.replanning_attempts = value.into();
        self
    }

    /// Validate and grid-round a duration, matching `_nonnegative_duration`.
    pub fn try_with_duration_seconds(mut self, value: f64) -> Result<Self, BudgetError> {
        self.duration_seconds = normalize_duration(value)?;
        Ok(self)
    }

    /// Set an exact decimal cost, rejecting non-finite or negative values.
    pub fn try_with_cost(mut self, value: Decimal) -> Result<Self, BudgetError> {
        validate_cost(&value)?;
        self.cost = value;
        Ok(self)
    }

    /// Parse and set a decimal cost string, matching `Decimal(str(value))`.
    pub fn try_with_cost_str(self, value: &str) -> Result<Self, BudgetError> {
        self.try_with_cost(Decimal::parse(value)?)
    }

    /// Set a counter from a possibly-negative integer, preserving validation.
    pub fn try_with_model_calls_i64(self, value: i64) -> Result<Self, BudgetError> {
        Ok(self.with_model_calls(Counter::try_from_i64(value)?))
    }

    /// Set a counter from a possibly-negative integer, preserving validation.
    pub fn try_with_tool_calls_i64(self, value: i64) -> Result<Self, BudgetError> {
        Ok(self.with_tool_calls(Counter::try_from_i64(value)?))
    }

    /// Set a counter from a possibly-negative integer, preserving validation.
    pub fn try_with_total_tokens_i64(self, value: i64) -> Result<Self, BudgetError> {
        Ok(self.with_total_tokens(Counter::try_from_i64(value)?))
    }

    /// Set a counter from a possibly-negative integer, preserving validation.
    pub fn try_with_artifact_bytes_i64(self, value: i64) -> Result<Self, BudgetError> {
        Ok(self.with_artifact_bytes(Counter::try_from_i64(value)?))
    }

    /// Model-call counter.
    #[must_use]
    pub const fn model_calls(&self) -> &Counter {
        &self.model_calls
    }

    /// Tool-call counter.
    #[must_use]
    pub const fn tool_calls(&self) -> &Counter {
        &self.tool_calls
    }

    /// Total-token counter.
    #[must_use]
    pub const fn total_tokens(&self) -> &Counter {
        &self.total_tokens
    }

    /// `total_tokens` alias used by the Python property `tokens`.
    #[must_use]
    pub const fn tokens(&self) -> &Counter {
        &self.total_tokens
    }

    /// Artifact-byte counter.
    #[must_use]
    pub const fn artifact_bytes(&self) -> &Counter {
        &self.artifact_bytes
    }

    /// Duration in seconds on the whole-microsecond grid.
    #[must_use]
    pub const fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }

    /// Exact decimal cost.
    #[must_use]
    pub const fn cost(&self) -> &Decimal {
        &self.cost
    }

    /// Replanning-attempt counter.
    #[must_use]
    pub const fn replanning_attempts(&self) -> &Counter {
        &self.replanning_attempts
    }

    /// `replanning_attempts` alias used by the Python property `replanning`.
    #[must_use]
    pub const fn replanning(&self) -> &Counter {
        &self.replanning_attempts
    }

    /// `BudgetUsage.__add__`: integer sums, microsecond-grid duration sums and
    /// context-rounded decimal costs.
    pub fn add(&self, other: &Self) -> Result<Self, BudgetError> {
        let micros = duration_microseconds(self.duration_seconds)?
            + duration_microseconds(other.duration_seconds)?;
        let duration_seconds = normalize_duration(seconds_from_microseconds(&micros)?)?;
        let cost = self.cost.add(&other.cost)?;
        Ok(Self {
            model_calls: self.model_calls.add(&other.model_calls),
            tool_calls: self.tool_calls.add(&other.tool_calls),
            total_tokens: self.total_tokens.add(&other.total_tokens),
            artifact_bytes: self.artifact_bytes.add(&other.artifact_bytes),
            duration_seconds,
            cost,
            replanning_attempts: self.replanning_attempts.add(&other.replanning_attempts),
        })
    }

    /// `BudgetUsage.__sub__`: fails closed on any dimension that would become
    /// negative, exactly like the Python `exceeds` guard.
    pub fn sub(&self, other: &Self) -> Result<Self, BudgetError> {
        if self.exceeds(other) {
            return Err(BudgetError::CounterUnderflow);
        }
        let micros = duration_microseconds(self.duration_seconds)?
            - duration_microseconds(other.duration_seconds)?;
        let duration_seconds = normalize_duration(seconds_from_microseconds(&micros)?)?;
        let cost = self.cost.sub(&other.cost)?;
        Ok(Self {
            model_calls: self
                .model_calls
                .checked_sub(&other.model_calls)
                .ok_or(BudgetError::CounterUnderflow)?,
            tool_calls: self
                .tool_calls
                .checked_sub(&other.tool_calls)
                .ok_or(BudgetError::CounterUnderflow)?,
            total_tokens: self
                .total_tokens
                .checked_sub(&other.total_tokens)
                .ok_or(BudgetError::CounterUnderflow)?,
            artifact_bytes: self
                .artifact_bytes
                .checked_sub(&other.artifact_bytes)
                .ok_or(BudgetError::CounterUnderflow)?,
            duration_seconds,
            cost,
            replanning_attempts: self
                .replanning_attempts
                .checked_sub(&other.replanning_attempts)
                .ok_or(BudgetError::CounterUnderflow)?,
        })
    }

    /// Python's awkwardly named `exceeds`: `true` when `self` is below
    /// `other` on any dimension.
    #[must_use]
    pub fn exceeds(&self, other: &Self) -> bool {
        self.model_calls < other.model_calls
            || self.tool_calls < other.tool_calls
            || self.total_tokens < other.total_tokens
            || self.artifact_bytes < other.artifact_bytes
            || self.duration_seconds < other.duration_seconds
            || self.cost < other.cost
            || self.replanning_attempts < other.replanning_attempts
    }

    /// `BudgetUsage.as_dict()`: integer counters stay integers, duration is a
    /// float and cost is the exact `str(Decimal)`.
    #[must_use]
    pub fn as_dict(&self) -> Value {
        let mut object = Map::new();
        object.insert("model_calls".to_owned(), counter_value(&self.model_calls));
        object.insert("tool_calls".to_owned(), counter_value(&self.tool_calls));
        object.insert("total_tokens".to_owned(), counter_value(&self.total_tokens));
        object.insert(
            "artifact_bytes".to_owned(),
            counter_value(&self.artifact_bytes),
        );
        object.insert(
            "duration_seconds".to_owned(),
            float_value(self.duration_seconds),
        );
        object.insert("cost".to_owned(), decimal_string_value(&self.cost));
        object.insert(
            "replanning_attempts".to_owned(),
            counter_value(&self.replanning_attempts),
        );
        Value::Object(object)
    }

    /// Python `_merge_usage`: start from an optional base usage and add a
    /// delta's increments, re-validating each dimension exactly like the
    /// Python keyword-argument path.
    pub fn merge(base: Option<&Self>, delta: &UsageDelta) -> Result<Self, BudgetError> {
        let mut values = match base {
            Some(usage) => usage.clone(),
            None => Self::default(),
        };
        let increment = normalize_duration(delta.duration_seconds)?;
        values.duration_seconds = normalize_duration(values.duration_seconds + increment)?;
        let cost_increment = delta.cost.clone();
        validate_cost(&cost_increment)?;
        let cost = values.cost.add(&cost_increment)?;
        validate_cost(&cost)?;
        values.cost = cost;
        values.model_calls = values.model_calls.add(&delta.model_calls);
        values.tool_calls = values.tool_calls.add(&delta.tool_calls);
        values.total_tokens = values.total_tokens.add(&delta.total_tokens);
        values.artifact_bytes = values.artifact_bytes.add(&delta.artifact_bytes);
        values.replanning_attempts = values.replanning_attempts.add(&delta.replanning_attempts);
        Ok(values)
    }
}

/// Validate a decimal cost: finite (always true for [`Decimal`]) and
/// nonnegative, with signed zero allowed.
pub(crate) fn validate_cost(cost: &Decimal) -> Result<(), BudgetError> {
    if cost.is_negative() && !cost.is_zero() {
        return Err(BudgetError::Validation(
            "cost must be finite and nonnegative".to_owned(),
        ));
    }
    Ok(())
}

/// Keyword-style charge increments, mirroring `**kwargs` merged by
/// `_merge_usage`. Aliases `tokens`, `artifact_size` and `replanning` are
/// available as builder methods.
#[derive(Clone, Debug, Default)]
pub struct UsageDelta {
    /// Model calls to add.
    pub model_calls: Counter,
    /// Tool calls to add.
    pub tool_calls: Counter,
    /// Total tokens to add.
    pub total_tokens: Counter,
    /// Artifact bytes to add.
    pub artifact_bytes: Counter,
    /// Duration seconds to add (validated and grid-rounded on merge).
    pub duration_seconds: f64,
    /// Cost to add (validated on merge).
    pub cost: Decimal,
    /// Replanning attempts to add.
    pub replanning_attempts: Counter,
}

impl UsageDelta {
    /// An empty delta.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add model calls.
    #[must_use]
    pub fn model_calls(mut self, value: impl Into<Counter>) -> Self {
        self.model_calls = value.into();
        self
    }

    /// Add tool calls.
    #[must_use]
    pub fn tool_calls(mut self, value: impl Into<Counter>) -> Self {
        self.tool_calls = value.into();
        self
    }

    /// Add total tokens.
    #[must_use]
    pub fn total_tokens(mut self, value: impl Into<Counter>) -> Self {
        self.total_tokens = value.into();
        self
    }

    /// `total_tokens` alias matching the Python `tokens` keyword.
    #[must_use]
    pub fn tokens(self, value: impl Into<Counter>) -> Self {
        self.total_tokens(value)
    }

    /// Add artifact bytes.
    #[must_use]
    pub fn artifact_bytes(mut self, value: impl Into<Counter>) -> Self {
        self.artifact_bytes = value.into();
        self
    }

    /// `artifact_bytes` alias matching the Python `artifact_size` keyword.
    #[must_use]
    pub fn artifact_size(self, value: impl Into<Counter>) -> Self {
        self.artifact_bytes(value)
    }

    /// Add duration seconds.
    #[must_use]
    pub fn duration_seconds(mut self, value: f64) -> Self {
        self.duration_seconds = value;
        self
    }

    /// Add an exact decimal cost.
    #[must_use]
    pub fn cost(mut self, value: Decimal) -> Self {
        self.cost = value;
        self
    }

    /// Parse and add a decimal cost string.
    pub fn cost_str(mut self, value: &str) -> Result<Self, BudgetError> {
        self.cost = Decimal::parse(value)?;
        Ok(self)
    }

    /// Add replanning attempts.
    #[must_use]
    pub fn replanning_attempts(mut self, value: impl Into<Counter>) -> Self {
        self.replanning_attempts = value.into();
        self
    }

    /// `replanning_attempts` alias matching the Python `replanning` keyword.
    #[must_use]
    pub fn replanning(self, value: impl Into<Counter>) -> Self {
        self.replanning_attempts(value)
    }

    /// True when every dimension is zero (used by tests and wrappers).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.model_calls.is_zero()
            && self.tool_calls.is_zero()
            && self.total_tokens.is_zero()
            && self.artifact_bytes.is_zero()
            && self.duration_seconds == 0.0
            && self.cost.is_zero()
            && self.replanning_attempts.is_zero()
    }
}

/// Default arguments of `record_model_call`.
#[derive(Clone, Debug)]
pub struct ModelCallCharge {
    /// Number of model calls (default 1).
    pub count: Counter,
    /// Tokens consumed.
    pub tokens: Counter,
    /// Duration consumed.
    pub duration_seconds: f64,
    /// Cost consumed.
    pub cost: Decimal,
}

impl Default for ModelCallCharge {
    fn default() -> Self {
        Self {
            count: Counter::one(),
            tokens: Counter::zero(),
            duration_seconds: 0.0,
            cost: Decimal::zero(),
        }
    }
}

impl ModelCallCharge {
    /// A charge with the Python defaults (one model call, no tokens).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the call count.
    #[must_use]
    pub fn count(mut self, value: impl Into<Counter>) -> Self {
        self.count = value.into();
        self
    }

    /// Set consumed tokens.
    #[must_use]
    pub fn tokens(mut self, value: impl Into<Counter>) -> Self {
        self.tokens = value.into();
        self
    }

    /// Set consumed duration.
    #[must_use]
    pub fn duration_seconds(mut self, value: f64) -> Self {
        self.duration_seconds = value;
        self
    }

    /// Set consumed cost.
    #[must_use]
    pub fn cost(mut self, value: Decimal) -> Self {
        self.cost = value;
        self
    }

    /// Parse and set consumed cost.
    pub fn cost_str(mut self, value: &str) -> Result<Self, BudgetError> {
        self.cost = Decimal::parse(value)?;
        Ok(self)
    }
}

/// Default arguments of `record_tool_call`.
#[derive(Clone, Debug)]
pub struct ToolCallCharge {
    /// Number of tool calls (default 1).
    pub count: Counter,
    /// Duration consumed.
    pub duration_seconds: f64,
    /// Artifact bytes consumed.
    pub artifact_bytes: Counter,
    /// Cost consumed.
    pub cost: Decimal,
}

impl Default for ToolCallCharge {
    fn default() -> Self {
        Self {
            count: Counter::one(),
            duration_seconds: 0.0,
            artifact_bytes: Counter::zero(),
            cost: Decimal::zero(),
        }
    }
}

impl ToolCallCharge {
    /// A charge with the Python defaults (one tool call).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the call count.
    #[must_use]
    pub fn count(mut self, value: impl Into<Counter>) -> Self {
        self.count = value.into();
        self
    }

    /// Set consumed duration.
    #[must_use]
    pub fn duration_seconds(mut self, value: f64) -> Self {
        self.duration_seconds = value;
        self
    }

    /// Set consumed artifact bytes.
    #[must_use]
    pub fn artifact_bytes(mut self, value: impl Into<Counter>) -> Self {
        self.artifact_bytes = value.into();
        self
    }

    /// Set consumed cost.
    #[must_use]
    pub fn cost(mut self, value: Decimal) -> Self {
        self.cost = value;
        self
    }

    /// Parse and set consumed cost.
    pub fn cost_str(mut self, value: &str) -> Result<Self, BudgetError> {
        self.cost = Decimal::parse(value)?;
        Ok(self)
    }
}

/// A read-only view of `(budget, usage, reserved)` with a `remaining`
/// computation matching `BudgetSnapshot.remaining`.
#[derive(Clone, Debug, PartialEq)]
pub struct BudgetSnapshot {
    /// The configured budget.
    pub budget: RunBudget,
    /// Accumulated usage.
    pub usage: BudgetUsage,
    /// Outstanding reservations.
    pub reserved: BudgetUsage,
}

impl BudgetSnapshot {
    /// Construct a snapshot.
    #[must_use]
    pub fn new(budget: RunBudget, usage: BudgetUsage, reserved: BudgetUsage) -> Self {
        Self {
            budget,
            usage,
            reserved,
        }
    }

    /// Remaining budget, clamped at zero on every dimension.
    pub fn remaining(&self) -> Result<BudgetUsage, BudgetError> {
        let budget = &self.budget;
        let usage = &self.usage;
        let reserved = &self.reserved;
        Ok(BudgetUsage {
            model_calls: remaining_counter(
                budget.max_model_calls(),
                usage.model_calls(),
                reserved.model_calls(),
            ),
            tool_calls: remaining_counter(
                budget.max_tool_calls(),
                usage.tool_calls(),
                reserved.tool_calls(),
            ),
            total_tokens: remaining_counter(
                budget.max_total_tokens(),
                usage.total_tokens(),
                reserved.total_tokens(),
            ),
            artifact_bytes: remaining_counter(
                budget.max_artifact_bytes(),
                usage.artifact_bytes(),
                reserved.artifact_bytes(),
            ),
            duration_seconds: remaining_duration(
                budget.max_duration_seconds(),
                usage.duration_seconds(),
                reserved.duration_seconds(),
            )?,
            cost: remaining_cost(budget.max_cost(), usage.cost(), reserved.cost())?,
            replanning_attempts: remaining_counter(
                budget.max_replanning_attempts(),
                usage.replanning_attempts(),
                reserved.replanning_attempts(),
            ),
        })
    }
}

fn remaining_counter(max: &Counter, usage: &Counter, reserved: &Counter) -> Counter {
    let value = BigInt::from(max.as_biguint().clone())
        - BigInt::from(usage.as_biguint().clone())
        - BigInt::from(reserved.as_biguint().clone());
    if value <= BigInt::from(0u32) {
        Counter::zero()
    } else {
        Counter::from(value.magnitude().clone())
    }
}

fn remaining_duration(max: f64, usage: f64, reserved: f64) -> Result<f64, BudgetError> {
    let raw = max - usage - reserved;
    // Python `max(0, value)`; the result is passed through the usage
    // constructor which re-grids it and raises OverflowError when the
    // microsecond product leaves the float range.
    let clamped = if raw > 0.0 { raw } else { 0.0 };
    normalize_duration(clamped)
}

fn remaining_cost(
    max: &Decimal,
    usage: &Decimal,
    reserved: &Decimal,
) -> Result<Decimal, BudgetError> {
    let raw = max.sub(usage)?.sub(reserved)?;
    // Python `max(Decimal("0"), raw)` returns the literal zero unless `raw`
    // is strictly greater.
    if raw > Decimal::zero() {
        Ok(raw)
    } else {
        Ok(Decimal::zero())
    }
}

/// Build the exhaustion error details for a prospective charge.
pub(crate) fn exhaustion(
    dimension: BudgetDimension,
    requested: BudgetValue,
    remaining: BudgetValue,
    limit: BudgetValue,
) -> BudgetError {
    BudgetError::Exhausted(Box::new(ExhaustedBudget {
        dimension,
        requested,
        remaining,
        limit,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_keeps_signed_zero_and_arbitrary_counters() {
        let usage = BudgetUsage::new()
            .with_model_calls(Counter::parse("10000000000000000000000000000000000000000").unwrap())
            .try_with_cost_str("-0.00")
            .unwrap()
            .try_with_duration_seconds(-0.0)
            .unwrap();
        let dict = usage.as_dict();
        assert_eq!(dict["cost"], Value::String("-0.00".to_owned()));
        assert_eq!(dict["duration_seconds"], serde_json::json!(0.0));
        assert_eq!(
            dict["model_calls"].to_string().len(),
            41,
            "arbitrary counters are preserved"
        );
    }

    #[test]
    fn exceeds_matches_the_python_direction() {
        let small = BudgetUsage::new().with_tool_calls(1u32);
        let large = BudgetUsage::new().with_tool_calls(2u32);
        assert!(small.exceeds(&large));
        assert!(!large.exceeds(&small));
    }

    #[test]
    fn subtraction_rejects_negative_dimensions() {
        let error = BudgetUsage::new()
            .with_tool_calls(1u32)
            .sub(&BudgetUsage::new().with_tool_calls(2u32))
            .unwrap_err();
        assert_eq!(error, BudgetError::CounterUnderflow);
    }

    #[test]
    fn merge_adds_deltas_to_a_base_usage() {
        let base = BudgetUsage::new().with_total_tokens(5u32);
        let delta = UsageDelta::new()
            .total_tokens(3u32)
            .duration_seconds(0.5)
            .cost_str("0.25")
            .unwrap();
        let merged = BudgetUsage::merge(Some(&base), &delta).unwrap();
        assert_eq!(merged.total_tokens(), &Counter::from(8u32));
        assert_eq!(merged.duration_seconds(), 0.5);
        assert_eq!(merged.cost().to_python_string(), "0.25");
    }

    #[test]
    fn merge_rejects_negative_or_non_finite_inputs() {
        assert!(BudgetUsage::merge(None, &UsageDelta::new().duration_seconds(-0.5)).is_err());
        assert!(BudgetUsage::merge(None, &UsageDelta::new().duration_seconds(f64::NAN)).is_err());
        assert!(
            BudgetUsage::merge(None, &UsageDelta::new().cost(Decimal::parse("-1").unwrap()))
                .is_err()
        );
    }

    #[test]
    fn decimal_overflow_propagates_from_cost_arithmetic() {
        let left = BudgetUsage::new().try_with_cost_str("9e999999").unwrap();
        let right = BudgetUsage::new().try_with_cost_str("9e999999").unwrap();
        let error = left.add(&right).unwrap_err();
        assert_eq!(error.contract_error_name(), "Overflow");
        assert!(matches!(
            error,
            BudgetError::Decimal(crate::decimal::DecimalError::Overflow)
        ));
        assert_eq!(
            left.add(&right).unwrap_err(),
            BudgetError::Decimal(crate::decimal::DecimalError::Overflow)
        );
    }

    #[test]
    fn remaining_propagates_duration_overflow_like_python() {
        let budget = RunBudget::new()
            .try_with_max_duration_seconds(1e303)
            .unwrap();
        let snapshot = BudgetSnapshot::new(budget, BudgetUsage::new(), BudgetUsage::new());
        assert_eq!(
            snapshot.remaining().unwrap_err(),
            BudgetError::DurationRange
        );
    }
}
