//! Scalar contract types and their Python-compatible normalization.

use std::fmt;

use kala_budget::Decimal;
use kala_json::{CanonicalValue, Limits, canonical_dumps, parse_strict};
use num_bigint::BigInt;
use num_traits::ToPrimitive;

use crate::error::ContractError;
use crate::text::{Text, is_python_whitespace};
use crate::timestamp::Timestamp;

/// Re-exported SHA-256 digest type from the accepted foundation crate.
pub use kala_json::Sha256Digest;

/// A `NonEmptyText` field's validated representation.
///
/// Python exposes `NonEmptyText` as a constrained `str` type alias. In Rust
/// the validated value is a [`Text`]; every model entry point strips Pydantic
/// whitespace, rejects lone surrogates and enforces `1..=16_384` code points.
/// This alias names the contract type for consumers without introducing a
/// second, unvalidated string wrapper. Construct models through
/// [`crate::ContractModel`] rather than assigning this alias directly.
pub type NonEmptyText = Text;

/// The JSON domain used for model input and persisted canonical objects.
///
/// Python's contract models impose no tool-protocol size, depth, string, key
/// or collection limits. The only retained bound is CPython's default
/// 4,300-digit integer conversion limit, because `json.loads`/`json.dumps`
/// enforce it in the source implementation.
pub const fn model_json_limits() -> Limits {
    Limits {
        max_json_bytes: usize::MAX,
        max_string_length: usize::MAX,
        max_collection_items: usize::MAX,
        max_key_length: usize::MAX,
        max_json_depth: usize::MAX,
        max_integer_digits: Limits::DEFAULT_MAX_INTEGER_DIGITS,
    }
}

/// A safe identifier: `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$` after Python
/// `str.strip()`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identifier(String);

impl Identifier {
    /// Validate and normalize an identifier.
    pub fn new(value: &str) -> Result<Self, ContractError> {
        let normalized =
            value.trim_matches(|character: char| is_python_whitespace(u32::from(character)));
        if !is_identifier(normalized) {
            return Err(ContractError::new(
                "Identifier",
                "identifier must be nonempty and contain only safe characters",
            ));
        }
        Ok(Self(normalized.to_owned()))
    }

    /// The normalized ASCII identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn is_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 128 || !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

/// A lowercase `type/subtype` media type.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MediaType(String);

impl MediaType {
    /// Validate and normalize a media type.
    pub fn new(value: &str) -> Result<Self, ContractError> {
        let normalized =
            value.trim_matches(|character: char| is_python_whitespace(u32::from(character)));
        let normalized = normalized.to_lowercase();
        if !is_media_type(&normalized) {
            return Err(ContractError::new(
                "MediaType",
                "media_type must be a valid lowercase media type",
            ));
        }
        Ok(Self(normalized))
    }

    /// The normalized media type.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn is_media_type(value: &str) -> bool {
    let Some((kind, subtype)) = value.split_once('/') else {
        return false;
    };
    if subtype.contains('/')
        || !(1..=127).contains(&kind.len())
        || !(1..=127).contains(&subtype.len())
    {
        return false;
    }
    let valid_start = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    let valid_rest = |byte: u8| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
            )
    };
    kind.as_bytes().first().copied().is_some_and(valid_start)
        && kind.as_bytes()[1..].iter().copied().all(valid_rest)
        && subtype.as_bytes().first().copied().is_some_and(valid_start)
        && subtype.as_bytes()[1..].iter().copied().all(valid_rest)
}

/// A normalized artifact URI.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactUri(Text);

impl ArtifactUri {
    /// Validate, normalize and security-check an artifact URI.
    pub fn new(value: &Text) -> Result<Self, ContractError> {
        Ok(Self(crate::uri::normalize_artifact_uri(value)?))
    }

    /// Parse and normalize an artifact URI from a Rust string.
    pub fn parse(value: &str) -> Result<Self, ContractError> {
        Self::new(&Text::new(value))
    }

    /// The normalized URI text.
    pub const fn as_text(&self) -> &Text {
        &self.0
    }
}

