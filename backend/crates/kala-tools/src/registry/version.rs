//! The version ordering key.
//!
//! The registry sorts the registered versions of a tool with a key that
//! compares the dot separated segments of a version numerically when a
//! segment is a decimal digit run and lexicographically otherwise:
//!
//! ```text
//! "1.0.0"        -> ((0, 1, ""), (0, 0, ""), (0, 0, ""))
//! "1.0.10"       -> ((0, 1, ""), (0, 0, ""), (0, 10, ""))
//! "1.0.0-alpha"  -> ((0, 1, ""), (0, 0, ""), (1, 0, "alpha"))
//! ```
//!
//! A numeric segment therefore sorts before a non-numeric one, `"1.0.0"`
//! sorts before `"1.0.0-alpha"` and `"1.0"` sorts before `"1.0.0"` because a
//! shorter tuple sorts first when the common prefix is equal.
//!
//! The Python implementation calls `int(segment)` for a segment that
//! `str.isdigit()` accepts. Two CPython behaviours are reproduced here
//! because they are observable:
//!
//! * `str.isdigit()` accepts 128 characters that `str.isdecimal()` does not,
//!   such as the superscript digits. `int()` refuses those, so such a segment
//!   raises a `ValueError`.
//! * `int()` refuses a string with more than 4,300 digits, the same limit
//!   CPython applies to `json.loads` and `json.dumps`.

use num_bigint::BigInt;

use crate::registry::error::ToolRegistryError;

/// The maximum number of digits CPython converts from a string to an integer.
const MAX_INT_DIGITS: usize = 4_300;

/// One segment of a version ordering key.
///
/// The ordering reproduces the Python tuple `(0, int(segment), "")` for a
/// numeric segment and `(1, 0, segment)` for a non-numeric one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct VersionSegment {
    /// `0` for a numeric segment and `1` for a non-numeric segment.
    rank: u8,
    /// The numeric value of a numeric segment.
    value: BigInt,
    /// The text of a non-numeric segment.
    text: String,
}

impl VersionSegment {
    /// Whether the segment is a numeric segment.
    pub fn is_numeric(&self) -> bool {
        self.rank == 0
    }

    /// The numeric value of a numeric segment.
    pub fn value(&self) -> &BigInt {
        &self.value
    }

