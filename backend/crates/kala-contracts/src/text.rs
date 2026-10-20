//! Lossless contract text.
//!
//! Python strings are sequences of code points and can contain lone surrogate
//! code points. A Rust `String` cannot, so every free-form text field in this
//! crate uses [`Text`], which keeps the exact code points and, when they form
//! valid UTF-8, a borrowed-friendly UTF-8 copy. Identifiers, media types,
//! digests and operations are ASCII by validation and therefore use `String`
//! where that is more ergonomic.

use std::fmt;

use kala_json::JsonText;

/// Python-compatible text with code-point fidelity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Text {
    json: JsonText,
    utf8: Option<String>,
}

impl Text {
    /// Build text from a Rust string.
    pub fn new(value: &str) -> Self {
        Self {
            json: JsonText::new(value),
            utf8: Some(value.to_owned()),
        }
    }

    /// Build text from an already parsed canonical JSON string.
    pub fn from_json_text(value: JsonText) -> Self {
        let utf8 = value.to_utf8().ok();
        Self { json: value, utf8 }
    }

    /// Build text from explicit code points, allowing lone surrogates.
    pub fn from_code_points(code_points: Vec<u32>) -> Result<Self, crate::ContractError> {
        let json = JsonText::from_code_points(code_points).map_err(|_| {
            crate::ContractError::new("Text", "text contains an invalid code point")
        })?;
        Ok(Self::from_json_text(json))
    }

    /// The canonical JSON string representation (code points, including
    /// lone surrogates).
    pub fn as_json_text(&self) -> &JsonText {
        &self.json
    }

    /// The UTF-8 view, or `None` when the text contains a lone surrogate.
    pub fn as_str(&self) -> Option<&str> {
        self.utf8.as_deref()
    }

    /// The code points in order.
    pub fn code_points(&self) -> &[u32] {
        self.json.code_points()
    }

    /// Python `len()`: one unit per code point.
    pub fn len(&self) -> usize {
        self.json.len()
    }

    /// Whether the text is empty.
    pub fn is_empty(&self) -> bool {
        self.json.is_empty()
    }

    /// Whether any code point is a lone surrogate.
    pub fn has_surrogates(&self) -> bool {
        self.json.has_surrogates()
    }

    /// Convert to UTF-8, refusing lone surrogates.
    pub fn to_utf8(&self) -> Result<String, crate::ContractError> {
        match &self.utf8 {
            Some(value) => Ok(value.clone()),
            None => Err(crate::ContractError::new(
                "Text",
                "text contains a lone surrogate and cannot be encoded as UTF-8",
            )),
        }
    }

    /// Conversion for diagnostics only; lone surrogates become U+FFFD.
    pub fn to_string_lossy(&self) -> String {
        self.json.to_string_lossy()
    }

    /// Python `str.strip()`: Unicode White_Space plus U+001C..=U+001F.
    pub fn python_strip(&self) -> Self {
        self.strip_where(is_python_whitespace)
    }

    /// Pydantic `strip_whitespace=True`: Unicode White_Space only (the C0
    /// separators U+001C..=U+001F are deliberately preserved).
    pub fn pydantic_strip(&self) -> Self {
        self.strip_where(is_pydantic_whitespace)
    }

    /// Python `str.casefold()`.
    pub fn casefold(&self) -> Self {
        use unicode_casefold::UnicodeCaseFold;
        match &self.utf8 {
            Some(value) => Self::new(&value.case_fold().collect::<String>()),
            None => {
                let mut folded = Vec::with_capacity(self.json.code_points().len());
                for point in self.json.code_points() {
                    match char::from_u32(*point) {
                        Some(character) => {
                            folded.extend(std::iter::once(character).case_fold().map(u32::from));
                        }
                        None => folded.push(*point),
                    }
                }
                Self::from_code_points(folded).expect("existing code points stay valid")
            }
        }
    }

    /// Python case-insensitive comparison against an ASCII literal.
    pub fn casefold_eq_ascii(&self, expected: &str) -> bool {
        match &self.utf8 {
            Some(value) => {
                use unicode_casefold::UnicodeCaseFold;
                value.case_fold().eq(expected.chars())
            }
            None => false,
        }
    }

    fn strip_where(&self, predicate: fn(u32) -> bool) -> Self {
        let points = self.json.code_points();
        let start = points.iter().position(|point| !predicate(*point));
        let Some(start) = start else {
            return Self::from_json_text(JsonText::default());
        };
        let end = points
            .iter()
            .rposition(|point| !predicate(*point))
            .unwrap_or(start);
        let slice = &points[start..=end];
        if slice.iter().any(|point| (0xD800..=0xDFFF).contains(point)) {
            Self::from_code_points(slice.to_vec())
                .expect("an existing code-point slice stays valid")
        } else {
            let value: String = slice
                .iter()
                .filter_map(|point| char::from_u32(*point))
                .collect();
            Self::new(&value)
        }
    }
}

impl From<&str> for Text {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Text {
    fn from(value: String) -> Self {
        Self::new(&value)
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Text")
            .field(&self.to_string_lossy())
            .finish()
    }
}

impl fmt::Display for Text {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_string_lossy())
    }
}

/// Python `str.isspace()`.
pub fn is_python_whitespace(point: u32) -> bool {
    is_pydantic_whitespace(point) || (0x1C..=0x1F).contains(&point)
}

/// Unicode `White_Space` property, the set Pydantic trims for
/// `StringConstraints(strip_whitespace=True)`.
pub fn is_pydantic_whitespace(point: u32) -> bool {
    matches!(
        point,
        0x0009..=0x000D
            | 0x0020
            | 0x0085
            | 0x00A0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pydantic_and_python_strip_sets_differ_only_on_c0_separators() {
        assert_eq!(
            Text::new("\u{1c}x\u{1f}").python_strip().as_str(),
            Some("x")
        );
        assert_eq!(
            Text::new("\u{1c}x\u{1f}").pydantic_strip().as_str(),
            Some("\u{1c}x\u{1f}")
        );
        assert_eq!(Text::new("\u{200b}x\u{200b}").pydantic_strip().len(), 3);
        assert_eq!(
            Text::new("\u{3000}x\u{0085}").pydantic_strip().as_str(),
            Some("x")
        );
    }

    #[test]
    fn lone_surrogates_survive() {
        let text = Text::from_code_points(vec![0x61, 0xD800, 0x62]).unwrap();
        assert!(text.has_surrogates());
        assert_eq!(
            text.to_utf8(),
            Err(crate::ContractError::new(
                "Text",
                "text contains a lone surrogate and cannot be encoded as UTF-8"
            ))
        );
        assert_eq!(text.to_string_lossy(), "a\u{FFFD}b");
    }

    #[test]
    fn casefold_matches_the_examples_used_by_construction_profiles() {
        assert!(Text::new("BOX").casefold_eq_ascii("box"));
        assert_eq!(Text::new("Straße").casefold().as_str(), Some("strasse"));
        assert_eq!(Text::new("İ").casefold().as_str(), Some("i\u{307}"));
    }
}
