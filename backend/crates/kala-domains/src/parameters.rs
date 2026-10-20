//! The typed parameter-model surface shared by the three packages.

use kala_contracts::WireValue;
use kala_json::CanonicalValue;

use crate::error::ParameterError;

/// A validated, frozen domain parameter model.
///
/// Implementations accept only a JSON object, preserve the declared field
/// order for `model_dump(mode="json")`, and validate cross-field invariants
/// before construction succeeds.
pub trait ParameterModel: Clone + PartialEq + Send + Sync + 'static {
    /// The Python model class name, used in model-type messages.
    const MODEL_NAME: &'static str;
    /// The registry domain name (`mechanical`, `building`, `wing`).
    const DOMAIN_NAME: &'static str;
    /// Declared field names in model field order.
    const FIELD_NAMES: &'static [&'static str];

    /// Validate a parsed JSON value.
    fn from_value(value: &CanonicalValue) -> Result<Self, ParameterError>;
    /// The Python `model_dump(mode="json")` shape in field order.
    fn to_wire_value(&self) -> WireValue;
    /// The package parameter-schema digest.
    fn schema_digest() -> String;

    /// Parse a model JSON object document with Pydantic-compatible duplicate
    /// key handling (last value wins, first insertion position retained).
    ///
    /// Canonical/hash parsing remains strict; this is the ordinary model-input
    /// boundary, matching `model_validate_json`.
    fn from_json_str(source: &str) -> Result<Self, ParameterError> {
        crate::model_json::parse_str(source)
            .map_err(ParameterError::invalid_json)
            .and_then(crate::model_json::validate::<Self>)
    }

    /// Parse model JSON bytes with Pydantic-compatible duplicate key handling.
    fn from_json_bytes(source: &[u8]) -> Result<Self, ParameterError> {
        crate::model_json::parse_bytes(source)
            .map_err(ParameterError::invalid_json)
            .and_then(crate::model_json::validate::<Self>)
    }

    /// The ordered `model_dump(mode="json")` value.
    fn to_canonical_value(&self) -> Result<CanonicalValue, ParameterError> {
        self.to_wire_value().to_canonical_value().map_err(|error| {
            ParameterError::field(
                Self::MODEL_NAME,
                crate::error::ParameterErrorKind::Value,
                error.to_string(),
            )
        })
    }

    /// Compact UTF-8 JSON in field order, matching `json.dumps(model_dump,
    /// ensure_ascii=False, separators=(",", ":"))`.
    fn to_wire_json(&self) -> Result<String, ParameterError> {
        self.to_wire_value().encode(false).map_err(|error| {
            ParameterError::field(
                Self::MODEL_NAME,
                crate::error::ParameterErrorKind::Value,
                error.to_string(),
            )
        })
    }

    /// Compact ASCII JSON in field order, matching Pydantic's transport form.
    fn to_wire_json_ascii(&self) -> Result<String, ParameterError> {
        self.to_wire_value().encode(true).map_err(|error| {
            ParameterError::field(
                Self::MODEL_NAME,
                crate::error::ParameterErrorKind::Value,
                error.to_string(),
            )
        })
    }

    /// Canonical (sorted-key) JSON bytes.
    fn to_canonical_json(&self) -> Result<String, ParameterError> {
        let value = self.to_canonical_value()?;
        crate::spec::canonical_json(&value).map_err(|error| {
            ParameterError::field(
                Self::MODEL_NAME,
                crate::error::ParameterErrorKind::Value,
                error.to_string(),
            )
        })
    }
}
