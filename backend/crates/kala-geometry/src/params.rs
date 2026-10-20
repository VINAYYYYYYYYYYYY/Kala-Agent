//! JSON/serde parameter adapter that preserves Python's number-kind rules.
//!
//! Python distinguishes `bool`, `int`, and `float` at validation time:
//! booleans are always refused, integer JSON numbers are accepted for
//! tessellation counts, and floating JSON numbers are refused even when they
//! are integral (`2.0` is not an integer count). Parsing straight into `f64`
//! would silently erase that distinction, so this module keeps the three JSON
//! number kinds apart until the geometry validator consumes them.
//!
//! The adapter accepts a serde-deserialised JSON object through
//! [`PrimitiveParameters`] or a [`serde_json::Value`] through
//! [`PrimitiveParameters::from_json_value`]. It is an input adapter only: the
//! crate never serialises user parameters itself.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::{Deserializer, Error as DeError};
use serde_json::Value;

use crate::GeometryError;
use crate::limits::validate_finite_positive;

/// One JSON parameter value with the Python-relevant type distinction kept.
#[derive(Debug, Clone, PartialEq)]
pub enum ParameterValue {
    /// A JSON boolean. Python refuses booleans at every numeric validator.
    Bool(bool),
    /// A JSON integer that fits in `i128`.
    Integer(i128),
    /// A JSON integer too wide for `i128`, kept as its source decimal text.
    BigInteger(String),
    /// A JSON floating-point number.
    Float(f64),
    /// A JSON string.
    String(String),
    /// JSON `null`.
    Null,
    /// A JSON array.
    Sequence,
    /// A JSON object.
    Object,
}

impl ParameterValue {
    /// Convert a `serde_json` value while preserving integer/float/bool kind.
    #[must_use]
    pub fn from_json(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(value) => Self::Bool(*value),
            Value::Number(number) => {
                let text = number.to_string();
                if text.bytes().any(|byte| matches!(byte, b'.' | b'e' | b'E')) {
                    // JSON numbers are finite by grammar. A failed conversion
                    // can only mean an implementation limit, which is treated
                    // as non-finite by the numeric validators below.
                    Self::Float(number.as_f64().unwrap_or(f64::INFINITY))
                } else if let Ok(integer) = text.parse::<i128>() {
                    Self::Integer(integer)
                } else {
                    Self::BigInteger(text)
                }
            }
            Value::String(text) => Self::String(text.clone()),
            Value::Array(_) => Self::Sequence,
            Value::Object(_) => Self::Object,
        }
    }

    /// Validate as a finite positive millimetre dimension.
    ///
    /// # Errors
    ///
    /// Returns the Python validator message for non-numbers, booleans,
    /// non-finite values, and values outside the dimension bounds.
    pub fn as_dimension(&self, name: &str) -> Result<f64, GeometryError> {
        match self {
            Self::Integer(integer) => {
                let candidate = *integer as f64;
                validate_finite_positive(name, candidate)
            }
            Self::BigInteger(text) => {
                // Integers wider than i128 can still convert to a finite f64
                // (up to ~1e308); otherwise the finite check refuses them.
                let candidate = text.parse::<f64>().unwrap_or(f64::INFINITY);
                validate_finite_positive(name, candidate)
            }
            Self::Float(value) => validate_finite_positive(name, *value),
            Self::Bool(_) | Self::String(_) | Self::Null | Self::Sequence | Self::Object => Err(
                GeometryError::new(format!("{name} must be a finite number")),
            ),
        }
    }

    /// Validate as an integer count in an inclusive `u32` range.
    ///
    /// # Errors
    ///
    /// Returns the Python "must be an integer" message for booleans, floats,
    /// strings, nulls, arrays, and objects, or the inclusive range message for
    /// integers outside the bounds.
    pub fn as_count(&self, name: &str, minimum: u32, maximum: u32) -> Result<u32, GeometryError> {
        match self {
            Self::Integer(integer) => {
                crate::limits::validate_int_in_range(name, *integer, minimum, maximum)
            }
            // A value that did not fit in i128 is far outside every supported
            // count range, but it is still an integer as far as Python's type
            // check is concerned.
            Self::BigInteger(_) => Err(GeometryError::new(format!(
                "{name} must be between {minimum} and {maximum}"
            ))),
            Self::Bool(_)
            | Self::Float(_)
            | Self::String(_)
            | Self::Null
            | Self::Sequence
            | Self::Object => Err(GeometryError::new(format!("{name} must be an integer"))),
        }
    }
}

impl From<bool> for ParameterValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i32> for ParameterValue {
    fn from(value: i32) -> Self {
        Self::Integer(i128::from(value))
    }
}

impl From<u32> for ParameterValue {
    fn from(value: u32) -> Self {
        Self::Integer(i128::from(value))
    }
}

