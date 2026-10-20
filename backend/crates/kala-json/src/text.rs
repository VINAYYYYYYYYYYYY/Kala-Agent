use std::fmt;

use crate::error::CanonicalJsonError;

/// A JSON string represented as Unicode code points.
///
/// Python strings are sequences of code points and may contain lone surrogate
/// code points (`U+D800..=U+DFFF`) that a Rust `String` cannot hold. This type
/// preserves them so canonical output, length limits, equality and key ordering
/// match CPython exactly.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JsonText {
    code_points: Vec<u32>,
}

impl JsonText {
    /// Build text from a normal Rust string.
    pub fn new(value: &str) -> Self {
        Self {
            code_points: value.chars().map(u32::from).collect(),
        }
    }

    /// Build text from explicit Unicode code points, allowing lone surrogates.
    pub fn from_code_points(code_points: Vec<u32>) -> Result<Self, CanonicalJsonError> {
        if code_points.iter().any(|point| *point > 0x10_FFFF) {
            return Err(CanonicalJsonError::InvalidCodePoint);
        }
        Ok(Self { code_points })
    }

    /// The code points in order.
    pub fn code_points(&self) -> &[u32] {
        &self.code_points
    }

    /// Python's `len()`: one unit per code point.
    pub fn len(&self) -> usize {
        self.code_points.len()
    }

    /// Whether the text has no code points.
    pub fn is_empty(&self) -> bool {
        self.code_points.is_empty()
    }

    /// Whether any code point is a lone surrogate.
    pub fn has_surrogates(&self) -> bool {
        self.code_points
            .iter()
            .any(|point| (0xD800..=0xDFFF).contains(point))
    }

    /// Convert to a UTF-8 `String`, refusing lone surrogates.
    pub fn to_utf8(&self) -> Result<String, CanonicalJsonError> {
        let mut out = String::with_capacity(self.code_points.len());
        for point in &self.code_points {
            match char::from_u32(*point) {
                Some(character) => out.push(character),
                None => return Err(CanonicalJsonError::LoneSurrogate),
            }
        }
        Ok(out)
    }

    /// Convert to a UTF-8 `String`, replacing lone surrogates with U+FFFD.
    ///
    /// This is for diagnostics only; it is lossy and must not be used for
    /// digests.
    pub fn to_string_lossy(&self) -> String {
        self.code_points
            .iter()
            .map(|point| char::from_u32(*point).unwrap_or('\u{FFFD}'))
            .collect()
    }
}

impl From<&str> for JsonText {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for JsonText {
    fn from(value: String) -> Self {
        Self::new(&value)
    }
}

impl From<&String> for JsonText {
    fn from(value: &String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for JsonText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_string_lossy())
    }
}

impl fmt::Debug for JsonText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("JsonText")
            .field(&self.to_string_lossy())
            .finish()
    }
}
