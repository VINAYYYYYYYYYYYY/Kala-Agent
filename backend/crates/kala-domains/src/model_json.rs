//! Pydantic-compatible permissive model-input JSON parsing.
//!
//! Pydantic's `model_validate_json` accepts JSON documents with duplicate
//! object keys; the last value wins while the first insertion position is
//! retained, exactly like a Python `dict` assignment. The strict canonical
//! parser in `kala_json` deliberately refuses duplicates, so it cannot be the
//! ordinary model-input boundary.
//!
//! This module implements an iterative parser over an ordered representation.
//! It deliberately does not use `serde_json::Value`: serde's arbitrary
//! precision number is encoded as a private marker object
//! (`$serde_json::private::Number`), so an ordinary user key with that spelling
//! would be reinterpreted. Every object key and shape here is plain data, and
//! escaped surrogate pairs become their single Unicode code point like
//! Pydantic's JSON parser. Lone surrogate escapes are invalid JSON input.
//!
//! Canonical/hash parsing remains [`kala_json::parse_strict`]; this parser is
//! only the permissive model-input boundary.

use std::collections::HashMap;

use kala_json::{CanonicalObject, CanonicalValue, JsonText, Limits};
use num_bigint::BigInt;

use crate::error::{ParameterError, ParameterErrorKind};
use crate::parameters::ParameterModel;

/// One parsed model-input JSON value retaining object insertion order.
pub(crate) enum OrderedValue {
    Null,
    Bool(bool),
    Integer(BigInt),
    Float(f64),
    Text(JsonText),
    Array(Vec<OrderedValue>),
    Object(Vec<(JsonText, OrderedValue)>),
}

impl OrderedValue {
    /// The ordered entries when this value is a JSON object.
    pub(crate) fn object_entries(&self) -> Option<&[(JsonText, OrderedValue)]> {
        match self {
            Self::Object(entries) => Some(entries),
            _ => None,
        }
    }

    /// Convert to the canonical value model with an explicit work stack.
    ///
    /// Object keys are sorted because insertion order is no longer needed
    /// after duplicate keys have been collapsed and extras pre-checked.
    fn into_canonical(self) -> CanonicalValue {
        enum Work {
            Visit(OrderedValue),
            Array(usize),
            Object(Vec<JsonText>),
        }

        let mut work = vec![Work::Visit(self)];
        let mut results: Vec<CanonicalValue> = Vec::new();
        while let Some(task) = work.pop() {
            match task {
                Work::Visit(mut value) => match &mut value {
                    Self::Null => results.push(CanonicalValue::Null),
                    Self::Bool(value) => results.push(CanonicalValue::Bool(*value)),
                    Self::Integer(value) => {
                        results.push(CanonicalValue::Integer(std::mem::take(value)));
                    }
                    Self::Float(value) => results.push(CanonicalValue::Float(*value)),
                    Self::Text(value) => {
                        results.push(CanonicalValue::Text(std::mem::take(value)));
                    }
                    Self::Array(values) => {
                        let values = std::mem::take(values);
                        work.push(Work::Array(values.len()));
                        for item in values.into_iter().rev() {
                            work.push(Work::Visit(item));
                        }
                    }
                    Self::Object(entries) => {
                        let entries = std::mem::take(entries);
                        let keys = entries.iter().map(|(key, _)| key.clone()).collect();
                        work.push(Work::Object(keys));
                        for (_, item) in entries.into_iter().rev() {
                            work.push(Work::Visit(item));
                        }
                    }
                },
                Work::Array(len) => {
                    let start = results.len() - len;
                    let values = results.split_off(start);
                    results.push(CanonicalValue::Array(values));
                }
                Work::Object(keys) => {
                    let start = results.len() - keys.len();
                    let values = results.split_off(start);
                    let mut entries = CanonicalObject::new();
                    for (key, value) in keys.into_iter().zip(values) {
                        entries.insert(key, value);
                    }
                    results.push(CanonicalValue::Object(entries));
                }
            }
        }
        results.pop().expect("one parsed input value")
    }
}

impl Drop for OrderedValue {
    fn drop(&mut self) {
        // Iterative cleanup for deeply nested input. Each container is emptied
        // before its own drop runs, so a deeply nested document cannot
        // overflow the thread stack while being discarded.
        let mut pending: Vec<OrderedValue> = match self {
            Self::Array(items) => std::mem::take(items),
            Self::Object(entries) => std::mem::take(entries)
                .into_iter()
                .map(|(_, value)| value)
                .collect(),
            _ => return,
        };
        while let Some(mut value) = pending.pop() {
            match &mut value {
                Self::Array(items) => pending.append(items),
                Self::Object(entries) => {
                    pending.extend(std::mem::take(entries).into_iter().map(|(_, value)| value));
                }
                _ => {}
            }
        }
    }
}

