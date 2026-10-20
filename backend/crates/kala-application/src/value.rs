//! Validation helpers and canonical JSON plumbing shared by the application.
//!
//! Every rule here mirrors one named validator in
//! `kala_agent/application.py`; the error class and message text are part of
//! the observable behaviour.

use kala_contracts::{ApprovalKind, ApprovalStatus, Text, Timestamp, WireValue, WorkflowStatus};
use kala_json::{CanonicalValue, JsonText, Limits};
use kala_kernel::policy::PolicyDecisionStatus;
use num_bigint::BigInt;

use crate::error::ApplicationError;

/// The Python identifier pattern `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`.
pub const IDENTIFIER_PATTERN_MAX: usize = 128;

/// `MAX_IDENTIFIER_LENGTH`.
pub const MAX_IDENTIFIER_LENGTH: usize = 64;

/// `_require_identifier`: trim, match the safe pattern, then bound the length.
pub fn require_identifier(
    value: &str,
    field_name: &str,
    maximum: usize,
) -> Result<String, ApplicationError> {
    let normalized = kala_api::pytext::trim(value);
    if !is_safe_identifier(normalized) {
        return Err(ApplicationError::invalid_identifier(format!(
            "{field_name} must start with a letter or digit and contain only safe characters"
        )));
    }
    if normalized.chars().count() > maximum {
        return Err(ApplicationError::invalid_identifier(format!(
            "{field_name} must be at most {maximum} characters"
        )));
    }
    Ok(normalized.to_owned())
}

/// Whether one trimmed value matches the application identifier pattern.
#[must_use]
pub fn is_safe_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    let mut count = 1;
    for character in characters {
        if !(character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')) {
            return false;
        }
        count += 1;
        if count > IDENTIFIER_PATTERN_MAX {
            return false;
        }
    }
    true
}

/// `_require_text`: trim, refuse empty, bound the length.
pub fn require_text(
    value: &str,
    field_name: &str,
    maximum: usize,
) -> Result<String, ApplicationError> {
    let normalized = kala_api::pytext::trim(value);
    if normalized.is_empty() {
        return Err(ApplicationError::invalid_parameters(format!(
            "{field_name} cannot be empty"
        )));
    }
    if normalized.chars().count() > maximum {
        return Err(ApplicationError::invalid_parameters(format!(
            "{field_name} must be at most {maximum} characters"
        )));
    }
    Ok(normalized.to_owned())
}

/// `_require_limit`: an integer in `1..=maximum`, refusing booleans.
pub fn require_limit(value: i64, field_name: &str, maximum: i64) -> Result<i64, ApplicationError> {
    if !(1..=maximum).contains(&value) {
        return Err(ApplicationError::invalid_parameters(format!(
            "{field_name} must be between 1 and {maximum}"
        )));
    }
    Ok(value)
}

/// `_optional_text`: `None` stays `None`, anything else is text.
pub fn optional_text(
    value: Option<&str>,
    field_name: &str,
    maximum: usize,
) -> Result<Option<String>, ApplicationError> {
    match value {
        None => Ok(None),
        Some(value) => require_text(value, field_name, maximum).map(Some),
    }
}

/// `_require_digest`: exactly 64 lowercase hexadecimal characters.
pub fn require_digest(value: &str, field_name: &str) -> Result<String, ApplicationError> {
    let is_digest = value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !is_digest {
        return Err(ApplicationError::application(format!(
            "the recorded {field_name} is not a digest"
        )));
    }
    Ok(value.to_owned())
}

/// `_identifier_sequence`: a JSON array (or `None`) of safe identifiers.
pub fn identifier_sequence(
    value: Option<&CanonicalValue>,
    field_name: &str,
) -> Result<Vec<String>, ApplicationError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let CanonicalValue::Array(items) = value else {
        return Err(ApplicationError::application(format!(
            "the recorded {field_name} collection is unreadable"
        )));
    };
    items
        .iter()
        .map(|item| {
            let CanonicalValue::Text(text) = item else {
                return Err(ApplicationError::application(format!(
                    "the recorded {field_name} collection is unreadable"
                )));
            };
            require_identifier(&json_text_utf8(text), field_name, MAX_IDENTIFIER_LENGTH)
        })
        .collect()
}

/// `_optional_text` over a journalled JSON value.
pub fn optional_text_value(
    value: Option<&CanonicalValue>,
    field_name: &str,
    maximum: usize,
) -> Result<Option<String>, ApplicationError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if matches!(value, CanonicalValue::Null) {
        return Ok(None);
    }
    let CanonicalValue::Text(text) = value else {
        return Err(ApplicationError::application(format!(
            "the recorded {field_name} is not text"
        )));
    };
    require_text(&json_text_utf8(text), field_name, maximum).map(Some)
}

