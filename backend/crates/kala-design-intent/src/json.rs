//! Ordered JSON values and a CPython-compatible `json.loads` reader.
//!
//! `kala_json::parse_strict` deliberately refuses duplicate keys and
//! `NaN`/`Infinity` because it is the persisted canonical format. A model reply
//! is ordinary Python JSON, so this reader mirrors `json.loads`: duplicate keys
//! keep the last value at the first position, `NaN`/`Infinity` are accepted and
//! are refused later by `allow_inf_nan=False`, and syntax errors use CPython's
//! `JSONDecodeError.msg` vocabulary (which `parse_design_intent_text` exposes
//! verbatim).
//!
//! The reader is recursive with an explicit 512-level bound. That is far deeper
//! than any construction script and cannot overflow the thread stack on
//! attacker-sized input; CPython instead raises `RecursionError` near its
//! interpreter recursion limit.

use kala_contracts::Text;
use kala_json::{CanonicalObject, CanonicalValue, format_float};
use num_bigint::BigInt;

/// Maximum nesting the reply reader accepts.
pub(crate) const MAX_DEPTH: usize = 512;

/// One parsed JSON value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JsonValue {
    Null,
    Bool(bool),
    Integer(BigInt),
    /// May be non-finite: `NaN`, `Infinity`, `-Infinity`, or a literal that
    /// overflows binary64 such as `1e400`.
    Float(f64),
    Text(Text),
    Array(Vec<JsonValue>),
    /// Insertion order is retained; duplicate keys replace the value in place.
    Object(Vec<(Text, JsonValue)>),
}

impl JsonValue {
    pub(crate) fn get(&self, name: &str) -> Option<&JsonValue> {
        let Self::Object(entries) = self else {
            return None;
        };
        entries
            .iter()
            .find(|(key, _)| key.as_str() == Some(name))
            .map(|(_, value)| value)
    }

    pub(crate) fn entries(&self) -> Option<&[(Text, JsonValue)]> {
        match self {
            Self::Object(entries) => Some(entries),
            _ => None,
        }
    }

    pub(crate) fn as_text(&self) -> Option<&Text> {
        match self {
            Self::Text(value) => Some(value),
            _ => None,
        }
    }

    /// Convert to the strict canonical value model. Fails on non-finite
    /// numbers because canonical values cannot represent them.
    pub(crate) fn to_canonical(&self) -> Option<CanonicalValue> {
        Some(match self {
            Self::Null => CanonicalValue::Null,
            Self::Bool(value) => CanonicalValue::Bool(*value),
            Self::Integer(value) => CanonicalValue::Integer(value.clone()),
            Self::Float(value) => CanonicalValue::float(*value).ok()?,
            Self::Text(value) => CanonicalValue::Text(value.as_json_text().clone()),
            Self::Array(values) => CanonicalValue::Array(
                values
                    .iter()
                    .map(Self::to_canonical)
                    .collect::<Option<Vec<_>>>()?,
            ),
            Self::Object(entries) => {
                let mut object = CanonicalObject::new();
                for (key, value) in entries {
                    object.insert(key.as_json_text().clone(), value.to_canonical()?);
                }
                CanonicalValue::Object(object)
            }
        })
    }

    /// Python `str(value)`, used in discriminated-union tag errors.
    pub(crate) fn python_str(&self) -> String {
        self.python_repr(false)
    }

    fn python_repr(&self, nested: bool) -> String {
        match self {
            Self::Null => "None".to_owned(),
            Self::Bool(true) => "True".to_owned(),
            Self::Bool(false) => "False".to_owned(),
            Self::Integer(value) => value.to_string(),
            Self::Float(value) => format_float(*value).unwrap_or_else(|_| {
                if value.is_nan() {
                    "nan".to_owned()
                } else if value.is_sign_negative() {
                    "-inf".to_owned()
                } else {
                    "inf".to_owned()
                }
            }),
            Self::Text(value) => {
                if nested {
                    python_text_repr(value)
                } else {
                    value.to_string_lossy()
                }
            }
            Self::Array(values) => {
                let parts: Vec<String> =
                    values.iter().map(|value| value.python_repr(true)).collect();
                format!("[{}]", parts.join(", "))
            }
            Self::Object(entries) => {
                let parts: Vec<String> = entries
                    .iter()
                    .map(|(key, value)| {
                        format!("{}: {}", python_text_repr(key), value.python_repr(true))
                    })
                    .collect();
                format!("{{{}}}", parts.join(", "))
            }
        }
    }
}

