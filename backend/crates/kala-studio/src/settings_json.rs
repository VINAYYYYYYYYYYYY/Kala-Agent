//! CPython `json.loads` projection for `studio-settings.json`.
//!
//! The store must accept exactly the documents CPython accepts. Two of them
//! are outside the reach of the reviewed parsers already used by this crate:
//! `serde_json` rejects an escaped lone surrogate, and the strict `kala-json`
//! parser rejects duplicate object keys. A Python-accepted settings file can
//! contain both at once (for example two `api_keys` members plus an unrelated
//! `"\ud800"` value), and neither parser sees the `api_keys` map at all, so
//! the store silently loses valid credentials and a later unrelated write can
//! persist that loss.
//!
//! This module implements the narrow CPython-compatible walk the settings
//! store needs, without touching the reviewed strict parser:
//!
//! * the strict JSON grammar evaluated iteratively (no parser depth limit of
//!   its own; CPython's own process-dependent scanner stack is the only
//!   accepted-depth boundary),
//! * CPython's `NaN`, `Infinity` and `-Infinity` constants and number tokens
//!   of any magnitude,
//! * duplicate object keys with Python's last-wins semantics, with keys and
//!   string values decoded to code points so escaped lone surrogates survive
//!   byte-for-byte,
//! * CPython's default 4,300-digit integer conversion limit, reported in scan
//!   order. That failure is a `ValueError` the source does not catch: reads
//!   become the sanitized 500, and a settings write refuses with a 422 that
//!   carries the exact CPython message.
//!
//! Only the settings projection is produced: the **last** `api_keys` member
//! of the root object and, inside its object value, the **last** string value
//! of each supported provider. Everything else is validated and ignored.

use std::collections::BTreeMap;

use kala_json::JsonText;

use crate::settings::SUPPORTED_CREDENTIAL_PROVIDERS;

/// CPython's `sys.int_info.default_max_str_digits`: the source never changes
/// it, so an integer literal with more digits raises `ValueError`.
pub const INTEGER_DIGIT_LIMIT: usize = 4_300;