/// Validate an identifier stored inside a journalled JSON object.
pub fn require_stored_identifier(
    value: Option<&CanonicalValue>,
    field_name: &str,
    maximum: usize,
) -> Result<String, ApplicationError> {
    let Some(CanonicalValue::Text(text)) = value else {
        return Err(ApplicationError::invalid_identifier(format!(
            "{field_name} must be a string"
        )));
    };
    require_identifier(&json_text_utf8(text), field_name, maximum)
}

/// Validate a digest stored inside a journalled JSON object.
pub fn require_stored_digest(
    value: Option<&CanonicalValue>,
    field_name: &str,
) -> Result<String, ApplicationError> {
    let Some(CanonicalValue::Text(text)) = value else {
        return Err(ApplicationError::application(format!(
            "the recorded {field_name} is not a digest"
        )));
    };
    require_digest(&json_text_utf8(text), field_name)
}

/// Validate a nonnegative integer stored inside a journalled JSON object.
pub fn require_stored_sequence(value: Option<&CanonicalValue>) -> Result<BigInt, ApplicationError> {
    let Some(CanonicalValue::Integer(integer)) = value else {
        return Err(ApplicationError::application(
            "the recorded sequence is not a revision sequence",
        ));
    };
    if integer < &BigInt::from(0) {
        return Err(ApplicationError::application(
            "the recorded sequence is not a revision sequence",
        ));
    }
    Ok(integer.clone())
}

/// The UTF-8 view of stored JSON text.
///
/// Journal payloads written by this root are valid UTF-8. A stored string that
/// carries a lone surrogate (possible only in a foreign writer's canonical
/// JSON) is rendered with U+FFFD for diagnostics instead of panicking.
#[must_use]
pub fn json_text_utf8(value: &JsonText) -> String {
    value.to_utf8().unwrap_or_else(|_| value.to_string_lossy())
}

/// The UTF-8 view of a contract `Text` value.
#[must_use]
pub fn text_string(value: &Text) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string_lossy())
}

/// The borrowed UTF-8 view of a contract `Text` value, or the empty string.
#[must_use]
pub fn text_ref(value: &Text) -> &str {
    value.as_str().unwrap_or("")
}

/// A borrowed UTF-8 view shared by contract text and Rust strings.
pub trait AppText {
    /// The view.
    fn as_app_str(&self) -> &str;
}

impl AppText for Text {
    fn as_app_str(&self) -> &str {
        text_ref(self)
    }
}

impl AppText for str {
    fn as_app_str(&self) -> &str {
        self
    }
}

impl AppText for String {
    fn as_app_str(&self) -> &str {
        self.as_str()
    }
}

/// Read a field from a canonical JSON object.
#[must_use]
pub fn field<'a>(object: &'a CanonicalValue, name: &str) -> Option<&'a CanonicalValue> {
    match object {
        CanonicalValue::Object(map) => map.get(&JsonText::new(name)),
        _ => None,
    }
}

/// `canonical_json_dumps` with an explicit byte bound.
pub fn canonical_dumps(
    value: &CanonicalValue,
    max_bytes: usize,
) -> Result<String, ApplicationError> {
    let limits = Limits {
        max_json_bytes: max_bytes,
        ..Limits::default()
    };
    kala_json::canonical_dumps(value, &limits).map_err(|error| {
        ApplicationError::application(format!("value cannot be canonicalized as JSON: {error}"))
    })
}

/// `canonical_json_digest` with the protocol limits.
pub fn canonical_digest(value: &CanonicalValue) -> Result<String, ApplicationError> {
    kala_json::canonical_json_digest(value)
        .map(|digest| digest.to_hex().to_owned())
        .map_err(|error| {
            ApplicationError::application(format!("value cannot be canonicalized as JSON: {error}"))
        })
}

/// `sha256_hex_text`.
#[must_use]
pub fn sha256_hex_text(value: &str) -> String {
    kala_tools::protocol::sha256_hex_text(value).to_owned()
}

/// Convert a kernel microsecond timestamp to the contract timestamp.
pub fn contract_timestamp(value: kala_kernel::KernelTimestamp) -> Timestamp {
    Timestamp::from_absolute_micros(value.unix_micros())
        .expect("kernel timestamps stay inside the contract range")
}

/// Convert a kernel timestamp to chrono UTC.
pub fn chrono_from_kernel(value: kala_kernel::KernelTimestamp) -> chrono::DateTime<chrono::Utc> {
    let micros = value.unix_micros();
    let seconds = micros.div_euclid(1_000_000);
    let microsecond = micros.rem_euclid(1_000_000);
    chrono::DateTime::from_timestamp(
        i64::try_from(seconds).expect("kernel timestamps fit i64 seconds"),
        u32::try_from(microsecond).expect("microsecond < 1_000_000") * 1_000,
    )
    .expect("kernel timestamps stay inside the chrono range")
}

