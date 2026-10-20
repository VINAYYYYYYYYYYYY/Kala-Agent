//! Pydantic-compatible parameter coercion shared by the three models.
//!
//! The input representation is [`kala_json::CanonicalValue`], never
//! `serde_json::Value`. JSON integers, floats, booleans and strings therefore
//! stay distinct and a user object key such as `$serde_json::private::Number`
//! is ordinary data rather than a serializer tag.

use kala_contracts::Text;
use kala_json::{CanonicalObject, CanonicalValue, JsonText, format_float};
use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};

use crate::error::{ParameterError, ParameterErrorKind};

/// Require a JSON object input for a parameter model.
pub(crate) fn as_object<'a>(
    value: &'a CanonicalValue,
    model: &str,
) -> Result<&'a CanonicalObject, ParameterError> {
    match value {
        CanonicalValue::Object(object) => Ok(object),
        _ => Err(ParameterError::field(
            model,
            ParameterErrorKind::ModelType,
            format!("Input should be a valid dictionary or instance of {model}"),
        )),
    }
}

/// Look up a declared field by its UTF-8 name.
pub(crate) fn get<'a>(object: &'a CanonicalObject, name: &str) -> Option<&'a CanonicalValue> {
    object
        .iter()
        .find(|(key, _)| key.to_utf8().is_ok_and(|key| key == name))
        .map(|(_, value)| value)
}

/// Refuse every object key that is not a declared model field.
pub(crate) fn reject_extra(
    object: &CanonicalObject,
    allowed: &[&str],
) -> Result<(), ParameterError> {
    for key in object.keys() {
        let rendered = key.to_string_lossy();
        if !allowed.contains(&rendered.as_str()) {
            return Err(ParameterError::field(
                rendered,
                ParameterErrorKind::ExtraForbidden,
                "Extra inputs are not permitted",
            ));
        }
    }
    Ok(())
}

/// Coerce one value to a finite float inside an inclusive range.
pub(crate) fn coerce_float(
    field: &str,
    value: &CanonicalValue,
    minimum: f64,
    maximum: f64,
) -> Result<f64, ParameterError> {
    let parsed = match value {
        CanonicalValue::Float(number) => *number,
        CanonicalValue::Integer(number) => {
            let converted = number.to_f64().filter(|value| value.is_finite());
            converted.ok_or_else(|| {
                ParameterError::field(
                    field,
                    ParameterErrorKind::Type,
                    "Input should be a valid number",
                )
            })?
        }
        CanonicalValue::Bool(flag) => {
            if *flag {
                1.0
            } else {
                0.0
            }
        }
        CanonicalValue::Text(text) => {
            let source = pydantic_text(text);
            source
                .as_deref()
                .and_then(parse_float_text)
                .ok_or_else(|| {
                    ParameterError::field(
                        field,
                        ParameterErrorKind::FloatParsing,
                        "Input should be a valid number, unable to parse string as a number",
                    )
                })?
        }
        _ => {
            return Err(ParameterError::field(
                field,
                ParameterErrorKind::Type,
                "Input should be a valid number",
            ));
        }
    };
    apply_float_range(field, parsed, minimum, maximum)
}

fn apply_float_range(
    field: &str,
    value: f64,
    minimum: f64,
    maximum: f64,
) -> Result<f64, ParameterError> {
    if value.is_nan() || value == f64::INFINITY {
        return Err(ParameterError::field(
            field,
            ParameterErrorKind::LessThanEqual,
            format!(
                "Input should be less than or equal to {}",
                format_bound(maximum)
            ),
        ));
    }
    if value == f64::NEG_INFINITY {
        return Err(ParameterError::field(
            field,
            ParameterErrorKind::GreaterThanEqual,
            format!(
                "Input should be greater than or equal to {}",
                format_bound(minimum)
            ),
        ));
    }
    if value < minimum {
        return Err(ParameterError::field(
            field,
            ParameterErrorKind::GreaterThanEqual,
            format!(
                "Input should be greater than or equal to {}",
                format_bound(minimum)
            ),
        ));
    }
    if value > maximum {
        return Err(ParameterError::field(
            field,
            ParameterErrorKind::LessThanEqual,
            format!(
                "Input should be less than or equal to {}",
                format_bound(maximum)
            ),
        ));
    }
    Ok(value)
}

