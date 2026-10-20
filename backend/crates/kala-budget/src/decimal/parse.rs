//! Decimal literal parsing compatible with `decimal.Decimal(str)`.
//!
//! CPython first applies `_PyUnicode_TransformDecimalAndSpaceToASCII`, which
//! maps every Unicode `Nd` digit to the matching ASCII digit and then strips
//! leading/trailing whitespace and all underscores, before handing the string
//! to `libmpdec`'s `mpd_qset_string`. The `libmpdec` construction bounds are
//! checked on the *stored* exponent (the written exponent minus the number of
//! fractional digits) and on the adjusted exponent of a nonzero coefficient:
//!
//! - `MIN_ETINY <= stored exponent <= MAX_EMAX`
//! - for a nonzero coefficient, `stored exponent + digits - 1 <= MAX_EMAX`
//!
//! Written exponents are compared as decimal text, so a million-digit exponent
//! never allocates a big integer just to be rejected.

use super::{Decimal, PYTHON_MAX_EMAX, PYTHON_MIN_ETINY};
use crate::unicode;
use num_bigint::BigUint;

/// Errors from parsing a decimal literal.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DecimalParseError {
    /// The text is not a valid finite decimal literal.
    #[error("decimal string is not a valid decimal literal")]
    InvalidSyntax,
    /// The text is a non-finite literal (`NaN`, `sNaN`, `Infinity`).
    #[error("decimal string denotes a non-finite value")]
    NonFinite,
    /// The literal exponent does not fit the supported range.
    #[error("decimal exponent is outside the supported range")]
    ExponentOutOfRange,
}

pub(super) fn parse_decimal(text: &str) -> Result<Decimal, DecimalParseError> {
    // `decimal.Decimal` trims its own whitespace set, drops every underscore
    // and maps Unicode `Nd` digits to ASCII before the grammar is checked.
    let mut normalized = String::with_capacity(text.len());
    for character in text
        .trim_matches(|character: char| unicode::is_python_decimal_space(character))
        .chars()
    {
        if character == '_' {
            continue;
        }
        match unicode::decimal_digit_ascii(character) {
            Some(digit) => normalized.push(char::from(b'0' + digit)),
            None => normalized.push(character),
        }
    }
    parse_normalized(&normalized)
}

/// Parse a string that already had whitespace, underscores and Unicode digits
/// normalized, exactly like the transformed input `libmpdec` receives.
fn parse_normalized(text: &str) -> Result<Decimal, DecimalParseError> {
    let (negative, rest) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    if rest.is_empty() {
        return Err(DecimalParseError::InvalidSyntax);
    }
    if is_non_finite(rest) {
        return Err(DecimalParseError::NonFinite);
    }

    let (mantissa, exponent_text) = split_exponent(rest)?;
    let (integer_digits, fraction_digits) = split_mantissa(mantissa)?;
    let mut digits = String::with_capacity(integer_digits.len() + fraction_digits.len());
    digits.push_str(&integer_digits);
    digits.push_str(&fraction_digits);

    let stored_exponent = stored_exponent(exponent_text, fraction_digits.len())?;

    let coefficient =
        BigUint::parse_bytes(digits.as_bytes(), 10).ok_or(DecimalParseError::InvalidSyntax)?;
    if coefficient != BigUint::default() {
        let significant_digits =
            digits.len() - digits.bytes().take_while(|byte| *byte == b'0').count();
        let adjusted = stored_exponent + significant_digits as i128 - 1;
        if adjusted > i128::from(PYTHON_MAX_EMAX) {
            return Err(DecimalParseError::ExponentOutOfRange);
        }
    }

    let exponent =
        i64::try_from(stored_exponent).map_err(|_| DecimalParseError::ExponentOutOfRange)?;
    Ok(Decimal::from_parts(negative, coefficient, exponent))
}

