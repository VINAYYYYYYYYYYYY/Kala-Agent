use crate::error::CanonicalJsonError;

/// Format a finite `f64` exactly like CPython's `repr(float)`.
///
/// CPython uses the shortest decimal string that round-trips and chooses
/// decimal notation for decimal exponents in `-4..=15`; outside that range it
/// uses scientific notation with an explicit exponent sign and at least two
/// exponent digits. Integral floats keep a trailing `.0` in decimal notation,
/// and negative zero stays `-0.0`.
///
/// Ryu supplies the shortest round-trip digits. Its pretty formatter uses a
/// slightly different lower exponent threshold, so the digits and decimal
/// exponent are reformatted here following CPython's rules.
pub fn format_float(value: f64) -> Result<String, CanonicalJsonError> {
    if !value.is_finite() {
        return Err(CanonicalJsonError::NonFinite);
    }
    if value == 0.0 {
        return Ok(if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .to_owned());
    }

    let mut buffer = ryu::Buffer::new();
    let rendered = buffer.format_finite(value);
    let negative = rendered.starts_with('-');
    let unsigned = rendered.strip_prefix('-').unwrap_or(rendered);

    let (mut digits, exponent) = split_ryu(unsigned);
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }

    let mut out = String::with_capacity(rendered.len() + 2);
    if negative {
        out.push('-');
    }

    if !(-4..16).contains(&exponent) {
        out.push(digits.as_bytes()[0] as char);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        if exponent < 0 {
            out.push('-');
        } else {
            out.push('+');
        }
        let magnitude = exponent.unsigned_abs();
        if magnitude < 10 {
            out.push('0');
        }
        out.push_str(&magnitude.to_string());
        return Ok(out);
    }

    if exponent >= 0 {
        let integer_digits = exponent as usize + 1;
        if digits.len() <= integer_digits {
            out.push_str(&digits);
            for _ in digits.len()..integer_digits {
                out.push('0');
            }
            out.push_str(".0");
        } else {
            out.push_str(&digits[..integer_digits]);
            out.push('.');
            out.push_str(&digits[integer_digits..]);
        }
    } else {
        out.push_str("0.");
        for _ in 0..(-exponent - 1) {
            out.push('0');
        }
        out.push_str(&digits);
    }
    Ok(out)
}

/// Split ryu's pretty output into significant digits (without the decimal
/// point) and the base-10 exponent of the first digit.
fn split_ryu(rendered: &str) -> (String, i32) {
    if let Some(index) = rendered.find('e') {
        let mantissa = &rendered[..index];
        let exponent: i32 = rendered[index + 1..].parse().expect("ryu exponent");
        let digits: String = mantissa
            .chars()
            .filter(|character| *character != '.')
            .collect();
        return (digits, exponent);
    }

    let (integer, fraction) = match rendered.split_once('.') {
        Some(parts) => parts,
        None => (rendered, ""),
    };
    let combined = format!("{integer}{fraction}");
    let first_significant = combined
        .bytes()
        .position(|byte| byte != b'0')
        .unwrap_or(combined.len());
    let digits = combined[first_significant..].to_owned();
    let exponent = integer.len() as i32 - first_significant as i32 - 1;
    (digits, exponent)
}

#[cfg(test)]
mod tests {
    use super::format_float;

    fn cases() -> Vec<(f64, &'static str)> {
        vec![
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (1.5, "1.5"),
            (1e-4, "0.0001"),
            (1e-5, "1e-05"),
            (1e-7, "1e-07"),
            (1e15, "1000000000000000.0"),
            (1e16, "1e+16"),
            (1.5e16, "1.5e+16"),
            (5e-324, "5e-324"),
            (f64::MAX, "1.7976931348623157e+308"),
            (1.2345678901234568e16, "1.2345678901234568e+16"),
            (123.456, "123.456"),
            (-1.234e-5, "-1.234e-05"),
        ]
    }

    #[test]
    fn matches_known_cpython_repr_values() {
        for (value, expected) in cases() {
            assert_eq!(format_float(value).unwrap(), expected, "value {value:?}");
        }
    }

    #[test]
    fn rejects_non_finite_values() {
        assert!(format_float(f64::NAN).is_err());
        assert!(format_float(f64::INFINITY).is_err());
        assert!(format_float(f64::NEG_INFINITY).is_err());
    }
}