/// Coerce an integer and enforce the declared inclusive bounds with the exact
/// Pydantic message shape.
pub(crate) fn coerce_int_in_range(
    field: &str,
    value: &CanonicalValue,
    minimum: i64,
    maximum: i64,
) -> Result<i64, ParameterError> {
    let parsed = match value {
        CanonicalValue::Integer(number) => number.clone(),
        CanonicalValue::Float(number) => {
            if !number.is_finite() {
                return Err(ParameterError::field(
                    field,
                    ParameterErrorKind::FiniteNumber,
                    "Input should be a finite number",
                ));
            }
            if number.fract() != 0.0 {
                return Err(ParameterError::field(
                    field,
                    ParameterErrorKind::IntegerFromFloat,
                    "Input should be a valid integer, got a number with a fractional part",
                ));
            }
            integer_from_f64(field, *number)?
        }
        CanonicalValue::Bool(flag) => BigInt::from(u8::from(*flag)),
        CanonicalValue::Text(text) => {
            let source = pydantic_text(text);
            source
                .as_deref()
                .and_then(parse_integer_text)
                .ok_or_else(|| {
                    ParameterError::field(
                        field,
                        ParameterErrorKind::IntegerParsing,
                        "Input should be a valid integer, unable to parse string as an integer",
                    )
                })?
        }
        _ => {
            return Err(ParameterError::field(
                field,
                ParameterErrorKind::Type,
                "Input should be a valid integer",
            ));
        }
    };
    let minimum_value = BigInt::from(minimum);
    let maximum_value = BigInt::from(maximum);
    if parsed < minimum_value {
        return Err(ParameterError::field(
            field,
            ParameterErrorKind::GreaterThanEqual,
            format!("Input should be greater than or equal to {}", minimum),
        ));
    }
    if parsed > maximum_value {
        return Err(ParameterError::field(
            field,
            ParameterErrorKind::LessThanEqual,
            format!("Input should be less than or equal to {}", maximum),
        ));
    }
    parsed.to_i64().ok_or_else(|| {
        ParameterError::field(
            field,
            ParameterErrorKind::Type,
            "Input should be a valid integer",
        )
    })
}

fn integer_from_f64(field: &str, value: f64) -> Result<BigInt, ParameterError> {
    if value.fract() != 0.0 {
        return Err(ParameterError::field(
            field,
            ParameterErrorKind::IntegerFromFloat,
            "Input should be a valid integer, got a number with a fractional part",
        ));
    }
    // `1e30` is a valid Python int. Build it through the decimal rendering of
    // the exact f64 value instead of narrowing to i64.
    let rendered = format_float(value).map_err(|_| {
        ParameterError::field(
            field,
            ParameterErrorKind::Type,
            "Input should be a valid integer",
        )
    })?;
    let source = if rendered.contains('e') {
        // `format_float` uses scientific notation outside -4..16. Expand it
        // exactly for integral values.
        expand_exponent(&rendered).ok_or_else(|| {
            ParameterError::field(
                field,
                ParameterErrorKind::Type,
                "Input should be a valid integer",
            )
        })?
    } else {
        rendered
    };
    parse_integer_text(source.trim_end_matches(".0")).ok_or_else(|| {
        ParameterError::field(
            field,
            ParameterErrorKind::Type,
            "Input should be a valid integer",
        )
    })
}

fn expand_exponent(rendered: &str) -> Option<String> {
    let (mantissa, exponent) = rendered.split_once('e')?;
    let exponent: i32 = exponent.parse().ok()?;
    let negative = mantissa.starts_with('-');
    let unsigned = mantissa.trim_start_matches('-');
    let mut digits: String = unsigned
        .chars()
        .filter(|character| *character != '.')
        .collect();
    let decimal_places = unsigned
        .split_once('.')
        .map(|(_, fraction)| fraction.len() as i32)
        .unwrap_or(0);
    let shift = exponent - decimal_places;
    if shift >= 0 {
        for _ in 0..shift {
            digits.push('0');
        }
    } else {
        let drop = (-shift) as usize;
        if drop > digits.len() {
            return None;
        }
        let keep = digits.len() - drop;
        if digits[keep..].chars().any(|character| character != '0') {
            return None;
        }
        digits.truncate(keep);
    }
    Some(if negative {
        format!("-{digits}")
    } else {
        digits
    })
}

