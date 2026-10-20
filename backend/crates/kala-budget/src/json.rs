//! Small JSON helpers that preserve Python numeric input shapes.
//!
//! With `serde_json`'s `arbitrary_precision` feature, numbers keep their exact
//! lexical text, so huge integers stay integers instead of collapsing to
//! floats. These helpers convert between kernel values and JSON without
//! losing that distinction.

use crate::counter::Counter;
use crate::decimal::{Decimal, DecimalParseError};
use serde_json::{Number, Value};
use std::str::FromStr;

/// Encode an arbitrary-precision counter as a JSON integer.
pub(crate) fn counter_value(counter: &Counter) -> Value {
    Value::Number(
        Number::from_str(&counter.to_decimal_string()).expect("decimal digits are a JSON number"),
    )
}

/// Encode a finite float.
pub(crate) fn float_value(value: f64) -> Value {
    Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

/// Encode an exact decimal as its `str(Decimal)` string.
pub(crate) fn decimal_string_value(decimal: &Decimal) -> Value {
    Value::String(decimal.to_python_string())
}

/// Return the exact lexical text of a JSON number.
pub(crate) fn number_text(value: &Value) -> Option<&str> {
    value.as_number().map(|number| number.as_str())
}

/// True when the JSON number is written as an integer literal (no fraction or
/// exponent), matching Python's `json.loads` int-vs-float decision.
pub(crate) fn is_integer_text(text: &str) -> bool {
    let bytes = text.as_bytes();
    let digits = match bytes.first() {
        Some(b'-') | Some(b'+') => &bytes[1..],
        _ => bytes,
    };
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

/// Parse a JSON integer literal into an arbitrary-precision counter.
pub(crate) fn counter_from_integer_text(
    text: &str,
) -> Result<Counter, crate::counter::CounterError> {
    Counter::parse(text)
}

/// Parse a JSON integer literal into an exact decimal the way Python's
/// `Decimal(str(int))` does. JSON `-0` loads as the int `0` in Python, so the
/// sign is not preserved on this path.
pub(crate) fn decimal_from_integer_text(text: &str) -> Result<Decimal, DecimalParseError> {
    let bytes = text.as_bytes();
    let digits = match bytes.first() {
        Some(b'-') | Some(b'+') => &bytes[1..],
        _ => bytes,
    };
    if bytes.first() == Some(&b'-') && !digits.is_empty() && digits.iter().all(|byte| *byte == b'0')
    {
        return Ok(Decimal::zero());
    }
    Decimal::parse(text)
}

/// True when the JSON number text is an exact integer value (used where the
/// Python kernel accepts `int` but rejects `float`).
pub(crate) fn json_integer_text(value: &Value) -> Option<&str> {
    let text = number_text(value)?;
    if is_integer_text(text) {
        Some(text)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_text_matches_python_json_classification() {
        assert!(is_integer_text("100"));
        assert!(is_integer_text("-0"));
        assert!(!is_integer_text("100.0"));
        assert!(!is_integer_text("1e2"));
    }

    #[test]
    fn huge_integers_survive_a_json_round_trip() {
        let value: Value =
            serde_json::from_str("10000000000000000000000000000000000000000").expect("json");
        let text = json_integer_text(&value).expect("integer");
        let counter = counter_from_integer_text(text).expect("counter");
        assert_eq!(counter.to_decimal_string().len(), 41);
    }

    #[test]
    fn json_integer_negative_zero_is_positive_zero() {
        let value: Value = serde_json::from_str("-0").expect("json");
        let text = json_integer_text(&value).expect("integer");
        let decimal = decimal_from_integer_text(text).expect("decimal");
        assert_eq!(decimal.to_python_string(), "0");
        // The float path keeps the signed zero, like `str(-0.0)`.
        let float_value: Value = serde_json::from_str("-0.0").expect("json");
        let text = crate::json::number_text(&float_value).expect("number");
        let float: f64 = text.parse().expect("float");
        let decimal = Decimal::from_python_float(float).expect("decimal");
        assert_eq!(decimal.to_python_string(), "-0.0");
    }
}
