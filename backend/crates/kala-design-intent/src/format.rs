//! Python float formatting and parsing used by the design modules.
//!
//! Labels, summaries, tolerance strings and parser notes interpolate numbers
//! with `:g` (default precision 6) exactly as the Python f-strings do. Numeric
//! strings accepted by Pydantic follow Python's `float()` grammar, including
//! PEP 515 underscores and Unicode decimal digits.

use crate::pytext::is_pydantic_whitespace;

/// Python `format(value, "g")`.
pub fn format_g(value: f64) -> String {
    format_g_with(value, 6)
}

/// Python `format(value, f".{precision}g")`.
pub fn format_g_with(value: f64, precision: usize) -> String {
    if value.is_nan() {
        return if value.is_sign_negative() {
            "-nan".to_owned()
        } else {
            "nan".to_owned()
        };
    }
    if value.is_infinite() {
        return if value < 0.0 {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".to_owned()
        } else {
            "0".to_owned()
        };
    }
    let precision = precision.max(1);
    // Rust's fixed-precision scientific formatting uses the same
    // correctly-rounded decimal conversion and round-half-even tie break as
    // CPython, so the significant digits are already rounded here.
    let scientific = format!("{:.*e}", precision - 1, value);
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("scientific formatting contains an exponent");
    let exponent: i32 = exponent.parse().expect("Rust prints a decimal exponent");
    let negative = mantissa.starts_with('-');
    let digits: Vec<char> = mantissa
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect();
    debug_assert_eq!(digits.len(), precision);
    let mut trimmed: &[char] = &digits;
    while trimmed.len() > 1 && trimmed[trimmed.len() - 1] == '0' {
        trimmed = &trimmed[..trimmed.len() - 1];
    }
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if exponent < -4 || exponent >= precision as i32 {
        out.push(trimmed[0]);
        if trimmed.len() > 1 {
            out.push('.');
            out.extend(trimmed[1..].iter());
        }
        out.push('e');
        out.push(if exponent < 0 { '-' } else { '+' });
        let magnitude = exponent.unsigned_abs();
        if magnitude < 10 {
            out.push('0');
        }
        out.push_str(&magnitude.to_string());
        return out;
    }
    if exponent >= 0 {
        let integer_length = exponent as usize + 1;
        if digits.len() <= integer_length {
            out.extend(digits.iter());
            for _ in digits.len()..integer_length {
                out.push('0');
            }
        } else {
            out.extend(digits[..integer_length].iter());
            out.push('.');
            out.extend(digits[integer_length..].iter());
        }
    } else {
        out.push_str("0.");
        for _ in 0..(-exponent - 1) {
            out.push('0');
        }
        out.extend(digits.iter());
    }
    while out.ends_with('0') && out.contains('.') {
        out.pop();
    }
    if out.ends_with('.') {
        out.pop();
    }
    out
}

