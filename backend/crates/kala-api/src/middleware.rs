//! Transport-level middleware for the local Kala HTTP adapter.
//!
//! Two controls live here and nothing else: a request body size limit that
//! answers every method, and a fixed set of security headers on every
//! response. The rest of the request boundary, loopback addressing, the launch
//! token, and cross-origin refusals, lives in [`crate::boundary`]. No
//! middleware here authenticates a caller, inspects a payload for a business
//! rule, or reaches the network.
//!
//! The Python source is an ASGI middleware stack. This crate keeps the same
//! decisions in the same order, but expresses them over a transport-neutral
//! [`RequestContext`] so the gates can be exercised without an HTTP parser in
//! between, exactly like the Python test client does.

use http::HeaderName;
use http::header::HeaderMap;

use crate::envelope::error_response;

/// The largest request body this transport accepts.
pub const MAX_REQUEST_BODY_BYTES: usize = 262_144;

/// The smallest request body limit this transport accepts.
pub const MIN_REQUEST_BODY_BYTES: usize = 1_024;

/// The largest number of characters a received header value may contribute.
pub const MAX_HEADER_VALUE_LENGTH: usize = 512;

/// The fixed security header set every response carries.
pub const SECURITY_HEADERS: [(&str, &str); 7] = [
    ("cache-control", "no-store"),
    (
        "content-security-policy",
        "default-src 'none'; frame-ancestors 'none'; base-uri 'none'",
    ),
    ("cross-origin-opener-policy", "same-origin"),
    ("cross-origin-resource-policy", "same-origin"),
    ("referrer-policy", "no-referrer"),
    ("x-content-type-options", "nosniff"),
    ("x-frame-options", "DENY"),
];

/// The response headers that advertise the implementation and are removed.
pub const STRIPPED_RESPONSE_HEADERS: [&str; 2] = ["server", "x-powered-by"];

/// The message a refused oversized body carries.
pub const PAYLOAD_TOO_LARGE_MESSAGE: &str = "the request body exceeds the configured size limit";

/// Return a usable request body limit.
///
/// # Errors
///
/// Returns a sanitized message when the limit is outside the accepted range.
/// The Python source also refuses a `bool`, because Python counts `bool` as an
/// `int`; a Rust `usize` cannot carry that mistake, so the range message is
/// the only failure this function can produce.
pub fn validate_max_body_bytes(value: i64) -> Result<usize, String> {
    if !(MIN_REQUEST_BODY_BYTES as i64..=MAX_REQUEST_BODY_BYTES as i64).contains(&value) {
        return Err(format!(
            "max_body_bytes must be between {MIN_REQUEST_BODY_BYTES} and {MAX_REQUEST_BODY_BYTES}"
        ));
    }
    Ok(value as usize)
}

/// Return the one sanitized JSON body every transport refusal uses.
///
/// The bytes are the sorted layout: `json.dumps(..., separators=(",", ":"),
/// sort_keys=True)`.
#[must_use]
pub fn refusal_body(code: &str, message: &str, details: &[&str]) -> Vec<u8> {
    error_response(code, message, details).sorted_bytes()
}

/// Decode a header value the way the ASGI boundary does: Latin-1, bounded.
#[must_use]
pub fn decode_header_value(value: &[u8]) -> String {
    let text: String = value.iter().map(|byte| char::from(*byte)).collect();
    crate::envelope::truncate_chars(&text, MAX_HEADER_VALUE_LENGTH)
}

/// The raw headers of a request, as the transport received them.
///
/// The names are lower case and the values keep their bytes, so the boundary
/// judges exactly what arrived.
#[must_use]
pub fn raw_headers(headers: &HeaderMap) -> Vec<(Vec<u8>, Vec<u8>)> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str().as_bytes().to_vec(), value.as_bytes().to_vec()))
        .collect()
}

/// Return every value sent for one header, bounded and safely decoded.
#[must_use]
pub fn header_values(headers: &[(Vec<u8>, Vec<u8>)], name: &str) -> Vec<String> {
    headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case(name.as_bytes()))
        .map(|(_, value)| decode_header_value(value))
        .collect()
}

/// Return the first value sent for one header, or `None` when it is absent.
#[must_use]
pub fn header_value(headers: &[(Vec<u8>, Vec<u8>)], name: &str) -> Option<String> {
    header_values(headers, name).into_iter().next()
}

