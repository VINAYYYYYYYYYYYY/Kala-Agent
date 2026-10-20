//! Shared Python text, canonical JSON and dynamic-value helpers.

use kala_contracts::Text;
use kala_json::{CanonicalValue, Sha256Digest, canonical_digest, canonical_dumps};

use crate::json_input::ModelJsonError;

/// Python `str.isspace()`.
///
/// CPython treats Unicode `White_Space` plus the four C0 information
/// separators `U+001C..=U+001F` as whitespace.
pub fn is_python_whitespace(point: u32) -> bool {
    matches!(
        point,
        0x0009..=0x000D
            | 0x001C..=0x001F
            | 0x0020
            | 0x0085
            | 0x00A0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
    )
}

/// Whether any code point in `value` is a C0 control (`ord < 32`).
pub fn has_control_character(value: &Text) -> bool {
    value.code_points().iter().copied().any(|point| point < 32)
}

/// Lossy UTF-8 for diagnostics and error messages.
pub fn display_text(value: &Text) -> String {
    value.to_string_lossy()
}

/// Extract a JSON string as lossless Python text.
pub fn value_text(value: &CanonicalValue) -> Option<Text> {
    value
        .as_text()
        .map(|text| Text::from_json_text(text.clone()))
}

/// Extract a strict JSON boolean.
pub fn value_bool(value: &CanonicalValue) -> Option<bool> {
    match value {
        CanonicalValue::Bool(value) => Some(*value),
        _ => None,
    }
}

/// Extract a strict JSON integer (never a boolean or float).
pub fn value_integer(value: &CanonicalValue) -> Option<&num_bigint::BigInt> {
    match value {
        CanonicalValue::Integer(value) => Some(value),
        _ => None,
    }
}

/// Canonical ASCII JSON bytes with the unbounded model/state domain.
pub fn canonical_state_json(value: &CanonicalValue) -> Result<String, ModelJsonError> {
    canonical_dumps(value, &kala_contracts::model_json_limits())
        .map_err(|error| ModelJsonError::new(error.to_string()))
}

/// SHA-256 of the canonical ASCII JSON bytes with the unbounded model domain.
///
/// This is exactly Python
/// `hashlib.sha256(json.dumps(value, allow_nan=False, ensure_ascii=True,
/// separators=(",", ":"), sort_keys=True).encode("utf-8")).hexdigest()`.
pub fn canonical_state_digest(value: &CanonicalValue) -> Result<Sha256Digest, ModelJsonError> {
    canonical_digest(value, &kala_contracts::model_json_limits())
        .map_err(|error| ModelJsonError::new(error.to_string()))
}