/// Coerce a JSON string to one exact enum value.
pub(crate) fn coerce_enum<T: Copy>(
    field: &str,
    value: &CanonicalValue,
    allowed: &[(&str, T)],
) -> Result<T, ParameterError> {
    let rendered = match value {
        CanonicalValue::Text(text) => text.to_utf8().ok(),
        _ => None,
    };
    if let Some(rendered) = rendered {
        if let Some((_, resolved)) = allowed.iter().find(|(name, _)| *name == rendered.as_str()) {
            return Ok(*resolved);
        }
    }
    let values = allowed
        .iter()
        .map(|(name, _)| format!("'{name}'"))
        .collect::<Vec<_>>();
    let list = match values.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [first, second] => format!("{first} or {second}"),
        _ => {
            let (last, rest) = values.split_last().expect("nonempty");
            format!("{} or {last}", rest.join(", "))
        }
    };
    Err(ParameterError::field(
        field,
        ParameterErrorKind::Enum,
        format!("Input should be {list}"),
    ))
}

fn pydantic_text(text: &JsonText) -> Option<String> {
    Text::from_json_text(text.clone())
        .pydantic_strip()
        .to_utf8()
        .ok()
}

fn format_bound(value: f64) -> String {
    let rendered = format_float(value).unwrap_or_else(|_| value.to_string());
    rendered
        .strip_suffix(".0")
        .map(str::to_owned)
        .unwrap_or(rendered)
}

fn parse_float_text(source: &str) -> Option<f64> {
    if !underscores_are_between_digits(source) {
        return None;
    }
    let cleaned = source.replace('_', "");
    cleaned.parse::<f64>().ok()
}

fn underscores_are_between_digits(source: &str) -> bool {
    let bytes = source.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'_' {
            continue;
        }
        if index == 0 || index + 1 >= bytes.len() {
            return false;
        }
        if !bytes[index - 1].is_ascii_digit() || !bytes[index + 1].is_ascii_digit() {
            return false;
        }
    }
    true
}

fn parse_integer_text(source: &str) -> Option<BigInt> {
    let bytes = source.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut index = 0;
    let negative = match bytes[0] {
        b'-' => {
            index += 1;
            true
        }
        b'+' => {
            index += 1;
            false
        }
        _ => false,
    };
    let digits_start = index;
    let mut digits = String::new();
    let mut last_was_digit = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_digit() {
            digits.push(char::from(byte));
            last_was_digit = true;
            index += 1;
        } else if byte == b'_'
            && last_was_digit
            && index + 1 < bytes.len()
            && bytes[index + 1].is_ascii_digit()
        {
            index += 1;
            last_was_digit = false;
        } else {
            break;
        }
    }
    if index == digits_start || digits.is_empty() {
        return None;
    }
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        let fraction_start = index;
        while index < bytes.len() && bytes[index] == b'0' {
            index += 1;
        }
        if index == fraction_start {
            return None;
        }
    }
    if index != bytes.len() {
        return None;
    }
    let magnitude = BigInt::parse_bytes(digits.as_bytes(), 10)?;
    Some(if negative {
        -magnitude
    } else if magnitude.is_zero() {
        BigInt::zero()
    } else {
        magnitude
    })
}

/// Python `repr()` for the simple string keys used in domain error messages.
pub(crate) fn python_string_repr(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_text_matches_pydantic_acceptance() {
        for source in ["2", "+2", "-0", "0002", "1_0", "2.0", "2.00", " 2 "] {
            assert!(parse_integer_text(source.trim()).is_some(), "{source}");
        }
        for source in [
            "", "2.5", "2.", "2e0", "_10", "10_", "1__0", "2.0_0", "2._0", "٢",
        ] {
            assert!(parse_integer_text(source.trim()).is_none(), "{source}");
        }
    }

    #[test]
    fn float_text_matches_python_syntax() {
        assert_eq!(parse_float_text("8."), Some(8.0));
        assert_eq!(parse_float_text("+.5"), Some(0.5));
        assert_eq!(parse_float_text("1_0.5"), Some(10.5));
        assert!(parse_float_text("_10.5").is_none());
        assert!(parse_float_text("10.5_").is_none());
        assert!(parse_float_text("1__0.5").is_none());
        assert!(parse_float_text("1,5").is_none());
    }
}
