//! A CPython-compatible JSON reader.
//!
//! `serde_json` is the parser: it rejects trailing content, duplicate keys
//! are resolved the way CPython resolves them (the last one wins) and the
//! parser is bounded, so an attacker-sized document cannot overflow the stack.
//!
//! Two CPython behaviours are reproduced explicitly:
//!
//! * `NaN`, `Infinity` and `-Infinity` are accepted by CPython's `json`
//!   module. They are rewritten to `null` before parsing. Every field of the
//!   sandbox specification is type checked afterwards, so a non-finite number
//!   is refused with the same malformed-payload error CPython produces when
//!   `int(nan)` raises `ValueError`.
//! * An unpaired UTF-16 surrogate escape is refused. CPython accepts it and
//!   produces a lone surrogate; the port refuses the document instead.

use serde_json::Value;

/// The error of a refused document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonParseError {
    /// A short, sanitised reason.
    pub detail: String,
}

impl std::fmt::Display for JsonParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for JsonParseError {}

/// Parse a JSON document with CPython's `json.loads` leniency.
pub fn parse_json(text: &str) -> Result<Value, JsonParseError> {
    let normalized = rewrite_non_finite(text);
    serde_json::from_str(&normalized).map_err(|error| JsonParseError {
        detail: error.to_string(),
    })
}

/// Replace the bare non-finite constants that CPython accepts.
fn rewrite_non_finite(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if in_string {
            output.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        if character == '"' {
            in_string = true;
            output.push('"');
            continue;
        }
        if let Some((token, replacement)) = non_finite_at(text, index) {
            output.push_str(replacement);
            // Skip the characters of the consumed token.
            for _ in 1..token.chars().count() {
                characters.next();
            }
            continue;
        }
        output.push(character);
    }
    output
}

fn non_finite_at(text: &str, index: usize) -> Option<(&'static str, &'static str)> {
    let rest = &text[index..];
    for (token, replacement) in [("NaN", "null"), ("Infinity", "null"), ("-Infinity", "null")] {
        if !rest.starts_with(token) {
            continue;
        }
        let after = &rest[token.len()..];
        if let Some(next) = after.chars().next() {
            if next.is_alphanumeric() || next == '_' || next == '.' {
                continue;
            }
        }
        // A `-` only introduces the constant when it is not the sign of a
        // number: CPython rejects `-Infinity`? No, it accepts it.
        if token.starts_with('-') && index > 0 {
            let previous = text[..index].chars().next_back().unwrap_or(' ');
            if previous.is_ascii_digit() || previous == '.' {
                continue;
            }
        }
        return Some((token, replacement));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_documents() {
        let value = parse_json("{\"a\": 1}").expect("document");
        assert_eq!(value["a"], serde_json::json!(1));
    }

    #[test]
    fn accepts_cpython_non_finite_constants() {
        let value =
            parse_json("{\"a\": NaN, \"b\": Infinity, \"c\": -Infinity}").expect("document");
        assert!(value["a"].is_null());
        assert!(value["b"].is_null());
        assert!(value["c"].is_null());
    }

    #[test]
    fn keeps_non_finite_text_inside_strings() {
        let value = parse_json("{\"a\": \"NaN\"}").expect("document");
        assert_eq!(value["a"], serde_json::json!("NaN"));
    }

    #[test]
    fn refuses_trailing_content() {
        assert!(parse_json("{} {}").is_err());
        assert!(parse_json("{not json").is_err());
    }

    #[test]
    fn keeps_the_last_duplicate_key() {
        let value = parse_json("{\"a\": 1, \"a\": 2}").expect("document");
        assert_eq!(value["a"], serde_json::json!(2));
    }
}
