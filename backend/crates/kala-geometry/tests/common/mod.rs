//! Shared test support: fixture loading, SHA-256, and CPython-compatible
//! compact JSON formatting.
//!
//! The float formatter is test-only. It reproduces CPython's `repr(float)`
//! shape (shortest round-trip digits, scientific notation when the decimal
//! exponent is below -4 or at least 16, `e±NN` with at least two exponent
//! digits) so arrays emitted by Rust can be hashed exactly the way the corpus
//! generator hashed Python arrays. Production canonical JSON remains owned by
//! the separate `kala-json` crate.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .to_path_buf()
}

pub fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/corpus/v1")
        .join(name)
}

pub fn load_json(name: &str) -> Value {
    let path = fixture_path(name);
    let bytes = fs::read(&path)
        .unwrap_or_else(|error| panic!("failed to read fixture {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("failed to parse fixture {}: {error}", path.display()))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut text, "{byte:02x}").unwrap();
    }
    text
}

pub fn as_f64(value: &Value) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("expected a JSON number, found {value}"))
}

pub fn as_u64(value: &Value) -> u64 {
    value
        .as_u64()
        .unwrap_or_else(|| panic!("expected a JSON unsigned integer, found {value}"))
}

pub fn as_str(value: &Value) -> &str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("expected a JSON string, found {value}"))
}

pub fn as_array(value: &Value) -> &Vec<Value> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("expected a JSON array, found {value}"))
}

pub fn as_object(value: &Value) -> &serde_json::Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("expected a JSON object, found {value}"))
}

/// Render one finite `f64` exactly as CPython `repr(float)` would.
///
/// # Panics
///
/// Panics for NaN or infinity, which the corpus never contains and Python's
/// `allow_nan=False` encoder would refuse.
pub fn python_float_repr(value: f64) -> String {
    assert!(
        value.is_finite(),
        "compact JSON hashing refuses non-finite floats"
    );
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0".to_owned()
        } else {
            "0.0".to_owned()
        };
    }

    // `ryu` chooses the shortest round-trip digits with CPython's
    // ties-to-even choice when more than one shortest decimal round-trips.
    // Rust's `LowerExp`/`Debug` formatting can choose the other tie (for
    // example the exact midpoint -142453022352814.625 becomes ...63 rather
    // than CPython's ...62), so parse ryu's shortest spelling instead.
    let mut buffer = ryu::Buffer::new();
    let rendered = buffer.format_finite(value);
    let (negative, body) = match rendered.strip_prefix('-') {
        Some(body) => (true, body),
        None => (false, rendered),
    };

    let (digits, exponent) = if let Some((mantissa, exponent)) = body.split_once('e') {
        let exponent: i32 = exponent.parse().expect("ryu exponent is an integer");
        (
            mantissa
                .chars()
                .filter(|character| *character != '.')
                .collect::<String>(),
            exponent,
        )
    } else {
        let (integer, fraction) = body.split_once('.').unwrap_or((body, ""));
        let integer = integer.trim_start_matches('0');
        if integer.is_empty() {
            // 0.dddd: the exponent is set by the leading zeros.
            let first_nonzero = fraction
                .find(|character| character != '0')
                .expect("nonzero finite float has a nonzero digit");
            let mut digits = fraction[first_nonzero..].to_owned();
            while digits.ends_with('0') {
                digits.pop();
            }
            (digits, -(first_nonzero as i32) - 1)
        } else {
            let mut digits = format!("{integer}{fraction}");
            while digits.ends_with('0') {
                digits.pop();
            }
            (digits, integer.len() as i32 - 1)
        }
    };

    let mut output = String::new();
    if negative {
        output.push('-');
    }

    if !(-4..16).contains(&exponent) {
        output.push(digits.as_bytes()[0] as char);
        if digits.len() > 1 {
            output.push('.');
            output.push_str(&digits[1..]);
        }
        output.push('e');
        if exponent < 0 {
            output.push('-');
        } else {
            output.push('+');
        }
        use std::fmt::Write as _;
        write!(&mut output, "{:02}", exponent.unsigned_abs()).unwrap();
        return output;
    }

    if exponent >= 0 {
        let integer_digits = exponent as usize + 1;
        if digits.len() <= integer_digits {
            output.push_str(&digits);
            for _ in digits.len()..integer_digits {
                output.push('0');
            }
            output.push_str(".0");
        } else {
            output.push_str(&digits[..integer_digits]);
            output.push('.');
            output.push_str(&digits[integer_digits..]);
        }
    } else {
        output.push_str("0.");
        for _ in 0..(-exponent - 1) {
            output.push('0');
        }
        output.push_str(&digits);
    }
    output
}

/// Compact JSON array of floats exactly as Python's `json.dumps` emits it.
pub fn compact_json_floats(values: &[f64]) -> Vec<u8> {
    let mut output = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&python_float_repr(*value));
    }
    output.push(']');
    output.into_bytes()
}

/// Compact JSON array of unsigned integers.
pub fn compact_json_u32(values: &[u32]) -> Vec<u8> {
    let mut output = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        use std::fmt::Write as _;
        write!(&mut output, "{value}").unwrap();
    }
    output.push(']');
    output.into_bytes()
}

pub fn digest_floats(values: &[f64]) -> String {
    sha256_hex(&compact_json_floats(values))
}

pub fn digest_u32(values: &[u32]) -> String {
    sha256_hex(&compact_json_u32(values))
}

/// Read a hex-encoded `f64` bit pattern.
pub fn f64_from_bits_hex(text: &str) -> f64 {
    let bits = u64::from_str_radix(text, 16).expect("hex f64 bits");
    f64::from_bits(bits)
}

/// Hex-encode an `f64` bit pattern for lossless oracle exchange.
pub fn f64_bits_hex(value: f64) -> String {
    format!("{:016x}", value.to_bits())
}