/// Parse model JSON text with Pydantic-compatible duplicate-key handling.
pub(crate) fn parse_str(text: &str) -> Result<OrderedValue, String> {
    let mut parser = Parser {
        text,
        bytes: text.as_bytes(),
        position: 0,
    };
    parser.parse_document()
}

/// Parse model JSON bytes with Pydantic-compatible duplicate-key handling.
pub(crate) fn parse_bytes(bytes: &[u8]) -> Result<OrderedValue, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid_json())?;
    parse_str(text)
}

/// Validate a parsed model input for one parameter model.
///
/// Extras are checked before field validation, matching Pydantic's error
/// ordering, and in insertion order so the first reported extra is the first
/// one the document declared.
pub(crate) fn validate<P: ParameterModel>(parsed: OrderedValue) -> Result<P, ParameterError> {
    let entries = parsed.object_entries().ok_or_else(|| {
        ParameterError::root(ParameterErrorKind::ModelType, "Input should be an object")
    })?;
    for (key, _) in entries {
        let rendered = key.to_string_lossy();
        if !P::FIELD_NAMES.contains(&rendered.as_str()) {
            return Err(ParameterError::field(
                rendered,
                ParameterErrorKind::ExtraForbidden,
                "Extra inputs are not permitted",
            ));
        }
    }
    P::from_value(&parsed.into_canonical())
}

fn invalid_json() -> String {
    "value must be valid JSON".to_owned()
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    position: usize,
}

enum Frame {
    Array(Vec<OrderedValue>),
    Object {
        entries: Vec<(JsonText, OrderedValue)>,
        index: HashMap<JsonText, usize>,
        key: Option<JsonText>,
    },
}

enum Attach {
    NextValue,
    Close,
}