/// Result of the CPython-compatible settings walk.
pub enum SettingsDocument {
    /// `json.loads` succeeded. The map holds the last decoded string value of
    /// each provider found in the last `api_keys` object; values that are not
    /// strings (or a last `api_keys` that is not an object) are absent.
    Parsed(BTreeMap<&'static str, JsonText>),
    /// `json.loads` raised `JSONDecodeError`; the source treats the file as
    /// `{}`.
    Malformed,
    /// `json.loads` raised the integer conversion `ValueError`; the contained
    /// text is the exact CPython message.
    IntegerLimit(String),
}

/// The exact CPython 3.14 `ValueError` text for an over-long integer.
#[must_use]
pub fn integer_limit_message(digits: usize) -> String {
    format!(
        "Exceeds the limit ({INTEGER_DIGIT_LIMIT} digits) for integer string conversion: \
         value has {digits} digits; use sys.set_int_max_str_digits() to increase the limit"
    )
}

/// Parse a settings document with CPython `json.loads` semantics.
#[must_use]
pub fn parse_settings_document(text: &str) -> SettingsDocument {
    let mut parser = Parser {
        text,
        bytes: text.as_bytes(),
        position: 0,
    };
    match parser.parse_document() {
        Ok(api_keys) => api_keys,
        Err(SettingsParseError::Malformed) => SettingsDocument::Malformed,
        Err(SettingsParseError::IntegerLimit(message)) => SettingsDocument::IntegerLimit(message),
    }
}

/// An internal failure; converted to [`SettingsDocument`] by the entry point.
enum SettingsParseError {
    Malformed,
    IntegerLimit(String),
}

/// The last `api_keys` value of the root object.
enum LastApiKeys {
    /// No root `api_keys` member was seen.
    Unseen,
    /// The last root `api_keys` member was not an object.
    NotObject,
    /// The last root `api_keys` member was an object; each entry is the last
    /// value for a supported provider (`None` when it was not a string).
    Object(BTreeMap<&'static str, Option<JsonText>>),
}

/// A completed value, only as detailed as the projection needs.
enum ValueSummary {
    /// A string value (needed for provider candidates).
    Text(JsonText),
    /// A closed `api_keys`-tagged object.
    ApiKeysObject(BTreeMap<&'static str, Option<JsonText>>),
    /// A closed root object.
    Root(LastApiKeys),
    /// Any other value (scalar, array or unrelated object).
    Other,
}

/// Which object a frame belongs to.
enum ObjectKind {
    /// The top-level object.
    Root,
    /// The object value of the root's `api_keys` member.
    ApiKeys,
    /// Any other object.
    Other,
}

/// One open container.
enum Frame {
    Array,
    Object {
        kind: ObjectKind,
        /// The decoded key whose value is currently being parsed.
        key: Option<JsonText>,
        providers: BTreeMap<&'static str, Option<JsonText>>,
        api_keys: LastApiKeys,
    },
}

/// What to do after a completed value has been attached to its parent frame.
enum Attach {
    /// The frame continues; the caller parses the next value.
    NextValue,
    /// The frame's closing token was consumed.
    Close,
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    position: usize,
}

impl Parser<'_> {
    fn parse_document(&mut self) -> Result<SettingsDocument, SettingsParseError> {
        self.skip_whitespace();
        let mut frames: Vec<Frame> = Vec::new();
        let mut current: Option<ValueSummary> = None;

        loop {
            if current.is_none() {
                self.skip_whitespace();
                match self.peek() {
                    Some(b'{') => {
                        self.position += 1;
                        self.skip_whitespace();
                        let kind = object_kind(&frames);
                        if self.peek() == Some(b'}') {
                            self.position += 1;
                            current =
                                Some(closed_object(kind, BTreeMap::new(), LastApiKeys::Unseen));
                        } else {
                            let key = self.parse_string()?;
                            self.skip_whitespace();
                            self.expect_byte(b':')?;
                            frames.push(Frame::Object {
                                kind,
                                key: Some(key),
                                providers: BTreeMap::new(),
                                api_keys: LastApiKeys::Unseen,
                            });
                            continue;
                        }
                    }
                    Some(b'[') => {
                        self.position += 1;
                        self.skip_whitespace();
                        if self.peek() == Some(b']') {
                            self.position += 1;
                            current = Some(ValueSummary::Other);
                        } else {
                            frames.push(Frame::Array);
                            continue;
                        }
                    }
                    _ => current = Some(self.parse_scalar()?),
                }
            }

            // A completed value is attached to its parent. Closing a container
            // produces the parent's next completed value.
            let mut attach = true;
            while attach {
                let Some(frame) = frames.last_mut() else {
                    self.skip_whitespace();
                    if self.position != self.bytes.len() {
                        return Err(SettingsParseError::Malformed);
                    }
                    let summary = current.take().ok_or(SettingsParseError::Malformed)?;
                    let api_keys = match summary {
                        ValueSummary::Root(api_keys) => api_keys,
                        _ => LastApiKeys::Unseen,
                    };
                    return Ok(SettingsDocument::Parsed(finalize(api_keys)));
                };

                let action = match frame {
                    Frame::Array => {
                        self.skip_whitespace();
                        match self.peek() {
                            Some(b',') => {
                                self.position += 1;
                                current = None;
                                Attach::NextValue
                            }
                            Some(b']') => {
                                self.position += 1;
                                Attach::Close
                            }
                            _ => return Err(SettingsParseError::Malformed),
                        }
                    }
                    Frame::Object {
                        kind,
                        key,
                        providers,
                        api_keys,
                    } => {
                        let pending = key.take().ok_or(SettingsParseError::Malformed)?;
                        let value = current.take().ok_or(SettingsParseError::Malformed)?;
                        match kind {
                            ObjectKind::Root => {
                                if matches_ascii(&pending, "api_keys") {
                                    *api_keys = match value {
                                        ValueSummary::ApiKeysObject(map) => {
                                            LastApiKeys::Object(map)
                                        }
                                        _ => LastApiKeys::NotObject,
                                    };
                                }
                            }
                            ObjectKind::ApiKeys => {
                                if let Some(name) = provider_name(&pending) {
                                    let candidate = match value {
                                        ValueSummary::Text(text) => Some(text),
                                        _ => None,
                                    };
                                    providers.insert(name, candidate);
                                }
                            }
                            ObjectKind::Other => {}
                        }
                        self.skip_whitespace();
                        match self.peek() {
                            Some(b',') => {
                                self.position += 1;
                                self.skip_whitespace();
                                let next = self.parse_string()?;
                                self.skip_whitespace();
                                self.expect_byte(b':')?;
                                *key = Some(next);
                                current = None;
                                Attach::NextValue
                            }
                            Some(b'}') => {
                                self.position += 1;
                                Attach::Close
                            }
                            _ => return Err(SettingsParseError::Malformed),
                        }
                    }
                };

                match action {
                    Attach::NextValue => attach = false,
                    Attach::Close => {
                        let frame = frames.pop().expect("an open frame was matched");
                        current = Some(match frame {
                            Frame::Array => ValueSummary::Other,
                            Frame::Object {
                                kind,
                                providers,
                                api_keys,
                                ..
                            } => closed_object(kind, providers, api_keys),
                        });
                    }
                }
            }
        }
    }

