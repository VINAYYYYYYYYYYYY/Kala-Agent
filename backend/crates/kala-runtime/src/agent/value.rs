//! The bounded field validators and the JSON freeze of the agent runtime.
//!
//! The Python module validates every task, catalog step, outcome and run
//! result field in a `__post_init__` method, with a fixed order of checks.
//! The Rust port moves the same checks into constructors and `with_*`
//! methods, so every mutation re-validates exactly like
//! `dataclasses.replace` does in Python.
//!
//! Two Python checks are type checks on an arbitrary `object` and are
//! therefore compile-time properties in Rust:
//!
//! * `"{field} must be a string"` — the value is a `&str` in Rust;
//! * `"{field} must be a number"` — the value is an `Option<f64>` in Rust.
//!
//! Every *value* check is reproduced exactly, including the message text.

use kala_json::{CanonicalObject, CanonicalValue};

use crate::gateway::{code_point_len, python_strip};

use super::error::AgentError;

/// The maximum length of a bounded text, in Unicode code points.
pub const MAX_BOUNDED_TEXT: usize = 512;

/// The maximum length of an error text, in Unicode code points.
pub const MAX_ERROR_TEXT: usize = 1_024;

/// The deepest nesting a frozen task argument may use.
///
/// The Python module refuses a value that nests deeper than 16 levels below
/// the value it was handed.
pub const MAX_ARGUMENT_DEPTH: usize = 16;

/// The maximum length of a safe identifier, in characters.
const MAX_IDENTIFIER_LENGTH: usize = 128;

/// Validate and normalize a bounded identifier.
///
/// The identifier is stripped with CPython's `str.strip()` and must then
/// match `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$` in full, exactly like the
/// Python regular expression.
pub fn bounded_identifier(value: &str, field_name: &str) -> Result<String, AgentError> {
    let normalized = python_strip(value);
    if !is_safe_identifier(normalized) {
        return Err(AgentError::plan(format!(
            "{field_name} must be a safe identifier"
        )));
    }
    Ok(normalized.to_owned())
}

/// Whether a text is a safe identifier.
///
/// The check is the byte-exact equivalent of the Python regular expression
/// `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$` under `fullmatch`.
fn is_safe_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_IDENTIFIER_LENGTH || !bytes[0].is_ascii_alphanumeric()
    {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

/// Validate and bound a text field.
///
/// The length bound is counted in Unicode code points, like CPython's `len()`.
pub fn bounded_text(value: &str, field_name: &str, maximum: usize) -> Result<String, AgentError> {
    if code_point_len(value) > maximum {
        return Err(AgentError::plan(format!(
            "{field_name} exceeds the bounded length"
        )));
    }
    Ok(value.to_owned())
}

/// Validate a bounded number of seconds.
///
/// `None` is accepted and returned unchanged. A value that is not strictly
/// positive, or that exceeds 3,600 seconds, is refused. A NaN is refused as
/// well, because the Python comparison `0.0 < parsed` is false for it.
pub fn bounded_seconds(value: Option<f64>, field_name: &str) -> Result<Option<f64>, AgentError> {
    let Some(parsed) = value else {
        return Ok(None);
    };
    if !(parsed > 0.0 && parsed <= 3_600.0) {
        return Err(AgentError::plan(format!(
            "{field_name} must be positive and bounded"
        )));
    }
    Ok(Some(parsed))
}

/// Normalize a collection of identifiers.
///
/// Every entry is validated first, then the collection is sorted and
/// deduplicated, exactly like the Python set comprehension followed by
/// `sorted`.
pub fn identifier_tuple(values: &[String], field_name: &str) -> Result<Vec<String>, AgentError> {
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        normalized.push(bounded_identifier(value, field_name)?);
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized.to_owned())
}

/// Validate a collection of bounded reasons.
///
/// The order of the collection is preserved and the entries are not
/// deduplicated, exactly like the Python generator expression.
pub fn bounded_reasons(values: &[String], field_name: &str) -> Result<Vec<String>, AgentError> {
    let mut bounded = Vec::with_capacity(values.len());
    for value in values {
        bounded.push(bounded_text(
            value,
            &format!("{field_name} entry"),
            MAX_ERROR_TEXT,
        )?);
    }
    Ok(bounded)
}

