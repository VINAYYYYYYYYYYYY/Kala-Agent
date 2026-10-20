//! Persistence-domain canonical JSON.
//!
//! `kala_agent.contracts.models.CanonicalJsonObject` is deliberately **not**
//! `kala_agent.tools.protocol._normalize_json`: a stored event payload or
//! snapshot accepts strings longer than 32,768 code points, keys longer than
//! 256, arrays longer than 2,048, nesting deeper than 48 and payloads larger
//! than 1 MiB, subject only to CPython's parser and integer digit limits.
//!
//! The default [`kala_json::Limits`] are therefore never applied here. The
//! explicit [`persistence_limits`] raise every size bound. Depth is also
//! unbounded: the native parser, validator, encoder and value lifecycle are
//! iterative and use heap stacks, so they do not inherit the former 1,024
//! level syntactic recursion ceiling. Very deep well-formed documents are
//! accepted like the Python contract; memory use is proportional to the input
//! and the number of nodes, not to an artificial depth constant.

use kala_json::{CanonicalValue, Limits, Sha256Digest, canonical_dumps, parse_strict};

use crate::error::PersistenceError;

/// Persistence has no fixed JSON nesting limit.
///
/// This value is the [`Limits`] sentinel for "no depth bound". It is retained
/// as a named constant for callers that previously referred to the numeric
/// cap; it no longer restricts persisted objects. The native parser,
/// validator, encoder, clone, equality, debug and drop paths are all
/// nonrecursive, so depth is limited only by available memory and the input
/// itself, exactly like the unbounded Python `CanonicalJsonObject` source
/// contract.
pub const MAX_PERSISTENCE_JSON_DEPTH: usize = usize::MAX;

/// `kala_agent.contracts.models.CanonicalJsonObject` limits.
///
/// Every size bound is effectively unbounded (`usize::MAX`): the Python
/// contract has no tool-protocol size caps and imposes no fixed nesting limit.
/// CPython's default 4,300-digit integer conversion limit is preserved because
/// both `json.loads(int)` and `json.dumps(int)` enforce it in the source.
pub const fn persistence_limits() -> Limits {
    Limits {
        max_json_bytes: usize::MAX,
        max_string_length: usize::MAX,
        max_collection_items: usize::MAX,
        max_key_length: usize::MAX,
        max_json_depth: MAX_PERSISTENCE_JSON_DEPTH,
        max_integer_digits: kala_json::Limits::DEFAULT_MAX_INTEGER_DIGITS,
    }
}

/// Canonicalize a persisted object supplied as JSON text.
///
/// This mirrors the string branch of `contracts.models._canonical_json_object`:
/// duplicate keys, non-finite numbers, invalid UTF-8 and trailing input are
/// refused, the top level must be an object, and the result is CPython
/// `ensure_ascii=True`, sorted, compact JSON.
pub fn canonicalize_json_object(source: &str) -> Result<String, PersistenceError> {
    let value = parse_strict(source.as_bytes(), &persistence_limits())?;
    canonicalize_parsed_object(&value)
}

/// Canonicalize a persisted object supplied as UTF-8 bytes.
pub fn canonicalize_json_object_bytes(source: &[u8]) -> Result<String, PersistenceError> {
    let value = parse_strict(source, &persistence_limits())?;
    canonicalize_parsed_object(&value)
}

/// Canonicalize an already typed value, which must be a JSON object.
pub fn canonicalize_object_value(value: &CanonicalValue) -> Result<String, PersistenceError> {
    canonicalize_parsed_object(value)
}

/// Parse a persisted object without re-serializing it.
pub fn parse_json_object(source: &str) -> Result<CanonicalValue, PersistenceError> {
    let value = parse_strict(source.as_bytes(), &persistence_limits())?;
    match value {
        CanonicalValue::Object(_) => Ok(value),
        _ => Err(PersistenceError::JsonObjectRequired),
    }
}

/// Parse UTF-8 bytes as a persisted object without re-serializing them.
pub fn parse_json_object_bytes(source: &[u8]) -> Result<CanonicalValue, PersistenceError> {
    let value = parse_strict(source, &persistence_limits())?;
    match value {
        CanonicalValue::Object(_) => Ok(value),
        _ => Err(PersistenceError::JsonObjectRequired),
    }
}

fn canonicalize_parsed_object(value: &CanonicalValue) -> Result<String, PersistenceError> {
    match value {
        CanonicalValue::Object(_) => Ok(canonical_dumps(value, &persistence_limits())?),
        _ => Err(PersistenceError::JsonObjectRequired),
    }
}

/// Whether a stored string is exactly 64 lowercase hexadecimal digits.
///
/// [`Sha256Digest::from_hex`] also accepts uppercase, so a stored chain field
/// must pass this check **before** it is converted to a typed digest.
pub fn is_lowercase_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .as_bytes()
            .iter()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Parse a stored digest after enforcing the exact lowercase representation.
pub fn parse_stored_digest(value: &str) -> Option<Sha256Digest> {
    if is_lowercase_sha256_hex(value) {
        Sha256Digest::from_hex(value)
    } else {
        None
    }
}

/// SHA-256 of the UTF-8 bytes of a JSON string.
pub fn sha256_text(value: &str) -> Sha256Digest {
    kala_json::payload_digest(value)
}