impl Parser<'_> {
    fn parse_document(&mut self) -> Result<OrderedValue, String> {
        self.skip_whitespace();
        let mut frames: Vec<Frame> = Vec::new();
        let mut current: Option<OrderedValue> = None;

        loop {
            if current.is_none() {
                self.skip_whitespace();
                match self.bytes.get(self.position).copied() {
                    Some(b'{') => {
                        self.position += 1;
                        self.skip_whitespace();
                        if self.bytes.get(self.position) == Some(&b'}') {
                            self.position += 1;
                            current = Some(OrderedValue::Object(Vec::new()));
                        } else {
                            let key = self.parse_object_key()?;
                            frames.push(Frame::Object {
                                entries: Vec::new(),
                                index: HashMap::new(),
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
                            current = Some(OrderedValue::Array(Vec::new()));
                        } else {
                            frames.push(Frame::Array(Vec::new()));
                            continue;
                        }
                    }
                    _ => current = Some(self.parse_scalar()?),
                }
            }

            loop {
                let Some(frame) = frames.last_mut() else {
                    self.skip_whitespace();
                    if self.position != self.bytes.len() {
                        return Err(invalid_json());
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
                            _ => return Err(invalid_json()),
                        }
                    }
                    Frame::Object {
                        entries,
                        index,
                        key: slot,
                    } => {
                        let key = slot.take().expect("pending object key");
                        let value = current.take().expect("object value");
                        insert_entry(entries, index, key, value);
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
                            _ => return Err(invalid_json()),
                        }
                    }
                };

                match action {
                    Attach::NextValue => break,
                    Attach::Close => {
                        let frame = frames.pop().expect("open frame");
                        current = Some(match frame {
                            Frame::Array(items) => OrderedValue::Array(items),
                            Frame::Object { entries, .. } => OrderedValue::Object(entries),
                        });
                    }
                }
            }
        }
    }

    fn parse_scalar(&mut self) -> Result<OrderedValue, String> {
        let Some(byte) = self.bytes.get(self.position).copied() else {
            return Err(invalid_json());
        };
        match byte {
            b'"' => Ok(OrderedValue::Text(self.parse_string()?)),
            b't' => {
                self.expect_literal(b"true")?;
                Ok(OrderedValue::Bool(true))
            }
            b'f' => {
                self.expect_literal(b"false")?;
                Ok(OrderedValue::Bool(false))
            }
            b'n' => {
                self.expect_literal(b"null")?;
                Ok(OrderedValue::Null)
            }
            b'N' => {
                self.expect_literal(b"NaN")?;
                Ok(OrderedValue::Float(f64::NAN))
            }
            b'I' => {
                self.expect_literal(b"Infinity")?;
                Ok(OrderedValue::Float(f64::INFINITY))
            }
            b'-' if self.remaining().starts_with(b"-Infinity") => {
                self.position += b"-Infinity".len();
                Ok(OrderedValue::Float(f64::NEG_INFINITY))
            }
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(invalid_json()),
        }
    }

    fn parse_object_key(&mut self) -> Result<JsonText, String> {
        self.skip_whitespace();
        if self.bytes.get(self.position) != Some(&b'"') {
            return Err(invalid_json());
        }
        let key = self.parse_string()?;
        self.skip_whitespace();
        if self.bytes.get(self.position) != Some(&b':') {
            return Err(invalid_json());
        }
        self.position += 1;
        self.skip_whitespace();
        Ok(key)
    }

    fn parse_string(&mut self) -> Result<JsonText, String> {
        self.position += 1; // opening quote
        let mut code_points: Vec<u32> = Vec::new();
        loop {
            let Some(byte) = self.bytes.get(self.position).copied() else {
                return Err(invalid_json());
            };
            match byte {
                b'"' => {
                    self.position += 1;
                    return JsonText::from_code_points(code_points).map_err(|_| invalid_json());
                }
                b'\\' => {
                    self.position += 1;
                    let Some(escape) = self.bytes.get(self.position).copied() else {
                        return Err(invalid_json());
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
                            match first {
                                0xD800..=0xDBFF => {
                                    // Pydantic's JSON parser requires an
                                    // immediately following low surrogate for
                                    // a high-surrogate escape.
                                    if self.bytes.get(self.position) != Some(&b'\\')
                                        || self.bytes.get(self.position + 1) != Some(&b'u')
                                    {
                                        return Err(invalid_json());
                                    }
                                    self.position += 2;
                                    let second = self.parse_hex_quad()?;
                                    if !(0xDC00..=0xDFFF).contains(&second) {
                                        return Err(invalid_json());
                                    }
                                    let combined = 0x1_0000
                                        + ((u32::from(first) - 0xD800) << 10)
                                        + (u32::from(second) - 0xDC00);
                                    code_points.push(combined);
                                }
                                0xDC00..=0xDFFF => return Err(invalid_json()),
                                _ => code_points.push(u32::from(first)),
                            }
                        }
                        _ => return Err(invalid_json()),
                    }
                }
                0x00..=0x1F => return Err(invalid_json()),
                0x20..=0x7F => {
                    code_points.push(u32::from(byte));
                    self.position += 1;
                }
                _ => {
                    let character = self.text[self.position..]
                        .chars()
                        .next()
                        .ok_or_else(invalid_json)?;
                    code_points.push(u32::from(character));
                    self.position += character.len_utf8();
                }
            }
        }
    }

    fn parse_hex_quad(&mut self) -> Result<u16, String> {
        let mut value = 0u16;
        for _ in 0..4 {
            let Some(byte) = self.bytes.get(self.position).copied() else {
                return Err(invalid_json());
            };
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err(invalid_json()),
            };
            value = (value << 4) | u16::from(digit);
            self.position += 1;
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<OrderedValue, String> {
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
            _ => return Err(invalid_json()),
        }
        let mut is_float = false;
        if self.bytes.get(self.position) == Some(&b'.') {
            is_float = true;
            self.position += 1;
            if !matches!(self.bytes.get(self.position).copied(), Some(b'0'..=b'9')) {
                return Err(invalid_json());
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
                return Err(invalid_json());
            }
            while matches!(self.bytes.get(self.position).copied(), Some(b'0'..=b'9')) {
                self.position += 1;
            }
        }
        let token = &self.text[start..self.position];
        if is_float {
            // Pydantic's parser accepts literals whose decimal value
            // overflows binary64 as infinity; the field constraints then
            // reject them with the ordinary range message.
            let value: f64 = token.parse().map_err(|_| invalid_json())?;
            Ok(OrderedValue::Float(value))
        } else {
            let digit_count = self.position - integer_start;
            if digit_count > Limits::DEFAULT_MAX_INTEGER_DIGITS {
                return Err(invalid_json());
            }
            let value = BigInt::parse_bytes(token.as_bytes(), 10).ok_or_else(invalid_json)?;
            Ok(OrderedValue::Integer(value))
        }
    }

    fn expect_literal(&mut self, literal: &[u8]) -> Result<(), String> {
        if self.remaining().starts_with(literal) {
            self.position += literal.len();
            Ok(())
        } else {
            Err(invalid_json())
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

fn insert_entry(
    entries: &mut Vec<(JsonText, OrderedValue)>,
    index: &mut HashMap<JsonText, usize>,
    key: JsonText,
    value: OrderedValue,
) {
    if let Some(position) = index.get(&key).copied() {
        // Python dict assignment keeps the first insertion position and
        // replaces the value, which is exactly the observable behaviour of
        // Pydantic's duplicate keys.
        entries[position].1 = value;
    } else {
        index.insert(key.clone(), entries.len());
        entries.push((key, value));
    }
}
