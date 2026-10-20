//! Ordinary model/state JSON loading with CPython `json.loads` semantics.
//!
//! Persisted kernel state is loaded by Python's `json.loads`, not by the
//! strict canonical parser: duplicate object keys use last-wins semantics and
//! escaped lone surrogates survive as code points. The canonical/hash path
//! remains [`kala_json::parse_strict`] and is never used for ordinary input.
//!
//! Non-finite JSON constants (`NaN`, `Infinity`, `-Infinity`) are refused at
//! this boundary. Every kernel field that consumes them is a string, integer,
//! boolean, list or timestamp, so the accepted typed states are identical; the
//! parse error class differs from Python's later field error for a non-finite
//! value in an ignored field.

use kala_json::{CanonicalObject, CanonicalValue, JsonText, Limits};
use num_bigint::BigInt;

/// A permissive model-state JSON parse failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelJsonError {
    message: String,
}

impl ModelJsonError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The diagnostic text.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ModelJsonError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ModelJsonError {}

/// Parse ordinary model/state JSON text with last-wins duplicates.
pub fn parse_model_json(text: &str) -> Result<CanonicalValue, ModelJsonError> {
    let mut parser = Parser {
        text,
        bytes: text.as_bytes(),
        position: 0,
    };
    let value = parser.parse_value(0)?;
    parser.skip_whitespace();
    if parser.position != parser.bytes.len() {
        return Err(invalid());
    }
    Ok(value)
}

fn invalid() -> ModelJsonError {
    ModelJsonError::new("value must be valid JSON")
}

const MAX_DEPTH: usize = 512;

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    position: usize,
}

