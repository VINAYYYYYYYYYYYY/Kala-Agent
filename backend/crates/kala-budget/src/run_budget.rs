//! `RunBudget`: the seven configured limits with Python's defaults and
//! validation rules.

use crate::counter::Counter;
use crate::decimal::Decimal;
use crate::duration::python_float_parse;
use crate::errors::BudgetError;
use crate::json::{
    counter_from_integer_text, counter_value, decimal_from_integer_text, decimal_string_value,
    float_value, json_integer_text, number_text,
};
use crate::usage::validate_cost;
use num_traits::ToPrimitive;
use serde_json::{Map, Value};

const ALLOWED_KEYS: [&str; 7] = [
    "max_model_calls",
    "max_tool_calls",
    "max_total_tokens",
    "max_artifact_bytes",
    "max_duration_seconds",
    "max_cost",
    "max_replanning_attempts",
];

/// Budget limits mirroring `kala_agent.contracts.RunBudget`.
#[derive(Clone, Debug)]
pub struct RunBudget {
    max_model_calls: Counter,
    max_tool_calls: Counter,
    max_total_tokens: Counter,
    max_artifact_bytes: Counter,
    max_duration_seconds: f64,
    max_cost: Decimal,
    max_replanning_attempts: Counter,
}

impl Default for RunBudget {
    fn default() -> Self {
        Self {
            max_model_calls: Counter::from(32u32),
            max_tool_calls: Counter::from(64u32),
            max_total_tokens: Counter::from(100_000u32),
            max_artifact_bytes: Counter::from(1_000_000_000u32),
            max_duration_seconds: 3_600.0,
            max_cost: Decimal::zero(),
            max_replanning_attempts: Counter::from(3u32),
        }
    }
}

impl PartialEq for RunBudget {
    fn eq(&self, other: &Self) -> bool {
        self.max_model_calls == other.max_model_calls
            && self.max_tool_calls == other.max_tool_calls
            && self.max_total_tokens == other.max_total_tokens
            && self.max_artifact_bytes == other.max_artifact_bytes
            && self.max_duration_seconds == other.max_duration_seconds
            && self.max_cost == other.max_cost
            && self.max_replanning_attempts == other.max_replanning_attempts
    }
}

impl Eq for RunBudget {}

impl RunBudget {
    /// The Python default budget.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum model calls.
    #[must_use]
    pub fn with_max_model_calls(mut self, value: impl Into<Counter>) -> Self {
        self.max_model_calls = value.into();
        self
    }

    /// Set the maximum tool calls.
    #[must_use]
    pub fn with_max_tool_calls(mut self, value: impl Into<Counter>) -> Self {
        self.max_tool_calls = value.into();
        self
    }

    /// Set the maximum total tokens.
    #[must_use]
    pub fn with_max_total_tokens(mut self, value: impl Into<Counter>) -> Self {
        self.max_total_tokens = value.into();
        self
    }

    /// Set the maximum artifact bytes.
    #[must_use]
    pub fn with_max_artifact_bytes(mut self, value: impl Into<Counter>) -> Self {
        self.max_artifact_bytes = value.into();
        self
    }

    /// Set the maximum replanning attempts.
    #[must_use]
    pub fn with_max_replanning_attempts(mut self, value: impl Into<Counter>) -> Self {
        self.max_replanning_attempts = value.into();
        self
    }

    /// Set the maximum duration, rejecting negative or non-finite values.
    /// Negative zero is accepted and preserved, exactly like the Python
    /// `FiniteNonNegativeFloat` validator.
    pub fn try_with_max_duration_seconds(mut self, value: f64) -> Result<Self, BudgetError> {
        if !value.is_finite() || value < 0.0 {
            return Err(BudgetError::Validation(
                "max_duration_seconds must be finite and nonnegative".to_owned(),
            ));
        }
        self.max_duration_seconds = value;
        Ok(self)
    }

    /// Set the maximum cost, rejecting negative values (signed zero allowed).
    pub fn try_with_max_cost(mut self, value: Decimal) -> Result<Self, BudgetError> {
        validate_cost(&value)?;
        self.max_cost = value;
        Ok(self)
    }

    /// Parse and set the maximum cost.
    pub fn try_with_max_cost_str(self, value: &str) -> Result<Self, BudgetError> {
        self.try_with_max_cost(Decimal::parse(value)?)
    }

    /// Maximum model calls.
    #[must_use]
    pub const fn max_model_calls(&self) -> &Counter {
        &self.max_model_calls
    }

    /// Maximum tool calls.
    #[must_use]
    pub const fn max_tool_calls(&self) -> &Counter {
        &self.max_tool_calls
    }

    /// Maximum total tokens.
    #[must_use]
    pub const fn max_total_tokens(&self) -> &Counter {
        &self.max_total_tokens
    }

    /// Maximum artifact bytes.
    #[must_use]
    pub const fn max_artifact_bytes(&self) -> &Counter {
        &self.max_artifact_bytes
    }

