//! Duration handling: the kernel keeps durations on a whole-microsecond grid
//! using Python's `round(seconds * 1_000_000)` (ties to even) and stores the
//! result as a float `microseconds / 1_000_000` exactly like the Python
//! kernel does.

use crate::errors::BudgetError;
use num_bigint::BigInt;
use num_traits::Zero;
use std::str::FromStr;

/// Microseconds per second, matching `_MICROSECONDS_PER_SECOND`.
pub const MICROSECONDS_PER_SECOND: u64 = 1_000_000;

/// Error from parsing a Python-style float string.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FloatParseError {
    /// The text is not a valid Python float literal.
    #[error("float literal is invalid")]
    Invalid,
}

/// Validate and grid-round a duration exactly like `_nonnegative_duration`.
///
/// Python accepts ints, floats, decimal strings and rejects booleans,
/// non-finite values and negative values. Negative zero is accepted and
/// normalizes to positive zero through the microsecond grid.
pub fn normalize_duration(seconds: f64) -> Result<f64, BudgetError> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(BudgetError::Validation(
            "duration_seconds must be finite and nonnegative".to_owned(),
        ));
    }
    let micros = duration_microseconds(seconds)?;
    seconds_from_microseconds(&micros)
}

/// Python `round(seconds * 1_000_000)` returning an arbitrary-precision
/// integer, or `DurationRange` where Python raises `OverflowError`.
pub fn duration_microseconds(seconds: f64) -> Result<BigInt, BudgetError> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(BudgetError::Validation(
            "duration_seconds must be finite and nonnegative".to_owned(),
        ));
    }
    let scaled = seconds * MICROSECONDS_PER_SECOND as f64;
    if !scaled.is_finite() {
        // Python raises OverflowError from `round(inf)`; surface an explicit
        // error instead of saturating.
        return Err(BudgetError::DurationRange);
    }
    Ok(integral_f64_to_bigint(scaled.round_ties_even()))
}

/// Python `microseconds / 1_000_000` as a correctly rounded float.
pub fn seconds_from_microseconds(microseconds: &BigInt) -> Result<f64, BudgetError> {
    if microseconds.is_zero() {
        return Ok(0.0);
    }
    let text = format!("{microseconds}e-6");
    let value = f64::from_str(&text).map_err(|_| BudgetError::DurationRange)?;
    if !value.is_finite() {
        return Err(BudgetError::DurationRange);
    }
    Ok(value)
}

/// Convert an integral (fract == 0) finite float to an exact `BigInt`.
pub(crate) fn integral_f64_to_bigint(value: f64) -> BigInt {
    if value == 0.0 {
        return BigInt::zero();
    }
    let bits = value.to_bits();
    let negative = (bits >> 63) == 1;
    let raw_exponent = ((bits >> 52) & 0x7ff) as i64;
    let raw_mantissa = bits & ((1u64 << 52) - 1);
    let (mantissa, exponent) = if raw_exponent == 0 {
        (raw_mantissa, -1074i64)
    } else {
        (raw_mantissa | (1u64 << 52), raw_exponent - 1075)
    };
    let magnitude = if exponent >= 0 {
        BigInt::from(mantissa) << u32::try_from(exponent).unwrap_or(0)
    } else {
        BigInt::from(mantissa) >> u32::try_from(-exponent).unwrap_or(0)
    };
    if negative { -magnitude } else { magnitude }
}