impl fmt::Display for ArtifactUri {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

/// A canonical JSON object stored as its canonical ASCII JSON text.
///
/// This mirrors `kala_agent.contracts.models.CanonicalJsonObject`: JSON text
/// input is strictly parsed (duplicate keys, non-finite numbers and trailing
/// input are refused), an already parsed object is accepted, and the result is
/// CPython `ensure_ascii=True`, sorted, compact JSON. The persisted domain is
/// deliberately unbounded; tool-protocol limits do not apply.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalJsonObject(String);

impl CanonicalJsonObject {
    /// The empty object, `"{}"`.
    pub fn empty() -> Self {
        Self("{}".to_owned())
    }

    /// Canonicalize a JSON value, accepting text or an already parsed object.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        match value {
            CanonicalValue::Text(source) => {
                let text = Text::from_json_text(source.clone());
                let parsed = parse_text_source(&text)?;
                if !matches!(parsed, CanonicalValue::Object(_)) {
                    return Err(non_object());
                }
                let canonical = canonical_dumps(&parsed, &model_json_limits())
                    .map_err(|_| invalid_json_object())?;
                Ok(Self(canonical))
            }
            CanonicalValue::Object(object) => {
                let canonical = canonical_dumps(value, &model_json_limits())
                    .map_err(|_| invalid_json_object())?;
                let _ = object;
                Ok(Self(canonical))
            }
            _ => Err(non_object()),
        }
    }

    /// Canonicalize a parsed object value.
    pub fn from_object(object: kala_json::CanonicalObject) -> Result<Self, ContractError> {
        Self::from_value(&CanonicalValue::Object(object))
    }

    /// Parse JSON object text.
    pub fn parse(source: &str) -> Result<Self, ContractError> {
        Self::from_value(&CanonicalValue::text(source))
    }

    /// The canonical ASCII JSON text (also the field's wire representation).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parse the canonical text back into the lossless JSON value model.
    pub fn to_value(&self) -> Result<CanonicalValue, ContractError> {
        parse_strict(self.0.as_bytes(), &model_json_limits())
            .map_err(|error| ContractError::new("CanonicalJsonObject", error.to_string()))
    }
}

impl Default for CanonicalJsonObject {
    fn default() -> Self {
        Self::empty()
    }
}

impl fmt::Display for CanonicalJsonObject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn non_object() -> ContractError {
    ContractError::new("CanonicalJsonObject", "value must be a JSON object")
}

fn invalid_json_object() -> ContractError {
    ContractError::new("CanonicalJsonObject", "value must be a valid JSON object")
}

fn parse_text_source(text: &Text) -> Result<CanonicalValue, ContractError> {
    let bytes = json_source_bytes(text);
    parse_strict(&bytes, &model_json_limits()).map_err(|_| invalid_json_object())
}

/// Convert contract text into bytes accepted by the strict JSON parser.
///
/// Raw lone surrogates inside a JSON string are rewritten to `\uXXXX`
/// escapes, which decodes to the same code point. A surrogate outside a string
/// remains invalid JSON, exactly as in CPython.
fn json_source_bytes(text: &Text) -> Vec<u8> {
    if !text.has_surrogates() {
        return text.to_utf8().expect("no surrogates").into_bytes();
    }
    let mut out = String::new();
    for point in text.code_points() {
        if (0xD800..=0xDFFF).contains(point) {
            use std::fmt::Write as _;
            let _ = write!(out, "\\u{point:04x}");
        } else if let Some(character) = char::from_u32(*point) {
            out.push(character);
        }
    }
    out.into_bytes()
}

/// A finite nonnegative binary64 float. Negative zero is accepted and kept.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct FiniteNonNegativeFloat(f64);

impl FiniteNonNegativeFloat {
    /// Validate a float. Negative zero is accepted.
    pub fn new(value: f64) -> Result<Self, ContractError> {
        if !value.is_finite() || value < 0.0 {
            return Err(ContractError::new(
                "FiniteNonNegativeFloat",
                "numeric values must be finite and nonnegative",
            ));
        }
        Ok(Self(value))
    }

    /// The value.
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Default for FiniteNonNegativeFloat {
    fn default() -> Self {
        Self(0.0)
    }
}

/// A finite float in the inclusive interval `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct UnitInterval(f64);

impl UnitInterval {
    /// Validate a unit interval value.
    pub fn new(value: f64) -> Result<Self, ContractError> {
        if !value.is_finite() || value < 0.0 {
            return Err(ContractError::new(
                "UnitInterval",
                "numeric values must be finite and nonnegative",
            ));
        }
        if value > 1.0 {
            return Err(ContractError::new(
                "UnitInterval",
                "score must be between zero and one",
            ));
        }
        Ok(Self(value))
    }

