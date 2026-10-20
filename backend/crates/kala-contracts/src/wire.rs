//! Ordered wire values, canonical serialization and the model trait.

use std::fmt::Write as _;

use kala_json::{
    CanonicalObject, CanonicalValue, Limits, Sha256Digest, canonical_digest, canonical_dumps,
    format_float,
};
use num_bigint::BigInt;

use crate::error::ContractError;
use crate::text::Text;

/// A JSON value in Python model field order.
///
/// Unlike [`CanonicalValue`], object entries keep insertion order. This is the
/// representation of `model.model_dump(mode="json")`; canonical hashing sorts
/// the same value afterwards.
#[derive(Clone, Debug, PartialEq)]
pub enum WireValue {
    /// JSON `null`.
    Null,
    /// JSON boolean.
    Bool(bool),
    /// Arbitrary-precision integer.
    Integer(BigInt),
    /// Finite binary64 float, including signed zero.
    Float(f64),
    /// Contract text (may contain lone surrogates).
    Text(Text),
    /// Ordered array.
    Array(Vec<WireValue>),
    /// Ordered object.
    Object(Vec<(Text, WireValue)>),
}

impl WireValue {
    /// Build an ordered object from static field names.
    pub fn object(fields: impl IntoIterator<Item = (&'static str, WireValue)>) -> Self {
        Self::Object(
            fields
                .into_iter()
                .map(|(name, value)| (Text::new(name), value))
                .collect(),
        )
    }

    /// Build an array.
    pub fn array(values: impl IntoIterator<Item = WireValue>) -> Self {
        Self::Array(values.into_iter().collect())
    }

    /// Convert to the canonical JSON value model.
    pub fn to_canonical_value(&self) -> Result<CanonicalValue, ContractError> {
        Ok(match self {
            Self::Null => CanonicalValue::Null,
            Self::Bool(value) => CanonicalValue::Bool(*value),
            Self::Integer(value) => CanonicalValue::Integer(value.clone()),
            Self::Float(value) => CanonicalValue::float(*value)
                .map_err(|error| ContractError::new("WireValue", error.to_string()))?,
            Self::Text(value) => CanonicalValue::Text(value.as_json_text().clone()),
            Self::Array(values) => CanonicalValue::Array(
                values
                    .iter()
                    .map(Self::to_canonical_value)
                    .collect::<Result<_, _>>()?,
            ),
            Self::Object(fields) => {
                let mut object = CanonicalObject::new();
                for (name, value) in fields {
                    object.insert(name.as_json_text().clone(), value.to_canonical_value()?);
                }
                CanonicalValue::Object(object)
            }
        })
    }

    /// Encode as Python `json.dumps` bytes.
    ///
    /// `ensure_ascii=false` is the HTTP wire form (Starlette `JSONResponse`);
    /// `ensure_ascii=true` is the transport/test form used by Pydantic's JSON
    /// dump and by this crate's differential oracle. Lone surrogates are
    /// representable only through escaping, so the non-ASCII form fails on
    /// them exactly where Python's UTF-8 encode fails.
    pub fn encode(&self, ensure_ascii: bool) -> Result<String, ContractError> {
        let mut out = String::new();
        self.write_json(ensure_ascii, &mut out)?;
        Ok(out)
    }

    fn write_json(&self, ensure_ascii: bool, out: &mut String) -> Result<(), ContractError> {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(true) => out.push_str("true"),
            Self::Bool(false) => out.push_str("false"),
            Self::Integer(value) => {
                let rendered = value.to_string();
                let digits = rendered.trim_start_matches('-').len();
                if digits > Limits::DEFAULT_MAX_INTEGER_DIGITS {
                    return Err(ContractError::new(
                        "WireValue",
                        "value cannot be canonicalized as JSON",
                    ));
                }
                out.push_str(&rendered);
            }
            Self::Float(value) => {
                if !value.is_finite() {
                    return Err(ContractError::new(
                        "WireValue",
                        "JSON numbers must be finite",
                    ));
                }
                out.push_str(&format_float(*value).expect("finite float"));
            }
            Self::Text(value) => write_text(value, ensure_ascii, out)?,
            Self::Array(values) => {
                out.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    value.write_json(ensure_ascii, out)?;
                }
                out.push(']');
            }
            Self::Object(fields) => {
                out.push('{');
                for (index, (name, value)) in fields.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write_text(name, ensure_ascii, out)?;
                    out.push(':');
                    value.write_json(ensure_ascii, out)?;
                }
                out.push('}');
            }
        }
        Ok(())
    }
}

