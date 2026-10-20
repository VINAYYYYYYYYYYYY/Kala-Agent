//! CPython `json.loads` first-error character offsets.
//!
//! FastAPI answers a malformed `application/json` body with
//! `body.<e.pos>: json_invalid`, where `e.pos` is CPython's zero-based
//! character offset in the decoded document. `serde_json` reports a
//! different position for some malformed documents (partial literals,
//! invalid escapes, malformed numbers, unterminated strings), so this module
//! walks the document with CPython's scanner rules and returns the offset
//! CPython would report.
//!
//! It only supplies a *position*: the route still rejects the request with
//! the same 422 status, and the reviewed canonical hash parser is not
//! touched. `None` means the scanner found no CPython error (for example a
//! well-formed document with a lone surrogate escape that CPython accepts);
//! the caller then falls back to the serde-derived offset.
//!
//! Rules reproduced (verified against CPython 3.14.8 in the repository
//! virtualenv):
//!
//! * whitespace is ` \t\n\r` only;
//! * `NaN`/`Infinity`/`-Infinity` are accepted at a value position;
//! * a partial literal reports the *start* of the literal
//!   (`tru` → 0, `[tru]` → 1);
//! * an invalid simple escape reports the backslash, an invalid `\uXXXX`
//!   reports the `u`;
//! * an unterminated string reports its opening quote;
//! * a number stops after its longest valid prefix (`1.`, `1e`, `01`, ...),
//!   so the following character becomes "Extra data" or a delimiter error;
//! * a trailing comma in an array/object reports the comma.

/// CPython's first-error character offset, or `None` when CPython accepts
/// the document's syntax.
#[must_use]
pub fn python_json_error_offset(text: &str) -> Option<usize> {
    let chars: Vec<char> = text.chars().collect();
    Scanner::new(&chars).run()
}

struct Scanner<'a> {
    chars: &'a [char],
    index: usize,
    frames: Vec<Frame>,
    root: Expect,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    Value,
    AfterValue,
    Key,
    Colon,
}

struct Frame {
    object: bool,
    expect: Expect,
    /// Position of the comma that opened the current expectation, used for
    /// CPython's "illegal trailing comma" diagnostics.
    comma: Option<usize>,
}

impl<'a> Scanner<'a> {
    fn new(chars: &'a [char]) -> Self {
        Self {
            chars,
            index: 0,
            frames: Vec::new(),
            root: Expect::Value,
        }
    }

    fn current(&self) -> Expect {
        self.frames.last().map_or(self.root, |frame| frame.expect)
    }

    fn set_current(&mut self, expect: Expect) {
        match self.frames.last_mut() {
            Some(frame) => frame.expect = expect,
            None => self.root = expect,
        }
    }