/// Freeze a JSON value into an immutable task argument.
///
/// The Python implementation walks the value and refuses a non-finite float,
/// an empty object key and a value that nests deeper than 16 levels. The
/// canonical value model of the accepted `kala-json` crate is already
/// immutable, so the freeze only performs the three checks.
///
/// The depth is checked *before* the recursion, so the recursion is at most
/// 18 frames deep and a hostile document cannot overflow the call stack.
pub fn freeze_value(value: &CanonicalValue) -> Result<CanonicalValue, AgentError> {
    freeze_value_recursive(value, 0)
}

/// Freeze a JSON value recursively.
///
/// The depth bound of [`MAX_ARGUMENT_DEPTH`] keeps the recursion at most 18
/// frames deep, so a hostile document cannot overflow the call stack: the
/// depth is checked *before* the recursion, and a document that nests deeper
/// than the bound is refused at the bound.
fn freeze_value_recursive(
    value: &CanonicalValue,
    depth: usize,
) -> Result<CanonicalValue, AgentError> {
    if depth > MAX_ARGUMENT_DEPTH {
        return Err(AgentError::plan(
            "task arguments exceed the supported nesting depth",
        ));
    }
    match value {
        CanonicalValue::Float(number) if !number.is_finite() => Err(AgentError::plan(
            "task arguments must contain finite numbers",
        )),
        CanonicalValue::Object(entries) => {
            let mut frozen = CanonicalObject::new();
            for (key, item) in entries {
                if key.is_empty() {
                    return Err(AgentError::plan(
                        "task argument keys must be nonempty strings",
                    ));
                }
                frozen.insert(key.clone(), freeze_value_recursive(item, depth + 1)?);
            }
            Ok(CanonicalValue::Object(frozen))
        }
        CanonicalValue::Array(items) => {
            let mut frozen = Vec::with_capacity(items.len());
            for item in items {
                frozen.push(freeze_value_recursive(item, depth + 1)?);
            }
            Ok(CanonicalValue::Array(frozen))
        }
        other => Ok(other.clone()),
    }
}

/// Freeze a JSON object into an immutable task argument.
///
/// The object must be a JSON object and every key must be a nonempty string.
/// The values are frozen from depth zero, exactly like the Python
/// `_freeze_object`.
pub fn freeze_object(
    value: &CanonicalValue,
    field_name: &str,
) -> Result<CanonicalValue, AgentError> {
    let CanonicalValue::Object(entries) = value else {
        return Err(AgentError::plan(format!(
            "{field_name} must be a JSON object"
        )));
    };
    let mut frozen = CanonicalObject::new();
    for (key, item) in entries {
        if key.is_empty() {
            return Err(AgentError::plan(format!(
                "{field_name} keys must be nonempty strings"
            )));
        }
        frozen.insert(key.clone(), freeze_value_recursive(item, 0)?);
    }
    Ok(CanonicalValue::Object(frozen))
}

/// Thaw a frozen JSON value.
///
/// The Python implementation converts the immutable mapping and tuple
/// containers back into plain dictionaries and lists. The canonical JSON
/// encoding of both shapes is identical, so the Rust port is the identity.
#[must_use]
pub fn thaw(value: &CanonicalValue) -> CanonicalValue {
    value.clone()
}

/// Thaw a frozen JSON object.
///
/// The Python implementation preserves the insertion order of the mapping.
/// The canonical object of the accepted `kala-json` crate is sorted by code
/// point, and the canonical JSON encoding sorts the keys as well, so the
/// encoded bytes are identical.
#[must_use]
pub fn thaw_object(value: &CanonicalValue) -> CanonicalValue {
    value.clone()
}

/// Truncate a text to a number of Unicode code points.
///
/// The Python module slices with `text[:maximum]`, which counts code points.
#[must_use]
pub fn truncate_code_points(value: &str, maximum: usize) -> String {
    if code_point_len(value) <= maximum {
        return value.to_owned();
    }
    value.chars().take(maximum).collect::<String>()
}