    /// The value.
    pub const fn get(self) -> f64 {
        self.0
    }
}

/// An arbitrary-precision nonnegative integer.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonNegativeInteger(BigInt);

impl NonNegativeInteger {
    /// Validate an arbitrary-precision integer.
    pub fn new(value: BigInt) -> Result<Self, ContractError> {
        if value.sign() == num_bigint::Sign::Minus {
            return Err(ContractError::new(
                "NonNegativeInteger",
                "integer values must be nonnegative",
            ));
        }
        Ok(Self(value))
    }

    /// The integer value.
    pub const fn as_bigint(&self) -> &BigInt {
        &self.0
    }

    /// Convert to `u64` when it fits.
    pub fn to_u64(&self) -> Option<u64> {
        self.0.to_u64()
    }

    /// The value zero.
    pub fn zero() -> Self {
        Self(BigInt::from(0))
    }

    /// Build from a `u64`.
    pub fn from_u64(value: u64) -> Self {
        Self(BigInt::from(value))
    }
}

impl Default for NonNegativeInteger {
    fn default() -> Self {
        Self::zero()
    }
}

/// A finite, nonnegative Python `Decimal` with its textual scale intact.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NonNegativeDecimal(Decimal);

impl NonNegativeDecimal {
    /// Validate an already parsed decimal.
    pub fn new(value: Decimal) -> Result<Self, ContractError> {
        if value.compare(&Decimal::zero()).is_lt() {
            return Err(ContractError::new(
                "NonNegativeDecimal",
                "cost must be finite and nonnegative",
            ));
        }
        Ok(Self(value))
    }

    /// Parse a decimal literal using CPython `Decimal(str(value))` rules.
    pub fn parse(value: &str) -> Result<Self, ContractError> {
        let parsed = Decimal::parse(value).map_err(|_| {
            ContractError::new("NonNegativeDecimal", "cost must be a nonnegative decimal")
        })?;
        Self::new(parsed)
    }

    /// The decimal value.
    pub const fn get(&self) -> &Decimal {
        &self.0
    }

    /// CPython `str(Decimal)`.
    pub fn to_python_string(&self) -> String {
        self.0.to_python_string()
    }
}

impl Default for NonNegativeDecimal {
    fn default() -> Self {
        Self(Decimal::zero())
    }
}

// ---------------------------------------------------------------------------
// Value-domain extraction used by the model parsers.
// ---------------------------------------------------------------------------

pub(crate) fn as_text(value: &CanonicalValue) -> Option<Text> {
    value
        .as_text()
        .map(|text| Text::from_json_text(text.clone()))
}

pub(crate) fn as_nonempty_text(value: &CanonicalValue) -> Result<Text, String> {
    let text = as_text(value).ok_or_else(|| "Input should be a valid string".to_owned())?;
    if text.has_surrogates() {
        return Err(
            "Input should be a valid string, unable to parse raw data as a unicode string"
                .to_owned(),
        );
    }
    let stripped = text.pydantic_strip();
    if stripped.is_empty() {
        return Err("String should have at least 1 character".to_owned());
    }
    if stripped.len() > 16_384 {
        return Err("String should have at most 16384 characters".to_owned());
    }
    Ok(stripped)
}

pub(crate) fn as_identifier(value: &CanonicalValue) -> Result<Identifier, String> {
    let text = as_text(value).ok_or_else(|| "Input should be a valid string".to_owned())?;
    let stripped = text.python_strip();
    let utf8 = stripped
        .to_utf8()
        .map_err(|_| "identifier must be nonempty and contain only safe characters".to_owned())?;
    Identifier::new(&utf8).map_err(|error| error.message().to_owned())
}

pub(crate) fn as_digest(value: &CanonicalValue) -> Result<Sha256Digest, String> {
    let text = as_text(value).ok_or_else(|| "Input should be a valid string".to_owned())?;
    let stripped = text.python_strip();
    let utf8 = stripped
        .to_utf8()
        .map_err(|_| "digest must be a 64-character hexadecimal SHA-256 digest".to_owned())?;
    let lowered = utf8.to_lowercase();
    if lowered.len() != 64
        || !lowered
            .as_bytes()
            .iter()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err("digest must be a 64-character hexadecimal SHA-256 digest".to_owned());
    }
    Sha256Digest::from_hex(&lowered)
        .ok_or_else(|| "digest must be a 64-character hexadecimal SHA-256 digest".to_owned())
}

