//! Python `str.isspace` / `str.strip` semantics.
//!
//! Pydantic's `str_strip_whitespace` uses the Rust `Unicode White_Space`
//! property, but the Python boundary and configuration code calls
//! `str.strip()` and `str.isspace()`, which also treat U+001C..U+001F as
//! whitespace. The two behaviours live in different layers and must not be
//! conflated.

/// Whether Python considers the character whitespace.
#[must_use]
pub fn is_space(character: char) -> bool {
    character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
}

/// Python `str.strip()`.
#[must_use]
pub fn trim(value: &str) -> &str {
    value.trim_matches(is_space)
}

/// Python `str.lower()`.
///
/// CPython lowercases with the full Unicode mapping, and so does Rust, but the
/// result can be longer than the input (`İ` becomes two code points). The
/// length of the lowered value is what the Python source measures.
#[must_use]
pub fn lower(value: &str) -> String {
    value.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn information_separators_are_python_whitespace() {
        assert_eq!(trim("\u{1c} x \u{1f}"), "x");
        assert!(is_space('\u{1c}'));
        assert!(!'\u{1c}'.is_whitespace());
    }

    #[test]
    fn non_breaking_space_is_trimmed_like_python() {
        assert_eq!(trim("\u{a0}x\u{a0}"), "x");
        assert!(is_space('\u{a0}'));
    }

    #[test]
    fn lower_matches_python_for_latin1() {
        assert_eq!(lower("LOCALHOST"), "localhost");
        assert_eq!(lower("ÀÉÎ"), "àéî");
    }
}