    /// Maximum duration seconds.
    #[must_use]
    pub const fn max_duration_seconds(&self) -> f64 {
        self.max_duration_seconds
    }

    /// Maximum decimal cost.
    #[must_use]
    pub const fn max_cost(&self) -> &Decimal {
        &self.max_cost
    }

    /// Maximum replanning attempts.
    #[must_use]
    pub const fn max_replanning_attempts(&self) -> &Counter {
        &self.max_replanning_attempts
    }

    /// `RunBudget.model_dump(mode="json")` representation.
    #[must_use]
    pub fn as_dict(&self) -> Value {
        let mut object = Map::new();
        object.insert(
            "max_model_calls".to_owned(),
            counter_value(&self.max_model_calls),
        );
        object.insert(
            "max_tool_calls".to_owned(),
            counter_value(&self.max_tool_calls),
        );
        object.insert(
            "max_total_tokens".to_owned(),
            counter_value(&self.max_total_tokens),
        );
        object.insert(
            "max_artifact_bytes".to_owned(),
            counter_value(&self.max_artifact_bytes),
        );
        object.insert(
            "max_duration_seconds".to_owned(),
            float_value(self.max_duration_seconds),
        );
        object.insert("max_cost".to_owned(), decimal_string_value(&self.max_cost));
        object.insert(
            "max_replanning_attempts".to_owned(),
            counter_value(&self.max_replanning_attempts),
        );
        Value::Object(object)
    }

    /// Validate a JSON object exactly like `RunBudget.model_validate` during
    /// state import: unknown keys are rejected, missing keys fall back to the
    /// Python defaults and each value must keep its Python-visible type.
    pub fn from_json(value: &Value) -> Result<Self, BudgetError> {
        let object = value
            .as_object()
            .ok_or_else(|| BudgetError::Validation("max budget must be a mapping".to_owned()))?;
        let mut unexpected: Vec<&str> = object
            .keys()
            .map(String::as_str)
            .filter(|key| !ALLOWED_KEYS.contains(key))
            .collect();
        unexpected.sort_unstable();
        if !unexpected.is_empty() {
            return Err(BudgetError::Validation(format!(
                "budget has unexpected keys: {}",
                unexpected.join(", ")
            )));
        }

        let mut budget = Self::default();
        if let Some(field) = object.get("max_model_calls") {
            budget.max_model_calls = counter_field(field, "max_model_calls")?;
        }
        if let Some(field) = object.get("max_tool_calls") {
            budget.max_tool_calls = counter_field(field, "max_tool_calls")?;
        }
        if let Some(field) = object.get("max_total_tokens") {
            budget.max_total_tokens = counter_field(field, "max_total_tokens")?;
        }
        if let Some(field) = object.get("max_artifact_bytes") {
            budget.max_artifact_bytes = counter_field(field, "max_artifact_bytes")?;
        }
        if let Some(field) = object.get("max_duration_seconds") {
            budget.max_duration_seconds = duration_field(field)?;
        }
        if let Some(field) = object.get("max_cost") {
            budget.max_cost = cost_field(field)?;
        }
        if let Some(field) = object.get("max_replanning_attempts") {
            budget.max_replanning_attempts = counter_field(field, "max_replanning_attempts")?;
        }
        Ok(budget)
    }
}

fn counter_field(value: &Value, field: &str) -> Result<Counter, BudgetError> {
    let text = json_integer_text(value)
        .ok_or_else(|| BudgetError::Validation(format!("{field} must be a nonnegative integer")))?;
    counter_from_integer_text(text).map_err(|error| {
        BudgetError::Validation(format!("{field} must be a nonnegative integer: {error}"))
    })
}

fn duration_field(value: &Value) -> Result<f64, BudgetError> {
    let parsed = match value {
        Value::String(text) => {
            python_float_parse(text).map_err(|_| nonnegative_float_error("max_duration_seconds"))?
        }
        Value::Number(_) => {
            let text = number_text(value)
                .ok_or_else(|| nonnegative_float_error("max_duration_seconds"))?;
            if crate::json::is_integer_text(text) {
                let integer = num_bigint::BigInt::parse_bytes(text.as_bytes(), 10)
                    .ok_or_else(|| nonnegative_float_error("max_duration_seconds"))?;
                integer
                    .to_f64()
                    .ok_or_else(|| nonnegative_float_error("max_duration_seconds"))?
            } else {
                text.parse::<f64>()
                    .map_err(|_| nonnegative_float_error("max_duration_seconds"))?
            }
        }
        _ => return Err(nonnegative_float_error("max_duration_seconds")),
    };
    if !parsed.is_finite() || parsed < 0.0 {
        return Err(nonnegative_float_error("max_duration_seconds"));
    }
    Ok(parsed)
}

