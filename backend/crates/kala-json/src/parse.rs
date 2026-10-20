use num_bigint::BigInt;

use crate::dump::validate;
use crate::error::CanonicalJsonError;
use crate::limits::Limits;
use crate::text::JsonText;
use crate::value::{CanonicalObject, CanonicalValue};

/// Strictly parse JSON bytes and enforce the bounded protocol limits.
///
/// This mirrors `kala_agent.tools.protocol.strict_json_loads`:
///
/// * the raw byte length is checked before UTF-8 decoding,
/// * invalid UTF-8 is distinguished from invalid JSON,
/// * duplicate object keys are rejected even when the values are equal,
/// * `NaN`, `Infinity` and `-Infinity` are rejected with the Python constant
///   messages,
/// * floats that overflow to infinity are rejected while underflow to zero is
///   accepted,
/// * integers keep arbitrary precision (subject to the CPython digit limit),
/// * trailing data, a UTF-8 BOM and top-level scalars follow Python behaviour
///   (scalars are accepted),
/// * nesting, string, key, collection and canonical byte limits are enforced.
///
/// The parser is an explicit-stack state machine rather than recursive
/// descent, so a persisted object nested far beyond any ordinary recursion
/// limit is parsed on an ordinary thread stack. Tool-protocol depth (48) is
/// still enforced exactly by [`validate`] after parsing. A syntactically
/// invalid document of arbitrary nesting depth fails closed without
/// recursing; the heap stack is bounded by the input byte cap in the tool
/// domain and by the document size in the effectively unbounded persistence
/// domain.
pub fn parse_strict(payload: &[u8], limits: &Limits) -> Result<CanonicalValue, CanonicalJsonError> {
    if payload.len() > limits.max_json_bytes {
        return Err(CanonicalJsonError::InputTooLarge);
    }
    let text = std::str::from_utf8(payload).map_err(|_| CanonicalJsonError::InvalidUtf8)?;
    let mut parser = Parser {
        text,
        bytes: text.as_bytes(),
        position: 0,
        limits,
    };
    let value = parser.parse_document()?;
    validate(&value, 0, limits)?;
    Ok(value)
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    position: usize,
    limits: &'a Limits,
}

/// One open container in the explicit parser stack.
enum Frame {
    Array(Vec<CanonicalValue>),
    Object {
        entries: CanonicalObject,
        key: Option<JsonText>,
    },
}

/// What to do after attaching a completed value to the top frame.
enum Attach {
    /// The frame continues and another value follows a comma.
    NextValue,
    /// The frame's closing token was consumed.
    Close,
}

