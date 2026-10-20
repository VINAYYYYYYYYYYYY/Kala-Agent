//! CPython-compatible decimal and float text formatting.

use super::Decimal;

pub(super) fn to_python_string(value: &Decimal) -> String {
    let digits = if value.is_zero() {
        "0".to_owned()
    } else {
        value.coefficient().to_str_radix(10)
    };
    let exponent = value.exponent();
    let digit_count = digits.len() as i128;
    let adjusted = i128::from(exponent) + digit_count - 1;

    let body = if exponent <= 0 && adjusted >= -6 {
        if exponent == 0 {
            digits
        } else {
            let point = digit_count + i128::from(exponent);
            if point > 0 {
                let point = point as usize;
                format!("{}.{}", &digits[..point], &digits[point..])
            } else {
                let zeros = (-point) as usize;
                format!("0.{}{}", "0".repeat(zeros), digits)
            }
        }
    } else {
        let mantissa = if digits.len() == 1 {
            digits
        } else {
            format!("{}.{}", &digits[..1], &digits[1..])
        };
        let sign = if adjusted >= 0 { '+' } else { '-' };
        format!("{mantissa}E{sign}{}", adjusted.abs())
    };

    if value.is_negative() {
        format!("-{body}")
    } else {
        body
    }
}

/// Format a float the way Python's `str(float)`/`repr(float)` does.
///
/// This is the reviewed `kala_json::format_float` implementation, re-exported
/// through the budget crate so the kernel keeps its existing wrapper API.
/// `format_float` rejects non-finite values; the wrapper keeps Python's text
/// for those (a caller can still stringify any `f64`), so only finite values
/// take the shared path.
pub(super) fn python_float_repr(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    }
    kala_json::format_float(value).expect("finite floats are always formattable")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decimal(text: &str) -> Decimal {
        Decimal::parse(text).expect("valid decimal")
    }

    #[test]
    fn strings_match_cpython_general_algorithm() {
        let cases = [
            ("0", "0"),
            ("-0", "-0"),
            ("0.0", "0.0"),
            ("-0.0", "-0.0"),
            ("0E-1000000", "0E-1000000"),
            ("0E+3", "0E+3"),
            ("1.00", "1.00"),
            ("0.10", "0.10"),
            ("0.2", "0.2"),
            ("1E+3", "1E+3"),
            ("100.000", "100.000"),
            ("1.2300", "1.2300"),
            ("1e-30", "1E-30"),
            ("1e+40", "1E+40"),
            ("1e-999999", "1E-999999"),
            ("1e+999999", "1E+999999"),
            (
                "123456789012345678901234567890.12345",
                "123456789012345678901234567890.12345",
            ),
            (
                "1.23456789012345678901234567890123456789",
                "1.23456789012345678901234567890123456789",
            ),
            ("0.0000000000000000000000000000001", "1E-31"),
            ("0.000001", "0.000001"),
            ("0.0000001", "1E-7"),
        ];
        for (input, expected) in cases {
            assert_eq!(decimal(input).to_python_string(), expected, "{input}");
        }
    }

    #[test]
    fn float_repr_matches_python_examples() {
        assert_eq!(python_float_repr(0.0), "0.0");
        assert_eq!(python_float_repr(-0.0), "-0.0");
        assert_eq!(python_float_repr(1.0), "1.0");
        assert_eq!(python_float_repr(0.1), "0.1");
        assert_eq!(python_float_repr(100.0), "100.0");
        assert_eq!(python_float_repr(1e15), "1000000000000000.0");
        assert_eq!(python_float_repr(1e16), "1e+16");
        assert_eq!(python_float_repr(1e-4), "0.0001");
        assert_eq!(python_float_repr(1e-5), "1e-05");
        assert_eq!(python_float_repr(1.5e-7), "1.5e-07");
        assert_eq!(python_float_repr(1.2345678901234567), "1.2345678901234567");
        assert_eq!(python_float_repr(3600.0), "3600.0");
        // Non-finite values keep Python's text even though the shared
        // `kala_json::format_float` path rejects them.
        assert_eq!(python_float_repr(f64::NAN), "nan");
        assert_eq!(python_float_repr(f64::INFINITY), "inf");
        assert_eq!(python_float_repr(f64::NEG_INFINITY), "-inf");
    }
}