    /// Parse one scalar value: string, literal, constant or number.
    fn parse_scalar(&mut self) -> Result<ValueSummary, SettingsParseError> {
        match self.peek() {
            Some(b'"') => Ok(ValueSummary::Text(self.parse_string()?)),
            Some(b't') => {
                self.expect_literal(b"true")?;
                Ok(ValueSummary::Other)
            }
            Some(b'f') => {
                self.expect_literal(b"false")?;
                Ok(ValueSummary::Other)
            }
            Some(b'n') => {
                self.expect_literal(b"null")?;
                Ok(ValueSummary::Other)
            }
            // CPython's `json` accepts these non-standard constants by
            // default (`parse_constant`). Their value is never a provider
            // string, so only the token has to be recognized; the following
            // attach step requires a delimiter.
            Some(b'N') => {
                self.expect_literal(b"NaN")?;
                Ok(ValueSummary::Other)
            }
            Some(b'I') => {
                self.expect_literal(b"Infinity")?;
                Ok(ValueSummary::Other)
            }
            Some(b'-') => {
                if self.remaining().starts_with(b"-Infinity") {
                    self.position += "-Infinity".len();
                    Ok(ValueSummary::Other)
                } else {
                    // `-NaN` and similar are not accepted by CPython.
                    self.parse_number()
                }
            }
            Some(b'0'..=b'9') => self.parse_number(),
            _ => Err(SettingsParseError::Malformed),
        }
    }

