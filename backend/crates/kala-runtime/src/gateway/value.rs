//! Bounded field validation and JSON freezing.
//!
//! The Python module validates every request, message, artifact and response
//! field in the dataclass `__post_init__` method. The Rust port moves the
//! same checks into constructors and `with_*` methods, so every mutation
//! re-validates exactly like `dataclasses.replace` does in Python.

use std::cmp::Ordering;

use kala_budget::{Decimal, DecimalParseError};
use kala_json::CanonicalValue;

use super::decimal::DecimalArithmeticError;
use super::error::ModelError;
use super::text::{code_point_len, python_strip};

/// The maximum nesting depth a structured output value may use.
pub const MAX_STRUCTURED_OUTPUT_DEPTH: usize = 32;

/// The identifier shape every bounded identifier must have.
fn is_safe_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    let mut count = 1usize;
    for character in characters {
        if count > 128 {
            return false;
        }
        if !(character.is_ascii_alphanumeric()
            || character == '.'
            || character == '_'
            || character == ':'
            || character == '-')
        {
            return false;
        }
        count += 1;
    }
    count <= 128
}

/// Validate and normalize a bounded identifier.
///
/// The Python implementation accepts any object and rejects a non-string with
/// `"{field} must be a string"`; the Rust port takes a `&str`, so the type
/// check is a compile-time property.
pub fn bounded_identifier(value: &str, field_name: &str) -> Result<String, ModelError> {
    let normalized = python_strip(value);
    if !is_safe_identifier(normalized) {
        return Err(ModelError::request(format!(
            "{field_name} must be a safe identifier"
        )));
    }
    Ok(normalized.to_owned())
}

/// Validate and bound a text field.
///
/// The length bound is counted in Unicode code points, like CPython's `len()`.
pub fn bounded_text(value: &str, field_name: &str, maximum: usize) -> Result<String, ModelError> {
    if value.contains('\u{0}') {
        return Err(ModelError::request(format!(
            "{field_name} cannot contain null bytes"
        )));
    }
    if code_point_len(value) > maximum {
        return Err(ModelError::request(format!(
            "{field_name} exceeds the bounded length"
        )));
    }
    Ok(value.to_owned())
}

/// Validate a nonnegative integer field.
pub fn nonnegative_int(value: i128, field_name: &str) -> Result<i128, ModelError> {
    if value < 0 {
        return Err(ModelError::request(format!(
            "{field_name} must be nonnegative"
        )));
    }
    Ok(value)
}

/// Validate a finite number inside an inclusive range.
pub fn bounded_float(
    value: f64,
    field_name: &str,
    minimum: f64,
    maximum: f64,
) -> Result<f64, ModelError> {
    if !value.is_finite() || !(minimum <= value && value <= maximum) {
        return Err(ModelError::request(format!(
            "{field_name} must be finite and within its bounds"
        )));
    }
    Ok(value)
}

/// A decimal input, mirroring the Python values `_nonnegative_decimal`
/// accepts.
#[derive(Clone, Debug)]
pub enum DecimalInput {
    /// An already built decimal.
    Decimal(Decimal),
    /// A decimal literal.
    Text(String),
    /// A float, which CPython converts through `str(value)`.
    Float(f64),
    /// An integer.
    Integer(i128),
}

/// Validate a nonnegative decimal cost.
///
/// The four Python messages are preserved exactly: a value that is not a
/// number at all is `must be numeric`, an unparsable literal is `must be a
/// nonnegative decimal`, and a negative or non-finite value is `must be a
/// finite nonnegative decimal`.
pub fn nonnegative_decimal(value: &DecimalInput, field_name: &str) -> Result<Decimal, ModelError> {
    let parsed = match value {
        DecimalInput::Decimal(value) => value.clone(),
        DecimalInput::Text(text) => match Decimal::parse(text) {
            Ok(value) => value,
            Err(DecimalParseError::NonFinite) => {
                return Err(ModelError::request(format!(
                    "{field_name} must be a finite nonnegative decimal"
                )));
            }
            Err(_) => {
                return Err(ModelError::request(format!(
                    "{field_name} must be a nonnegative decimal"
                )));
            }
        },
        DecimalInput::Float(value) => {
            if !value.is_finite() {
                return Err(ModelError::request(format!(
                    "{field_name} must be a finite nonnegative decimal"
                )));
            }
            Decimal::from_python_float(*value).map_err(|_| {
                ModelError::request(format!("{field_name} must be a nonnegative decimal"))
            })?
        }
        DecimalInput::Integer(value) => Decimal::parse(&value.to_string()).map_err(|_| {
            ModelError::request(format!("{field_name} must be a nonnegative decimal"))
        })?,
    };
    if parsed.compare(&Decimal::zero()) == Ordering::Less {
        return Err(ModelError::request(format!(
            "{field_name} must be a finite nonnegative decimal"
        )));
    }
    Ok(parsed)
}

/// The deepest node of a JSON value, counted from the value itself.
///
/// The traversal is iterative, so a hostile document cannot overflow the call
/// stack.
#[must_use]
pub fn json_depth(value: &CanonicalValue) -> usize {
    let mut stack: Vec<(&CanonicalValue, usize)> = vec![(value, 0)];
    let mut deepest = 0usize;
    while let Some((current, depth)) = stack.pop() {
        if depth > deepest {
            deepest = depth;
        }
        match current {
            CanonicalValue::Array(items) => {
                for item in items {
                    stack.push((item, depth + 1));
                }
            }
            CanonicalValue::Object(entries) => {
                for item in entries.values() {
                    stack.push((item, depth + 1));
                }
            }
            _ => {}
        }
    }
    deepest
}

/// Freeze a JSON value, refusing a document that nests too deeply.
///
/// The Python implementation returns an immutable copy and refuses a nesting
/// deeper than 32 levels. The Rust value is already immutable, so the port
/// only performs the depth check.
pub fn freeze_json(value: &CanonicalValue) -> Result<CanonicalValue, ModelError> {
    if json_depth(value) > MAX_STRUCTURED_OUTPUT_DEPTH {
        return Err(ModelError::request(
            "structured output nesting exceeds the supported depth",
        ));
    }
    Ok(value.clone())
}

/// Thaw a frozen JSON value.
///
/// The Python implementation converts the immutable mapping and tuple
/// containers back into plain dictionaries and lists. The canonical JSON
/// encoding of both shapes is identical, so the Rust port is the identity.
#[must_use]
pub fn thaw_json(value: &CanonicalValue) -> CanonicalValue {
    value.clone()
}

/// Map a decimal arithmetic failure onto the provider error the Python
/// implementation would have raised as an uncaught `decimal` exception.
pub fn decimal_failure(error: DecimalArithmeticError) -> ModelError {
    ModelError::provider(error.python_class_name())
}
