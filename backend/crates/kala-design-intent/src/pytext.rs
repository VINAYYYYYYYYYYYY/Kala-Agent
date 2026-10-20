//! CPython-compatible text helpers.
//!
//! The design modules lower-case, strip and length-check user text. Python
//! counts code points and its whitespace set is `str.isspace()`, which includes
//! the C0 separators U+001C..=U+001F that Unicode `White_Space` omits. These
//! helpers keep that behaviour explicit.

/// Python `str.isspace()`.
pub(crate) fn is_python_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{001C}'..='\u{001F}'
            | '\u{0085}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

/// Unicode `White_Space`, the set Pydantic trims for
/// `StringConstraints(strip_whitespace=True)` and CPython's `float()` ignores
/// around a numeric literal.
pub(crate) fn is_pydantic_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{0085}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

/// Python `str.strip()` on valid UTF-8.
pub(crate) fn python_strip(text: &str) -> &str {
    text.trim_matches(is_python_whitespace)
}

/// Python `str.splitlines()[0]` for valid UTF-8, without allocating.
pub(crate) fn python_first_line(text: &str) -> &str {
    for (index, character) in text.char_indices() {
        let line_break = matches!(
            character,
            '\n' | '\r'
                | '\u{000b}'
                | '\u{000c}'
                | '\u{001c}'
                | '\u{001d}'
                | '\u{001e}'
                | '\u{0085}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if line_break {
            return &text[..index];
        }
    }
    text
}

/// Python `text[:limit]` for valid UTF-8.
pub(crate) fn python_truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// `re.sub(r"\s+", " ", prompt.strip().lower())`.
///
/// Every character Python's `\s` matches is collapsed to one ASCII space, so
/// the regular expressions in the prompt parser only ever see ordinary spaces.
pub(crate) fn normalise_lowered(prompt: &str) -> String {
    let stripped = python_strip(prompt);
    let lowered: String = stripped.chars().flat_map(char::to_lowercase).collect();
    let mut out = String::with_capacity(lowered.len());
    let mut pending_space = false;
    for character in lowered.chars() {
        if is_python_whitespace(character) {
            pending_space = true;
        } else {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(character);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_whitespace_includes_c0_separators() {
        assert!(is_python_whitespace('\u{1c}'));
        assert!(is_python_whitespace('\u{3000}'));
        assert!(!is_python_whitespace('\u{200b}'));
    }

    #[test]
    fn normalise_collapses_and_lowers() {
        assert_eq!(
            normalise_lowered("  Size\u{00a0}an  L-BRACKET \n"),
            "size an l-bracket"
        );
        assert_eq!(normalise_lowered("\u{1c}\u{1d}"), "");
    }

    #[test]
    fn truncate_counts_code_points() {
        assert_eq!(python_truncate("😀ab", 2), "😀a");
    }
}