pub(crate) fn as_media_type(value: &CanonicalValue) -> Result<MediaType, String> {
    let text = as_text(value).ok_or_else(|| "Input should be a valid string".to_owned())?;
    let stripped = text.python_strip();
    let utf8 = stripped
        .to_utf8()
        .map_err(|_| "media_type must be a valid lowercase media type".to_owned())?;
    MediaType::new(&utf8).map_err(|error| error.message().to_owned())
}

pub(crate) fn as_artifact_uri(value: &CanonicalValue) -> Result<ArtifactUri, String> {
    let text = as_text(value).ok_or_else(|| "Input should be a valid string".to_owned())?;
    ArtifactUri::new(&text).map_err(|error| error.message().to_owned())
}

pub(crate) fn as_canonical_object(value: &CanonicalValue) -> Result<CanonicalJsonObject, String> {
    CanonicalJsonObject::from_value(value).map_err(|error| error.message().to_owned())
}

pub(crate) fn as_bool_strict(value: &CanonicalValue) -> Result<bool, String> {
    match value {
        CanonicalValue::Bool(value) => Ok(*value),
        _ => Err("Input should be a valid boolean".to_owned()),
    }
}

pub(crate) fn as_nonnegative_integer(value: &CanonicalValue) -> Result<NonNegativeInteger, String> {
    match value {
        CanonicalValue::Integer(value) => {
            NonNegativeInteger::new(value.clone()).map_err(|error| error.message().to_owned())
        }
        _ => Err("integer values must be nonnegative integers".to_owned()),
    }
}

pub(crate) fn as_float_nonnegative(
    value: &CanonicalValue,
) -> Result<FiniteNonNegativeFloat, String> {
    let parsed = match value {
        CanonicalValue::Bool(_) => {
            return Err("numeric values must be finite and nonnegative".to_owned());
        }
        CanonicalValue::Integer(number) => number
            .to_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| "numeric values must be finite and nonnegative".to_owned())?,
        CanonicalValue::Float(number) => *number,
        CanonicalValue::Text(text) => {
            let utf8 = text
                .to_utf8()
                .map_err(|_| "numeric values must be finite and nonnegative".to_owned())?;
            kala_budget::python_float_parse(&utf8)
                .map_err(|_| "numeric values must be finite and nonnegative".to_owned())?
        }
        _ => return Err("numeric values must be finite and nonnegative".to_owned()),
    };
    FiniteNonNegativeFloat::new(parsed).map_err(|error| error.message().to_owned())
}

pub(crate) fn as_unit_interval(value: &CanonicalValue) -> Result<UnitInterval, String> {
    let parsed = as_float_nonnegative(value)?;
    UnitInterval::new(parsed.get()).map_err(|error| error.message().to_owned())
}

pub(crate) fn as_decimal_nonnegative(value: &CanonicalValue) -> Result<NonNegativeDecimal, String> {
    let parsed = match value {
        CanonicalValue::Bool(_) => {
            return Err("cost must be a nonnegative decimal".to_owned());
        }
        CanonicalValue::Integer(number) => Decimal::parse(&number.to_string())
            .map_err(|_| "cost must be a nonnegative decimal".to_owned())?,
        CanonicalValue::Float(number) => Decimal::from_python_float(*number)
            .map_err(|_| "cost must be a nonnegative decimal".to_owned())?,
        CanonicalValue::Text(text) => {
            let utf8 = text
                .to_utf8()
                .map_err(|_| "cost must be a nonnegative decimal".to_owned())?;
            Decimal::parse(&utf8).map_err(|_| "cost must be a nonnegative decimal".to_owned())?
        }
        _ => return Err("cost must be a nonnegative decimal".to_owned()),
    };
    NonNegativeDecimal::new(parsed).map_err(|error| error.message().to_owned())
}

pub(crate) fn as_timestamp(value: &CanonicalValue) -> Result<Timestamp, String> {
    Timestamp::from_value(value).map_err(|error| error.message().to_owned())
}