impl<'a> Parser<'a> {
    fn parse_document(&mut self) -> Result<CanonicalValue, CanonicalJsonError> {
        self.skip_whitespace();
        let mut frames: Vec<Frame> = Vec::new();
        let mut current: Option<CanonicalValue> = None;

        loop {
            if current.is_none() {
                self.skip_whitespace();
                match self.bytes.get(self.position).copied() {
                    Some(b'{') => {
                        self.position += 1;
                        self.skip_whitespace();
                        if self.bytes.get(self.position) == Some(&b'}') {
                            self.position += 1;
                            current = Some(CanonicalValue::Object(CanonicalObject::new()));
                        } else {
                            let key = self.parse_object_key()?;
                            frames.push(Frame::Object {
                                entries: CanonicalObject::new(),
                                key: Some(key),
                            });
                            continue;
                        }
                    }
                    Some(b'[') => {
                        self.position += 1;
                        self.skip_whitespace();
                        if self.bytes.get(self.position) == Some(&b']') {
                            self.position += 1;
                            current = Some(CanonicalValue::Array(Vec::new()));
                        } else {
                            frames.push(Frame::Array(Vec::new()));
                            continue;
                        }
                    }
                    _ => current = Some(self.parse_scalar()?),
                }
            }

            // A completed value is attached to its parent. Closing a container
            // produces a new completed value that may itself close its parent.
            loop {
                let Some(frame) = frames.last_mut() else {
                    self.skip_whitespace();
                    if self.position != self.bytes.len() {
                        return Err(CanonicalJsonError::NotJson);
                    }
                    return Ok(current.take().expect("root value"));
                };

                let action = match frame {
                    Frame::Array(items) => {
                        items.push(current.take().expect("array item"));
                        self.skip_whitespace();
                        match self.bytes.get(self.position).copied() {
                            Some(b',') => {
                                self.position += 1;
                                Attach::NextValue
                            }
                            Some(b']') => {
                                self.position += 1;
                                Attach::Close
                            }
                            _ => return Err(CanonicalJsonError::NotJson),
                        }
                    }
                    Frame::Object { entries, key: slot } => {
                        let key = slot.take().expect("pending object key");
                        let value = current.take().expect("object value");
                        if entries.insert(key, value).is_some() {
                            return Err(CanonicalJsonError::DuplicateKey);
                        }
                        self.skip_whitespace();
                        match self.bytes.get(self.position).copied() {
                            Some(b',') => {
                                self.position += 1;
                                let next_key = self.parse_object_key()?;
                                *slot = Some(next_key);
                                Attach::NextValue
                            }
                            Some(b'}') => {
                                self.position += 1;
                                Attach::Close
                            }
                            _ => return Err(CanonicalJsonError::NotJson),
                        }
                    }
                };

                match action {
                    Attach::NextValue => break,
                    Attach::Close => {
                        let frame = frames.pop().expect("open frame");
                        current = Some(match frame {
                            Frame::Array(items) => CanonicalValue::Array(items),
                            Frame::Object { entries, .. } => CanonicalValue::Object(entries),
                        });
                    }
                }
            }
        }
    }

    fn parse_scalar(&mut self) -> Result<CanonicalValue, CanonicalJsonError> {
        let Some(byte) = self.bytes.get(self.position).copied() else {
            return Err(CanonicalJsonError::NotJson);
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
            b'N' => {
                if self.remaining().starts_with(b"NaN") {
                    Err(CanonicalJsonError::NonFiniteConstant("NaN"))
                } else {
                    Err(CanonicalJsonError::NotJson)
                }
            }
            b'I' => {
                if self.remaining().starts_with(b"Infinity") {
                    Err(CanonicalJsonError::NonFiniteConstant("Infinity"))
                } else {
                    Err(CanonicalJsonError::NotJson)
                }
            }
            b'-' if self.remaining().starts_with(b"-Infinity") => {
                Err(CanonicalJsonError::NonFiniteConstant("-Infinity"))
            }
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(CanonicalJsonError::NotJson),
        }
    }

    /// Consume an object key, the colon and any following whitespace.
    fn parse_object_key(&mut self) -> Result<JsonText, CanonicalJsonError> {
        self.skip_whitespace();
        if self.bytes.get(self.position) != Some(&b'"') {
            return Err(CanonicalJsonError::NotJson);
        }
        let key = self.parse_string()?;
        self.skip_whitespace();
        if self.bytes.get(self.position) != Some(&b':') {
            return Err(CanonicalJsonError::NotJson);
        }
        self.position += 1;
        self.skip_whitespace();
        Ok(key)
    }