impl From<i64> for ParameterValue {
    fn from(value: i64) -> Self {
        Self::Integer(i128::from(value))
    }
}

impl From<u64> for ParameterValue {
    fn from(value: u64) -> Self {
        Self::Integer(i128::from(value))
    }
}

impl From<f64> for ParameterValue {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<&str> for ParameterValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for ParameterValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

/// A complete primitive parameter mapping keyed in deterministic order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PrimitiveParameters {
    entries: BTreeMap<String, ParameterValue>,
}

impl PrimitiveParameters {
    /// Create an empty mapping (all defaults apply).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from a deserialised JSON value.
    ///
    /// `null` means "no parameters", matching Python's
    /// `dict(parameters or {})`. Any other non-object value is rejected with a
    /// sanitized adapter message; the studio schema rejects those shapes
    /// before geometry in the Python service.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError`] when `value` is neither `null` nor an object.
    pub fn from_json_value(value: &Value) -> Result<Self, GeometryError> {
        if value.is_null() {
            return Ok(Self::default());
        }
        let object = value
            .as_object()
            .ok_or_else(|| GeometryError::new("parameters must be a JSON object"))?;
        let mut entries = BTreeMap::new();
        for (name, value) in object {
            entries.insert(name.clone(), ParameterValue::from_json(value));
        }
        Ok(Self { entries })
    }

    /// Build from a JSON string containing an object or `null`.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError`] for invalid JSON or a non-object top level.
    pub fn from_json_str(text: &str) -> Result<Self, GeometryError> {
        let value: Value = serde_json::from_str(text)
            .map_err(|error| GeometryError::new(format!("invalid parameter JSON: {error}")))?;
        Self::from_json_value(&value)
    }

    /// Look up one parameter.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ParameterValue> {
        self.entries.get(name)
    }

    /// Return whether a parameter is present.
    #[must_use]
    pub fn contains_key(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Insert or replace one parameter.
    pub fn insert(
        &mut self,
        name: impl Into<String>,
        value: impl Into<ParameterValue>,
    ) -> Option<ParameterValue> {
        self.entries.insert(name.into(), value.into())
    }

    /// Iterate over `(name, value)` pairs in deterministic key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ParameterValue)> {
        self.entries
            .iter()
            .map(|(name, value)| (name.as_str(), value))
    }

    /// Number of supplied parameters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return whether no parameters were supplied.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return supplied names that are not in `allowed`.
    #[must_use]
    pub fn unknown_keys(&self, allowed: &[&str]) -> Vec<&str> {
        self.entries
            .keys()
            .map(String::as_str)
            .filter(|name| !allowed.contains(name))
            .collect()
    }
}

impl<'de> Deserialize<'de> for PrimitiveParameters {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Self::from_json_value(&value).map_err(D::Error::custom)
    }
}

impl FromIterator<(String, ParameterValue)> for PrimitiveParameters {
    fn from_iter<T: IntoIterator<Item = (String, ParameterValue)>>(iter: T) -> Self {
        Self {
            entries: iter.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_integer_float_and_bool_stay_distinct() {
        let params = PrimitiveParameters::from_json_str(
            r#"{"i": 2, "f": 2.0, "b": true, "big": 100000000000000000000000000000000000000000}"#,
        )
        .unwrap();
        assert_eq!(params.get("i"), Some(&ParameterValue::Integer(2)));
        assert_eq!(params.get("f"), Some(&ParameterValue::Float(2.0)));
        assert_eq!(params.get("b"), Some(&ParameterValue::Bool(true)));
        assert!(matches!(
            params.get("big"),
            Some(ParameterValue::BigInteger(_))
        ));

        assert_eq!(
            params
                .get("i")
                .unwrap()
                .as_count("subdivisions", 1, 64)
                .unwrap(),
            2
        );
        assert_eq!(
            params
                .get("f")
                .unwrap()
                .as_count("subdivisions", 1, 64)
                .unwrap_err()
                .message(),
            "subdivisions must be an integer"
        );
        assert_eq!(
            params
                .get("b")
                .unwrap()
                .as_dimension("size_x")
                .unwrap_err()
                .message(),
            "size_x must be a finite number"
        );
    }

    #[test]
    fn serde_deserialize_preserves_kinds() {
        let params: PrimitiveParameters =
            serde_json::from_str(r#"{"subdivisions": 2, "size_x": 10.0}"#).unwrap();
        assert_eq!(
            params.get("subdivisions"),
            Some(&ParameterValue::Integer(2))
        );
        assert_eq!(params.get("size_x"), Some(&ParameterValue::Float(10.0)));
    }

    #[test]
    fn null_means_no_parameters() {
        assert!(
            PrimitiveParameters::from_json_value(&Value::Null)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            PrimitiveParameters::from_json_value(&Value::Array(Vec::new()))
                .unwrap_err()
                .message(),
            "parameters must be a JSON object"
        );
    }
}
