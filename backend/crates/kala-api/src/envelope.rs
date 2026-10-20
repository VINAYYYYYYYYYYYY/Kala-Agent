//! The portable error envelope shared by the local adapters.
//!
//! Every failure the local HTTP surfaces report leaves the process in this one
//! small, stable JSON shape: an error code, a bounded human message, and
//! optional sanitized details. Nothing here imports the composition root, the
//! tool plane, or a platform-specific component, so the envelope can be loaded
//! by a transport that only needs to answer a refusal.
//!
//! Two byte layouts exist in the Python source and both are reproduced here:
//!
//! * the *ordered* body, which is what the FastAPI error handlers write:
//!   `{"error":{"code":..,"message":..},"details":[..]}`, encoded with
//!   `json.dumps(..., ensure_ascii=False, separators=(",", ":"))`;
//! * the *sorted* body, which is what every transport refusal writes:
//!   `{"details":[..],"error":{"code":..,"message":..}}`, encoded with
//!   `json.dumps(..., sort_keys=True)`, that is with `ensure_ascii=True` and
//!   keys sorted by code point.
//!
//! The sorted body is exactly what the accepted `kala-json` crate produces, so
//! it is emitted through that crate. The ordered body is *not* canonical JSON:
//! `kala-json` sorts keys, and the Python error handlers do not, so the ordered
//! body is encoded here with the same CPython escaping rules.

use kala_json::{CanonicalValue, JsonText};

/// The largest number of detail entries an error body may carry.
pub const MAX_ERROR_DETAILS: usize = 16;

/// The largest number of code points a message or a detail entry may carry.
pub const MAX_ERROR_MESSAGE_LENGTH: usize = 256;

/// The error code and the bounded human message of one failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorDetail {
    /// The stable machine readable code.
    pub code: String,
    /// The bounded human readable message.
    pub message: String,
}

/// The only error shape this API returns. It never carries a traceback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorResponse {
    /// The code and the message.
    pub error: ErrorDetail,
    /// The sanitized details, already bounded.
    pub details: Vec<String>,
}

impl ErrorResponse {
    /// The byte layout the FastAPI error handlers write.
    ///
    /// The keys keep the declaration order of the Python model and the string
    /// escaping is CPython's `ensure_ascii=False` escaping.
    #[must_use]
    pub fn ordered_bytes(&self) -> Vec<u8> {
        let mut out = String::with_capacity(96 + self.details.len() * 32);
        out.push_str("{\"error\":{\"code\":");
        encode_json_string(&self.error.code, &mut out);
        out.push_str(",\"message\":");
        encode_json_string(&self.error.message, &mut out);
        out.push_str("},\"details\":[");
        for (index, detail) in self.details.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            encode_json_string(detail, &mut out);
        }
        out.push_str("]}");
        out.into_bytes()
    }

    /// The byte layout every transport refusal writes.
    ///
    /// The keys are sorted and the escaping is CPython's `ensure_ascii=True`
    /// escaping, which is what the accepted `kala-json` crate emits.
    #[must_use]
    pub fn sorted_bytes(&self) -> Vec<u8> {
        let details = CanonicalValue::array(
            self.details
                .iter()
                .map(|detail| CanonicalValue::Text(JsonText::new(detail))),
        );
        let error = CanonicalValue::object([
            (
                "code",
                CanonicalValue::Text(JsonText::new(&self.error.code)),
            ),
            (
                "message",
                CanonicalValue::Text(JsonText::new(&self.error.message)),
            ),
        ]);
        let document = CanonicalValue::object([("details", details), ("error", error)]);
        match kala_json::canonical_dumps(&document, &kala_json::Limits::default()) {
            Ok(rendered) => rendered.into_bytes(),
            // The envelope is bounded to a few hundred bytes of ASCII-safe
            // text, so the canonical limits cannot reject it. Fall back to the
            // ordered layout rather than panicking if that ever changes.
            Err(_) => self.ordered_bytes(),
        }
    }
}