/// Python `float(text)`, returning `None` for a `ValueError`.
///
/// The grammar matches CPython's `PyOS_string_to_double` path used by
/// `float()`: optional surrounding Unicode whitespace (not the C0 separators),
/// a sign, `inf`/`infinity`/`nan` case-insensitively, decimal digits
/// (including non-ASCII decimal digits), PEP 515 underscores between digits,
/// and an exponent.
pub(crate) fn parse_python_float(text: &str) -> Option<f64> {
    let trimmed = trim_float_whitespace(text);
    if trimmed.is_empty() {
        return None;
    }
    let mut characters: Vec<char> = trimmed.chars().collect();
    let negative = match characters.first() {
        Some('+') => {
            characters.remove(0);
            false
        }
        Some('-') => {
            characters.remove(0);
            true
        }
        _ => false,
    };
    if characters.is_empty() {
        return None;
    }
    let body: String = characters.iter().collect();
    let lowered = body.to_ascii_lowercase();
    if lowered == "inf" || lowered == "infinity" {
        return Some(if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    if lowered == "nan" {
        return Some(if negative { -f64::NAN } else { f64::NAN });
    }
    let normalized = normalize_decimal_literal(&characters)?;
    let parsed: f64 = normalized.parse().ok()?;
    Some(if negative { -parsed } else { parsed })
}

/// CPython's `float()` trimming: Unicode `White_Space` only, so U+001C is a
/// `ValueError` rather than ignored whitespace.
fn trim_float_whitespace(text: &str) -> &str {
    text.trim_matches(is_pydantic_whitespace)
}

fn is_digit(character: char) -> bool {
    digit_value(character).is_some()
}

/// Unicode `Nd` decimal value, matching `Py_UNICODE_TODECIMAL`.
fn digit_value(character: char) -> Option<u32> {
    if character.is_ascii_digit() {
        return Some(u32::from(character) - u32::from('0'));
    }
    let point = u32::from(character);
    // The mathematical digit blocks are five contiguous runs of ten digits.
    if (0x1D7CE..=0x1D7FF).contains(&point) {
        return Some((point - 0x1D7CE) % 10);
    }
    const STARTS: &[u32] = &[
        0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66, 0x0CE6,
        0x0D66, 0x0DE6, 0x0E50, 0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0,
        0x1A80, 0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0,
        0xAA50, 0xABF0, 0xFF10, 0x104A0, 0x10D30, 0x11066, 0x110F0, 0x11136, 0x111D0, 0x112F0,
        0x11450, 0x114D0, 0x11650, 0x116C0, 0x11730, 0x118E0, 0x11950, 0x11C50, 0x11D50, 0x11DA0,
        0x11F50, 0x16A60, 0x16AC0, 0x16B50, 0x1E140, 0x1E2F0, 0x1E4F0, 0x1E950, 0x1FBF0,
    ];
    for start in STARTS {
        if (*start..*start + 10).contains(&point) {
            return Some(point - start);
        }
    }
    None
}

/// Rewrite a run of Unicode decimal digits as ASCII digits.
pub(crate) fn ascii_digits(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        let value = digit_value(character)?;
        out.push(char::from_digit(value, 10)?);
    }
    Some(out)
}

/// Rewrite a decimal literal to ASCII, validating Python's grammar.
fn normalize_decimal_literal(characters: &[char]) -> Option<String> {
    let mut out = String::with_capacity(characters.len());
    let mut index = 0;
    let mut saw_integer_digit = false;
    let mut saw_fraction_digit = false;

    if index < characters.len() && is_digit(characters[index]) {
        saw_integer_digit = true;
        out.push(char::from_digit(digit_value(characters[index])?, 10)?);
        index += 1;
        while index < characters.len() {
            let next = characters[index];
            if is_digit(next) {
                out.push(char::from_digit(digit_value(next)?, 10)?);
                index += 1;
            } else if next == '_' && index + 1 < characters.len() && is_digit(characters[index + 1])
            {
                index += 1;
            } else {
                break;
            }
        }
    }
    if !saw_integer_digit {
        // A leading decimal point is allowed: `.5`.
        if characters.first() != Some(&'.') {
            return None;
        }
    }
    if index < characters.len() && characters[index] == '.' {
        out.push('.');
        index += 1;
        while index < characters.len() {
            let next = characters[index];
            if is_digit(next) {
                saw_fraction_digit = true;
                out.push(char::from_digit(digit_value(next)?, 10)?);
                index += 1;
            } else if next == '_' && index + 1 < characters.len() && is_digit(characters[index + 1])
            {
                index += 1;
            } else {
                break;
            }
        }
    }
    if !saw_integer_digit && !saw_fraction_digit {
        return None;
    }
    if index < characters.len() && matches!(characters[index], 'e' | 'E') {
        let mut exponent = String::from("e");
        index += 1;
        if index < characters.len() && matches!(characters[index], '+' | '-') {
            exponent.push(characters[index]);
            index += 1;
        }
        let mut saw_exponent_digit = false;
        while index < characters.len() {
            let next = characters[index];
            if is_digit(next) {
                saw_exponent_digit = true;
                exponent.push(char::from_digit(digit_value(next)?, 10)?);
                index += 1;
            } else if next == '_' && index + 1 < characters.len() && is_digit(characters[index + 1])
            {
                index += 1;
            } else {
                break;
            }
        }
        if !saw_exponent_digit {
            return None;
        }
        out.push_str(&exponent);
    }
    if index != characters.len() {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_format_matches_python_examples() {
        let cases = [
            (0.0, "0"),
            (-0.0, "-0"),
            (1.0, "1"),
            (1.5, "1.5"),
            (60.0, "60"),
            (5.5, "5.5"),
            (1_000_000.0, "1e+06"),
            (999_999.5, "1e+06"),
            (999_999.4, "999999"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (0.000_123_456_789, "0.000123457"),
            (1e-7, "1e-07"),
            (1e20, "1e+20"),
            (123_456_789.0, "1.23457e+08"),
            (1.234_567_89, "1.23457"),
            (100.0, "100"),
            (0.1 + 0.2, "0.3"),
            (2.0 / 3.0, "0.666667"),
            (-2.0 / 3.0, "-0.666667"),
            (1.0 / 3.0, "0.333333"),
            (1_234.567_890_123, "1234.57"),
            (0.999_999_9, "1"),
            (9.999_999, "10"),
            (99.999_999, "100"),
            (999_999.9, "1e+06"),
        ];
        for (value, expected) in cases {
            assert_eq!(format_g(value), expected, "format_g({value:?})");
        }
    }

    #[test]
    fn float_grammar_matches_python() {
        assert_eq!(parse_python_float("60"), Some(60.0));
        assert_eq!(parse_python_float(" 60.5 "), Some(60.5));
        assert_eq!(parse_python_float("6_0"), Some(60.0));
        assert_eq!(parse_python_float("1_0.5"), Some(10.5));
        assert_eq!(parse_python_float("1e1_0"), Some(1e10));
        assert_eq!(parse_python_float(".5"), Some(0.5));
        assert_eq!(parse_python_float("5."), Some(5.0));
        assert_eq!(parse_python_float("١٢٣"), Some(123.0));
        assert_eq!(parse_python_float("١٢.٥"), Some(12.5));
        assert!(parse_python_float("inf").unwrap().is_infinite());
        assert!(parse_python_float("-Infinity").unwrap().is_infinite());
        assert!(parse_python_float("nan").unwrap().is_nan());
        for bad in ["", "60_", "_60", "1__0", "1e", "1e+", "0x10", "\u{1c}60"] {
            assert_eq!(parse_python_float(bad), None, "{bad:?}");
        }
    }
}