/// Parse a float the way Python's `float(text)` does for the accepted
/// budget inputs (including non-finite literals; callers validate finiteness).
///
/// Python normalizes Unicode `Nd` digits to ASCII before parsing and allows
/// underscores only between digits. Its whitespace set is the Unicode
/// `White_Space` property (`char::is_whitespace`), which deliberately differs
/// from [`crate::Decimal`] parsing: the ASCII separators `U+001C..U+001F` are
/// *not* accepted here.
pub fn python_float_parse(text: &str) -> Result<f64, FloatParseError> {
    let trimmed = text.trim_matches(char::is_whitespace);
    let (negative, body) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    let lower = body.to_ascii_lowercase();
    if lower == "inf" || lower == "infinity" {
        return Ok(if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    if lower == "nan" {
        return Ok(f64::NAN);
    }
    if body.is_empty() {
        return Err(FloatParseError::Invalid);
    }

    let characters: Vec<char> = body.chars().collect();
    let mut cleaned = String::with_capacity(body.len() + 1);
    for (index, character) in characters.iter().enumerate() {
        if *character == '_' {
            let previous_is_digit =
                index > 0 && crate::unicode::decimal_digit_ascii(characters[index - 1]).is_some();
            let next_is_digit = index + 1 < characters.len()
                && crate::unicode::decimal_digit_ascii(characters[index + 1]).is_some();
            if !previous_is_digit || !next_is_digit {
                return Err(FloatParseError::Invalid);
            }
            continue;
        }
        match crate::unicode::decimal_digit_ascii(*character) {
            Some(digit) => cleaned.push(char::from(b'0' + digit)),
            None => cleaned.push(*character),
        }
    }
    if cleaned.is_empty() {
        return Err(FloatParseError::Invalid);
    }
    let value = f64::from_str(&cleaned).map_err(|_| FloatParseError::Invalid)?;
    Ok(if negative { -value } else { value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grids_durations_like_cpython_round() {
        assert_eq!(normalize_duration(0.0).expect("zero"), 0.0);
        assert_eq!(normalize_duration(-0.0).expect("negative zero"), 0.0);
        assert_eq!(normalize_duration(1e-7).expect("round down"), 0.0);
        assert_eq!(normalize_duration(5e-7).expect("tie to even"), 0.0);
        assert_eq!(normalize_duration(1.5e-6).expect("tie to even"), 2e-6);
        assert_eq!(normalize_duration(1.2345675).expect("round"), 1.234568);
        assert_eq!(normalize_duration(123.4567895).expect("round"), 123.45679);
        assert_eq!(normalize_duration(3600.0).expect("default"), 3600.0);
    }

    #[test]
    fn rejects_negative_non_finite_and_overflowing_durations() {
        assert!(normalize_duration(-1.0).is_err());
        assert!(normalize_duration(f64::NAN).is_err());
        assert!(normalize_duration(f64::INFINITY).is_err());
        assert_eq!(
            normalize_duration(1e308).unwrap_err(),
            BudgetError::DurationRange
        );
    }

    #[test]
    fn sums_microsecond_grids_without_wrapping() {
        let left = duration_microseconds(0.0000005).expect("micros");
        let right = duration_microseconds(0.0000005).expect("micros");
        assert_eq!(
            seconds_from_microseconds(&(left + right)).expect("sum"),
            0.0
        );
        let huge = duration_microseconds(1e300).expect("micros");
        assert!(seconds_from_microseconds(&huge).expect("seconds") > 0.0);
    }

    #[test]
    fn parses_python_float_text() {
        assert_eq!(python_float_parse(" 1.5 ").expect("float"), 1.5);
        assert_eq!(python_float_parse("1_000").expect("underscore"), 1000.0);
        assert!(python_float_parse("1__0").is_err());
        assert!(python_float_parse("nan").expect("nan").is_nan());
        assert!(python_float_parse("inf").expect("inf").is_infinite());
    }

    #[test]
    fn parses_unicode_digits_and_underscores_like_python_float() {
        assert_eq!(
            python_float_parse("\u{0661}.\u{0662}\u{0663}").expect("arabic"),
            1.23
        );
        assert_eq!(
            python_float_parse("\u{FF11}\u{FF12}.\u{FF13}\u{FF14}").expect("fullwidth"),
            12.34
        );
        assert_eq!(
            python_float_parse("\u{1D7D9}\u{1D7DA}.\u{1D7DB}").expect("math digits"),
            12.3
        );
        assert_eq!(
            python_float_parse("1e\u{0662}").expect("unicode exponent"),
            100.0
        );
        assert_eq!(
            python_float_parse("\u{0661}_\u{0662}").expect("unicode underscores"),
            12.0
        );
        assert_eq!(
            python_float_parse("1_0.0_0e1_0").expect("many underscores"),
            100_000_000_000.0
        );
        assert_eq!(
            python_float_parse("\u{A0}1\u{3000}").expect("unicode spaces"),
            1.0
        );
        // Python float accepts the Unicode whitespace set but not the ASCII
        // separators U+001C..U+001F that `Decimal` trims.
        assert!(python_float_parse("\u{85}1\u{85}").is_ok());
        assert!(python_float_parse("\u{1C}1\u{1C}").is_err());
        // Value-preserving forms and lookalikes.
        assert_eq!(python_float_parse("1.").expect("trailing point"), 1.0);
        assert_eq!(python_float_parse(".5").expect("leading point"), 0.5);
        assert_eq!(
            python_float_parse("1.e2").expect("point before exponent"),
            100.0
        );
        assert!(python_float_parse("\u{00B2}").is_err());
        assert!(python_float_parse("1_").is_err());
        assert!(python_float_parse("1._0").is_err());
        assert!(python_float_parse("1e_1").is_err());
        assert!(python_float_parse("1e1__0").is_err());
    }
}
