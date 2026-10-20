//! The patterns the request models declare.
//!
//! Pydantic matches a `pattern` constraint with the Rust `regex` crate, so an
//! anchored pattern is a full match of the whole value and `$` is the end of
//! the value, not the end of a line. The three patterns the API declares are
//! simple ASCII classes, so they are matched here directly instead of through
//! a regular expression engine: the accepted set is exactly the same, and no
//! pattern is ever compiled from caller supplied text.

/// The pattern of a stable identifier.
pub const IDENTIFIER_PATTERN: &str = r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$";

/// The pattern of a design identifier.
pub const DESIGN_ID_PATTERN: &str = r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$";

/// The pattern of a SHA-256 digest.
pub const DIGEST_PATTERN: &str = r"^[0-9a-f]{64}$";

/// Whether a value matches one of the declared patterns.
///
/// Only the three patterns of this crate are recognised; an unknown pattern
/// matches nothing, so a caller can never smuggle a pattern into the matcher.
#[must_use]
pub fn matches(pattern: &str, value: &str) -> bool {
    match pattern {
        IDENTIFIER_PATTERN => matches_identifier(value, 128),
        DESIGN_ID_PATTERN => matches_identifier(value, 64),
        DIGEST_PATTERN => matches_digest(value),
        _ => false,
    }
}

/// Whether a value is an identifier of at most `maximum` code points.
///
/// The first code point is an ASCII letter or digit, and the rest are ASCII
/// letters or digits with `.`, `_`, `:` and `-`.
fn matches_identifier(value: &str, maximum: usize) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    let mut length = 1usize;
    for character in characters {
        if !character.is_ascii_alphanumeric() && !matches!(character, '.' | '_' | ':' | '-') {
            return false;
        }
        length += 1;
        if length > maximum {
            return false;
        }
    }
    true
}

/// Whether a value is a lowercase SHA-256 digest.
fn matches_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The kinds the patterns report, kept beside the matcher.
pub mod pattern_kind {
    pub use crate::validation::kind::STRING_PATTERN_MISMATCH;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::kind;

    #[test]
    fn identifiers_follow_the_declared_pattern() {
        assert!(matches(IDENTIFIER_PATTERN, "a"));
        assert!(matches(IDENTIFIER_PATTERN, "A.b_c:d-e"));
        assert!(matches(IDENTIFIER_PATTERN, "0"));
        assert!(!matches(IDENTIFIER_PATTERN, ""));
        assert!(!matches(IDENTIFIER_PATTERN, ".a"));
        assert!(!matches(IDENTIFIER_PATTERN, "a b"));
        assert!(!matches(IDENTIFIER_PATTERN, "a\n"));
        assert!(!matches(IDENTIFIER_PATTERN, &"a".repeat(129)));
        assert!(matches(IDENTIFIER_PATTERN, &"a".repeat(128)));
    }

    #[test]
    fn design_identifiers_are_bounded() {
        assert!(matches(DESIGN_ID_PATTERN, &"a".repeat(64)));
        assert!(!matches(DESIGN_ID_PATTERN, &"a".repeat(65)));
    }

    #[test]
    fn digests_are_lowercase_hexadecimal() {
        assert!(matches(DIGEST_PATTERN, &"a".repeat(64)));
        assert!(!matches(DIGEST_PATTERN, &"A".repeat(64)));
        assert!(!matches(DIGEST_PATTERN, &"a".repeat(63)));
        assert!(!matches(DIGEST_PATTERN, &"g".repeat(64)));
    }

    #[test]
    fn an_unknown_pattern_matches_nothing() {
        assert!(!matches("^.*$", "anything"));
    }

    #[test]
    fn the_pattern_kind_is_the_pydantic_one() {
        assert_eq!(
            pattern_kind::STRING_PATTERN_MISMATCH,
            kind::STRING_PATTERN_MISMATCH
        );
    }
}