/// Convert a chrono UTC instant to the kernel microsecond timestamp.
pub fn kernel_from_chrono(value: chrono::DateTime<chrono::Utc>) -> kala_kernel::KernelTimestamp {
    let micros =
        i128::from(value.timestamp()) * 1_000_000 + i128::from(value.timestamp_subsec_micros());
    kala_kernel::KernelTimestamp::from_unix_micros(micros)
        .expect("chrono timestamps stay inside the kernel range")
}

/// Convert a contract timestamp to chrono UTC.
pub fn chrono_from_contract(value: &Timestamp) -> chrono::DateTime<chrono::Utc> {
    let micros = value.absolute_micros();
    let seconds = micros.div_euclid(1_000_000);
    let microsecond = micros.rem_euclid(1_000_000);
    chrono::DateTime::from_timestamp(
        i64::try_from(seconds).expect("contract timestamps fit i64 seconds"),
        u32::try_from(microsecond).expect("microsecond < 1_000_000") * 1_000,
    )
    .expect("contract timestamps stay inside the chrono range")
}

/// Convert a contract timestamp to the kernel timestamp.
pub fn kernel_from_contract(value: &Timestamp) -> kala_kernel::KernelTimestamp {
    kala_kernel::KernelTimestamp::from_unix_micros(value.absolute_micros())
        .expect("contract timestamps stay inside the kernel range")
}

// ---------------------------------------------------------------------------
// Wire-value constructors used by every DTO.
// ---------------------------------------------------------------------------

/// A text wire value.
#[must_use]
pub fn wire_text(value: &str) -> WireValue {
    WireValue::Text(Text::new(value))
}

/// An optional text wire value.
#[must_use]
pub fn wire_opt_text(value: Option<&str>) -> WireValue {
    match value {
        Some(value) => wire_text(value),
        None => WireValue::Null,
    }
}

/// An integer wire value.
#[must_use]
pub fn wire_integer(value: &BigInt) -> WireValue {
    WireValue::Integer(value.clone())
}

/// An optional integer wire value.
#[must_use]
pub fn wire_opt_integer(value: Option<&BigInt>) -> WireValue {
    match value {
        Some(value) => wire_integer(value),
        None => WireValue::Null,
    }
}

/// A boolean wire value.
#[must_use]
pub fn wire_bool(value: bool) -> WireValue {
    WireValue::Bool(value)
}

/// An optional float wire value.
#[must_use]
pub fn wire_opt_float(value: Option<f64>) -> WireValue {
    match value {
        Some(value) => WireValue::Float(value),
        None => WireValue::Null,
    }
}

/// An array of text wire values.
#[must_use]
pub fn wire_texts(values: &[String]) -> WireValue {
    WireValue::Array(values.iter().map(|value| wire_text(value)).collect())
}

/// A workflow status wire value.
#[must_use]
pub fn wire_workflow_status(value: WorkflowStatus) -> WireValue {
    wire_text(value.as_str())
}

/// An approval kind wire value.
#[must_use]
pub fn wire_approval_kind(value: Option<ApprovalKind>) -> WireValue {
    match value {
        Some(value) => wire_text(value.as_str()),
        None => WireValue::Null,
    }
}

/// An approval status wire value.
#[must_use]
pub fn wire_approval_status(value: ApprovalStatus) -> WireValue {
    wire_text(value.as_str())
}

/// A policy decision status wire value.
#[must_use]
pub fn wire_policy_status(value: Option<PolicyDecisionStatus>) -> WireValue {
    match value {
        Some(value) => wire_text(value.as_str()),
        None => WireValue::Null,
    }
}

/// A contract timestamp wire value.
#[must_use]
pub fn wire_timestamp(value: &Timestamp) -> WireValue {
    wire_text(&value.to_wire_string())
}

/// An optional contract timestamp wire value.
#[must_use]
pub fn wire_opt_timestamp(value: Option<&Timestamp>) -> WireValue {
    match value {
        Some(value) => wire_timestamp(value),
        None => WireValue::Null,
    }
}

/// A canonical JSON value as a wire value.
#[must_use]
pub fn wire_value(value: &CanonicalValue) -> WireValue {
    match value {
        CanonicalValue::Null => WireValue::Null,
        CanonicalValue::Bool(flag) => WireValue::Bool(*flag),
        CanonicalValue::Integer(integer) => WireValue::Integer(integer.clone()),
        CanonicalValue::Float(float) => WireValue::Float(*float),
        CanonicalValue::Text(text) => WireValue::Text(Text::from_json_text(text.clone())),
        CanonicalValue::Array(items) => WireValue::Array(items.iter().map(wire_value).collect()),
        CanonicalValue::Object(object) => WireValue::Object(
            object
                .iter()
                .map(|(key, item)| (Text::from_json_text(key.clone()), wire_value(item)))
                .collect(),
        ),
    }
}