    /// Parse and validate one number token, enforcing CPython's integer digit
    /// limit at the moment the token completes (Python's scan order).
    fn parse_number(&mut self) -> Result<ValueSummary, SettingsParseError> {
        if self.peek() == Some(b'-') {
            self.position += 1;
        }
        let integer_start = self.position;
        match self.peek() {
            Some(b'0') => self.position += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.position += 1;
                }
            }
            _ => return Err(SettingsParseError::Malformed),
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.position += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(SettingsParseError::Malformed);
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.position += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.position += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.position += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(SettingsParseError::Malformed);
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.position += 1;
            }
        }
        if !is_float {
            let digits = self.position - integer_start;
            if digits > INTEGER_DIGIT_LIMIT {
                return Err(SettingsParseError::IntegerLimit(integer_limit_message(
                    digits,
                )));
            }
        }
        Ok(ValueSummary::Other)
    }

    /// Parse one JSON string into code points, combining a `\u` surrogate pair
    /// exactly like CPython and keeping a lone surrogate escape as a lone code
    /// point.
    fn parse_string(&mut self) -> Result<JsonText, SettingsParseError> {
        if self.peek() != Some(b'"') {
            return Err(SettingsParseError::Malformed);
        }
        self.position += 1;
        let mut points: Vec<u32> = Vec::new();
        loop {
            let byte = self.peek().ok_or(SettingsParseError::Malformed)?;
            match byte {
                b'"' => {
                    self.position += 1;
                    return JsonText::from_code_points(points)
                        .map_err(|_| SettingsParseError::Malformed);
                }
                b'\\' => {
                    self.position += 1;
                    let escape = self.peek().ok_or(SettingsParseError::Malformed)?;
                    self.position += 1;
                    match escape {
                        b'"' => points.push(0x22),
                        b'\\' => points.push(0x5C),
                        b'/' => points.push(0x2F),
                        b'b' => points.push(0x08),
                        b'f' => points.push(0x0C),
                        b'n' => points.push(0x0A),
                        b'r' => points.push(0x0D),
                        b't' => points.push(0x09),
                        b'u' => {
                            let first = self.parse_hex4()?;
                            if (0xD800..=0xDBFF).contains(&first)
                                && self.remaining().starts_with(b"\\u")
                            {
                                let saved = self.position;
                                self.position += 2;
                                if let Ok(second) = self.parse_hex4()
                                    && (0xDC00..=0xDFFF).contains(&second)
                                {
                                    let combined =
                                        0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00);
                                    points.push(combined);
                                    continue;
                                }
                                // Not a valid pair: CPython keeps the lone
                                // high surrogate and decodes the following
                                // escape separately.
                                self.position = saved;
                            }
                            points.push(first);
                        }
                        _ => return Err(SettingsParseError::Malformed),
                    }
                }
                0x00..=0x1F => return Err(SettingsParseError::Malformed),
                _ => {
                    let character = self.text[self.position..]
                        .chars()
                        .next()
                        .ok_or(SettingsParseError::Malformed)?;
                    points.push(u32::from(character));
                    self.position += character.len_utf8();
                }
            }
        }
    }

    /// Read exactly four hexadecimal digits.
    fn parse_hex4(&mut self) -> Result<u32, SettingsParseError> {
        let digits = self
            .bytes
            .get(self.position..self.position + 4)
            .ok_or(SettingsParseError::Malformed)?;
        let mut value = 0u32;
        for byte in digits {
            let digit = match byte {
                b'0'..=b'9' => u32::from(byte - b'0'),
                b'a'..=b'f' => u32::from(byte - b'a') + 10,
                b'A'..=b'F' => u32::from(byte - b'A') + 10,
                _ => return Err(SettingsParseError::Malformed),
            };
            value = value * 16 + digit;
        }
        self.position += 4;
        Ok(value)
    }

    fn expect_literal(&mut self, literal: &[u8]) -> Result<(), SettingsParseError> {
        if self.remaining().starts_with(literal) {
            self.position += literal.len();
            Ok(())
        } else {
            Err(SettingsParseError::Malformed)
        }
    }

    fn expect_byte(&mut self, byte: u8) -> Result<(), SettingsParseError> {
        if self.peek() == Some(byte) {
            self.position += 1;
            Ok(())
        } else {
            Err(SettingsParseError::Malformed)
        }
    }

    fn skip_whitespace(&mut self) {
        // CPython's json scanner accepts exactly these four whitespace bytes.
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.position += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn remaining(&self) -> &[u8] {
        &self.bytes[self.position..]
    }
}

/// The kind of an object opened at the current value position.
fn object_kind(frames: &[Frame]) -> ObjectKind {
    match frames.last() {
        None => ObjectKind::Root,
        Some(Frame::Object {
            kind: ObjectKind::Root,
            key: Some(key),
            ..
        }) if matches_ascii(key, "api_keys") => ObjectKind::ApiKeys,
        _ => ObjectKind::Other,
    }
}