impl Parser<'_> {
    fn parse_value(&mut self, depth: usize) -> Result<CanonicalValue, ModelJsonError> {
        if depth > MAX_DEPTH {
            return Err(invalid());
        }
        self.skip_whitespace();
        let Some(byte) = self.bytes.get(self.position).copied() else {
            return Err(invalid());
        };
        match byte {
            b'"' => Ok(CanonicalValue::Text(self.parse_string()?)),
            b't' => {
                self.expect_literal(b"true")?;
                Ok(CanonicalValue::Bool(true))
            }
            b'f' => {
                self.expect_literal(b"false")?;
                Ok(CanonicalValue::Bool(false))
            }
            b'n' => {
                self.expect_literal(b"null")?;
                Ok(CanonicalValue::Null)
            }
            b'N' | b'I' => Err(ModelJsonError::new("JSON numbers must be finite")),
            b'-' if self.remaining().starts_with(b"-Infinity") => {
                Err(ModelJsonError::new("JSON numbers must be finite"))
            }
            b'-' | b'0'..=b'9' => self.parse_number(),
            b'[' => {
                self.position += 1;
                self.skip_whitespace();
                let mut values = Vec::new();
                if self.bytes.get(self.position) == Some(&b']') {
                    self.position += 1;
                    return Ok(CanonicalValue::Array(values));
                }
                loop {
                    values.push(self.parse_value(depth + 1)?);
                    self.skip_whitespace();
                    match self.bytes.get(self.position).copied() {
                        Some(b',') => {
                            self.position += 1;
                            continue;
                        }
                        Some(b']') => {
                            self.position += 1;
                            break;
                        }
                        _ => return Err(invalid()),
                    }
                }
                Ok(CanonicalValue::Array(values))
            }
            b'{' => {
                self.position += 1;
                self.skip_whitespace();
                let mut entries = CanonicalObject::new();
                if self.bytes.get(self.position) == Some(&b'}') {
                    self.position += 1;
                    return Ok(CanonicalValue::Object(entries));
                }
                loop {
                    self.skip_whitespace();
                    if self.bytes.get(self.position) != Some(&b'"') {
                        return Err(invalid());
                    }
                    let key = self.parse_string()?;
                    self.skip_whitespace();
                    if self.bytes.get(self.position) != Some(&b':') {
                        return Err(invalid());
                    }
                    self.position += 1;
                    let value = self.parse_value(depth + 1)?;
                    // Python dict assignment: last value wins and insertion
                    // order is no longer observable for our sorted view.
                    entries.insert(key, value);
                    self.skip_whitespace();
                    match self.bytes.get(self.position).copied() {
                        Some(b',') => {
                            self.position += 1;
                            continue;
                        }
                        Some(b'}') => {
                            self.position += 1;
                            break;
                        }
                        _ => return Err(invalid()),
                    }
                }
                Ok(CanonicalValue::Object(entries))
            }
            _ => Err(invalid()),
        }
    }

    fn parse_string(&mut self) -> Result<JsonText, ModelJsonError> {
        self.position += 1; // opening quote
        let mut code_points: Vec<u32> = Vec::new();
        loop {
            let Some(byte) = self.bytes.get(self.position).copied() else {
                return Err(invalid());
            };
            match byte {
                b'"' => {
                    self.position += 1;
                    return JsonText::from_code_points(code_points).map_err(|_| invalid());
                }
                b'\\' => {
                    self.position += 1;
                    let Some(escape) = self.bytes.get(self.position).copied() else {
                        return Err(invalid());
                    };
                    self.position += 1;
                    match escape {
                        b'"' => code_points.push(u32::from(b'"')),
                        b'\\' => code_points.push(u32::from(b'\\')),
                        b'/' => code_points.push(u32::from(b'/')),
                        b'b' => code_points.push(0x08),
                        b'f' => code_points.push(0x0C),
                        b'n' => code_points.push(0x0A),
                        b'r' => code_points.push(0x0D),
                        b't' => code_points.push(0x09),
                        b'u' => {
                            let first = self.parse_hex_quad()?;
                            // CPython combines a high-surrogate escape only
                            // with an immediately following low surrogate;
                            // otherwise each code unit is kept.
                            if (0xD800..=0xDBFF).contains(&first) {
                                let saved = self.position;
                                if self.bytes.get(self.position) == Some(&b'\\')
                                    && self.bytes.get(self.position + 1) == Some(&b'u')
                                {
                                    self.position += 2;
                                    match self.parse_hex_quad() {
                                        Ok(second) if (0xDC00..=0xDFFF).contains(&second) => {
                                            code_points.push(
                                                0x1_0000
                                                    + ((u32::from(first) - 0xD800) << 10)
                                                    + (u32::from(second) - 0xDC00),
                                            );
                                            continue;
                                        }
                                        _ => self.position = saved,
                                    }
                                }
                            }
                            code_points.push(u32::from(first));
                        }
                        _ => return Err(invalid()),
                    }
                }
                0x00..=0x1F => return Err(invalid()),
                0x20..=0x7F => {
                    code_points.push(u32::from(byte));
                    self.position += 1;
                }
                _ => {
                    let character = self.text[self.position..]
                        .chars()
                        .next()
                        .ok_or_else(invalid)?;
                    code_points.push(u32::from(character));
                    self.position += character.len_utf8();
                }
            }
        }
    }

    fn parse_hex_quad(&mut self) -> Result<u16, ModelJsonError> {
        let mut value = 0u16;
        for _ in 0..4 {
            let Some(byte) = self.bytes.get(self.position).copied() else {
                return Err(invalid());
            };
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err(invalid()),
            };
            value = (value << 4) | u16::from(digit);
            self.position += 1;
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<CanonicalValue, ModelJsonError> {
        let start = self.position;
        if self.bytes.get(self.position) == Some(&b'-') {
            self.position += 1;
        }
        let integer_start = self.position;
        match self.bytes.get(self.position).copied() {
            Some(b'0') => self.position += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.bytes.get(self.position).copied(), Some(b'0'..=b'9')) {
                    self.position += 1;
                }
            }
            _ => return Err(invalid()),
        }
        let mut is_float = false;
        if self.bytes.get(self.position) == Some(&b'.') {
            is_float = true;
            self.position += 1;
            if !matches!(self.bytes.get(self.position).copied(), Some(b'0'..=b'9')) {
                return Err(invalid());
            }
            while matches!(self.bytes.get(self.position).copied(), Some(b'0'..=b'9')) {
                self.position += 1;
            }
        }
        if matches!(self.bytes.get(self.position).copied(), Some(b'e' | b'E')) {
            is_float = true;
            self.position += 1;
            if matches!(self.bytes.get(self.position).copied(), Some(b'+' | b'-')) {
                self.position += 1;
            }
            if !matches!(self.bytes.get(self.position).copied(), Some(b'0'..=b'9')) {
                return Err(invalid());
            }
            while matches!(self.bytes.get(self.position).copied(), Some(b'0'..=b'9')) {
                self.position += 1;
            }
        }
        let token = &self.text[start..self.position];
        if is_float {
            let value: f64 = token.parse().map_err(|_| invalid())?;
            if !value.is_finite() {
                return Err(ModelJsonError::new("JSON numbers must be finite"));
            }
            CanonicalValue::float(value).map_err(|_| invalid())
        } else {
            let digit_count = self.position - integer_start;
            if digit_count > Limits::DEFAULT_MAX_INTEGER_DIGITS {
                return Err(invalid());
            }
            let value = BigInt::parse_bytes(token.as_bytes(), 10).ok_or_else(invalid)?;
            Ok(CanonicalValue::Integer(value))
        }
    }

    fn expect_literal(&mut self, literal: &[u8]) -> Result<(), ModelJsonError> {
        if self.remaining().starts_with(literal) {
            self.position += literal.len();
            Ok(())
        } else {
            Err(invalid())
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(
            self.bytes.get(self.position).copied(),
            Some(b' ' | b'\t' | b'\n' | b'\r')
        ) {
            self.position += 1;
        }
    }

    fn remaining(&self) -> &[u8] {
        &self.bytes[self.position..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kala_contracts::model_json_limits;
    use kala_json::canonical_dumps;

    #[test]
    fn duplicate_keys_are_last_wins() {
        let value = parse_model_json(r#"{"a":1,"b":2,"a":3}"#).expect("valid");
        let text = canonical_dumps(&value, &model_json_limits()).expect("canonical");
        assert_eq!(text, r#"{"a":3,"b":2}"#);
    }

    #[test]
    fn lone_surrogates_survive() {
        let value = parse_model_json(r#"{"a":"\ud800"}"#).expect("valid");
        let text = canonical_dumps(&value, &model_json_limits()).expect("canonical");
        assert_eq!(text, r#"{"a":"\ud800"}"#);
    }

    #[test]
    fn rejects_non_finite_and_trailing_input() {
        assert!(parse_model_json("NaN").is_err());
        assert!(parse_model_json(r#"{"a":1}x"#).is_err());
        assert!(parse_model_json(r#"{"a":"\ud800"}"#).is_ok());
    }
}
