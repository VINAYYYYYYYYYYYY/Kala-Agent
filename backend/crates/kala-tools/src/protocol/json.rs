//! Bounded canonical JSON and digests.
//!
//! The Python module implements canonical JSON itself with `json.dumps(...,
//! ensure_ascii=True, separators=(",", ":"), sort_keys=True)`. The accepted
//! `kala-json` crate already produces exactly those bytes, so this module
//! delegates to it and only keeps the protocol specific parts: the byte
//! budgets, the error classes and the digest helper.
//!
//! The error mapping is exact:
//!
//! * `canonical_json_dumps` raises `JsonEnvelopeError` with the same message
//!   the Python module raises, including
//!   `"value cannot be canonicalized as JSON"` for an integer that exceeds
//!   CPython's 4,300 digit conversion limit.
//! * `strict_json_loads` raises `JsonEnvelopeError` with the same message,
//!   including `"payload is not strict JSON"` for the same integer limit,
//!   because CPython raises the limit while decoding the document.

use kala_json::{CanonicalJsonError, CanonicalValue, Limits, canonical_dumps, parse_strict};
use sha2::{Digest, Sha256};

use super::error::ToolProtocolError;

/// The protocol limits with a custom canonical byte budget.
///
/// The Python module always applies the string, key, collection and depth
/// bounds and only lets the caller choose the canonical byte budget.
fn limits(max_bytes: usize) -> Limits {
    Limits {
        max_json_bytes: max_bytes,
        ..Limits::protocol()
    }
}

/// Translate a `kala-json` failure into the Python `JsonEnvelopeError` text.
///
/// `canonicalizable` selects between the two Python messages that share the
/// same underlying cause: the dump path reports
/// `"value cannot be canonicalized as JSON"` while the load path reports
/// `"payload is not strict JSON"`.
fn json_error(error: CanonicalJsonError, canonicalizable: bool) -> ToolProtocolError {
    let message = match error {
        CanonicalJsonError::InvalidUtf8 => "payload is not valid UTF-8".to_owned(),
        CanonicalJsonError::NotJson => "payload is not strict JSON".to_owned(),
        CanonicalJsonError::DuplicateKey => {
            "duplicate JSON object keys are not permitted".to_owned()
        }
        CanonicalJsonError::NonStringKey => "JSON object keys must be strings".to_owned(),
        CanonicalJsonError::NonFiniteConstant(value) => {
            format!("JSON constant {value} is not permitted")
        }
        CanonicalJsonError::NonFinite => "JSON numbers must be finite".to_owned(),
        CanonicalJsonError::DepthExceeded => "JSON nesting exceeds the supported depth".to_owned(),
        CanonicalJsonError::StringTooLong => "JSON string exceeds the bounded length".to_owned(),
        CanonicalJsonError::KeyTooLong => "JSON object key exceeds the bounded length".to_owned(),
        CanonicalJsonError::CollectionTooLarge => {
            "JSON collection exceeds the bounded item count".to_owned()
        }
        CanonicalJsonError::InputTooLarge => "payload exceeds the bounded size".to_owned(),
        CanonicalJsonError::OutputTooLarge => "canonical JSON exceeds the bounded size".to_owned(),
        CanonicalJsonError::NotSerializable => "value is not JSON serializable".to_owned(),
        CanonicalJsonError::NotCanonicalizable => {
            if canonicalizable {
                "value cannot be canonicalized as JSON".to_owned()
            } else {
                "payload is not strict JSON".to_owned()
            }
        }
        CanonicalJsonError::InvalidChainText => "chain field cannot be encoded as UTF-8".to_owned(),
        CanonicalJsonError::InvalidCodePoint => {
            "text contains a code point outside Unicode".to_owned()
        }
        CanonicalJsonError::LoneSurrogate => {
            "text contains a lone surrogate and cannot be encoded as UTF-8".to_owned()
        }
    };
    ToolProtocolError::json_envelope(message)
}

/// Canonicalize a value to the protocol's canonical JSON bytes.
///
/// The result is pure ASCII, keys are sorted by Unicode code point and there
/// is no insignificant whitespace, exactly like the Python module. `max_bytes`
/// bounds the canonical output; the string, key, collection and depth bounds
/// always apply.
pub fn canonical_json_dumps(
    value: &CanonicalValue,
    max_bytes: usize,
) -> Result<String, ToolProtocolError> {
    canonical_dumps(value, &limits(max_bytes)).map_err(|error| json_error(error, true))
}

/// Strictly decode a JSON payload.
///
/// The payload must be valid UTF-8 and strict JSON: duplicate object keys,
/// `NaN`, `Infinity`, `-Infinity`, non-finite floats, trailing data and a
/// leading byte order mark are all refused.
pub fn strict_json_loads(
    payload: &[u8],
    max_bytes: usize,
) -> Result<CanonicalValue, ToolProtocolError> {
    parse_strict(payload, &limits(max_bytes)).map_err(|error| json_error(error, false))
}

/// The lowercase hexadecimal SHA-256 of the given bytes.
///
/// This is the Python `hashlib.sha256(payload).hexdigest()` of
/// `kala_agent.tools.protocol.sha256_hex`.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let output = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in output {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The lowercase hexadecimal SHA-256 of the UTF-8 encoding of `data`.
pub fn sha256_hex_text(data: &str) -> String {
    sha256_hex(data.as_bytes())
}

/// The SHA-256 of the canonical JSON encoding of a value.
///
/// The canonical encoding is pure ASCII, so the digest is taken over the
/// encoded text directly.
pub fn canonical_json_digest(value: &CanonicalValue) -> Result<String, ToolProtocolError> {
    let encoded = canonical_json_dumps(value, crate::protocol::MAX_JSON_BYTES)?;
    Ok(sha256_hex_text(&encoded))
}