    fn skip_whitespace(&mut self) {
        while self
            .chars
            .get(self.index)
            .is_some_and(|character| matches!(character, ' ' | '\t' | '\n' | '\r'))
        {
            self.index += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    fn run(mut self) -> Option<usize> {
        loop {
            self.skip_whitespace();
            match self.current() {
                Expect::Value => {
                    let start = self.index;
                    let Some(character) = self.peek() else {
                        return Some(start);
                    };
                    match character {
                        '"' => {
                            if let Some(offset) = self.string_error() {
                                return Some(offset);
                            }
                            self.set_current(Expect::AfterValue);
                        }
                        '{' => {
                            self.index += 1;
                            self.skip_whitespace();
                            if self.peek() == Some('}') {
                                self.index += 1;
                                self.set_current(Expect::AfterValue);
                            } else {
                                self.frames.push(Frame {
                                    object: true,
                                    expect: Expect::Key,
                                    comma: None,
                                });
                            }
                        }
                        '[' => {
                            self.index += 1;
                            self.skip_whitespace();
                            if self.peek() == Some(']') {
                                self.index += 1;
                                self.set_current(Expect::AfterValue);
                            } else {
                                self.frames.push(Frame {
                                    object: false,
                                    expect: Expect::Value,
                                    comma: None,
                                });
                            }
                        }
                        't' => {
                            if !self.consume_literal("true") {
                                return Some(start);
                            }
                            self.set_current(Expect::AfterValue);
                        }
                        'f' => {
                            if !self.consume_literal("false") {
                                return Some(start);
                            }
                            self.set_current(Expect::AfterValue);
                        }
                        'n' => {
                            if !self.consume_literal("null") {
                                return Some(start);
                            }
                            self.set_current(Expect::AfterValue);
                        }
                        'N' => {
                            if !self.consume_literal("NaN") {
                                return Some(start);
                            }
                            self.set_current(Expect::AfterValue);
                        }
                        'I' => {
                            if !self.consume_literal("Infinity") {
                                return Some(start);
                            }
                            self.set_current(Expect::AfterValue);
                        }
                        '-' => {
                            self.index += 1;
                            if self.peek().is_some_and(|next| next.is_ascii_digit()) {
                                self.scan_number();
                                self.set_current(Expect::AfterValue);
                            } else if self.consume_literal("Infinity") {
                                self.set_current(Expect::AfterValue);
                            } else {
                                return Some(start);
                            }
                        }
                        '0'..='9' => {
                            self.scan_number();
                            self.set_current(Expect::AfterValue);
                        }
                        // `]`, `}`, `,`, `:`, `+`, `.`, `\`, ... report the
                        // value position; a closing bracket right after a
                        // comma is CPython's illegal trailing comma.
                        _ => {
                            let trailing = self
                                .frames
                                .last()
                                .filter(|frame| !frame.object)
                                .and_then(|frame| frame.comma);
                            if character == ']' && trailing.is_some() {
                                return trailing;
                            }
                            return Some(start);
                        }
                    }
                }
                Expect::AfterValue => {
                    if self.frames.is_empty() {
                        self.skip_whitespace();
                        if self.index < self.chars.len() {
                            return Some(self.index);
                        }
                        return None;
                    }
                    self.skip_whitespace();
                    let Some(character) = self.peek() else {
                        return Some(self.index);
                    };
                    let object = self.frames.last().is_some_and(|frame| frame.object);
                    match (object, character) {
                        (false, ',') => {
                            self.index += 1;
                            let comma = self.index - 1;
                            if let Some(frame) = self.frames.last_mut() {
                                frame.expect = Expect::Value;
                                frame.comma = Some(comma);
                            }
                        }
                        (false, ']') => {
                            self.index += 1;
                            self.frames.pop();
                            self.set_current(Expect::AfterValue);
                        }
                        (true, ',') => {
                            self.index += 1;
                            let comma = self.index - 1;
                            if let Some(frame) = self.frames.last_mut() {
                                frame.expect = Expect::Key;
                                frame.comma = Some(comma);
                            }
                        }
                        (true, '}') => {
                            self.index += 1;
                            self.frames.pop();
                            self.set_current(Expect::AfterValue);
                        }
                        _ => return Some(self.index),
                    }
                }
                Expect::Key => {
                    let Some(character) = self.peek() else {
                        return Some(self.index);
                    };
                    match character {
                        '"' => {
                            if let Some(offset) = self.string_error() {
                                return Some(offset);
                            }
                            self.set_current(Expect::Colon);
                        }
                        '}' => {
                            // Only reachable after a comma: CPython reports
                            // the trailing comma, not the closing brace.
                            if let Some(comma) = self.frames.last().and_then(|frame| frame.comma) {
                                return Some(comma);
                            }
                            return Some(self.index);
                        }
                        _ => return Some(self.index),
                    }
                }
                Expect::Colon => {
                    let Some(character) = self.peek() else {
                        return Some(self.index);
                    };
                    if character == ':' {
                        self.index += 1;
                        self.set_current(Expect::Value);
                    } else {
                        return Some(self.index);
                    }
                }
            }
        }
    }

    /// Whether the literal at the cursor matches exactly; a prefix or longer
    /// run fails at the literal start, like CPython.
    fn consume_literal(&mut self, literal: &str) -> bool {
        for (offset, expected) in literal.chars().enumerate() {
            if self.chars.get(self.index + offset) != Some(&expected) {
                return false;
            }
        }
        self.index += literal.len();
        true
    }

    /// Scan one JSON string, returning the CPython offset on failure.
    fn string_error(&mut self) -> Option<usize> {
        self.scan_string().err()
    }

    /// Scan one JSON string, checking CPython's escape and control rules.
    ///
    /// # Errors
    ///
    /// Returns the CPython offset for an invalid escape, an invalid `\uXXXX`
    /// sequence, a raw control character or an unterminated string.
    fn scan_string(&mut self) -> Result<(), usize> {
        let start = self.index;
        self.index += 1;
        loop {
            let Some(character) = self.peek() else {
                // CPython reports the opening quote of the string.
                return Err(start);
            };
            match character {
                '"' => {
                    self.index += 1;
                    return Ok(());
                }
                '\\' => {
                    let Some(escape) = self.chars.get(self.index + 1).copied() else {
                        return Err(start);
                    };
                    match escape {
                        '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' => self.index += 2,
                        'u' => {
                            let hex_ok = (0..4).all(|offset| {
                                self.chars
                                    .get(self.index + 2 + offset)
                                    .is_some_and(|value| value.is_ascii_hexdigit())
                            });
                            if !hex_ok {
                                // CPython reports the `u` of the sequence.
                                return Err(self.index + 1);
                            }
                            self.index += 6;
                        }
                        _ => return Err(self.index),
                    }
                }
                character if (character as u32) < 0x20 => return Err(self.index),
                _ => self.index += 1,
            }
        }
    }

    /// Consume the longest valid JSON number prefix at the cursor.
    ///
    /// CPython never errors inside a number: `1.`, `1e`, `01` stop after the
    /// valid prefix and the caller reports the next character.
    fn scan_number(&mut self) {
        if self.peek() == Some('0') {
            self.index += 1;
        } else {
            while self.peek().is_some_and(|value| value.is_ascii_digit()) {
                self.index += 1;
            }
        }
        if self.peek() == Some('.')
            && self
                .chars
                .get(self.index + 1)
                .is_some_and(char::is_ascii_digit)
        {
            self.index += 2;
            while self.peek().is_some_and(|value| value.is_ascii_digit()) {
                self.index += 1;
            }
        }
        if self.peek().is_some_and(|value| matches!(value, 'e' | 'E')) {
            let mut lookahead = self.index + 1;
            if self
                .chars
                .get(lookahead)
                .is_some_and(|value| matches!(value, '+' | '-'))
            {
                lookahead += 1;
            }
            if self.chars.get(lookahead).is_some_and(char::is_ascii_digit) {
                self.index = lookahead;
                while self.peek().is_some_and(|value| value.is_ascii_digit()) {
                    self.index += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(text: &str) -> Option<usize> {
        python_json_error_offset(text)
    }

    #[test]
    fn well_formed_documents_have_no_error() {
        for text in [
            "{}",
            "[]",
            "[1, 2, 3]",
            r#"{"a": "b", "c": [true, false, null]}"#,
            "  1 ",
            "NaN",
            "Infinity",
            "-Infinity",
            "[NaN, Infinity, -Infinity]",
            r#""\ud800""#,
            r#"{"a": "\ud83d\ude00"}"#,
            "1e+5",
            "-0",
            "1.5e-3",
        ] {
            assert_eq!(position(text), None, "{text:?}");
        }
    }

    #[test]
    fn partial_literals_report_the_literal_start() {
        assert_eq!(position("tru"), Some(0));
        assert_eq!(position("nul"), Some(0));
        assert_eq!(position("t"), Some(0));
        assert_eq!(position("[tru]"), Some(1));
        assert_eq!(position(r#"{"a":tru}"#), Some(5));
        // Complete literals followed by garbage are extra data at the next
        // character instead.
        assert_eq!(position("nulll"), Some(4));
        assert_eq!(position("truex"), Some(4));
        assert_eq!(position("Infinityx"), Some(8));
    }

    #[test]
    fn escape_errors_report_the_backslash_or_the_u() {
        assert_eq!(position(r#"{"a":"\q"}"#), Some(6));
        assert_eq!(position(r#"{"a":"\x"}"#), Some(6));
        assert_eq!(position(r#"{"a":"\u12"}"#), Some(7));
        assert_eq!(position(r#"{"a":"\u12g4"}"#), Some(7));
        assert_eq!(position(r#"{"a":"\u"}"#), Some(7));
        assert_eq!(position("\"\\q"), Some(1));
        assert_eq!(position("\"\\"), Some(0));
    }

    #[test]
    fn unterminated_strings_report_their_opening_quote() {
        assert_eq!(position("\"abc"), Some(0));
        assert_eq!(position(r#"{"a":"abc}"#), Some(5));
        assert_eq!(position("\""), Some(0));
    }

    #[test]
    fn malformed_numbers_stop_after_their_valid_prefix() {
        assert_eq!(position("1."), Some(1));
        assert_eq!(position("1e"), Some(1));
        assert_eq!(position("1e+"), Some(1));
        assert_eq!(position("1.e5"), Some(1));
        assert_eq!(position("--1"), Some(0));
        assert_eq!(position("-"), Some(0));
        assert_eq!(position(".5"), Some(0));
        assert_eq!(position("01"), Some(1));
        assert_eq!(position("-01"), Some(2));
        assert_eq!(position("0x1"), Some(1));
        assert_eq!(position("[1.]"), Some(2));
        assert_eq!(position("[1e]"), Some(2));
        assert_eq!(position("[01]"), Some(2));
        assert_eq!(position(r#"{"a":01}"#), Some(6));
        assert_eq!(position("[.5]"), Some(1));
        assert_eq!(position("[--1]"), Some(1));
    }

    #[test]
    fn structural_errors_match_cpython_positions() {
        assert_eq!(position("{"), Some(1));
        assert_eq!(position("   "), Some(3));
        assert_eq!(position("[1,2"), Some(4));
        assert_eq!(position(r#"{"a":1"#), Some(6));
        assert_eq!(position(r#"{"a""#), Some(4));
        assert_eq!(position(r#"{"a":"#), Some(5));
        assert_eq!(position("[1,"), Some(3));
        assert_eq!(position(r#"{"a":1,"#), Some(7));
        assert_eq!(position("[1,]"), Some(2));
        assert_eq!(position(r#"{"a":1,}"#), Some(6));
        assert_eq!(position("[1,,2]"), Some(3));
        assert_eq!(position(r#"{"a":1,,}"#), Some(7));
        assert_eq!(position("[,1]"), Some(1));
        assert_eq!(position("{,}"), Some(1));
        assert_eq!(position(r#"{"a":}"#), Some(5));
        assert_eq!(position("[}"), Some(1));
        assert_eq!(position("]"), Some(0));
        assert_eq!(position("}"), Some(0));
        assert_eq!(position(r#"{"a" 1}"#), Some(5));
        assert_eq!(position("[1 2]"), Some(3));
        assert_eq!(position("1.5.2"), Some(3));
        assert_eq!(position("1e5."), Some(3));
        assert_eq!(position(r#"{"a":1}{"#), Some(7));
        assert_eq!(position("{\"a\":1} x"), Some(8));
        assert_eq!(position(r#"{"a":-}"#), Some(5));
        assert_eq!(position("[truex]"), Some(5));
        assert_eq!(position("-NaN"), Some(0));
    }

    #[test]
    fn control_characters_report_their_own_position() {
        assert_eq!(position("\"a\nb\""), Some(2));
        assert_eq!(position("\"a\tb\""), Some(2));
        assert_eq!(position("\"a\u{1}b\""), Some(2));
    }
}
