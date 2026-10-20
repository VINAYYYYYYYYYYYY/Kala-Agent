//! Python `str.isspace` / `str.strip` semantics.
//!
//! Pydantic's `str_strip_whitespace` uses the Rust `Unicode White_Space`
//! property, but the Python provider/settings/token code calls `str.strip()`
//! and `re \s`, which also treat U+001C..U+001F as whitespace. The two
//! behaviours are needed in different layers and must not be conflated.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn information_separators_are_python_whitespace() {
        assert_eq!(trim("\u{1c} x \u{1f}"), "x");
        assert!(is_space('\u{1c}'));
        assert!(!'\u{1c}'.is_whitespace());
    }
}