fn python_text_repr(text: &Text) -> String {
    let mut out = String::new();
    out.push('\'');
    for point in text.code_points() {
        match char::from_u32(*point) {
            Some('\'') => out.push_str("\\'"),
            Some('\\') => out.push_str("\\\\"),
            Some('\n') => out.push_str("\\n"),
            Some('\r') => out.push_str("\\r"),
            Some('\t') => out.push_str("\\t"),
            Some(_) if (*point < 0x20) => {
                out.push_str(&format!("\\x{point:02x}"));
            }
            Some(character) => out.push(character),
            None => out.push('\u{FFFD}'),
        }
    }
    out.push('\'');
    out
}

/// Convert a strict canonical value into the ordered reply value model.
pub(crate) fn from_canonical(value: &CanonicalValue) -> JsonValue {
    match value {
        CanonicalValue::Null => JsonValue::Null,
        CanonicalValue::Bool(value) => JsonValue::Bool(*value),
        CanonicalValue::Integer(value) => JsonValue::Integer(value.clone()),
        CanonicalValue::Float(value) => JsonValue::Float(*value),
        CanonicalValue::Text(value) => JsonValue::Text(Text::from_json_text(value.clone())),
        CanonicalValue::Array(values) => {
            JsonValue::Array(values.iter().map(from_canonical).collect())
        }
        CanonicalValue::Object(entries) => JsonValue::Object(
            entries
                .iter()
                .map(|(key, value)| (Text::from_json_text(key.clone()), from_canonical(value)))
                .collect(),
        ),
    }
}