/// Case-insensitive non-finite literals, including the `NaN`/`sNaN` payload
/// forms `decimal.Decimal` accepts (`nan123`, `snan123`).
fn is_non_finite(rest: &str) -> bool {
    let lower = rest.to_ascii_lowercase();
    if matches!(lower.as_str(), "inf" | "infinity" | "nan" | "snan") {
        return true;
    }
    ["nan", "snan"].iter().any(|prefix| {
        lower.strip_prefix(prefix).is_some_and(|payload| {
            !payload.is_empty() && payload.bytes().all(|byte| byte.is_ascii_digit())
        })
    })
}

/// Split the optional `e`/`E` exponent suffix, rejecting a second exponent
/// marker or an empty exponent.
fn split_exponent(rest: &str) -> Result<(&str, Option<&str>), DecimalParseError> {
    match rest.find(['e', 'E']) {
        Some(index) => {
            let exponent = &rest[index + 1..];
            if exponent.contains(['e', 'E']) {
                return Err(DecimalParseError::InvalidSyntax);
            }
            Ok((&rest[..index], Some(exponent)))
        }
        None => Ok((rest, None)),
    }
}

/// Split the mantissa into integer and fraction digit text, requiring the
/// CPython grammar: at least one digit, at most one decimal point.
fn split_mantissa(mantissa: &str) -> Result<(String, String), DecimalParseError> {
    let mut integer_digits = String::new();
    let mut fraction_digits = String::new();
    let mut seen_point = false;
    for character in mantissa.chars() {
        match character {
            '.' if !seen_point => seen_point = true,
            '.' => return Err(DecimalParseError::InvalidSyntax),
            digit if digit.is_ascii_digit() => {
                if seen_point {
                    fraction_digits.push(digit);
                } else {
                    integer_digits.push(digit);
                }
            }
            _ => return Err(DecimalParseError::InvalidSyntax),
        }
    }
    if integer_digits.is_empty() && fraction_digits.is_empty() {
        return Err(DecimalParseError::InvalidSyntax);
    }
    Ok((integer_digits, fraction_digits))
}

/// Validate the written exponent and return the stored exponent
/// (`written exponent - fractional digit count`) as an `i128`.
///
/// The written exponent is compared as text first, so exponents with billions
/// of digits are rejected without allocating a multi-gigabyte integer.
fn stored_exponent(
    exponent_text: Option<&str>,
    fraction_digits: usize,
) -> Result<i128, DecimalParseError> {
    let Some(text) = exponent_text else {
        let stored = -(i128::try_from(fraction_digits)
            .map_err(|_| DecimalParseError::ExponentOutOfRange)?);
        return check_stored_exponent(stored);
    };

    let (negative, magnitude_text) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    if magnitude_text.is_empty() || !magnitude_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(DecimalParseError::InvalidSyntax);
    }

    let fraction =
        i128::try_from(fraction_digits).map_err(|_| DecimalParseError::ExponentOutOfRange)?;
    let significant = magnitude_text.trim_start_matches('0');
    let magnitude = if significant.is_empty() {
        0
    } else if significant.len() > 19 {
        // The largest accepted magnitude is MIN_ETINY's 19 digits
        // (1_999_999_999_999_999_997), so a longer literal cannot be stored.
        return Err(DecimalParseError::ExponentOutOfRange);
    } else {
        significant
            .parse::<u128>()
            .map_err(|_| DecimalParseError::ExponentOutOfRange)?
    };

    let stored = if negative {
        let allowed = MIN_ETINY_MAGNITUDE.saturating_sub(fraction as u128);
        if magnitude > allowed {
            return Err(DecimalParseError::ExponentOutOfRange);
        }
        -(magnitude as i128) - fraction
    } else {
        let allowed = PYTHON_MAX_EMAX as u128 + fraction as u128;
        if magnitude > allowed {
            return Err(DecimalParseError::ExponentOutOfRange);
        }
        magnitude as i128 - fraction
    };
    check_stored_exponent(stored)
}

/// CPython's `MIN_ETINY` magnitude: `-MIN_ETINY` (19 digits).
const MIN_ETINY_MAGNITUDE: u128 = 1_999_999_999_999_999_997;

