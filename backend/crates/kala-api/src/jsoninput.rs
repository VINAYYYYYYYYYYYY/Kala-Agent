//! The JSON reader of a request body.
//!
//! The Python transport reads a body with `json.loads`, and the framework
//! reports the position of a decoding failure in the detail line it writes
//! (`body.29: json_invalid: JSON decode error`). Two properties of that
//! reader therefore have to be reproduced exactly:
//!
//! * **the failure position** is the character index at which CPython's
//!   scanner stopped. It is not always the end of the document and not always
//!   the first offending character: a missing value is reported where the
//!   value was expected, an unterminated string is reported where the string
//!   started, and a document that carries something after its value is
//!   reported where that something starts;
//! * **the accepted documents** are the ones `json.loads` accepts, which is a
//!   wider set than the strict protocol reader of the accepted `kala-json`
//!   crate: a repeated object key is *last wins* rather than a failure, and
//!   `NaN`, `Infinity` and `-Infinity` are numbers rather than failures.
//!
//! The reader is an explicit-stack state machine: a document nested far
//! beyond any ordinary recursion limit is read on an ordinary thread stack,
//! and a document that is malformed at any depth fails closed without
//! recursing.
//!
//! The value model is the canonical one of the accepted `kala-json` crate, so
//! the keys of an object are kept sorted by code point. That is the same
//! difference the schemas module already declares: the *order* of the
//! undeclared fields of a rejected body is the order of the value model, not
//! the order of the input.

use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use num_bigint::BigInt;

/// The failure of the reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JsonError {
    /// The character index at which CPython's scanner stopped.
    ///
    /// The Python source writes it as the second part of the location of the
    /// failure (`body.29`), so the index counts characters and not bytes.
    pub position: usize,
}

/// Read one JSON document the way CPython's `json.loads` reads it.
///
/// A leading UTF-8 byte order mark is stripped, because the Python source
/// reads a body as bytes and CPython then decodes those bytes as `utf-8-sig`.
///
/// # Errors
///
/// Returns the position at which the scanner stopped when the document is not
/// valid JSON.
pub fn parse(text: &str) -> Result<CanonicalValue, JsonError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = Reader {
        text,
        bytes: text.as_bytes(),
        position: 0,
    };
    reader.document()
}

/// One open container of the reader.
enum Frame {
    /// An array whose elements are read.
    Array {
        /// The elements read so far.
        items: Vec<CanonicalValue>,
    },
    /// An object whose members are read.
    Object {
        /// The members read so far, last wins for a repeated key.
        entries: CanonicalObject,
        /// The name of the member that is being read.
        key: Option<JsonText>,
    },
}

/// What the reader expects next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    /// A value.
    Value,
    /// A value or the end of an array.
    ValueOrEnd,
    /// A member name or the end of an object.
    KeyOrEnd,
    /// A member name, after a comma.
    Key,
    /// The colon that follows a member name.
    Colon,
    /// A comma or the end of the container.
    CommaOrEnd,
    /// The place a completed value is written to.
    Place,
    /// The end of the document.
    Done,
}

/// The reader of one document.
struct Reader<'a> {
    /// The document.
    text: &'a str,
    /// The bytes of the document.
    bytes: &'a [u8],
    /// The byte offset of the reader.
    position: usize,
}