/// Build a bounded, secret-free error body.
///
/// The details keep their order, the first [`MAX_ERROR_DETAILS`] entries only,
/// and each entry is cut to [`MAX_ERROR_MESSAGE_LENGTH`] code points. The
/// message is cut the same way. The code is never cut, because it is a
/// constant of this crate or of the composition root.
#[must_use]
pub fn error_response(code: &str, message: &str, details: &[&str]) -> ErrorResponse {
    let bounded_details = details
        .iter()
        .take(MAX_ERROR_DETAILS)
        .map(|detail| truncate_chars(detail, MAX_ERROR_MESSAGE_LENGTH))
        .collect();
    ErrorResponse {
        error: ErrorDetail {
            code: code.to_owned(),
            message: truncate_chars(message, MAX_ERROR_MESSAGE_LENGTH),
        },
        details: bounded_details,
    }
}

/// Truncate a string to a number of Unicode code points, matching Python
/// slicing.
#[must_use]
pub fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

/// CPython's `json.dumps(..., ensure_ascii=False)` string escaping.
///
/// Only the characters CPython escapes are escaped: the quote, the backslash,
/// the five short control escapes and any other control character as
/// `\u00xx`. Everything else, including DEL and every non-ASCII code point,
/// is written verbatim and therefore ends up as UTF-8 in the response bytes.
fn encode_json_string(value: &str, out: &mut String) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            character if (character as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sorted_body_matches_cpython() {
        let body = error_response(
            "invalid_host",
            "the request host is not a loopback address",
            &[],
        );
        assert_eq!(
            String::from_utf8(body.sorted_bytes()).unwrap(),
            r#"{"details":[],"error":{"code":"invalid_host","message":"the request host is not a loopback address"}}"#
        );
    }

    #[test]
    fn the_ordered_body_matches_cpython() {
        let body = error_response(
            "invalid_host",
            "the request host is not a loopback address",
            &[],
        );
        assert_eq!(
            String::from_utf8(body.ordered_bytes()).unwrap(),
            r#"{"error":{"code":"invalid_host","message":"the request host is not a loopback address"},"details":[]}"#
        );
    }

    #[test]
    fn the_ordered_body_keeps_non_ascii_verbatim() {
        let body = error_response("invalid_request", "caf\u{e9} \u{2713}", &["d\u{e9}tail"]);
        assert_eq!(
            String::from_utf8(body.ordered_bytes()).unwrap(),
            "{\"error\":{\"code\":\"invalid_request\",\"message\":\"caf\u{e9} \u{2713}\"},\"details\":[\"d\u{e9}tail\"]}"
        );
    }

    #[test]
    fn the_sorted_body_escapes_non_ascii() {
        let body = error_response("invalid_request", "caf\u{e9}", &["d\u{e9}tail"]);
        assert_eq!(
            String::from_utf8(body.sorted_bytes()).unwrap(),
            r#"{"details":["d\u00e9tail"],"error":{"code":"invalid_request","message":"caf\u00e9"}}"#
        );
    }

    #[test]
    fn details_and_messages_are_bounded() {
        let long = "x".repeat(400);
        let body = error_response("code", &long, &[long.as_str(); 20]);
        assert_eq!(body.error.message.chars().count(), MAX_ERROR_MESSAGE_LENGTH);
        assert_eq!(body.details.len(), MAX_ERROR_DETAILS);
        assert!(
            body.details
                .iter()
                .all(|entry| entry.chars().count() == MAX_ERROR_MESSAGE_LENGTH)
        );
    }

    #[test]
    fn control_characters_are_escaped_like_cpython() {
        let body = error_response("a\"b\\c\nd\u{1}e", "line\u{8}end", &[]);
        assert_eq!(
            String::from_utf8(body.ordered_bytes()).unwrap(),
            "{\"error\":{\"code\":\"a\\\"b\\\\c\\nd\\u0001e\",\"message\":\"line\\bend\"},\"details\":[]}"
        );
    }
}
