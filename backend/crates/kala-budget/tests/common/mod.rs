//! Shared test helpers: JSON comparison that treats numbers by value.
//!
//! `serde_json`'s `arbitrary_precision` feature keeps the exact lexical text
//! of every number, so `Number("2e-06")` and `Number("2e-6")` compare unequal
//! with `Value == Value` even though Python's `json.loads` yields the same
//! float. The corpus oracle is produced by Python, so tests compare numbers by
//! value: integers exactly, mixed int/float as Python does.

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use serde_json::{Number, Value};

fn is_integer_text(text: &str) -> bool {
    let bytes = text.as_bytes();
    let digits = match bytes.first() {
        Some(b'-') | Some(b'+') => &bytes[1..],
        _ => bytes,
    };
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

fn integer(text: &str) -> Option<BigInt> {
    BigInt::parse_bytes(text.as_bytes(), 10)
}

fn numbers_equal(left: &Number, right: &Number) -> bool {
    let left_text = left.as_str();
    let right_text = right.as_str();
    match (is_integer_text(left_text), is_integer_text(right_text)) {
        (true, true) => integer(left_text) == integer(right_text),
        (true, false) => {
            let left_value = integer(left_text).and_then(|value| value.to_f64());
            let right_value: f64 = right_text.parse().unwrap_or(f64::NAN);
            match (left_value, right_value.is_finite()) {
                (Some(left_value), true) => left_value.is_finite() && left_value == right_value,
                _ => false,
            }
        }
        (false, true) => {
            let right_value = integer(right_text).and_then(|value| value.to_f64());
            let left_value: f64 = left_text.parse().unwrap_or(f64::NAN);
            match (right_value, left_value.is_finite()) {
                (Some(right_value), true) => right_value.is_finite() && left_value == right_value,
                _ => false,
            }
        }
        (false, false) => {
            let left_value: f64 = left_text.parse().unwrap_or(f64::NAN);
            let right_value: f64 = right_text.parse().unwrap_or(f64::NAN);
            if left_value.is_nan() || right_value.is_nan() {
                left_text == right_text
            } else {
                left_value == right_value
            }
        }
    }
}

/// Structural equality with numeric-value semantics for JSON numbers.
#[must_use]
pub fn json_eq(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::String(left), Value::String(right)) => left == right,
        (Value::Number(left), Value::Number(right)) => numbers_equal(left, right),
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| json_eq(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, left_value)| {
                    right
                        .get(key)
                        .is_some_and(|right_value| json_eq(left_value, right_value))
                })
        }
        _ => false,
    }
}

/// Assert structural JSON equality and print the first differing path.
pub fn assert_json_eq(left: &Value, right: &Value, label: &str) {
    if !json_eq(left, right) {
        let path = first_difference(left, right, "$".to_owned());
        panic!(
            "{label}: first difference at {path}\nleft:  {:#}\nright: {:#}",
            at_path(left, &path),
            at_path(right, &path)
        );
    }
}

fn first_difference(left: &Value, right: &Value, path: String) -> String {
    match (left, right) {
        (Value::Array(left), Value::Array(right)) => {
            if left.len() != right.len() {
                return format!("{path}.len ({} vs {})", left.len(), right.len());
            }
            for (index, (left, right)) in left.iter().zip(right.iter()).enumerate() {
                if !json_eq(left, right) {
                    return first_difference(left, right, format!("{path}[{index}]"));
                }
            }
            path
        }
        (Value::Object(left), Value::Object(right)) => {
            for (key, left_value) in left {
                match right.get(key) {
                    Some(right_value) if json_eq(left_value, right_value) => {}
                    Some(right_value) => {
                        return first_difference(left_value, right_value, format!("{path}.{key}"));
                    }
                    None => return format!("{path}.{key} (missing on the right)"),
                }
            }
            for key in right.keys() {
                if !left.contains_key(key) {
                    return format!("{path}.{key} (missing on the left)");
                }
            }
            path
        }
        _ => path,
    }
}

fn at_path(value: &Value, path: &str) -> Value {
    let mut current = value;
    let mut rest = path.strip_prefix('$').unwrap_or(path);
    loop {
        if let Some(tail) = rest.strip_prefix('.') {
            let (key, remainder) = match tail.find(['.', '[']) {
                Some(index) => (&tail[..index], &tail[index..]),
                None => (tail, ""),
            };
            match current.get(key) {
                Some(next) => current = next,
                None => return Value::String(format!("<missing {key}>")),
            }
            rest = remainder;
        } else if let Some(tail) = rest.strip_prefix('[') {
            let (index_text, remainder) = match tail.find(']') {
                Some(index) => (&tail[..index], &tail[index + 1..]),
                None => (tail, ""),
            };
            match index_text
                .parse::<usize>()
                .ok()
                .and_then(|index| current.get(index))
            {
                Some(next) => current = next,
                None => return Value::String(format!("<missing index {index_text}>")),
            }
            rest = remainder;
        } else {
            return current.clone();
        }
    }
}