/// Parse with `json.loads` semantics. The error is CPython's `msg` text.
pub(crate) fn parse_python_json(text: &str) -> Result<JsonValue, String> {
    let mut parser = Parser { text, position: 0 };
    if parser.peek() == Some('\u{FEFF}') {
        return Err("Unexpected UTF-8 BOM (decode using utf-8-sig)".to_owned());
    }
    parser.skip_whitespace();
    let value = parser.parse_value(0)?;
    parser.skip_whitespace();
    if parser.position != parser.text.len() {
        return Err("Extra data".to_owned());
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a str,
    position: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.text[self.position..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.position += character.len_utf8();
        Some(character)
    }

    fn starts_with(&self, literal: &str) -> bool {
        self.text[self.position..].starts_with(literal)
    }

    fn skip_whitespace(&mut self) {
        while let Some(character) = self.peek() {
            if matches!(character, ' ' | '\t' | '\n' | '\r') {
                self.position += 1;
            } else {
                break;
            }
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<JsonValue, String> {
        if depth > MAX_DEPTH {
            return Err("JSON nesting exceeds the supported depth".to_owned());
        }
        self.skip_whitespace();
        match self.peek() {
            Some('"') => Ok(JsonValue::Text(self.parse_string()?)),
            Some('{') => self.parse_object(depth),
            Some('[') => self.parse_array(depth),
            Some('t') => {
                self.expect_literal("true")?;
                Ok(JsonValue::Bool(true))
            }
            Some('f') => {
                self.expect_literal("false")?;
                Ok(JsonValue::Bool(false))
            }
            Some('n') => {
                self.expect_literal("null")?;
                Ok(JsonValue::Null)
            }
            Some('N') => {
                self.expect_literal("NaN")?;
                Ok(JsonValue::Float(f64::NAN))
            }
            Some('I') => {
                self.expect_literal("Infinity")?;
                Ok(JsonValue::Float(f64::INFINITY))
            }
            Some('-') if self.starts_with("-Infinity") => {
                self.expect_literal("-Infinity")?;
                Ok(JsonValue::Float(f64::NEG_INFINITY))
            }
            Some(character) if character.is_ascii_digit() || character == '-' => {
                self.parse_number()
            }
            _ => Err("Expecting value".to_owned()),
        }
    }

    fn expect_literal(&mut self, literal: &str) -> Result<(), String> {
        if self.starts_with(literal) {
            self.position += literal.len();
            Ok(())
        } else {
            Err("Expecting value".to_owned())
        }
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.position;
        if self.peek() == Some('-') {
            self.position += 1;
        }
        match self.peek() {
            Some('0') => {
                self.position += 1;
            }
            Some(character) if character.is_ascii_digit() => {
                while matches!(self.peek(), Some(character) if character.is_ascii_digit()) {
                    self.position += 1;
                }
            }
            _ => return Err("Expecting value".to_owned()),
        }
        let integer_end = self.position;
        let mut is_float = false;
        if self.peek() == Some('.')
            && self.text[self.position + 1..]
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
        {
            is_float = true;
            self.position += 1;
            while matches!(self.peek(), Some(character) if character.is_ascii_digit()) {
                self.position += 1;
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let mut lookahead = self.position + 1;
            if matches!(self.text[lookahead..].chars().next(), Some('+' | '-')) {
                lookahead += 1;
            }
            if self.text[lookahead..]
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
            {
                is_float = true;
                self.position = lookahead;
                while matches!(self.peek(), Some(character) if character.is_ascii_digit()) {
                    self.position += 1;
                }
            }
        }
        let literal = &self.text[start..self.position];
        if !is_float {
            return BigInt::parse_bytes(literal.as_bytes(), 10)
                .map(JsonValue::Integer)
                .ok_or_else(|| "Expecting value".to_owned());
        }
        let _ = integer_end;
        literal
            .parse::<f64>()
            .map(JsonValue::Float)
            .map_err(|_| "Expecting value".to_owned())
    }

    fn parse_string(&mut self) -> Result<Text, String> {
        if self.bump() != Some('"') {
            return Err("Unterminated string starting at".to_owned());
        }
        let mut code_points: Vec<u32> = Vec::new();
        loop {
            let Some(character) = self.bump() else {
                return Err("Unterminated string starting at".to_owned());
            };
            match character {
                '"' => {
                    return Text::from_code_points(code_points)
                        .map_err(|_| "Unterminated string starting at".to_owned());
                }
                '\\' => {
                    let Some(escape) = self.bump() else {
                        return Err("Unterminated string starting at".to_owned());
                    };
                    let value = match escape {
                        '"' => 0x22,
                        '\\' => 0x5C,
                        '/' => 0x2F,
                        'b' => 0x08,
                        'f' => 0x0C,
                        'n' => 0x0A,
                        'r' => 0x0D,
                        't' => 0x09,
                        'u' => self.parse_unicode_escape()?,
                        _ => return Err("Invalid \\escape".to_owned()),
                    };
                    code_points.push(value);
                }
                character if u32::from(character) < 0x20 => {
                    return Err("Invalid control character at".to_owned());
                }
                character => code_points.push(u32::from(character)),
            }
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<u32, String> {
        let high = self.read_hex_quad()?;
        if (0xD800..=0xDBFF).contains(&high) && self.text[self.position..].starts_with("\\u") {
            let saved = self.position;
            self.position += 2;
            match self.read_hex_quad() {
                Ok(low) if (0xDC00..=0xDFFF).contains(&low) => {
                    return Ok(0x1_0000 + ((high - 0xD800) << 10) + (low - 0xDC00));
                }
                _ => {
                    self.position = saved;
                }
            }
        }
        Ok(high)
    }

    fn read_hex_quad(&mut self) -> Result<u32, String> {
        let mut value = 0u32;
        for _ in 0..4 {
            let Some(character) = self.bump() else {
                return Err("Invalid \\uXXXX escape".to_owned());
            };
            let Some(digit) = character.to_digit(16) else {
                return Err("Invalid \\uXXXX escape".to_owned());
            };
            value = (value << 4) | digit;
        }
        Ok(value)
    }

    fn parse_object(&mut self, depth: usize) -> Result<JsonValue, String> {
        self.bump();
        let mut entries: Vec<(Text, JsonValue)> = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(JsonValue::Object(entries));
        }
        loop {
            if self.peek() != Some('"') {
                return Err("Expecting property name enclosed in double quotes".to_owned());
            }
            let key = self.parse_string()?;
            self.skip_whitespace();
            if self.peek() != Some(':') {
                return Err("Expecting ':' delimiter".to_owned());
            }
            self.bump();
            let value = self.parse_value(depth + 1)?;
            if let Some(existing) = entries.iter_mut().find(|(existing, _)| *existing == key) {
                existing.1 = value;
            } else {
                entries.push((key, value));
            }
            self.skip_whitespace();
            match self.peek() {
                Some('}') => {
                    self.bump();
                    return Ok(JsonValue::Object(entries));
                }
                Some(',') => {
                    self.bump();
                    self.skip_whitespace();
                    if self.peek() == Some('}') {
                        return Err("Illegal trailing comma before end of object".to_owned());
                    }
                }
                _ => return Err("Expecting ',' delimiter".to_owned()),
            }
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<JsonValue, String> {
        self.bump();
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(']') {
            self.bump();
            return Ok(JsonValue::Array(values));
        }
        loop {
            values.push(self.parse_value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(']') => {
                    self.bump();
                    return Ok(JsonValue::Array(values));
                }
                Some(',') => {
                    self.bump();
                    self.skip_whitespace();
                    if self.peek() == Some(']') {
                        return Err("Illegal trailing comma before end of array".to_owned());
                    }
                }
                _ => return Err("Expecting ',' delimiter".to_owned()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_error_vocabulary() {
        let cases = [
            ("", "Expecting value"),
            (" ", "Expecting value"),
            ("{", "Expecting property name enclosed in double quotes"),
            ("{\"a\":}", "Expecting value"),
            ("{\"a\" 1}", "Expecting ':' delimiter"),
            ("{\"a\":1", "Expecting ',' delimiter"),
            ("[1 2]", "Expecting ',' delimiter"),
            ("[1,]", "Illegal trailing comma before end of array"),
            ("{\"a\":1,}", "Illegal trailing comma before end of object"),
            ("\"abc", "Unterminated string starting at"),
            ("\"\\q\"", "Invalid \\escape"),
            ("\"\\u12\"", "Invalid \\uXXXX escape"),
            ("\"a\nb\"", "Invalid control character at"),
            ("01", "Extra data"),
            ("1.", "Extra data"),
            ("+1", "Expecting value"),
            ("-", "Expecting value"),
            ("nul", "Expecting value"),
            ("{\"a\":1} x", "Extra data"),
            (
                "\u{FEFF}{}",
                "Unexpected UTF-8 BOM (decode using utf-8-sig)",
            ),
            ("{\"a\":1 \"b\":2}", "Expecting ',' delimiter"),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_python_json(text).unwrap_err(), expected, "{text:?}");
        }
    }

    #[test]
    fn duplicates_keep_the_first_position_and_the_last_value() {
        let value = parse_python_json(r#"{"a":1,"b":2,"a":3}"#).unwrap();
        let entries = value.entries().unwrap();
        assert_eq!(entries[0].0.as_str(), Some("a"));
        assert_eq!(entries[1].0.as_str(), Some("b"));
        assert!(matches!(entries[0].1, JsonValue::Integer(_)));
        assert_eq!(value.get("a").unwrap().python_str(), "3");
    }

    #[test]
    fn constants_and_surrogates_are_preserved() {
        let value = parse_python_json(r#"{"n":NaN,"i":-Infinity,"s":"\ud834\udd1e"}"#).unwrap();
        assert!(matches!(value.get("n"), Some(JsonValue::Float(number)) if number.is_nan()));
        assert!(
            matches!(value.get("i"), Some(JsonValue::Float(number)) if *number == f64::NEG_INFINITY)
        );
        assert_eq!(value.get("s").unwrap().as_text().unwrap().len(), 1);
    }
}
