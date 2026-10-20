//! The shared safe-identifier rule.
//!
//! The Studio journal validates `event_type` and `aggregate_id` with
//! `kala_agent.contracts.Identifier`, the core identifier rule: the value is
//! stripped with CPython's `str.strip()` whitespace set and must then match
//! `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`.
//!
//! The rule is implemented here rather than imported so this crate depends
//! only on the three accepted siblings named by the cell: `kala-json`,
//! `kala-persistence` and `kala-artifacts`. The accepted `kala-persistence`
//! crate keeps its copy private (`pub(crate)`), so the rule is restated and
//! pinned by this crate's own tests.

/// Return the CPython `str.strip()` whitespace set.
///
/// CPython's `str.strip()` also removes the information separators
/// `U+001C..=U+001F`, which Rust's `char::is_whitespace` omits. Using Rust's
/// `trim()` would therefore reject identifiers the product accepts.
fn is_python_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000D}'
            | '\u{001C}'..='\u{001F}'
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

/// Strip the value with CPython's `str.strip()` whitespace set.
pub(crate) fn python_strip(value: &str) -> &str {
    value.trim_matches(is_python_whitespace)
}

/// Return whether the value matches the safe identifier pattern.
///
/// The pattern is `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`: one ASCII
/// alphanumeric character followed by at most 127 further alphanumeric
/// characters, dots, underscores, colons or hyphens.
pub(crate) fn is_safe_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 128 {
        return false;
    }
    if !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

/// Normalize and validate an identifier, exactly like the Python contract.
///
/// The returned value is the stripped value, which is what the Python
/// `Identifier` contract stores and what the chain digest covers.
pub(crate) fn normalize_identifier(
    value: &str,
    field: &'static str,
) -> Result<String, crate::error::StudioJournalError> {
    let normalized = python_strip(value);
    if is_safe_identifier(normalized) {
        Ok(normalized.to_owned())
    } else {
        Err(crate::error::StudioJournalError::InvalidIdentifier { field })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_pattern_matches_python() {
        assert!(is_safe_identifier("chat.turn"));
        assert!(is_safe_identifier("A1._:-"));
        assert!(is_safe_identifier("0"));
        assert!(!is_safe_identifier(""));
        assert!(!is_safe_identifier("-leading"));
        assert!(!is_safe_identifier(".leading"));
        assert!(!is_safe_identifier("bad type"));
        assert!(!is_safe_identifier("../escape"));
        assert!(!is_safe_identifier("caf\u{e9}"));
        assert!(is_safe_identifier(&"a".repeat(128)));
        assert!(!is_safe_identifier(&"a".repeat(129)));
    }

    #[test]
    fn python_strip_removes_the_information_separators() {
        assert_eq!(python_strip("\u{1c} value \u{3000}"), "value");
        assert_eq!(python_strip("\u{00a0}value"), "value");
    }
}