    /// The text of a non-numeric segment.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// The ordering key of a version string.
///
/// The key is the tuple of segments, exactly like the Python implementation.
pub type VersionKey = Vec<VersionSegment>;

/// Build the ordering key of a version string.
///
/// The failure is the `ValueError` CPython raises when `int(segment)` cannot
/// parse a segment that `str.isdigit()` accepted.
pub fn version_key(version: &str) -> Result<VersionKey, ToolRegistryError> {
    let mut parts: VersionKey = Vec::new();
    for segment in version.split('.') {
        parts.push(version_segment(segment)?);
    }
    Ok(parts)
}

/// Build the ordering key of one version segment.
fn version_segment(segment: &str) -> Result<VersionSegment, ToolRegistryError> {
    if segment.is_empty() || !segment.chars().all(is_digit) {
        return Ok(VersionSegment {
            rank: 1,
            value: BigInt::from(0),
            text: segment.to_owned(),
        });
    }
    if !segment.chars().all(is_decimal_digit) {
        return Err(ToolRegistryError::value(format!(
            "invalid literal for int() with base 10: '{segment}'"
        )));
    }
    if segment.chars().count() > MAX_INT_DIGITS {
        return Err(ToolRegistryError::value(format!(
            "Exceeds the limit ({MAX_INT_DIGITS} digits) for integer string conversion: \
             value has {} digits; use sys.set_int_max_str_digits() to increase the limit",
            segment.chars().count()
        )));
    }
    let value = segment
        .chars()
        .map(|character| decimal_digit_value(character).unwrap_or_default())
        .fold(BigInt::from(0), |accumulator, digit| {
            accumulator * 10 + BigInt::from(digit)
        });
    Ok(VersionSegment {
        rank: 0,
        value,
        text: String::new(),
    })
}

/// Whether a character is a digit for CPython's `str.isdigit()`.
///
/// The set is the decimal digits plus the 128 characters that carry the
/// Unicode "digit" numeric property without being decimal, such as the
/// superscript and the circled digits.
fn is_digit(character: char) -> bool {
    is_decimal_digit(character) || is_non_decimal_digit(character)
}

/// Whether a character is a decimal digit for CPython's `str.isdecimal()`.
///
/// The set is the Unicode category `Nd`.
fn is_decimal_digit(character: char) -> bool {
    decimal_digit_value(character).is_some()
}

/// The decimal value of a decimal digit character.
///
/// CPython maps a character of the category `Nd` to the offset inside its
/// range, taken modulo ten, so the Kaktovik and the mathematical digit
/// ranges, which hold twenty and fifty characters, wrap around.
fn decimal_digit_value(character: char) -> Option<u32> {
    let code_point = u32::from(character);
    DECIMAL_DIGIT_RANGES
        .iter()
        .find(|(first, last)| code_point >= *first && code_point <= *last)
        .map(|(first, _)| (code_point - first) % 10)
}

/// Whether a character is a digit that CPython's `int()` refuses.
///
/// The set is the Unicode "digit" property outside the category `Nd`.
fn is_non_decimal_digit(character: char) -> bool {
    let code_point = u32::from(character);
    NON_DECIMAL_DIGIT_RANGES
        .iter()
        .any(|(first, last)| code_point >= *first && code_point <= *last)
}

/// The Unicode category `Nd`, as code point ranges.
const DECIMAL_DIGIT_RANGES: &[(u32, u32)] = &[
    (0x0030, 0x0039),
    (0x0660, 0x0669),
    (0x06F0, 0x06F9),
    (0x07C0, 0x07C9),
    (0x0966, 0x096F),
    (0x09E6, 0x09EF),
    (0x0A66, 0x0A6F),
    (0x0AE6, 0x0AEF),
    (0x0B66, 0x0B6F),
    (0x0BE6, 0x0BEF),
    (0x0C66, 0x0C6F),
    (0x0CE6, 0x0CEF),
    (0x0D66, 0x0D6F),
    (0x0DE6, 0x0DEF),
    (0x0E50, 0x0E59),
    (0x0ED0, 0x0ED9),
    (0x0F20, 0x0F29),
    (0x1040, 0x1049),
    (0x1090, 0x1099),
    (0x17E0, 0x17E9),
    (0x1810, 0x1819),
    (0x1946, 0x194F),
    (0x19D0, 0x19D9),
    (0x1A80, 0x1A89),
    (0x1A90, 0x1A99),
    (0x1B50, 0x1B59),
    (0x1BB0, 0x1BB9),
    (0x1C40, 0x1C49),
    (0x1C50, 0x1C59),
    (0xA620, 0xA629),
    (0xA8D0, 0xA8D9),
    (0xA900, 0xA909),
    (0xA9D0, 0xA9D9),
    (0xA9F0, 0xA9F9),
    (0xAA50, 0xAA59),
    (0xABF0, 0xABF9),
    (0xFF10, 0xFF19),
    (0x104A0, 0x104A9),
    (0x10D30, 0x10D39),
    (0x10D40, 0x10D49),
    (0x11066, 0x1106F),
    (0x110F0, 0x110F9),
    (0x11136, 0x1113F),
    (0x111D0, 0x111D9),
    (0x112F0, 0x112F9),
    (0x11450, 0x11459),
    (0x114D0, 0x114D9),
    (0x11650, 0x11659),
    (0x116C0, 0x116C9),
    (0x116D0, 0x116E3),
    (0x11730, 0x11739),
    (0x118E0, 0x118E9),
    (0x11950, 0x11959),
    (0x11BF0, 0x11BF9),
    (0x11C50, 0x11C59),
    (0x11D50, 0x11D59),
    (0x11DA0, 0x11DA9),
    (0x11F50, 0x11F59),
    (0x16130, 0x16139),
    (0x16A60, 0x16A69),
    (0x16AC0, 0x16AC9),
    (0x16B50, 0x16B59),
    (0x16D70, 0x16D79),
    (0x1CCF0, 0x1CCF9),
    (0x1D7CE, 0x1D7FF),
    (0x1E140, 0x1E149),
    (0x1E2F0, 0x1E2F9),
    (0x1E4F0, 0x1E4F9),
    (0x1E5F1, 0x1E5FA),
    (0x1E950, 0x1E959),
    (0x1FBF0, 0x1FBF9),
];

/// The Unicode "digit" property outside the category `Nd`.
const NON_DECIMAL_DIGIT_RANGES: &[(u32, u32)] = &[
    (0x00B2, 0x00B3),
    (0x00B9, 0x00B9),
    (0x1369, 0x1371),
    (0x19DA, 0x19DA),
    (0x2070, 0x2070),
    (0x2074, 0x2079),
    (0x2080, 0x2089),
    (0x2460, 0x2468),
    (0x2474, 0x247C),
    (0x2488, 0x2490),
    (0x24EA, 0x24EA),
    (0x24F5, 0x24FD),
    (0x24FF, 0x24FF),
    (0x2776, 0x277E),
    (0x2780, 0x2788),
    (0x278A, 0x2792),
    (0x10A40, 0x10A43),
    (0x10E60, 0x10E68),
    (0x11052, 0x1105A),
    (0x1F100, 0x1F10A),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn key(version: &str) -> VersionKey {
        version_key(version).expect("version key")
    }

    #[test]
    fn orders_numeric_segments_numerically() {
        assert!(key("1.0.0") < key("1.0.10"));
        assert!(key("2.0.0") > key("1.99.99"));
        assert!(key("1.0.0-alpha") > key("1.0.0"));
        assert!(key("1.0") < key("1.0.0"));
    }

    #[test]
    fn accepts_non_ascii_decimal_digits() {
        let key = key("٣.٤");
        assert!(key[0].is_numeric());
        assert_eq!(key[0].value().to_string(), "3");
        assert_eq!(key[1].value().to_string(), "4");
    }

    #[test]
    fn refuses_a_digit_that_int_cannot_parse() {
        let error = version_key("²").expect_err("superscript");
        assert_eq!(error.python_class_name(), "ValueError");
        assert_eq!(
            error.to_string(),
            "invalid literal for int() with base 10: '²'"
        );
    }

    #[test]
    fn refuses_an_over_long_digit_run() {
        let version = "1".repeat(4_301);
        let error = version_key(&version).expect_err("over long");
        assert_eq!(error.python_class_name(), "ValueError");
        assert!(
            error
                .to_string()
                .starts_with("Exceeds the limit (4300 digits)")
        );
    }

    #[test]
    fn treats_an_empty_version_as_one_empty_segment() {
        let key = key("");
        assert_eq!(key.len(), 1);
        assert!(!key[0].is_numeric());
        assert_eq!(key[0].text(), "");
    }
}
