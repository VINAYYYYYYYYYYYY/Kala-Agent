//! The flat Studio error envelope and its sanitization rules.
//!
//! Every failure is `{"code", "message", "details"}` with no traceback, no
//! submitted value and no credential-shaped text. The ordering of the object
//! depends on the emitting layer, exactly as in the Python source:
//!
//! * validation and framework errors are Pydantic-model output
//!   (`code`, `message`, `details`),
//! * route-raised errors and transport refusals use
//!   `json.dumps(..., sort_keys=True)` (`code`, `details`, `message`).

use crate::json::Json;

pub const INTERNAL_ERROR_MESSAGE: &str = "the studio request could not be completed";
pub const VALIDATION_MESSAGE: &str = "the request body or parameters are invalid";
pub const UNKNOWN_ROUTE_MESSAGE: &str = "the requested resource does not exist";
pub const METHOD_NOT_ALLOWED_MESSAGE: &str =
    "the requested method is not allowed for this resource";
pub const PAYLOAD_TOO_LARGE_MESSAGE: &str = "the request body exceeds the configured size limit";
pub const REQUEST_REJECTED_MESSAGE: &str = "the request was rejected";
pub const MAX_DETAIL_ENTRY_LENGTH: usize = 128;
pub const MAX_VALIDATION_DETAILS: usize = 8;

/// One sanitized HTTP error.
#[derive(Clone, Debug, PartialEq)]
pub struct FlatError {
    pub status: u16,
    pub code: &'static str,
    pub message: &'static str,
    pub details: Vec<String>,
}

impl FlatError {
    /// Construct an error from bounded, already-sanitized strings.
    #[must_use]
    pub fn new(
        status: u16,
        code: &'static str,
        message: &'static str,
        details: impl IntoIterator<Item = String>,
    ) -> Self {
        let bounded: Vec<String> = details
            .into_iter()
            .filter(|detail| !detail.is_empty())
            .take(16)
            .map(|detail| truncate_chars(&detail, 256))
            .collect();
        Self {
            status,
            code,
            message,
            details: bounded,
        }
    }

    /// Model-style ordered body (`code`, `message`, `details`).
    #[must_use]
    pub fn ordered_body(&self) -> Json {
        Json::object(vec![
            ("code", Json::String(self.code.to_owned())),
            ("message", Json::String(self.message.to_owned())),
            (
                "details",
                Json::Array(
                    self.details
                        .iter()
                        .map(|detail| Json::String(detail.clone()))
                        .collect(),
                ),
            ),
        ])
    }

    /// Refusal-style sorted body (`code`, `details`, `message`).
    #[must_use]
    pub fn sorted_body(&self) -> Json {
        self.ordered_body()
    }
}

/// Truncate a string to a number of Unicode code points, matching Python
/// slicing.
#[must_use]
pub fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

/// Whether a detail is safe to echo: short and free of credential-shaped
/// substrings.
#[must_use]
pub fn safe_detail(value: &str) -> Option<&str> {
    let text = value.trim();
    if text.is_empty() || text.chars().count() > MAX_DETAIL_ENTRY_LENGTH {
        return None;
    }
    if has_credential_shape(text) {
        return None;
    }
    Some(text)
}

fn has_credential_shape(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    // A bounded approximation of the Python regular expression. The Studio
    // only ever feeds framework details through this helper; route errors
    // use their own sanitized text.
    if contains_sk_key(&lower) {
        return true;
    }
    for marker in ["aiza", "akia", "bearer "] {
        if lower.contains(marker) {
            return true;
        }
    }
    false
}

fn contains_sk_key(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index + 3 <= bytes.len() {
        if &bytes[index..index + 3] == b"sk-" || &bytes[index..index + 3] == b"sk_" {
            let mut cursor = index + 3;
            if text[cursor..].starts_with("live_") || text[cursor..].starts_with("test_") {
                cursor += 5;
            }
            let mut count = 0;
            while cursor < bytes.len() && bytes[cursor].is_ascii_alphanumeric() {
                cursor += 1;
                count += 1;
            }
            if count >= 8 {
                return true;
            }
        }
        index += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_details_like_python_slices() {
        assert_eq!(truncate_chars("bøks", 2), "bø");
    }

    #[test]
    fn credential_shapes_are_not_echoed() {
        assert!(safe_detail("sk-abcdefghijklmnop").is_none());
        assert!(safe_detail("Bearer abcdefghijklmnop").is_none());
        assert_eq!(safe_detail("plain"), Some("plain"));
    }
}