/// The summary a closing object contributes to its parent.
fn closed_object(
    kind: ObjectKind,
    providers: BTreeMap<&'static str, Option<JsonText>>,
    api_keys: LastApiKeys,
) -> ValueSummary {
    match kind {
        ObjectKind::Root => ValueSummary::Root(api_keys),
        ObjectKind::ApiKeys => ValueSummary::ApiKeysObject(providers),
        ObjectKind::Other => ValueSummary::Other,
    }
}

/// Reduce the root `api_keys` record to the provider string candidates.
fn finalize(api_keys: LastApiKeys) -> BTreeMap<&'static str, JsonText> {
    let LastApiKeys::Object(entries) = api_keys else {
        return BTreeMap::new();
    };
    entries
        .into_iter()
        .filter_map(|(provider, value)| value.map(|text| (provider, text)))
        .collect()
}

/// Whether decoded text equals an ASCII literal, code point for code point.
fn matches_ascii(text: &JsonText, expected: &str) -> bool {
    text.code_points()
        .iter()
        .copied()
        .eq(expected.chars().map(u32::from))
}

/// The supported provider a decoded object key names, if any.
fn provider_name(key: &JsonText) -> Option<&'static str> {
    SUPPORTED_CREDENTIAL_PROVIDERS
        .iter()
        .copied()
        .find(|name| matches_ascii(key, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run the projection and render it as `provider=text` pairs for easy
    /// comparison with the live-Python outcomes repeated in the comments.
    fn projection(text: &str) -> Result<Vec<String>, ()> {
        match parse_settings_document(text) {
            SettingsDocument::Parsed(map) => Ok(map
                .into_iter()
                .map(|(provider, value)| format!("{provider}={}", value.to_string_lossy()))
                .collect()),
            SettingsDocument::Malformed => Err(()),
            SettingsDocument::IntegerLimit(message) => panic!("unexpected ValueError: {message}"),
        }
    }

    fn keys(text: &str) -> Vec<String> {
        projection(text).expect("a Python-accepted document")
    }

    #[test]
    fn last_wins_duplicates_at_root_and_nested_level() {
        // CPython: `json.loads` keeps the last `api_keys` member and the last
        // provider value, then `read_stored_keys` reads those.
        assert_eq!(
            keys(r#"{"api_keys":{},"api_keys":{"openai":"sk-last"}}"#),
            ["openai=sk-last"]
        );
        assert_eq!(
            keys(r#"{"api_keys":{"openai":"sk-first"},"api_keys":{"openai":"sk-second"}}"#),
            ["openai=sk-second"]
        );
        assert_eq!(
            keys(r#"{"api_keys":{"openai":"sk-first","openai":"sk-second"}}"#),
            ["openai=sk-second"]
        );
        assert_eq!(
            keys(r#"{"api_keys":{"openai":"sk-a","openai":5}}"#),
            Vec::<String>::new()
        );
        assert_eq!(
            keys(r#"{"api_keys":{"openai":5,"openai":"sk-a"}}"#),
            ["openai=sk-a"]
        );
        assert_eq!(
            keys(r#"{"api_keys":{"openai":"sk-a"},"api_keys":[]}"#),
            Vec::<String>::new()
        );
        assert_eq!(
            keys(r#"{"api_keys":[],"api_keys":{"openai":"sk-a"}}"#),
            ["openai=sk-a"]
        );
        assert_eq!(
            keys(r#"{"api_keys":null,"api_keys":{"openai":"sk-final"}}"#),
            ["openai=sk-final"]
        );
    }

    #[test]
    fn escaped_surrogates_survive_fields_and_values() {
        // REVIEW-3 probe: the duplicate root member is accepted by Python
        // (last wins) even though an unrelated field holds `"\ud800"`.
        let parsed = keys(
            r#"{"api_keys":{},"api_keys":{"openai":"synthetic-review-key"},"unused":"\ud800"}"#,
        );
        assert_eq!(parsed, ["openai=synthetic-review-key"]);

        // An actual surrogate in the provider value keeps its code point.
        match parse_settings_document(r#"{"api_keys":{"openai":"sk-\ud800"}}"#) {
            SettingsDocument::Parsed(map) => {
                let value = map.get("openai").expect("openai candidate");
                assert_eq!(value.code_points(), &[0x73, 0x6B, 0x2D, 0xD800]);
            }
            _ => panic!("the document is accepted by CPython"),
        }

        // A surrogate pair combines; a separated high surrogate stays lone.
        assert_eq!(
            keys(r#"{"api_keys":{"openai":"sk-\ud83d\ude00"}}"#),
            ["openai=sk-😀"]
        );
        match parse_settings_document(r#"{"api_keys":{"openai":"\ud83d\u0041"}}"#) {
            SettingsDocument::Parsed(map) => {
                assert_eq!(map["openai"].code_points(), &[0xD83D, 0x41]);
            }
            _ => panic!("the document is accepted by CPython"),
        }
    }

    #[test]
    fn decoded_keys_match_escaped_spellings() {
        assert_eq!(
            keys(r#"{"\u0061pi_keys":{"open\u0061i":"sk-escaped"}}"#),
            ["openai=sk-escaped"]
        );
        // A surrogate in an unrelated key does not disturb the projection.
        assert_eq!(
            keys(r#"{"api_keys":{"\ud800":"sk-x","openai":"sk-a"}}"#),
            ["openai=sk-a"]
        );
    }

    #[test]
    fn duplicate_surrogate_and_extended_documents_combine() {
        // The combination the strict parser cannot read: duplicates, a lone
        // surrogate in a provider value and an unrelated deep valid value.
        let mut document = String::from(
            r#"{"api_keys":{},"api_keys":{"openai":"sk-\ud800","openai":"sk-final"},"other":"#,
        );
        document.push_str(&"[".repeat(400));
        document.push_str(&"]".repeat(400));
        document.push_str(r#","nonfinite":NaN,"infinity":-Infinity,"huge":1e999}"#);
        match parse_settings_document(&document) {
            SettingsDocument::Parsed(map) => {
                assert_eq!(
                    map["openai"].code_points(),
                    &[0x73, 0x6B, 0x2D, 0x66, 0x69, 0x6E, 0x61, 0x6C]
                );
            }
            _ => panic!("the document is accepted by CPython"),
        }
    }

    #[test]
    fn nonfinite_constants_and_huge_numbers_are_values() {
        assert_eq!(
            keys(r#"{"api_keys":{"openai":"sk-a"},"v":NaN,"w":[Infinity,-Infinity]}"#),
            ["openai=sk-a"]
        );
        assert_eq!(
            keys(r#"{"api_keys":{"openai":"sk-a"},"v":1e999}"#),
            ["openai=sk-a"]
        );
        assert_eq!(
            keys(r#"{"api_keys":{"openai":"sk-a"},"v":-1e-999}"#),
            ["openai=sk-a"]
        );
        assert_eq!(
            keys(r#"{"api_keys":{"openai":"sk-a"},"v":"not a number 1e999"}"#),
            ["openai=sk-a"]
        );
    }

    #[test]
    fn integer_digit_limit_matches_cpython_scan_order() {
        let big = "9".repeat(4_301);
        let document = format!(r#"{{"api_keys":{{"openai":"sk-x"}},"unused":{big}}}"#);
        match parse_settings_document(&document) {
            SettingsDocument::IntegerLimit(message) => assert_eq!(
                message,
                "Exceeds the limit (4300 digits) for integer string conversion: value has 4301 digits; use sys.set_int_max_str_digits() to increase the limit"
            ),
            _ => panic!("CPython raises ValueError for the integer literal"),
        }

        // At the boundary the document is accepted; digits count excludes the
        // sign, and a float form is never an integer conversion.
        let at_limit = format!(
            r#"{{"unused":{},"api_keys":{{"openai":"sk-x"}}}}"#,
            "9".repeat(4_300)
        );
        assert_eq!(keys(&at_limit), ["openai=sk-x"]);
        let negative = format!(r#"{{"unused":-{big}}}"#);
        assert!(matches!(
            parse_settings_document(&negative),
            SettingsDocument::IntegerLimit(message) if message.contains("value has 4301 digits")
        ));
        let float_form = format!(r#"{{"unused":{big}.5}}"#);
        assert_eq!(keys(&float_form), Vec::<String>::new());
        let exponent = format!(r#"{{"unused":{big}e1}}"#);
        assert_eq!(keys(&exponent), Vec::<String>::new());
        // A digit string inside a string value or key is not a conversion.
        assert_eq!(
            keys(&format!(r#"{{"api_keys":{{"openai":"{big}"}}}}"#)),
            [format!("openai={big}")]
        );

        // Scan order: a big integer before a later syntax error still raises
        // the ValueError, while an earlier syntax error wins as Python's
        // JSONDecodeError.
        let big_first = format!(r#"{{"unused":{big},"bad":}}"#);
        assert!(matches!(
            parse_settings_document(&big_first),
            SettingsDocument::IntegerLimit(_)
        ));
        let bad_first = format!(r#"{{"bad":,"unused":{big}}}"#);
        assert!(matches!(
            parse_settings_document(&bad_first),
            SettingsDocument::Malformed
        ));
    }

    #[test]
    fn malformed_documents_are_rejected_like_cpython() {
        for document in [
            "",
            "not json",
            "{",
            "{} {}",
            "{\"api_keys\":{\"openai\":\"sk-x\"}} x",
            "{\"api_keys\":{\"openai\":\"sk-x\",}}",
            "{\"api_keys\":[\"sk-x\",]}",
            "{\"api_keys\":{\"openai\":\"sk-\\q\"}}",
            "{\"api_keys\":{\"openai\":\"sk-\\u12\"}}",
            "{\"api_keys\":{\"openai\":\"sk-\u{1}\"}}",
            "{\"api_keys\":{\"openai\":01}}",
            "{\"api_keys\":{\"openai\":1.}}",
            "{\"api_keys\":{\"openai\":1e}}",
            "{\"api_keys\":{\"openai\":+1}}",
            "{\"api_keys\":{\"openai\":.1}}",
            "{\"api_keys\":{\"openai\":NaN123}}",
            "{\"api_keys\":{\"openai\":-NaN}}",
            "{\"api_keys\":{\"openai\":nul}}",
            "{\"api_keys\":{\"openai\":trueX}}",
            "{\"api_keys\":{\"openai\":\"sk-x\"} \"openrouter\":\"sk-y\"}}",
            "\u{feff}{\"api_keys\":{\"openai\":\"sk-x\"}}",
            "{\"api_keys\" 1}",
        ] {
            assert!(
                matches!(
                    parse_settings_document(document),
                    SettingsDocument::Malformed
                ),
                "accepted {document:?}"
            );
        }
    }

    #[test]
    fn non_object_roots_and_values_are_ignored() {
        assert_eq!(keys("42"), Vec::<String>::new());
        assert_eq!(keys("\"string\""), Vec::<String>::new());
        assert_eq!(
            keys("[{\"api_keys\":{\"openai\":\"sk-x\"}}]"),
            Vec::<String>::new()
        );
        assert_eq!(keys(r#"{"api_keys":"sk-x"}"#), Vec::<String>::new());
        assert_eq!(
            keys(r#"{"api_keys":[{"openai":"sk-x"}]}"#),
            Vec::<String>::new()
        );
        assert_eq!(
            keys(r#"{"api_keys":{"openai":{"nested":"sk-x"}}}"#),
            Vec::<String>::new()
        );
        assert_eq!(
            keys(r#"{"api_keys":{"openai":"sk-x","openrouter":"sk-y"}}"#),
            ["openai=sk-x", "openrouter=sk-y"]
        );
    }
}
