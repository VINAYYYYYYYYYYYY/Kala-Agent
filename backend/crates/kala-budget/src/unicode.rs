//! Unicode character data required by CPython-compatible numeric parsing.
//!
//! CPython's `float()` and `decimal.Decimal()` both accept Unicode decimal
//! digits (`Nd`) and translate them to the matching ASCII digit before parsing
//! (see `_PyUnicode_TransformDecimalAndSpaceToASCII` in CPython). Rust's
//! standard library exposes only ASCII digit classification, so this module
//! embeds a compact table of contiguous `Nd` runs.
//!
//! The table was generated with the repository virtual environment interpreter
//! (CPython 3.14.8, `unicodedata.unidata_version == "16.0.0"`) through
//! `tools/generate_unicode_nd.py`; regenerate it with that script when the
//! oracle Unicode database changes. Python is never needed at build or run
//! time: the table is checked in and validated against live Python by the
//! differential test `unicode_digit_table_matches_live_python`.
//!
//! CPython's numeric parsers also differ in their whitespace sets:
//! `decimal.Decimal` trims every character for which `str.isspace()` is true
//! (that is the Unicode `White_Space` property plus `U+001C..U+001F`), while
//! `float()` trims only the Unicode `White_Space` characters, which is exactly
//! `char::is_whitespace()` in Rust. Do not merge the two rules.

/// Contiguous runs of Unicode `Nd` characters. Inside a run the decimal digit
/// value of a codepoint is `(codepoint - start) % 10`; runs may contain several
/// `0..9` cycles (the mathematical digit blocks do).
///
/// Unicode 16.0.0: 71 runs.
pub(crate) const ND_DIGIT_BLOCKS: &[(u32, u32)] = &[
    // ASCII digits must stay first: they are the overwhelmingly common case and
    // the binary search is only one comparison for them.
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

/// The ASCII digit an `Nd` character denotes, or `None` for any other
/// character. Non-decimal numeric characters (superscripts, vulgar fractions,
/// Roman numerals, ...) intentionally return `None`, matching CPython.
#[must_use]
pub(crate) fn decimal_digit_ascii(character: char) -> Option<u8> {
    let code = character as u32;
    if code < ND_DIGIT_BLOCKS[0].0 {
        return None;
    }
    let index = ND_DIGIT_BLOCKS.partition_point(|(start, _)| *start <= code);
    let (start, end) = ND_DIGIT_BLOCKS[index - 1];
    if code <= end {
        Some(((code - start) % 10) as u8)
    } else {
        None
    }
}

/// True for the characters CPython's `decimal.Decimal` treats as surrounding
/// whitespace: `str.isspace()` characters, which are the Unicode `White_Space`
/// property (`char::is_whitespace`) plus the four ASCII separators
/// `U+001C..U+001F` that Rust does not report as whitespace.
#[must_use]
pub(crate) fn is_python_decimal_space(character: char) -> bool {
    matches!(character, '\u{1C}'..='\u{1F}') || character.is_whitespace()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_table_covers_representative_blocks_and_lookalikes() {
        // ASCII and the blocks named in the review.
        assert_eq!(decimal_digit_ascii('0'), Some(0));
        assert_eq!(decimal_digit_ascii('9'), Some(9));
        assert_eq!(decimal_digit_ascii('\u{0661}'), Some(1)); // Arabic-Indic
        assert_eq!(decimal_digit_ascii('\u{06F2}'), Some(2)); // Extended Arabic-Indic
        assert_eq!(decimal_digit_ascii('\u{FF13}'), Some(3)); // fullwidth
        assert_eq!(decimal_digit_ascii('\u{1D7D9}'), Some(1)); // mathematical double-struck
        assert_eq!(decimal_digit_ascii('\u{1D7DB}'), Some(3));
        assert_eq!(decimal_digit_ascii('\u{1D7FF}'), Some(9));
        assert_eq!(decimal_digit_ascii('\u{116D0}'), Some(0)); // multi-cycle run start
        assert_eq!(decimal_digit_ascii('\u{116D9}'), Some(9));
        assert_eq!(decimal_digit_ascii('\u{116DA}'), Some(0));
        assert_eq!(decimal_digit_ascii('\u{1FBF0}'), Some(0)); // segmented digits (last run)
        // Non-`Nd` numeric lookalikes must not be accepted.
        assert_eq!(decimal_digit_ascii('\u{00B2}'), None); // superscript two
        assert_eq!(decimal_digit_ascii('\u{00BD}'), None); // one half
        assert_eq!(decimal_digit_ascii('\u{2160}'), None); // Roman numeral I
        assert_eq!(decimal_digit_ascii('\u{3007}'), None); // ideographic zero
        assert_eq!(decimal_digit_ascii('a'), None);
        assert_eq!(decimal_digit_ascii(char::MAX), None);
    }

    #[test]
    fn decimal_space_includes_ascii_separators_only_beyond_unicode_whitespace() {
        for character in [
            '\t', '\n', '\u{B}', '\u{C}', '\r', ' ', '\u{85}', '\u{A0}', '\u{2028}', '\u{3000}',
        ] {
            assert!(is_python_decimal_space(character), "{character:?}");
        }
        for character in ['\u{1C}', '\u{1D}', '\u{1E}', '\u{1F}'] {
            assert!(is_python_decimal_space(character), "{character:?}");
            assert!(
                !character.is_whitespace(),
                "{character:?} is not Rust whitespace"
            );
        }
        assert!(!is_python_decimal_space('\u{180E}'));
        assert!(!is_python_decimal_space('\u{200B}'));
    }
}
