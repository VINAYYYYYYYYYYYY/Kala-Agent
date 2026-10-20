//! Byte-compatible canonical JSON and journal hash-chain primitives.
//!
//! This crate is a native Rust reimplementation of the parts of
//! `kala_agent.tools.protocol` and `kala_agent.persistence.chain` that produce
//! persisted bytes and digests. It never shells out to Python and never calls a
//! Python runtime at production time; Python is used only by the integration
//! tests as an oracle.
//!
//! The important compatibility rules are:
//!
//! * object keys are sorted by Unicode code point,
//! * strings use CPython's `ensure_ascii=True` escaping, including UTF-16
//!   surrogate pairs for supplementary characters and lone surrogate escapes,
//! * floats use CPython's shortest round-trip `repr`, with decimal notation
//!   when the decimal exponent is in `-4..16` and scientific notation outside,
//! * integers keep arbitrary precision,
//! * duplicate keys, non-finite numbers, invalid UTF-8 and trailing input are
//!   rejected,
//! * journal digests use the exact nine length-prefixed parts and SHA-256.

#![forbid(unsafe_code)]

mod chain;
mod digest;
mod dump;
mod error;
mod float;
mod limits;
mod parse;
mod text;
mod value;

pub use chain::{
    CHAIN_ALGORITHM, CHAIN_DOMAIN, ChainVerification, ChainVerifyError, EventChainFields,
    GENESIS_PREVIOUS_DIGEST, chain_digest, chain_preimage, payload_digest, verify_chain,
};
pub use digest::Sha256Digest;
pub use dump::canonical_dumps;
pub use error::CanonicalJsonError;
pub use float::format_float;
pub use limits::Limits;
pub use parse::parse_strict;
pub use text::JsonText;
pub use value::{CanonicalObject, CanonicalValue};

/// SHA-256 of the canonical JSON encoding of a value.
///
/// The encoded bytes are always ASCII because [`canonical_dumps`] uses
/// Python's `ensure_ascii=True` escaping, so the digest can be computed
/// directly over the returned string.
pub fn canonical_digest(
    value: &CanonicalValue,
    limits: &Limits,
) -> Result<Sha256Digest, CanonicalJsonError> {
    let encoded = canonical_dumps(value, limits)?;
    Ok(digest::sha256_bytes(encoded.as_bytes()))
}

/// Canonical JSON digest with the production limits.
pub fn canonical_json_digest(value: &CanonicalValue) -> Result<Sha256Digest, CanonicalJsonError> {
    canonical_digest(value, &Limits::default())
}

/// Parse bytes and return the canonical-JSON encoding of the parsed value.
pub fn canonical_json_dumps(value: &CanonicalValue) -> Result<String, CanonicalJsonError> {
    canonical_dumps(value, &Limits::default())
}

/// Strictly parse JSON bytes using the production limits.
pub fn strict_json_loads(payload: &[u8]) -> Result<CanonicalValue, CanonicalJsonError> {
    parse_strict(payload, &Limits::default())
}
