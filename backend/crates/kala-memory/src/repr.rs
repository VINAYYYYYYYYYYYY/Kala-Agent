//! CPython-compatible `repr()` for text values.
//!
//! Two Python messages in the memory module depend on `str.__repr__`:
//!
//! * `MemoryConflictError` renders the offending source with `!r`.
//! * `MemoryNotFoundError` inherits `KeyError.__str__`, which renders its only
//!   argument with `repr()`.
//!
//! CPython chooses the quote character (`'` unless the text contains `'` and
//! no `"`), escapes the backslash, the quote character, tab, newline and
//! carriage return, and escapes every non-printable character as `\xNN`,
//! `\uNNNN` or `\UNNNNNNNN`. Printable characters — including printable
//! non-ASCII ones — are kept verbatim.
//!
//! Python's notion of "printable" excludes every Unicode *Other* and
//! *Separator* category except the ASCII space. Rust's standard library
//! exposes the *Control* and *White_Space* properties but not the full
//! general-category tables, so this implementation treats a character as
//! non-printable when it is a control character or a whitespace separator
//! other than the ASCII space. The unassigned (`Cn`), format (`Cf`),
//! surrogate (`Cs`) and private-use (`Co`) categories are therefore rendered
//! verbatim instead of escaped. That difference is unobservable for the
//! safe-identifier strings this crate formats, and it is listed in the crate
//! README under intentional differences.

/// Whether a character is printable in the CPython `str.isprintable()` sense,
/// as approximated by this crate.
fn is_python_printable(character: char) -> bool {
    if character == ' ' {
        return true;
    }
    if character.is_control() || character.is_whitespace() {
        return false;
    }
    true
}

/// CPython `repr()` of a `str` value.
pub fn python_repr_str(value: &str) -> String {
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
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other if other == quote => {
                out.push('\\');
                out.push(other);
            }
            other if is_python_printable(other) => out.push(other),
            other => {
                let code = u32::from(other);
                if code < 0x100 {
                    out.push_str(&format!("\\x{code:02x}"));
                } else if code < 0x1_0000 {
                    out.push_str(&format!("\\u{code:04x}"));
                } else {
                    out.push_str(&format!("\\U{code:08x}"));
                }
            }
        }
    }
    out.push(quote);
    out
}

#[cfg(test)]
mod tests {
    use super::python_repr_str;

    #[test]
    fn renders_plain_text_with_single_quotes() {
        assert_eq!(python_repr_str("source-1"), "'source-1'");
        assert_eq!(python_repr_str(""), "''");
    }

    #[test]
    fn switches_to_double_quotes_only_when_needed() {
        assert_eq!(python_repr_str("it's"), "\"it's\"");
        assert_eq!(python_repr_str("it's \"quoted\""), "'it\\'s \"quoted\"'");
    }

    #[test]
    fn escapes_backslash_and_control_characters() {
        assert_eq!(python_repr_str("a\\b"), "'a\\\\b'");
        assert_eq!(python_repr_str("a\tb"), "'a\\tb'");
        assert_eq!(python_repr_str("a\nb"), "'a\\nb'");
        assert_eq!(python_repr_str("a\rb"), "'a\\rb'");
        assert_eq!(python_repr_str("a\u{0}b"), "'a\\x00b'");
        assert_eq!(python_repr_str("a\u{2028}b"), "'a\\u2028b'");
    }
}