/// CPython `json.dumps` string escaping.
fn write_text(value: &Text, ensure_ascii: bool, out: &mut String) -> Result<(), ContractError> {
    out.push('"');
    for point in value.code_points() {
        match *point {
            0x22 => out.push_str("\\\""),
            0x5C => out.push_str("\\\\"),
            0x08 => out.push_str("\\b"),
            0x09 => out.push_str("\\t"),
            0x0A => out.push_str("\\n"),
            0x0C => out.push_str("\\f"),
            0x0D => out.push_str("\\r"),
            0x00..=0x1F => {
                let _ = write!(out, "\\u{point:04x}");
            }
            0x20..=0x7E => out.push(char::from_u32(*point).expect("ASCII")),
            0x7F..=0x10_FFFF if !ensure_ascii => match char::from_u32(*point) {
                Some(character) => out.push(character),
                None => {
                    return Err(ContractError::new(
                        "WireValue",
                        "text contains a lone surrogate and cannot be encoded as UTF-8",
                    ));
                }
            },
            0x7F..=0xFFFF => {
                let _ = write!(out, "\\u{point:04x}");
            }
            _ => {
                let adjusted = point - 0x1_0000;
                let high = 0xD800 + (adjusted >> 10);
                let low = 0xDC00 + (adjusted & 0x3FF);
                let _ = write!(out, "\\u{high:04x}\\u{low:04x}");
            }
        }
    }
    out.push('"');
    Ok(())
}

/// A validated contract model.
///
/// Implementors keep private fields, expose typed getters and can only be
/// constructed through [`ContractModel::from_value`] or one of the JSON entry
/// points. The trait supplies the wire/canonical/digest operations so callers
/// cannot accidentally hash the wire bytes or send canonical bytes on the
/// wire.
pub trait ContractModel: Sized {
    /// Python class name.
    const NAME: &'static str;

    /// Validate a parsed strict JSON value.
    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError>;

    /// The Python `model_dump(mode="json")` shape, in field order.
    fn to_wire_value(&self) -> WireValue;

    /// Parse model JSON with Pydantic-compatible duplicate-key handling.
    ///
    /// Duplicate object keys use last-wins semantics; canonical/hash JSON
    /// remains strict. Object insertion order is retained for construction
    /// models that override this entry point.
    fn from_json_str(text: &str) -> Result<Self, ContractError> {
        let parsed = crate::input::parse_json_str(text)
            .map_err(|error| ContractError::new(Self::NAME, error.message().to_owned()))?;
        let value = parsed.into_canonical();
        Self::from_value(&value)
    }

    /// Parse UTF-8 model JSON bytes with last-wins duplicate handling.
    fn from_json_bytes(bytes: &[u8]) -> Result<Self, ContractError> {
        let parsed = crate::input::parse_json_bytes(bytes)
            .map_err(|error| ContractError::new(Self::NAME, error.message().to_owned()))?;
        let value = parsed.into_canonical();
        Self::from_value(&value)
    }

    /// Starlette-style wire bytes: compact, field order preserved, UTF-8.
    fn to_wire_json(&self) -> Result<String, ContractError> {
        self.to_wire_value().encode(false)
    }

    /// ASCII-escaped wire form used for transport and tests.
    fn to_wire_json_ascii(&self) -> String {
        self.to_wire_value()
            .encode(true)
            .expect("ASCII escaping always succeeds")
    }

    /// The canonical JSON value (object keys sorted by code point).
    fn to_canonical_value(&self) -> Result<CanonicalValue, ContractError> {
        self.to_wire_value().to_canonical_value()
    }

    /// Canonical JSON bytes with the tool-protocol limits used by
    /// `kala_agent.tools.protocol.canonical_json_digest`.
    fn to_canonical_json(&self) -> Result<String, ContractError> {
        self.to_canonical_json_with(&Limits::default())
    }

    /// Canonical JSON bytes with explicit limits.
    fn to_canonical_json_with(&self, limits: &Limits) -> Result<String, ContractError> {
        let value = self.to_canonical_value()?;
        canonical_dumps(&value, limits)
            .map_err(|error| ContractError::new(Self::NAME, error.to_string()))
    }

    /// SHA-256 of the canonical JSON bytes with the protocol limits.
    fn canonical_digest(&self) -> Result<Sha256Digest, ContractError> {
        self.canonical_digest_with(&Limits::default())
    }

    /// SHA-256 of the canonical JSON bytes with explicit limits.
    fn canonical_digest_with(&self, limits: &Limits) -> Result<Sha256Digest, ContractError> {
        let value = self.to_canonical_value()?;
        canonical_digest(&value, limits)
            .map_err(|error| ContractError::new(Self::NAME, error.to_string()))
    }
}

/// Convenience for object keys in the canonical value map.
pub(crate) fn key(name: &str) -> kala_json::JsonText {
    kala_json::JsonText::new(name)
}
