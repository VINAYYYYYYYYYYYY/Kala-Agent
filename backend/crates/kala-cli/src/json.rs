//! CPython-compatible JSON for the command line's inputs and outputs.
//!
//! Two different JSON dialects meet at this boundary, and they are not the
//! same as the canonical JSON the rest of Kala hashes:
//!
//! * the *input* dialect is plain `json.loads` (the one `--param` and
//!   `--parameters-json` use): duplicate object keys keep the last value,
//!   `NaN`/`Infinity`/`-Infinity` are accepted, integers are unbounded and
//!   lone surrogate escapes are preserved, exactly as CPython decodes them;
//! * the *output* dialect is `json.dumps(payload, indent=2, sort_keys=True)`,
//!   the document every command prints.
//!
//! Both encoders escape strings the way `ensure_ascii=True` does, sort object
//! keys by code point, and render floats with CPython's shortest round-trip
//! repr (through [`kala_json::format_float`]).
//!
//! Both the parser and the encoders use explicit heap stacks, so a deeply
//! nested value from an operator's argv is bounded by memory rather than by
//! the thread stack.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use kala_json::{CanonicalValue, JsonText, format_float};
use num_bigint::BigInt;

/// The largest container depth CPython 3.14's `json` accepts before it raises
/// `RecursionError` on this platform.
pub const MAX_DEPTH: usize = 52_099;

/// The largest number of digits CPython converts between `int` and `str`
/// before `int` raises `ValueError`.
pub const MAX_INTEGER_DIGITS: usize = 4_300;

/// Why an input document is not valid `json.loads` input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The text is not a complete JSON value.
    Syntax,
    /// The value nests deeper than CPython accepts.
    TooDeep,
    /// An integer is wider than CPython's int/str conversion limit.
    IntegerLimit,
}

/// Parse one JSON document the way `json.loads` does.
///
/// # Errors
///
/// Returns [`LoadError::Syntax`] for malformed input, [`LoadError::TooDeep`]
/// when the value nests beyond [`MAX_DEPTH`], and
/// [`LoadError::IntegerLimit`] for an oversized integer.
pub fn loads(text: &str) -> Result<CanonicalValue, LoadError> {
    let mut parser = Parser {
        text: text.chars().collect(),
        position: 0,
    };
    parser.skip_space();
    let mut stack: Vec<Frame> = Vec::new();
    loop {
        let value = match stack.last() {
            Some(Frame::Object { pending: None, .. }) => {
                let key = parser.parse_key()?;
                if let Some(Frame::Object { pending, .. }) = stack.last_mut() {
                    *pending = Some(key);
                }
                match parser.parse_value(&mut stack)? {
                    Some(value) => value,
                    None => continue,
                }
            }
            _ => match parser.parse_value(&mut stack)? {
                Some(value) => value,
                None => continue,
            },
        };
        if let Some(value) = parser.attach(value, &mut stack)? {
            parser.skip_space();
            if parser.position != parser.text.len() {
                return Err(LoadError::Syntax);
            }
            return Ok(value);
        }
    }
}

/// Encode a value as `json.dumps(value, indent=2, sort_keys=True)`.
///
/// # Panics
///
/// Cannot panic: every value the CLI builds is finite and in range.
#[must_use]
pub fn pretty(value: &CanonicalValue) -> String {
    encode(value, Some(2))
}

