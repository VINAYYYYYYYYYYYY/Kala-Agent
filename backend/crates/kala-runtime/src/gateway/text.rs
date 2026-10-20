//! Python-compatible text helpers.
//!
//! The Python module leans on a handful of CPython string behaviours that the
//! Rust standard library spells differently. The helpers here reproduce them
//! exactly so the validation messages and the redaction results are
//! byte-identical.

/// Whether a character is whitespace for CPython's `str.isspace()`.
///
/// CPython's `str.isspace()` is true for the Unicode `White_Space` property
/// and additionally for the four ASCII control characters `U+001C` to
/// `U+001F`, which Rust's `char::is_whitespace` does not include.
#[must_use]
pub fn is_python_whitespace(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
}

/// CPython's `str.strip()`: remove leading and trailing whitespace.
#[must_use]
pub fn python_strip(value: &str) -> &str {
    value.trim_matches(is_python_whitespace)
}

/// CPython's `str.strip()` on an owned string.
#[must_use]
pub fn python_strip_owned(value: &str) -> String {
    python_strip(value).to_owned()
}

/// The number of Unicode code points, which is CPython's `len()` for `str`.
#[must_use]
pub fn code_point_len(value: &str) -> usize {
    value.chars().count()
}

/// CPython's `str.isspace()` for a whole string.
#[must_use]
pub fn is_python_blank(value: &str) -> bool {
    value.chars().all(is_python_whitespace)
}

/// Whether a byte is an ASCII letter or digit, the class the secret-name
/// boundary checks use.
#[must_use]
pub fn is_ascii_alphanumeric(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}

/// CPython's `repr()` of a string.
///
/// The Python module embeds a rejected capability in its error message with
/// `repr()`, so the Rust port reproduces the same quoting.
#[must_use]
pub fn python_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(value.len() + 2);
    out.push(quote);
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if other == quote => {
                out.push('\\');
                out.push(other);
            }
            other if (other as u32) < 0x20 || other as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", other as u32));
            }
            other => out.push(other),
        }
    }
    out.push(quote);
    out
}