    fn parse_string(&mut self) -> Result<JsonText, CanonicalJsonError> {
        self.position += 1; // opening quote
        let mut code_points: Vec<u32> = Vec::new();
        loop {
            let Some(byte) = self.bytes.get(self.position).copied() else {
                return Err(CanonicalJsonError::NotJson);
            };
            match byte {
                b'"' => {
                    self.position += 1;
                    return JsonText::from_code_points(code_points);
                }
                b'\\' => {
                    self.position += 1;
                    let Some(escape) = self.bytes.get(self.position).copied() else {
                        return Err(CanonicalJsonError::NotJson);
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
                            // when the immediately following escape is a low
                            // surrogate. Otherwise it keeps a lone code unit.
                            if (0xD800..=0xDBFF).contains(&first) {
                                let saved = self.position;
                                if self.bytes.get(self.position) == Some(&b'\\')
                                    && self.bytes.get(self.position + 1) == Some(&b'u')
                                {
                                    self.position += 2;
                                    match self.parse_hex_quad() {
                                        Ok(second) if (0xDC00..=0xDFFF).contains(&second) => {
                                            let combined = 0x1_0000
                                                + ((u32::from(first) - 0xD800) << 10)
                                                + (u32::from(second) - 0xDC00);
                                            code_points.push(combined);
                                            continue;
                                        }
                                        _ => {
                                            self.position = saved;
                                        }
                                    }
                                }
                            }
                            code_points.push(u32::from(first));
                        }
                        _ => return Err(CanonicalJsonError::NotJson),
                    }
                }
                0x00..=0x1F => return Err(CanonicalJsonError::NotJson),
                0x20..=0x7F => {
                    code_points.push(u32::from(byte));
                    self.position += 1;
                }
                _ => {
                    let character = self.text[self.position..]
                        .chars()
                        .next()
                        .ok_or(CanonicalJsonError::NotJson)?;
                    code_points.push(u32::from(character));
                    self.position += character.len_utf8();
                }
            }
        }
    }

    fn parse_hex_quad(&mut self) -> Result<u16, CanonicalJsonError> {
        let mut value = 0u16;
        for _ in 0..4 {
            let Some(byte) = self.bytes.get(self.position).copied() else {
                return Err(CanonicalJsonError::NotJson);
            };
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err(CanonicalJsonError::NotJson),
            };
            value = (value << 4) | u16::from(digit);
            self.position += 1;
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<CanonicalValue, CanonicalJsonError> {
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
            _ => return Err(CanonicalJsonError::NotJson),
        }
        let mut is_float = false;
        if self.bytes.get(self.position) == Some(&b'.') {
            is_float = true;
            self.position += 1;
            if !matches!(self.bytes.get(self.position).copied(), Some(b'0'..=b'9')) {
                return Err(CanonicalJsonError::NotJson);
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
                return Err(CanonicalJsonError::NotJson);
            }
            while matches!(self.bytes.get(self.position).copied(), Some(b'0'..=b'9')) {
                self.position += 1;
            }
        }
        let token = &self.text[start..self.position];
        if is_float {
            let value: f64 = token.parse().map_err(|_| CanonicalJsonError::NotJson)?;
            if !value.is_finite() {
                return Err(CanonicalJsonError::NonFinite);
            }
            Ok(CanonicalValue::Float(value))
        } else {
            let digit_count = self.position - integer_start;
            if self.limit_integer_digits(digit_count) {
                return Err(CanonicalJsonError::NotJson);
            }
            let value =
                BigInt::parse_bytes(token.as_bytes(), 10).ok_or(CanonicalJsonError::NotJson)?;
            Ok(CanonicalValue::Integer(value))
        }
    }

    fn limit_integer_digits(&self, digits: usize) -> bool {
        self.limits.max_integer_digits > 0 && digits > self.limits.max_integer_digits
    }

    fn expect_literal(&mut self, literal: &[u8]) -> Result<(), CanonicalJsonError> {
        if self.remaining().starts_with(literal) {
            self.position += literal.len();
            Ok(())
        } else {
            Err(CanonicalJsonError::NotJson)
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

    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.position..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<CanonicalValue, CanonicalJsonError> {
        parse_strict(text.as_bytes(), &Limits::default())
    }

    #[test]
    fn rejects_duplicate_keys_even_with_equal_values() {
        assert_eq!(
            parse(r#"{"a":1,"a":1}"#),
            Err(CanonicalJsonError::DuplicateKey)
        );
    }

    #[test]
    fn rejects_non_finite_tokens_and_overflow() {
        assert_eq!(
            parse("NaN"),
            Err(CanonicalJsonError::NonFiniteConstant("NaN"))
        );
        assert_eq!(
            parse("Infinity"),
            Err(CanonicalJsonError::NonFiniteConstant("Infinity"))
        );
        assert_eq!(
            parse("-Infinity"),
            Err(CanonicalJsonError::NonFiniteConstant("-Infinity"))
        );
        assert_eq!(parse("1e999"), Err(CanonicalJsonError::NonFinite));
        assert!(parse("1e-400").is_ok());
    }

    #[test]
    fn combines_escape_surrogate_pairs_only() {
        let value = parse(r#""\ud83d\ude00""#).unwrap();
        assert_eq!(
            value,
            CanonicalValue::Text(JsonText::from_code_points(vec![0x1F600]).unwrap())
        );
        let lone = parse(r#""\ud800""#).unwrap();
        assert_eq!(
            lone,
            CanonicalValue::Text(JsonText::from_code_points(vec![0xD800]).unwrap())
        );
        let separated = parse(r#""\ud83d\u0041""#).unwrap();
        assert_eq!(
            separated,
            CanonicalValue::Text(JsonText::from_code_points(vec![0xD83D, 0x41]).unwrap())
        );
    }

    #[test]
    fn rejects_invalid_json_shapes() {
        for text in [
            "",
            "{",
            "{} {}",
            "\u{feff}{}",
            "01",
            "1.",
            "+1",
            ".1",
            "nul",
        ] {
            assert!(parse(text).is_err(), "accepted {text:?}");
        }
    }

    #[test]
    fn parses_top_level_scalars_and_big_integers() {
        assert_eq!(
            parse("42").unwrap(),
            CanonicalValue::Integer(BigInt::from(42))
        );
        let big = parse("1180591620717411303424").unwrap();
        assert_eq!(
            big,
            CanonicalValue::Integer(BigInt::parse_bytes(b"1180591620717411303424", 10).unwrap())
        );
    }

    #[test]
    fn enforces_depth_limit() {
        let at_limit = format!("{}0{}", "[".repeat(48), "]".repeat(48));
        let over_limit = format!("{}0{}", "[".repeat(49), "]".repeat(49));
        assert!(parse(&at_limit).is_ok());
        assert_eq!(parse(&over_limit), Err(CanonicalJsonError::DepthExceeded));
    }

    #[test]
    fn parses_nested_containers_with_whitespace() {
        let value = parse(
            " \n { \"a\" : [ ] , \"b\" : { } , \"c\" : [ 1 , { \"d\" : [ true , null ] } ] } \t ",
        )
        .unwrap();
        assert_eq!(
            value,
            CanonicalValue::object([
                ("a", CanonicalValue::Array(Vec::new())),
                ("b", CanonicalValue::Object(CanonicalObject::new())),
                (
                    "c",
                    CanonicalValue::array([
                        CanonicalValue::integer(1),
                        CanonicalValue::object([(
                            "d",
                            CanonicalValue::array([
                                CanonicalValue::Bool(true),
                                CanonicalValue::Null,
                            ]),
                        )]),
                    ]),
                ),
            ])
        );
        assert_eq!(
            parse("{}").unwrap(),
            CanonicalValue::Object(CanonicalObject::new())
        );
        assert_eq!(parse("[]").unwrap(), CanonicalValue::Array(Vec::new()));
        assert_eq!(
            parse("[[],[{}]]").unwrap(),
            CanonicalValue::array([
                CanonicalValue::Array(Vec::new()),
                CanonicalValue::array([CanonicalValue::Object(CanonicalObject::new())]),
            ])
        );
    }

    #[test]
    fn rejects_trailing_commas_and_missing_separators() {
        for text in [
            "[1,]",
            "{\"a\":1,}",
            "[1 2]",
            "{\"a\":1 \"b\":2}",
            "{\"a\" 1}",
            "{\"a\":}",
            "[}",
            "{]",
            "{\"a\":1}}",
            "[[]",
        ] {
            assert!(parse(text).is_err(), "accepted {text:?}");
        }
    }

    #[test]
    fn rejects_duplicate_keys_in_nested_objects() {
        assert_eq!(
            parse(r#"{"outer":{"a":1,"a":2}}"#),
            Err(CanonicalJsonError::DuplicateKey)
        );
        assert_eq!(
            parse(r#"[{"a":1},{"a":1,"a":1}]"#),
            Err(CanonicalJsonError::DuplicateKey)
        );
    }
}