/// Return the declared body length, or `None` when there is none to trust.
///
/// A declared length that is not a plain ASCII digit string is ignored, which
/// is what the Python source does: the body is then buffered up to the limit
/// and replayed, so a chunked request without a declared length cannot bypass
/// the limit either.
#[must_use]
pub fn declared_length(headers: &[(Vec<u8>, Vec<u8>)]) -> Option<u128> {
    let raw = header_value(headers, "content-length")?;
    let candidate = crate::pytext::trim(&raw);
    if candidate.is_empty() || !candidate.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    candidate.parse::<u128>().ok().or(Some(u128::MAX))
}

/// Remove the implementation headers and add the fixed security header set.
///
/// Headers the application already set are preserved; the security headers
/// are only added when they are absent.
pub fn apply_security_headers(headers: &mut HeaderMap) {
    for name in STRIPPED_RESPONSE_HEADERS {
        headers.remove(name);
    }
    for (name, value) in SECURITY_HEADERS {
        let header_name = HeaderName::from_bytes(name.as_bytes()).expect("static header name");
        if !headers.contains_key(&header_name) {
            if let Ok(header_value) = value.parse::<http::HeaderValue>() {
                headers.insert(header_name, header_value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_limit_range_is_enforced() {
        assert_eq!(
            validate_max_body_bytes(0),
            Err(String::from(
                "max_body_bytes must be between 1024 and 262144"
            ))
        );
        assert_eq!(
            validate_max_body_bytes(MIN_REQUEST_BODY_BYTES as i64),
            Ok(MIN_REQUEST_BODY_BYTES)
        );
        assert_eq!(
            validate_max_body_bytes(MAX_REQUEST_BODY_BYTES as i64),
            Ok(MAX_REQUEST_BODY_BYTES)
        );
        assert!(validate_max_body_bytes(MAX_REQUEST_BODY_BYTES as i64 + 1).is_err());
        assert!(validate_max_body_bytes(-1).is_err());
    }

    #[test]
    fn the_refusal_body_is_the_sorted_layout() {
        assert_eq!(
            String::from_utf8(refusal_body("unauthorized", "no token", &[])).unwrap(),
            r#"{"details":[],"error":{"code":"unauthorized","message":"no token"}}"#
        );
    }

    #[test]
    fn header_values_are_bounded_and_decoded_as_latin1() {
        let headers = vec![
            (b"host".to_vec(), b"127.0.0.1".to_vec()),
            (b"host".to_vec(), b"evil.example".to_vec()),
            (b"x-long".to_vec(), vec![b'a'; 600]),
        ];
        assert_eq!(
            header_values(&headers, "host"),
            vec![String::from("127.0.0.1"), String::from("evil.example")]
        );
        assert_eq!(
            header_value(&headers, "host"),
            Some(String::from("127.0.0.1"))
        );
        assert_eq!(header_value(&headers, "origin"), None);
        assert_eq!(
            header_values(&headers, "x-long")[0].chars().count(),
            MAX_HEADER_VALUE_LENGTH
        );
    }

    #[test]
    fn declared_lengths_must_be_plain_digits() {
        let length = |value: &str| {
            declared_length(&[(b"content-length".to_vec(), value.as_bytes().to_vec())])
        };
        assert_eq!(length("0"), Some(0));
        assert_eq!(length("12"), Some(12));
        assert_eq!(length(" 12 "), Some(12));
        assert_eq!(length("-1"), None);
        assert_eq!(length("+1"), None);
        assert_eq!(length("1.5"), None);
        assert_eq!(length(""), None);
        assert_eq!(length("abc"), None);
        assert_eq!(length(&"9".repeat(40)), Some(u128::MAX));
    }

    #[test]
    fn security_headers_are_added_once() {
        let mut headers = HeaderMap::new();
        headers.insert("server", "uvicorn".parse().unwrap());
        headers.insert("cache-control", "public".parse().unwrap());
        apply_security_headers(&mut headers);
        assert!(!headers.contains_key("server"));
        assert_eq!(headers["cache-control"], "public");
        assert_eq!(headers["x-frame-options"], "DENY");
        assert_eq!(headers["content-security-policy"], SECURITY_HEADERS[1].1);
    }
}