/// Encode a value as `json.dumps(value)` (the default separators, no indent).
///
/// This is the exact byte string the CLI's parameter-size check measures.
#[must_use]
pub fn default_dumps(value: &CanonicalValue) -> String {
    encode(value, None)
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

enum Frame {
    Array(Vec<CanonicalValue>),
    Object {
        entries: BTreeMap<JsonText, CanonicalValue>,
        pending: Option<JsonText>,
    },
}

struct Parser {
    text: Vec<char>,
    position: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.text.get(self.position).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.position += 1;
        }
    }

    fn parse_value(&mut self, stack: &mut Vec<Frame>) -> Result<Option<CanonicalValue>, LoadError> {
        self.skip_space();
        match self.peek() {
            Some('[') => {
                self.position += 1;
                if stack.len() >= MAX_DEPTH {
                    return Err(LoadError::TooDeep);
                }
                stack.push(Frame::Array(Vec::new()));
                self.skip_space();
                if self.peek() == Some(']') {
                    self.position += 1;
                    return Ok(Some(CanonicalValue::Array(Vec::new())));
                }
                Ok(None)
            }
            Some('{') => {
                self.position += 1;
                if stack.len() >= MAX_DEPTH {
                    return Err(LoadError::TooDeep);
                }
                stack.push(Frame::Object {
                    entries: BTreeMap::new(),
                    pending: None,
                });
                self.skip_space();
                if self.peek() == Some('}') {
                    self.position += 1;
                    return Ok(Some(CanonicalValue::Object(BTreeMap::new())));
                }
                Ok(None)
            }
            Some(_) => Ok(Some(self.parse_scalar()?)),
            None => Err(LoadError::Syntax),
        }
    }

    fn parse_key(&mut self) -> Result<JsonText, LoadError> {
        self.skip_space();
        if self.peek() != Some('"') {
            return Err(LoadError::Syntax);
        }
        let key = self.parse_string()?;
        self.skip_space();
        if self.peek() != Some(':') {
            return Err(LoadError::Syntax);
        }
        self.position += 1;
        Ok(key)
    }

    fn parse_scalar(&mut self) -> Result<CanonicalValue, LoadError> {
        match self.peek() {
            Some('"') => Ok(CanonicalValue::Text(self.parse_string()?)),
            Some('t') => {
                self.expect_word("true")?;
                Ok(CanonicalValue::Bool(true))
            }
            Some('f') => {
                self.expect_word("false")?;
                Ok(CanonicalValue::Bool(false))
            }
            Some('n') => {
                self.expect_word("null")?;
                Ok(CanonicalValue::Null)
            }
            Some('N') => {
                self.expect_word("NaN")?;
                Ok(CanonicalValue::Float(f64::NAN))
            }
            Some('I') => {
                self.expect_word("Infinity")?;
                Ok(CanonicalValue::Float(f64::INFINITY))
            }
            Some('-') => {
                if self.text[self.position..].starts_with(&['-', 'I']) {
                    self.expect_word("-Infinity")?;
                    return Ok(CanonicalValue::Float(f64::NEG_INFINITY));
                }
                self.parse_number()
            }
            Some(character) if character.is_ascii_digit() => self.parse_number(),
            _ => Err(LoadError::Syntax),
        }
    }

    fn expect_word(&mut self, word: &str) -> Result<(), LoadError> {
        let mut index = self.position;
        for expected in word.chars() {
            if self.text.get(index) != Some(&expected) {
                return Err(LoadError::Syntax);
            }
            index += 1;
        }
        self.position = index;
        Ok(())
    }

    fn parse_number(&mut self) -> Result<CanonicalValue, LoadError> {
        let start = self.position;
        if self.peek() == Some('-') {
            self.position += 1;
        }
        // Integer part: `0` or a nonzero digit followed by digits.
        match self.peek() {
            Some('0') => {
                self.position += 1;
                if matches!(self.peek(), Some(character) if character.is_ascii_digit()) {
                    return Err(LoadError::Syntax);
                }
            }
            Some(character) if character.is_ascii_digit() => {
                while matches!(self.peek(), Some(character) if character.is_ascii_digit()) {
                    self.position += 1;
                }
            }
            _ => return Err(LoadError::Syntax),
        }
        let mut is_float = false;
        if self.peek() == Some('.') {
            is_float = true;
            self.position += 1;
            if !matches!(self.peek(), Some(character) if character.is_ascii_digit()) {
                return Err(LoadError::Syntax);
            }
            while matches!(self.peek(), Some(character) if character.is_ascii_digit()) {
                self.position += 1;
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            is_float = true;
            self.position += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.position += 1;
            }
            if !matches!(self.peek(), Some(character) if character.is_ascii_digit()) {
                return Err(LoadError::Syntax);
            }
            while matches!(self.peek(), Some(character) if character.is_ascii_digit()) {
                self.position += 1;
            }
        }
        let rendered: String = self.text[start..self.position].iter().collect();
        if is_float {
            let value: f64 = rendered.parse().map_err(|_| LoadError::Syntax)?;
            return Ok(CanonicalValue::Float(value));
        }
        let digits = rendered.trim_start_matches('-').len();
        if digits > MAX_INTEGER_DIGITS {
            return Err(LoadError::IntegerLimit);
        }
        let value = BigInt::parse_bytes(rendered.as_bytes(), 10).ok_or(LoadError::Syntax)?;
        Ok(CanonicalValue::Integer(value))
    }

    fn parse_string(&mut self) -> Result<JsonText, LoadError> {
        // The caller has checked the opening quote.
        self.position += 1;
        let mut code_points: Vec<u32> = Vec::new();
        loop {
            let Some(character) = self.peek() else {
                return Err(LoadError::Syntax);
            };
            match character {
                '"' => {
                    self.position += 1;
                    return JsonText::from_code_points(code_points).map_err(|_| LoadError::Syntax);
                }
                '\\' => {
                    self.position += 1;
                    let Some(escape) = self.peek() else {
                        return Err(LoadError::Syntax);
                    };
                    self.position += 1;
                    match escape {
                        '"' => code_points.push(u32::from('"')),
                        '\\' => code_points.push(u32::from('\\')),
                        '/' => code_points.push(u32::from('/')),
                        'b' => code_points.push(0x08),
                        'f' => code_points.push(0x0C),
                        'n' => code_points.push(0x0A),
                        'r' => code_points.push(0x0D),
                        't' => code_points.push(0x09),
                        'u' => {
                            let value = self.parse_hex_quad()?;
                            if (0xD800..=0xDBFF).contains(&value)
                                && self.text.get(self.position) == Some(&'\\')
                                && self.text.get(self.position + 1) == Some(&'u')
                            {
                                let saved = self.position;
                                self.position += 2;
                                let low = self.parse_hex_quad()?;
                                if (0xDC00..=0xDFFF).contains(&low) {
                                    let combined =
                                        0x1_0000 + ((value - 0xD800) << 10) + (low - 0xDC00);
                                    code_points.push(combined);
                                } else {
                                    self.position = saved;
                                    code_points.push(value);
                                }
                            } else {
                                code_points.push(value);
                            }
                        }
                        _ => return Err(LoadError::Syntax),
                    }
                }
                character if (character as u32) < 0x20 => return Err(LoadError::Syntax),
                character => {
                    self.position += 1;
                    code_points.push(u32::from(character));
                }
            }
        }
    }

    fn parse_hex_quad(&mut self) -> Result<u32, LoadError> {
        let mut value = 0u32;
        for _ in 0..4 {
            let Some(character) = self.peek() else {
                return Err(LoadError::Syntax);
            };
            let digit = character.to_digit(16).ok_or(LoadError::Syntax)?;
            value = (value << 4) | digit;
            self.position += 1;
        }
        Ok(value)
    }

    /// Attach a completed value to the innermost open container, closing every
    /// container that ends here. Returns the finished document at the root.
    fn attach(
        &mut self,
        mut value: CanonicalValue,
        stack: &mut Vec<Frame>,
    ) -> Result<Option<CanonicalValue>, LoadError> {
        loop {
            match stack.last_mut() {
                None => return Ok(Some(value)),
                Some(Frame::Array(items)) => {
                    items.push(value);
                    self.skip_space();
                    match self.peek() {
                        Some(',') => {
                            self.position += 1;
                            return Ok(None);
                        }
                        Some(']') => {
                            self.position += 1;
                            let Some(Frame::Array(items)) = stack.pop() else {
                                return Err(LoadError::Syntax);
                            };
                            value = CanonicalValue::Array(items);
                        }
                        _ => return Err(LoadError::Syntax),
                    }
                }
                Some(Frame::Object { entries, pending }) => {
                    let Some(key) = pending.take() else {
                        return Err(LoadError::Syntax);
                    };
                    entries.insert(key, value);
                    self.skip_space();
                    match self.peek() {
                        Some(',') => {
                            self.position += 1;
                            return Ok(None);
                        }
                        Some('}') => {
                            self.position += 1;
                            let Some(Frame::Object { entries, .. }) = stack.pop() else {
                                return Err(LoadError::Syntax);
                            };
                            value = CanonicalValue::Object(entries);
                        }
                        _ => return Err(LoadError::Syntax),
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

enum Task<'a> {
    Value(&'a CanonicalValue, usize),
    Key(&'a JsonText),
    Literal(&'static str),
    Indent(usize),
}

fn encode(value: &CanonicalValue, indent: Option<usize>) -> String {
    let mut out = String::new();
    let mut stack = vec![Task::Value(value, 0)];
    while let Some(task) = stack.pop() {
        match task {
            Task::Literal(literal) => out.push_str(literal),
            Task::Indent(level) => {
                if let Some(width) = indent {
                    let total = width.saturating_mul(level);
                    out.extend(std::iter::repeat_n(' ', total));
                }
            }
            Task::Key(key) => encode_text(key.code_points(), &mut out),
            Task::Value(value, level) => match value {
                CanonicalValue::Null => out.push_str("null"),
                CanonicalValue::Bool(true) => out.push_str("true"),
                CanonicalValue::Bool(false) => out.push_str("false"),
                CanonicalValue::Integer(number) => {
                    let _ = write!(out, "{number}");
                }
                CanonicalValue::Float(number) => out.push_str(&float_text(*number)),
                CanonicalValue::Text(text) => encode_text(text.code_points(), &mut out),
                CanonicalValue::Array(values) => {
                    if values.is_empty() {
                        out.push_str("[]");
                        continue;
                    }
                    out.push('[');
                    if indent.is_some() {
                        stack.push(Task::Literal("]"));
                        stack.push(Task::Indent(level));
                        stack.push(Task::Literal("\n"));
                        for (index, item) in values.iter().enumerate().rev() {
                            if index + 1 < values.len() {
                                stack.push(Task::Literal(","));
                            }
                            stack.push(Task::Value(item, level + 1));
                            stack.push(Task::Indent(level + 1));
                            stack.push(Task::Literal("\n"));
                        }
                    } else {
                        stack.push(Task::Literal("]"));
                        for (index, item) in values.iter().enumerate().rev() {
                            if index + 1 < values.len() {
                                stack.push(Task::Literal(", "));
                            }
                            stack.push(Task::Value(item, level + 1));
                        }
                    }
                }
                CanonicalValue::Object(entries) => {
                    if entries.is_empty() {
                        out.push_str("{}");
                        continue;
                    }
                    out.push('{');
                    if indent.is_some() {
                        stack.push(Task::Literal("}"));
                        stack.push(Task::Indent(level));
                        stack.push(Task::Literal("\n"));
                        for (index, (key, item)) in entries.iter().enumerate().rev() {
                            if index + 1 < entries.len() {
                                stack.push(Task::Literal(","));
                            }
                            stack.push(Task::Value(item, level + 1));
                            stack.push(Task::Literal(": "));
                            stack.push(Task::Key(key));
                            stack.push(Task::Indent(level + 1));
                            stack.push(Task::Literal("\n"));
                        }
                    } else {
                        stack.push(Task::Literal("}"));
                        for (index, (key, item)) in entries.iter().enumerate().rev() {
                            if index + 1 < entries.len() {
                                stack.push(Task::Literal(", "));
                            }
                            stack.push(Task::Value(item, level + 1));
                            stack.push(Task::Literal(": "));
                            stack.push(Task::Key(key));
                        }
                    }
                }
            },
        }
    }
    out
}

fn float_text(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value == f64::INFINITY {
        return "Infinity".to_owned();
    }
    if value == f64::NEG_INFINITY {
        return "-Infinity".to_owned();
    }
    format_float(value).expect("finite float")
}

/// CPython's `json.dumps(..., ensure_ascii=True)` string escaping.
fn encode_text(code_points: &[u32], out: &mut String) {
    out.push('"');
    for point in code_points {
        match *point {
            0x22 => out.push_str("\\\""),
            0x5C => out.push_str("\\\\"),
            0x08 => out.push_str("\\b"),
            0x09 => out.push_str("\\t"),
            0x0A => out.push_str("\\n"),
            0x0C => out.push_str("\\f"),
            0x0D => out.push_str("\\r"),
            0x00..=0x1F => {
                let _ = write!(out, "\\u{point:04x}");
            }
            0x20..=0x7E => out.push(char::from_u32(*point).expect("ASCII")),
            0x7F..=0xFFFF => {
                let _ = write!(out, "\\u{point:04x}");
            }
            _ => {
                let adjusted = point - 0x1_0000;
                let high = 0xD800 + (adjusted >> 10);
                let low = 0xDC00 + (adjusted & 0x3FF);
                let _ = write!(out, "\\u{high:04x}\\u{low:04x}");
            }
        }
    }
    out.push('"');
}