impl Reader<'_> {
    /// Read the whole document.
    fn document(&mut self) -> Result<CanonicalValue, JsonError> {
        let mut stack: Vec<Frame> = Vec::new();
        let mut pending: Option<CanonicalValue> = None;
        let mut root: Option<CanonicalValue> = None;
        let mut expect = Expect::Value;
        // The place of the comma that waits for a value or for a member name.
        // A container that ends right after that comma is a trailing comma,
        // and CPython reports the comma and not the end of the container.
        let mut comma_at: Option<usize> = None;

        self.skip_whitespace();
        if self.at_end() {
            return Err(self.failure(self.position));
        }

        loop {
            match expect {
                Expect::Value => {
                    self.skip_whitespace();
                    let Some(byte) = self.peek() else {
                        return Err(self.failure(self.position));
                    };
                    if byte == b']' {
                        if let Some(at) = comma_at {
                            return Err(self.failure(at));
                        }
                        return Err(self.failure(self.position));
                    }
                    comma_at = None;
                    match byte {
                        b'{' => {
                            self.position += 1;
                            stack.push(Frame::Object {
                                entries: CanonicalObject::new(),
                                key: None,
                            });
                            expect = Expect::KeyOrEnd;
                        }
                        b'[' => {
                            self.position += 1;
                            stack.push(Frame::Array { items: Vec::new() });
                            expect = Expect::ValueOrEnd;
                        }
                        _ => {
                            pending = Some(self.scalar()?);
                            expect = Expect::Place;
                        }
                    }
                }
                Expect::ValueOrEnd => {
                    self.skip_whitespace();
                    let Some(byte) = self.peek() else {
                        return Err(self.failure(self.position));
                    };
                    if byte == b']' {
                        self.position += 1;
                        let items = match stack.pop() {
                            Some(Frame::Array { items }) => items,
                            _ => unreachable!("the open frame is an array"),
                        };
                        pending = Some(CanonicalValue::Array(items));
                        expect = Expect::Place;
                    } else {
                        expect = Expect::Value;
                    }
                }
                Expect::KeyOrEnd => {
                    self.skip_whitespace();
                    let Some(byte) = self.peek() else {
                        return Err(self.failure(self.position));
                    };
                    if byte == b'}' {
                        self.position += 1;
                        let entries = match stack.pop() {
                            Some(Frame::Object { entries, .. }) => entries,
                            _ => unreachable!("the open frame is an object"),
                        };
                        pending = Some(CanonicalValue::Object(entries));
                        expect = Expect::Place;
                    } else {
                        expect = Expect::Key;
                    }
                }
                Expect::Key => {
                    self.skip_whitespace();
                    let Some(byte) = self.peek() else {
                        return Err(self.failure(self.position));
                    };
                    if byte == b'}' {
                        if let Some(at) = comma_at {
                            return Err(self.failure(at));
                        }
                        return Err(self.failure(self.position));
                    }
                    if byte != b'"' {
                        return Err(self.failure(self.position));
                    }
                    comma_at = None;
                    let name = self.string()?;
                    match stack.last_mut() {
                        Some(Frame::Object { key, .. }) => *key = Some(name),
                        _ => unreachable!("the open frame is an object"),
                    }
                    expect = Expect::Colon;
                }
                Expect::Colon => {
                    self.skip_whitespace();
                    if self.peek() != Some(b':') {
                        return Err(self.failure(self.position));
                    }
                    self.position += 1;
                    expect = Expect::Value;
                }
                Expect::CommaOrEnd => {
                    self.skip_whitespace();
                    let Some(byte) = self.peek() else {
                        return Err(self.failure(self.position));
                    };
                    match byte {
                        b',' => {
                            comma_at = Some(self.position);
                            self.position += 1;
                            expect = match stack.last() {
                                Some(Frame::Array { .. }) => Expect::Value,
                                _ => Expect::Key,
                            };
                        }
                        b']' => {
                            if !matches!(stack.last(), Some(Frame::Array { .. })) {
                                return Err(self.failure(self.position));
                            }
                            self.position += 1;
                            let items = match stack.pop() {
                                Some(Frame::Array { items }) => items,
                                _ => unreachable!("the open frame is an array"),
                            };
                            pending = Some(CanonicalValue::Array(items));
                            expect = Expect::Place;
                        }
                        b'}' => {
                            if !matches!(stack.last(), Some(Frame::Object { .. })) {
                                return Err(self.failure(self.position));
                            }
                            self.position += 1;
                            let entries = match stack.pop() {
                                Some(Frame::Object { entries, .. }) => entries,
                                _ => unreachable!("the open frame is an object"),
                            };
                            pending = Some(CanonicalValue::Object(entries));
                            expect = Expect::Place;
                        }
                        _ => return Err(self.failure(self.position)),
                    }
                }
                Expect::Place => {
                    let value = pending.take().expect("a value waits to be placed");
                    match stack.last_mut() {
                        None => {
                            root = Some(value);
                            expect = Expect::Done;
                        }
                        Some(Frame::Array { items }) => {
                            items.push(value);
                            expect = Expect::CommaOrEnd;
                        }
                        Some(Frame::Object { entries, key }) => {
                            let name = key.take().expect("the object names the member");
                            entries.insert(name, value);
                            expect = Expect::CommaOrEnd;
                        }
                    }
                }
                Expect::Done => break,
            }
        }

        // Nothing may follow the document.
        self.skip_whitespace();
        if !self.at_end() {
            return Err(self.failure(self.position));
        }
        Ok(root.expect("the document carries a value"))
    }

    /// Read one scalar value.
    fn scalar(&mut self) -> Result<CanonicalValue, JsonError> {
        let byte = self.peek().expect("the caller checked the end");
        match byte {
            b'"' => Ok(CanonicalValue::Text(self.string()?)),
            b't' => self.literal("true", CanonicalValue::Bool(true)),
            b'f' => self.literal("false", CanonicalValue::Bool(false)),
            b'n' => self.literal("null", CanonicalValue::Null),
            b'N' => self.literal("NaN", CanonicalValue::Float(f64::NAN)),
            b'I' => self.literal("Infinity", CanonicalValue::Float(f64::INFINITY)),
            b'-' => {
                if self.bytes[self.position..].starts_with(b"-Infinity") {
                    self.position += "-Infinity".len();
                    return Ok(CanonicalValue::Float(f64::NEG_INFINITY));
                }
                self.number()
            }
            b'0'..=b'9' => self.number(),
            _ => Err(self.failure(self.position)),
        }
    }

    /// Read one of the fixed words of the language.
    fn literal(&mut self, word: &str, value: CanonicalValue) -> Result<CanonicalValue, JsonError> {
        if self.bytes[self.position..].starts_with(word.as_bytes()) {
            self.position += word.len();
            Ok(value)
        } else {
            Err(self.failure(self.position))
        }
    }

    /// Read one number.
    ///
    /// The reader follows CPython's scanner: the number ends where the
    /// scanner stops, so a number that is followed by a letter is a number and
    /// not a failure, and whatever follows it is reported on its own. A number
    /// that carries no digit at all is a failure at the place the number
    /// started.
    fn number(&mut self) -> Result<CanonicalValue, JsonError> {
        let start = self.position;
        let mut floating = false;
        if self.peek() == Some(b'-') {
            self.position += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.position += 1;
            }
            Some(byte) if byte.is_ascii_digit() => {
                while matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
                    self.position += 1;
                }
            }
            _ => return Err(self.failure(start)),
        }
        if self.peek() == Some(b'.')
            && matches!(self.bytes.get(self.position + 1), Some(byte) if byte.is_ascii_digit())
        {
            floating = true;
            self.position += 1;
            while matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
                self.position += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            let mut offset = self.position + 1;
            if matches!(self.bytes.get(offset), Some(b'+') | Some(b'-')) {
                offset += 1;
            }
            if matches!(self.bytes.get(offset), Some(byte) if byte.is_ascii_digit()) {
                floating = true;
                self.position = offset;
                while matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
                    self.position += 1;
                }
            }
        }
        let text = &self.text[start..self.position];
        if floating {
            let value: f64 = text.parse().map_err(|_| self.failure(start))?;
            Ok(CanonicalValue::Float(value))
        } else {
            match text.parse::<BigInt>() {
                Ok(value) => Ok(CanonicalValue::Integer(value)),
                Err(_) => Err(self.failure(start)),
            }
        }
    }

    /// Read one string, the opening quote included.
    fn string(&mut self) -> Result<JsonText, JsonError> {
        let start = self.position;
        self.position += 1;
        let mut out = String::new();
        loop {
            let Some(byte) = self.peek() else {
                // The string is not terminated: CPython reports the place the
                // string started.
                return Err(self.failure(start));
            };
            match byte {
                b'"' => {
                    self.position += 1;
                    return Ok(JsonText::new(&out));
                }
                b'\\' => {
                    let escape_at = self.position;
                    self.position += 1;
                    let Some(escape) = self.peek() else {
                        return Err(self.failure(start));
                    };
                    self.position += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let first = self.hex4(escape_at + 1)?;
                            let code = if (0xD800..0xDC00).contains(&first) {
                                if self.peek() != Some(b'\\') {
                                    return Err(self.failure(self.position));
                                }
                                self.position += 1;
                                if self.peek() != Some(b'u') {
                                    return Err(self.failure(self.position));
                                }
                                self.position += 1;
                                let second = self.hex4(self.position - 1)?;
                                if !(0xDC00..0xE000).contains(&second) {
                                    return Err(self.failure(self.position));
                                }
                                0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&first) {
                                return Err(self.failure(self.position));
                            } else {
                                first
                            };
                            match char::from_u32(code) {
                                Some(character) => out.push(character),
                                None => {
                                    // A lone surrogate: CPython keeps it and
                                    // the value model of the transport cannot
                                    // express it.
                                    out.push('\u{FFFD}');
                                }
                            }
                        }
                        _ => return Err(self.failure(escape_at)),
                    }
                }
                byte if byte < 0x20 => return Err(self.failure(self.position)),
                _ => {
                    // Copy one whole character.
                    let rest = &self.text[self.position..];
                    let character = rest.chars().next().expect("the reader is inside the text");
                    out.push(character);
                    self.position += character.len_utf8();
                }
            }
        }
    }

    /// Read four hexadecimal digits, the position of the `u` included.
    fn hex4(&mut self, escape_at: usize) -> Result<u32, JsonError> {
        let mut value = 0_u32;
        for _ in 0..4 {
            let Some(byte) = self.peek() else {
                return Err(self.failure(escape_at));
            };
            let digit = match byte {
                b'0'..=b'9' => u32::from(byte - b'0'),
                b'a'..=b'f' => u32::from(byte - b'a') + 10,
                b'A'..=b'F' => u32::from(byte - b'A') + 10,
                _ => return Err(self.failure(escape_at)),
            };
            value = value * 16 + digit;
            self.position += 1;
        }
        Ok(value)
    }

    /// Skip the whitespace of the language.
    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.peek() {
            match byte {
                b' ' | b'\t' | b'\n' | b'\r' => self.position += 1,
                _ => break,
            }
        }
    }

    /// The byte at the reader, if there is one.
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    /// Whether the reader is at the end of the document.
    fn at_end(&self) -> bool {
        self.position >= self.bytes.len()
    }

    /// One failure at a byte offset, reported as a character index.
    fn failure(&self, byte_offset: usize) -> JsonError {
        let offset = byte_offset.min(self.bytes.len());
        let position = self.text[..offset].chars().count();
        JsonError { position }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical(text: &str) -> String {
        let value = parse(text).expect("the document is valid");
        kala_json::canonical_json_dumps(&value).expect("the value is canonical")
    }

    fn position(text: &str) -> usize {
        parse(text).expect_err("the document is invalid").position
    }

    #[test]
    fn a_document_is_read_like_the_python_reader() {
        assert_eq!(canonical("{\"a\": 1}"), "{\"a\":1}");
        assert_eq!(canonical("[1, 2, 3]"), "[1,2,3]");
        assert_eq!(canonical("  true  "), "true");
        assert_eq!(canonical("null"), "null");
        assert_eq!(canonical("1.5"), "1.5");
        assert_eq!(canonical("1e3"), "1000.0");
        assert_eq!(canonical("1.5e3"), "1500.0");
        assert_eq!(canonical("-2"), "-2");
        assert_eq!(canonical("\"a\\u00e9b\""), "\"a\\u00e9b\"");
        assert_eq!(canonical("{\"a\" : 1}"), "{\"a\":1}");
        assert_eq!(canonical("{\"a\":1 }"), "{\"a\":1}");
        assert_eq!(canonical("{ \"a\": 1 }"), "{\"a\":1}");
        assert_eq!(canonical("[]"), "[]");
        assert_eq!(canonical("{}"), "{}");
    }

    #[test]
    fn a_repeated_key_is_last_wins() {
        assert_eq!(canonical("{\"a\": 1, \"a\": 2}"), "{\"a\":2}");
    }

    #[test]
    fn the_python_constants_are_numbers() {
        let value = parse("NaN").expect("the document is valid");
        assert!(matches!(value, CanonicalValue::Float(number) if number.is_nan()));
        let value = parse("Infinity").expect("the document is valid");
        assert!(matches!(value, CanonicalValue::Float(number) if number.is_infinite()));
        let value = parse("-Infinity").expect("the document is valid");
        assert!(matches!(value, CanonicalValue::Float(number) if number.is_infinite()));
    }

    #[test]
    fn a_failure_carries_the_position_of_the_python_scanner() {
        assert_eq!(position(""), 0);
        assert_eq!(position(" "), 1);
        assert_eq!(position("{"), 1);
        assert_eq!(position("}"), 0);
        assert_eq!(position("["), 1);
        assert_eq!(position("]"), 0);
        assert_eq!(position("{\"a\": }"), 6);
        assert_eq!(position("{\"a\": 1,}"), 7);
        assert_eq!(position("{\"a\": 1"), 7);
        assert_eq!(position("[1, 2"), 5);
        assert_eq!(position("{\"a\": \"x\""), 9);
        assert_eq!(position("{\"a\": \"xyz"), 6);
        assert_eq!(position("{\"a\": 1} extra"), 9);
        assert_eq!(position("1 2"), 2);
        assert_eq!(position("-"), 0);
        assert_eq!(position("-x"), 0);
        assert_eq!(position("01"), 1);
        assert_eq!(position("1."), 1);
        assert_eq!(position("1e"), 1);
        assert_eq!(position("1e+"), 1);
        assert_eq!(position("0x10"), 1);
        assert_eq!(position("1_0"), 1);
        assert_eq!(position("tru"), 0);
        assert_eq!(position("truex"), 4);
        assert_eq!(position("nul"), 0);
        assert_eq!(position("nan"), 0);
        assert_eq!(position("'a'"), 0);
        assert_eq!(position("{'a': 1}"), 1);
        assert_eq!(position("{\"a\": 1,,}"), 8);
        assert_eq!(position("{\"a\": 1, \"b\": 2,}"), 15);
        assert_eq!(position("{\"a\" 1}"), 5);
        assert_eq!(position("{\"a\":}"), 5);
        assert_eq!(position("[1 2]"), 3);
        assert_eq!(position("{\"a\": 1 \"b\": 2}"), 8);
        assert_eq!(position("{\"a\": \"\\q\"}"), 7);
        assert_eq!(position("{\"a\": \"line\nbreak\"}"), 11);
        assert_eq!(position("{\"a\": \"\\u00zz\"}"), 8);
        assert_eq!(position("[1,]"), 2);
        assert_eq!(position("[1, ]"), 2);
        assert_eq!(position("{\"a\":1, }"), 6);
        assert_eq!(position("{\"a\":1,,}"), 7);
        assert_eq!(position("{\"a\":1,,\"b\":2}"), 7);
        assert_eq!(position("[1,2,]"), 4);
        assert_eq!(position("[,]"), 1);
        assert_eq!(position("{\"a\":1,}"), 6);
        assert_eq!(position("{\"a\" : 1 , }"), 9);
        assert_eq!(position("{\"a\":1,\"b\":2,}"), 12);
    }

    #[test]
    fn a_deep_document_is_read_without_recursing() {
        let depth = 2_000;
        let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        let value = parse(&text).expect("the document is valid");
        let mut current = &value;
        for _ in 0..depth {
            let CanonicalValue::Array(items) = current else {
                panic!("the document is an array");
            };
            current = &items[0];
        }
        assert!(matches!(current, CanonicalValue::Integer(number) if number == &BigInt::from(1)));
    }
}
