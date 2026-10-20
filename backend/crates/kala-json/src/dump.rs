use std::fmt::Write as _;

use crate::error::CanonicalJsonError;
use crate::float::format_float;
use crate::limits::{Limits, check_depth};
use crate::text::JsonText;
use crate::value::CanonicalValue;

/// One unit of iterative validation work.
enum Check<'a> {
    /// Validate a value at a nesting depth.
    Value(&'a CanonicalValue, usize),
    /// Validate one object key at its entry's position in traversal order.
    Key(&'a JsonText),
}

/// Validate a typed value against the bounded protocol limits.
///
/// Values are checked depth-first in the same order as
/// `kala_agent.tools.protocol._normalize_json`: depth first, then scalar or
/// container constraints, then children. Object iteration order is code-point
/// order rather than JSON input order, which only affects which of several
/// independent errors is reported first.
///
/// The traversal uses an explicit heap stack, so a deeply nested stored object
/// cannot overflow the thread stack while being checked.
pub(crate) fn validate(
    value: &CanonicalValue,
    depth: usize,
    limits: &Limits,
) -> Result<(), CanonicalJsonError> {
    let mut stack = vec![Check::Value(value, depth)];
    while let Some(check) = stack.pop() {
        match check {
            Check::Key(key) => {
                if key.len() > limits.max_key_length {
                    return Err(CanonicalJsonError::KeyTooLong);
                }
            }
            Check::Value(value, depth) => {
                check_depth(depth, limits)?;
                match value {
                    CanonicalValue::Null | CanonicalValue::Bool(_) => {}
                    CanonicalValue::Integer(number) => {
                        // CPython 3.11+ refuses int-to-str conversions above
                        // `sys.int_info.default_max_str_digits` (4300 by
                        // default).
                        if limits.max_integer_digits > 0 {
                            let rendered = number.to_str_radix(10);
                            let digits = rendered.trim_start_matches('-').len();
                            if digits > limits.max_integer_digits {
                                return Err(CanonicalJsonError::NotCanonicalizable);
                            }
                        }
                    }
                    CanonicalValue::Float(number) => {
                        if !number.is_finite() {
                            return Err(CanonicalJsonError::NonFinite);
                        }
                    }
                    CanonicalValue::Text(text) => {
                        if text.len() > limits.max_string_length {
                            return Err(CanonicalJsonError::StringTooLong);
                        }
                    }
                    CanonicalValue::Array(values) => {
                        if values.len() > limits.max_collection_items {
                            return Err(CanonicalJsonError::CollectionTooLarge);
                        }
                        let child_depth = depth.saturating_add(1);
                        for item in values.iter().rev() {
                            stack.push(Check::Value(item, child_depth));
                        }
                    }
                    CanonicalValue::Object(entries) => {
                        // Python applies MAX_COLLECTION_ITEMS only to arrays,
                        // not objects; object keys are bounded by
                        // MAX_KEY_LENGTH instead. Push the value before the
                        // key so the key check runs first in document order.
                        let child_depth = depth.saturating_add(1);
                        for (key, item) in entries.iter().rev() {
                            stack.push(Check::Value(item, child_depth));
                            stack.push(Check::Key(key));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// One unit of iterative encoding work.
enum Encode<'a> {
    Value(&'a CanonicalValue),
    Key(&'a JsonText),
    Literal(&'static str),
}

/// Serialize a value to canonical JSON bytes as a `String`.
///
/// The result is pure ASCII (`ensure_ascii=True`), keys are sorted by Unicode
/// code point, and there is no insignificant whitespace. `limits.max_json_bytes`
/// is applied to the encoded canonical bytes, not to an unescaped estimate.
/// Validation and encoding both use explicit heap stacks, so arbitrarily deep
/// stored values are encoded without recursion.
pub fn canonical_dumps(
    value: &CanonicalValue,
    limits: &Limits,
) -> Result<String, CanonicalJsonError> {
    validate(value, 0, limits)?;
    let mut out = String::new();
    let mut stack = vec![Encode::Value(value)];
    while let Some(task) = stack.pop() {
        match task {
            Encode::Literal(literal) => out.push_str(literal),
            Encode::Key(key) => encode_text(key.code_points(), &mut out),
            Encode::Value(value) => match value {
                CanonicalValue::Null => out.push_str("null"),
                CanonicalValue::Bool(true) => out.push_str("true"),
                CanonicalValue::Bool(false) => out.push_str("false"),
                CanonicalValue::Integer(number) => {
                    // The int-to-str digit limit was checked by `validate`.
                    let _ = write!(out, "{number}");
                }
                CanonicalValue::Float(number) => {
                    // `validate` has already rejected non-finite values.
                    let rendered = format_float(*number).expect("finite float");
                    out.push_str(&rendered);
                }
                CanonicalValue::Text(text) => encode_text(text.code_points(), &mut out),
                CanonicalValue::Array(values) => {
                    out.push('[');
                    stack.push(Encode::Literal("]"));
                    for (index, item) in values.iter().enumerate().rev() {
                        stack.push(Encode::Value(item));
                        if index > 0 {
                            stack.push(Encode::Literal(","));
                        }
                    }
                }
                CanonicalValue::Object(entries) => {
                    out.push('{');
                    stack.push(Encode::Literal("}"));
                    for (index, (key, item)) in entries.iter().enumerate().rev() {
                        stack.push(Encode::Value(item));
                        stack.push(Encode::Literal(":"));
                        stack.push(Encode::Key(key));
                        if index > 0 {
                            stack.push(Encode::Literal(","));
                        }
                    }
                }
            },
        }
    }
    if out.len() > limits.max_json_bytes {
        return Err(CanonicalJsonError::OutputTooLarge);
    }
    Ok(out)
}

/// CPython's `json.dumps(..., ensure_ascii=True)` string escaping.
fn encode_text(code_points: &[u32], out: &mut String) {
    out.push('"');
    for point in code_points {
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
            0x7F..=0xFFFF => {
                let _ = write!(out, "\\u{point:04x}");
            }
            _ => {
                let value = point - 0x1_0000;
                let high = 0xD800 + (value >> 10);
                let low = 0xDC00 + (value & 0x3FF);
                let _ = write!(out, "\\u{high:04x}\\u{low:04x}");
            }
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use num_bigint::BigInt;

    use super::*;
    use crate::text::JsonText;

    fn text(points: &[u32]) -> CanonicalValue {
        CanonicalValue::Text(JsonText::from_code_points(points.to_vec()).unwrap())
    }

    #[test]
    fn escapes_like_cpython() {
        let value = CanonicalValue::array([
            text(&[0x00, 0x1F, 0x7F]),
            text(&[b'"' as u32, b'\\' as u32, b'/' as u32]),
            text(&[0x03BA, 0x1F600, 0xD800]),
        ]);
        assert_eq!(
            canonical_dumps(&value, &Limits::default()).unwrap(),
            r#"["\u0000\u001f\u007f","\"\\/","\u03ba\ud83d\ude00\ud800"]"#
        );
    }

    #[test]
    fn sorts_keys_by_code_point_not_utf8_escape() {
        let value = CanonicalValue::object([
            ("z", CanonicalValue::from(1)),
            ("a", CanonicalValue::from(3)),
            ("ä", CanonicalValue::from(2)),
        ]);
        assert_eq!(
            canonical_dumps(&value, &Limits::default()).unwrap(),
            r#"{"a":3,"z":1,"\u00e4":2}"#
        );
    }

    #[test]
    fn preserves_integer_float_distinction_and_bigints() {
        let value = CanonicalValue::array([
            CanonicalValue::Integer(BigInt::from(1)),
            CanonicalValue::Float(1.0),
            CanonicalValue::Float(-0.0),
            CanonicalValue::Integer(BigInt::parse_bytes(b"1180591620717411303424", 10).unwrap()),
        ]);
        assert_eq!(
            canonical_dumps(&value, &Limits::default()).unwrap(),
            "[1,1.0,-0.0,1180591620717411303424]"
        );
    }

    #[test]
    fn enforces_output_byte_and_string_limits() {
        let value = CanonicalValue::from("aaaa");
        let limits = Limits {
            max_json_bytes: 5,
            ..Limits::default()
        };
        assert_eq!(
            canonical_dumps(&value, &limits),
            Err(CanonicalJsonError::OutputTooLarge)
        );
        let limits = Limits {
            max_string_length: 3,
            ..Limits::default()
        };
        assert_eq!(
            canonical_dumps(&value, &limits),
            Err(CanonicalJsonError::StringTooLong)
        );
    }

    #[test]
    fn collection_limit_applies_to_arrays_but_not_objects() {
        let array = CanonicalValue::Array(vec![CanonicalValue::Null; 2_049]);
        assert_eq!(
            canonical_dumps(&array, &Limits::default()),
            Err(CanonicalJsonError::CollectionTooLarge)
        );
        // Python's `_normalize_json` bounds mapping key length but not the
        // number of object entries.
        let object = CanonicalValue::object(
            (0..2_049).map(|index| (format!("k{index}"), CanonicalValue::Null)),
        );
        let rendered = canonical_dumps(&object, &Limits::default()).unwrap();
        assert!(rendered.starts_with("{\"k0\":null,"));
    }

    #[test]
    fn enforces_cpython_integer_digit_limit() {
        let too_wide =
            CanonicalValue::Integer(BigInt::parse_bytes("9".repeat(4301).as_bytes(), 10).unwrap());
        assert_eq!(
            canonical_dumps(&too_wide, &Limits::default()),
            Err(CanonicalJsonError::NotCanonicalizable)
        );
        let allowed =
            CanonicalValue::Integer(BigInt::parse_bytes("9".repeat(4300).as_bytes(), 10).unwrap());
        assert_eq!(
            canonical_dumps(&allowed, &Limits::default()).unwrap().len(),
            4300
        );
    }

    #[test]
    fn enforces_ascii_byte_cap_not_code_point_estimate() {
        // Four Greek letters are four code points but 26 canonical ASCII bytes.
        let value = text(&[0x03BA, 0x03BA, 0x03BA, 0x03BA]);
        let limits = Limits {
            max_json_bytes: 25,
            ..Limits::default()
        };
        assert_eq!(
            canonical_dumps(&value, &limits),
            Err(CanonicalJsonError::OutputTooLarge)
        );
        let limits = Limits {
            max_json_bytes: 26,
            ..Limits::default()
        };
        assert_eq!(
            canonical_dumps(&value, &limits).unwrap(),
            r#""\u03ba\u03ba\u03ba\u03ba""#
        );
    }
}