fn cost_field(value: &Value) -> Result<Decimal, BudgetError> {
    let parsed = match value {
        Value::String(text) => Decimal::parse(text)?,
        Value::Number(_) => {
            let text = number_text(value)
                .ok_or_else(|| BudgetError::Validation("max_cost must be numeric".to_owned()))?;
            if crate::json::is_integer_text(text) {
                decimal_from_integer_text(text)?
            } else {
                let float = text
                    .parse::<f64>()
                    .map_err(|_| BudgetError::Validation("max_cost must be numeric".to_owned()))?;
                if !float.is_finite() {
                    return Err(BudgetError::Validation(
                        "max_cost must be finite and nonnegative".to_owned(),
                    ));
                }
                Decimal::from_python_float(float)?
            }
        }
        _ => {
            return Err(BudgetError::Validation(
                "max_cost must be numeric".to_owned(),
            ));
        }
    };
    if parsed.is_negative() && !parsed.is_zero() {
        return Err(BudgetError::Validation(
            "max_cost must be finite and nonnegative".to_owned(),
        ));
    }
    Ok(parsed)
}

fn nonnegative_float_error(field: &str) -> BudgetError {
    BudgetError::Validation(format!("{field} must be finite and nonnegative"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_python_model() {
        let budget = RunBudget::new();
        assert_eq!(budget.max_model_calls(), &Counter::from(32u32));
        assert_eq!(budget.max_tool_calls(), &Counter::from(64u32));
        assert_eq!(budget.max_total_tokens(), &Counter::from(100_000u32));
        assert_eq!(
            budget.max_artifact_bytes(),
            &Counter::from(1_000_000_000u32)
        );
        assert_eq!(budget.max_duration_seconds(), 3600.0);
        assert_eq!(budget.max_cost().to_python_string(), "0");
        assert_eq!(budget.max_replanning_attempts(), &Counter::from(3u32));
    }

    #[test]
    fn json_round_trip_preserves_types() {
        let budget = RunBudget::new()
            .try_with_max_cost_str("1.00")
            .unwrap()
            .try_with_max_duration_seconds(10.0)
            .unwrap();
        let dict = budget.as_dict();
        assert_eq!(dict["max_cost"], Value::String("1.00".to_owned()));
        assert_eq!(dict["max_duration_seconds"], serde_json::json!(10.0));
        assert_eq!(budget, RunBudget::from_json(&dict).unwrap());
    }

    #[test]
    fn rejects_python_invalid_values() {
        assert!(RunBudget::new().try_with_max_cost_str("-1").is_err());
        assert!(RunBudget::new().try_with_max_cost_str("NaN").is_err());
        assert!(
            RunBudget::new()
                .try_with_max_duration_seconds(-1.0)
                .is_err()
        );
        assert_eq!(
            RunBudget::new()
                .try_with_max_duration_seconds(-0.0)
                .unwrap()
                .max_duration_seconds(),
            -0.0
        );
        let float_integer = serde_json::json!({"max_model_calls": 1.0});
        assert!(RunBudget::from_json(&float_integer).is_err());
        let negative = serde_json::json!({"max_tool_calls": -1});
        assert!(RunBudget::from_json(&negative).is_err());
    }

    #[test]
    fn accepts_unicode_digits_and_rejects_lookalikes_and_extreme_exponents() {
        // Repair round 2: valid Unicode `Nd` inputs and `Decimal`-only
        // whitespace are accepted on the JSON entry path.
        let unicode = RunBudget::from_json(&serde_json::json!({
            "max_cost": "\u{0661}.\u{0662}\u{0663}",
            "max_duration_seconds": "\u{0661}\u{0662}.\u{0663}",
        }))
        .expect("unicode inputs are valid");
        assert_eq!(unicode.max_cost().to_python_string(), "1.23");
        assert_eq!(unicode.max_duration_seconds(), 12.3);
        let trimmed = RunBudget::from_json(&serde_json::json!({
            "max_cost": "\u{1C}1\u{1C}",
        }))
        .expect("Decimal-only whitespace is valid");
        assert_eq!(trimmed.max_cost().to_python_string(), "1");

        // Non-`Nd` numeric lookalikes, float-only storage gaps and CPython's
        // construction exponent bounds stay rejected.
        for invalid in [
            serde_json::json!({"max_cost": "\u{00B2}"}),
            serde_json::json!({"max_cost": "\u{00BD}"}),
            serde_json::json!({"max_cost": "\u{2160}"}),
            serde_json::json!({"max_cost": "1e-1999999999999999998"}),
            serde_json::json!({"max_cost": "1e1000000000000000000"}),
            serde_json::json!({"max_cost": "99e999999999999999999"}),
            serde_json::json!({"max_duration_seconds": "\u{1C}1\u{1C}"}),
            serde_json::json!({"max_duration_seconds": "1__0"}),
        ] {
            assert!(RunBudget::from_json(&invalid).is_err(), "{invalid}");
        }

        let boundary = RunBudget::from_json(&serde_json::json!({
            "max_cost": "1e-1999999999999999997",
        }))
        .expect("MIN_ETINY boundary");
        assert_eq!(
            boundary.max_cost().to_python_string(),
            "1E-1999999999999999997"
        );
    }
}