fn check_stored_exponent(stored: i128) -> Result<i128, DecimalParseError> {
    if stored < i128::from(PYTHON_MIN_ETINY) || stored > i128::from(PYTHON_MAX_EMAX) {
        return Err(DecimalParseError::ExponentOutOfRange);
    }
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> (bool, String, i64) {
        let value = parse_decimal(text).expect("valid decimal");
        (
            value.is_negative(),
            value.coefficient().to_str_radix(10),
            value.exponent(),
        )
    }

    #[test]
    fn parses_plain_and_scientific_forms() {
        assert_eq!(parsed("0"), (false, "0".to_owned(), 0));
        assert_eq!(parsed("-0.00"), (true, "0".to_owned(), -2));
        assert_eq!(parsed(".5"), (false, "5".to_owned(), -1));
        assert_eq!(parsed("5."), (false, "5".to_owned(), 0));
        assert_eq!(parsed("+1_000.0_1"), (false, "100001".to_owned(), -2));
        assert_eq!(parsed("1e-7"), (false, "1".to_owned(), -7));
        assert_eq!(parsed("1E+3"), (false, "1".to_owned(), 3));
        assert_eq!(parsed("0E-1000000"), (false, "0".to_owned(), -1_000_000));
    }

    #[test]
    fn parses_whitespace_and_underscores_like_cpython() {
        assert_eq!(parsed("\t1 \n"), (false, "1".to_owned(), 0));
        assert_eq!(parsed("_1__0_"), (false, "10".to_owned(), 0));
        assert_eq!(parsed("1._5"), (false, "15".to_owned(), -1));
        assert_eq!(parsed("1_._5e_2"), (false, "15".to_owned(), 1));
        assert_eq!(parsed("_+1"), (false, "1".to_owned(), 0));
        assert_eq!(parsed("-_1"), (true, "1".to_owned(), 0));
        // CPython's Decimal whitespace set includes U+001C..U+001F, which
        // `char::is_whitespace` does not report.
        assert_eq!(parsed("\u{1C}1\u{1C}"), (false, "1".to_owned(), 0));
        assert_eq!(parsed("\u{1D}1\u{1F}"), (false, "1".to_owned(), 0));
        assert_eq!(parsed("\u{A0}1\u{3000}"), (false, "1".to_owned(), 0));
    }

    #[test]
    fn parses_unicode_decimal_digits_like_cpython() {
        // Arabic-Indic, Extended Arabic-Indic, fullwidth, mathematical
        // double-struck and a supplementary multi-cycle block.
        assert_eq!(
            parsed("\u{0661}.\u{0662}\u{0663}"),
            (false, "123".to_owned(), -2)
        );
        assert_eq!(parsed("\u{06F1}\u{06F2}"), (false, "12".to_owned(), 0));
        assert_eq!(
            parsed("\u{FF11}\u{FF12}.\u{FF13}\u{FF14}"),
            (false, "1234".to_owned(), -2)
        );
        assert_eq!(
            parsed("\u{1D7D9}\u{1D7DA}.\u{1D7DB}"),
            (false, "123".to_owned(), -1)
        );
        assert_eq!(parsed("1e\u{0662}"), (false, "1".to_owned(), 2));
        assert_eq!(parsed("\u{0661}e\u{FF12}"), (false, "1".to_owned(), 2));
        assert_eq!(parsed("\u{0661}_\u{0662}"), (false, "12".to_owned(), 0));
        // Non-`Nd` numeric lookalikes must stay rejected.
        for text in ["\u{00B2}", "\u{00BD}", "\u{2160}", "\u{3007}", "\u{1F100}"] {
            assert_eq!(
                parse_decimal(text).unwrap_err(),
                DecimalParseError::InvalidSyntax,
                "{text:?}"
            );
        }
    }

    #[test]
    fn rejects_non_finite_and_malformed_literals() {
        for text in [
            "", " ", "_", ".", "1e", "1e+", "0x10", "- 1", "1.2.3", "nan!", "1e+_", "1__e",
        ] {
            assert!(parse_decimal(text).is_err(), "{text} should be rejected");
        }
        for text in ["NaN", "sNaN123", "-Infinity", "inf", "n_a_n", "nan\u{0661}"] {
            assert_eq!(
                parse_decimal(text).unwrap_err(),
                DecimalParseError::NonFinite,
                "{text:?}"
            );
        }
        assert!(parse_decimal("infinity123").is_err());
    }

    #[test]
    fn enforces_cpython_construction_exponent_bounds() {
        // `MIN_ETINY`/`MAX_EMAX` boundaries from the live oracle.
        assert!(parse_decimal("1e-1999999999999999997").is_ok());
        assert!(parse_decimal("1e-1999999999999999998").is_err());
        assert!(parse_decimal("0e-1999999999999999997").is_ok());
        assert!(parse_decimal("0e-1999999999999999998").is_err());
        assert!(parse_decimal("1e999999999999999999").is_ok());
        assert!(parse_decimal("1e1000000000000000000").is_err());
        assert!(parse_decimal("0e999999999999999999").is_ok());
        assert!(parse_decimal("0e1000000000000000000").is_err());
        // The stored exponent is the written exponent minus the fraction
        // length, so a fractional coefficient shifts the accepted range.
        assert!(parse_decimal("0.1e1000000000000000000").is_ok());
        assert!(parse_decimal("0.1e1999999999999999999").is_err());
        assert!(parse_decimal("1.0e-1999999999999999996").is_ok());
        assert!(parse_decimal("1.0e-1999999999999999997").is_err());
        // A nonzero coefficient also bounds the adjusted exponent.
        assert!(parse_decimal("99e999999999999999998").is_ok());
        assert!(parse_decimal("99e999999999999999999").is_err());
        assert!(parse_decimal("123456789012345678901234567890e999999999999999969").is_ok());
        assert!(parse_decimal("123456789012345678901234567890e999999999999999970").is_ok());
        assert!(parse_decimal("123456789012345678901234567890e999999999999999970").is_ok());
        assert!(parse_decimal("123456789012345678901234567890e999999999999999971").is_err());
        // Written exponent digits with harmless leading zeros.
        assert!(parse_decimal("1e000000000000000000000000000000001").is_ok());
        assert!(parse_decimal("1e+0000000000000000000000000000000000000001").is_ok());
        assert!(parse_decimal("1e-0000000000000000000000000000000000000001").is_ok());
        let huge_magnitude = format!("1e{}", "1".repeat(20));
        assert_eq!(
            parse_decimal(&huge_magnitude).unwrap_err(),
            DecimalParseError::ExponentOutOfRange
        );
        // Extremely long written exponents are rejected without allocation.
        let huge = format!("1e1{}", "0".repeat(100_000));
        assert_eq!(
            parse_decimal(&huge).unwrap_err(),
            DecimalParseError::ExponentOutOfRange
        );
    }

    #[test]
    fn extreme_accepted_exponents_format_and_compare_without_panicking() {
        let lowest = parse_decimal("1e-1999999999999999997").expect("MIN_ETINY literal");
        assert_eq!(lowest.to_python_string(), "1E-1999999999999999997");
        assert_eq!(lowest.adjusted_exponent(), -1_999_999_999_999_999_997);
        let highest = parse_decimal("99e999999999999999998").expect("MAX_EMAX adjusted literal");
        assert_eq!(highest.to_python_string(), "9.9E+999999999999999999");
        assert_eq!(
            parse_decimal("1e999999999999999999")
                .expect("MAX_EMAX literal")
                .to_python_string(),
            "1E+999999999999999999"
        );
        assert_eq!(
            parse_decimal("9.9e999999999999999998")
                .expect("MAX_EMAX with fraction shift")
                .to_python_string(),
            "9.9E+999999999999999998"
        );
        // Far-apart extremes stay comparable and addable in bounded memory.
        assert_eq!(lowest.compare(&highest), std::cmp::Ordering::Less);
        assert!(lowest.add(&highest).is_err(), "context overflow");
        assert_eq!(
            lowest
                .add(&lowest)
                .expect("underflow clamps")
                .to_python_string(),
            "0E-1000026"
        );
    }
}
