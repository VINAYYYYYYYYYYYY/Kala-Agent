//! The security tests of the request boundary.
//!
//! These attack the boundary only: a rebound Host header, a missing or wrong
//! bearer token, a body that exceeds the limit on a method that carries none,
//! a wildcard address on the loopback allowlist, and a cross-origin request.
//! Every test runs offline and no test reaches a route by disabling a
//! control.
//!
//! They are the Rust counterparts of `tests/security/test_api_boundary.py`
//! and of the boundary cases in `tests/unit/test_api.py` of the repository.

use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::header::{HeaderName, HeaderValue};
use hyper::{Method, Request, StatusCode};
use kala_api::boundary::{
    API_TOKEN_FILE_MODE, API_TOKEN_FILE_NAME, MUTATING_METHODS, is_loopback_host,
    is_permitted_authority, issue_api_token, read_api_token, validate_api_token,
};
use kala_api::middleware::SECURITY_HEADERS;
use kala_api::probe::ProbeRouter;
use kala_api::server::{ApiState, handle_request};

const TOKEN: &str = "security-test-launch-token-0123456789";
const SECRET_SHAPED: &str = "sk-must-never-be-echoed-9876543210abcdef";

fn state() -> ApiState {
    ApiState::new(TOKEN, 1024, Arc::new(ProbeRouter)).expect("a valid state")
}

fn request(method: Method, headers: &[(&str, &str)], body: &[u8]) -> Request<Full<Bytes>> {
    let mut request = Request::builder()
        .method(method)
        .uri("/probe")
        .body(Full::new(Bytes::from(body.to_vec())))
        .expect("a valid request");
    for (name, value) in headers {
        request.headers_mut().append(
            HeaderName::from_bytes(name.as_bytes()).expect("a header name"),
            HeaderValue::from_bytes(value.as_bytes()).expect("a header value"),
        );
    }
    request
}

async fn answer(
    state: &ApiState,
    method: Method,
    headers: &[(&str, &str)],
    body: &[u8],
) -> (StatusCode, Vec<(String, String)>, Vec<u8>) {
    let response = handle_request(state, request(method, headers, body)).await;
    let status = response.status();
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body is readable")
        .to_bytes()
        .to_vec();
    (status, headers, body)
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, value)| value.as_str())
}

#[tokio::test]
async fn a_rebound_host_header_never_reaches_a_route() {
    let state = state();
    for host in [
        "evil.example",
        "attacker.example:8000",
        "127.0.0.1.evil.example",
        "localhost.evil.example",
        "127.0.0.1@evil.example",
        "0.0.0.0",
        "0.0.0.0:8000",
        "::",
        "[::]:8000",
        "10.0.0.5:8000",
        "169.254.169.254",
        "127.0.0.1.",
        "127.0.0.1:0",
        "127.0.0.1:99999",
        "127.0.0.1:port",
        "[::1",
        "",
    ] {
        let (status, _, body) = answer(
            &state,
            Method::POST,
            &[
                ("host", host),
                ("authorization", &format!("Bearer {TOKEN}")),
            ],
            b"{}",
        )
        .await;
        assert_eq!(status, 421, "{host}");
        let text = String::from_utf8(body).expect("a UTF-8 body");
        assert!(text.contains(r#""code":"invalid_host""#), "{host}: {text}");
    }
    for host in [
        "127.0.0.1",
        "127.0.0.1:8000",
        "localhost",
        "localhost:8000",
        "[::1]:8000",
    ] {
        let (status, _, _) = answer(&state, Method::GET, &[("host", host)], b"").await;
        assert_eq!(status, StatusCode::OK, "{host}");
    }
}

#[tokio::test]
async fn duplicate_host_headers_are_refused() {
    let state = state();
    let (status, _, body) = answer(
        &state,
        Method::GET,
        &[("host", "127.0.0.1"), ("host", "evil.example")],
        b"",
    )
    .await;
    assert_eq!(status, 421);
    assert!(String::from_utf8_lossy(&body).contains(r#""code":"invalid_host""#));
}

#[tokio::test]
async fn a_request_without_a_host_header_is_refused() {
    let state = state();
    let (status, _, _) = answer(&state, Method::GET, &[("host", "")], b"").await;
    assert_eq!(status, 421);
}

#[tokio::test]
async fn mutating_methods_require_the_launch_token() {
    let state = state();
    let (status, _, _) = answer(&state, Method::GET, &[("host", "127.0.0.1")], b"").await;
    assert_eq!(status, StatusCode::OK);
    for authorization in [
        None,
        Some(""),
        Some("Bearer"),
        Some("Bearer "),
        Some(&format!("Basic {TOKEN}")),
        Some(&format!("Bearer {TOKEN}x")),
        Some(&format!("bearer {}", &TOKEN[..TOKEN.len() - 1])),
        Some(&format!("Bearer {SECRET_SHAPED}")),
        Some(TOKEN),
        Some(&format!("Bearer {TOKEN}, Bearer {TOKEN}")),
    ] {
        let mut headers: Vec<(&str, &str)> = vec![("host", "127.0.0.1")];
        if let Some(value) = authorization {
            headers.push(("authorization", value));
        }
        let (status, response_headers, body) = answer(&state, Method::POST, &headers, b"{}").await;
        assert_eq!(status, 401, "{authorization:?}");
        assert_eq!(
            header_value(&response_headers, "www-authenticate"),
            Some("Bearer realm=\"kala-agent\"")
        );
        let text = String::from_utf8_lossy(&body);
        assert!(!text.contains(TOKEN));
        assert!(!text.contains(SECRET_SHAPED));
        assert!(!text.contains("Traceback"));
    }
    let wrong = format!("Bearer {}", "w".repeat(TOKEN.len()));
    let (status, _, _) = answer(
        &state,
        Method::POST,
        &[("host", "127.0.0.1"), ("authorization", &wrong)],
        b"{}",
    )
    .await;
    assert_eq!(status, 401);
    for prefix in ["Bearer ", "bearer ", "BEARER ", "Bearer  "] {
        let (status, _, _) = answer(
            &state,
            Method::POST,
            &[
                ("host", "127.0.0.1"),
                ("authorization", &format!("{prefix}{TOKEN}")),
            ],
            b"{}",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{prefix}");
    }
}

#[tokio::test]
async fn every_mutating_method_is_judged() {
    let state = state();
    for method in [
        Method::DELETE,
        Method::GET,
        Method::HEAD,
        Method::OPTIONS,
        Method::PATCH,
        Method::POST,
        Method::PUT,
    ] {
        let (status, _, _) = answer(&state, method.clone(), &[("host", "127.0.0.1")], b"").await;
        let mutating = MUTATING_METHODS
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(method.as_str()));
        if mutating {
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method}");
        } else {
            assert_ne!(status, StatusCode::UNAUTHORIZED, "{method}");
        }
    }
    assert_eq!(MUTATING_METHODS, ["DELETE", "PATCH", "POST", "PUT"]);
}

#[tokio::test]
async fn the_body_size_limit_answers_every_method() {
    let state = ApiState::new(TOKEN, 1024, Arc::new(ProbeRouter)).expect("a valid state");
    let oversized = vec![b'x'; 4096];
    for method in [
        Method::GET,
        Method::HEAD,
        Method::DELETE,
        Method::OPTIONS,
        Method::POST,
        Method::PUT,
        Method::PATCH,
    ] {
        let (status, headers, body) =
            answer(&state, method.clone(), &[("host", "127.0.0.1")], &oversized).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{method}");
        assert_eq!(
            header_value(&headers, "x-content-type-options"),
            Some("nosniff")
        );
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains(r#""code":"payload_too_large""#), "{method}");
        assert!(text.contains("the request body must not exceed 1024 bytes"));
    }
    let within = vec![b'y'; 64];
    let (status, _, _) = answer(&state, Method::GET, &[("host", "127.0.0.1")], &within).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn cross_origin_requests_are_refused_with_no_grant() {
    let state = state();
    for origin in [
        "http://evil.example",
        "http://localhost:5173",
        "http://127.0.0.1:5173",
        "http://0.0.0.0:5173",
        "https://127.0.0.1:5173",
        "null",
    ] {
        let (status, headers, body) = answer(
            &state,
            Method::GET,
            &[("host", "127.0.0.1"), ("origin", origin)],
            b"",
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{origin}");
        assert!(String::from_utf8_lossy(&body).contains(r#""code":"cross_origin_denied""#));
        assert!(
            !headers
                .iter()
                .any(|(name, _)| name.starts_with("access-control-")),
            "{origin}"
        );
    }
    let (status, _, _) = answer(&state, Method::GET, &[("host", "127.0.0.1")], b"").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn every_response_carries_the_security_headers() {
    let state = state();
    let (_, headers, _) = answer(&state, Method::GET, &[("host", "127.0.0.1")], b"").await;
    for (name, value) in SECURITY_HEADERS {
        assert_eq!(header_value(&headers, name), Some(value), "{name}");
    }
    assert!(header_value(&headers, "server").is_none());
    assert!(header_value(&headers, "x-powered-by").is_none());
}

#[tokio::test]
async fn error_responses_never_echo_submitted_values() {
    let state = state();
    let (status, _, body) = answer(
        &state,
        Method::POST,
        &[
            ("host", "127.0.0.1"),
            ("authorization", &format!("Bearer {SECRET_SHAPED}")),
        ],
        SECRET_SHAPED.as_bytes(),
    )
    .await;
    assert_eq!(status, 401);
    let text = String::from_utf8_lossy(&body);
    assert!(!text.contains(SECRET_SHAPED));
    assert!(!text.contains("Traceback"));
    assert!(text.contains(r#""error":{"code":"unauthorized""#));
}

#[test]
fn the_wildcard_address_is_never_loopback() {
    for host in [
        "0.0.0.0",
        "::",
        "[::]",
        "0:0:0:0:0:0:0:0",
        "0.0.0.0:8000",
        "[::]:8000",
        "*",
    ] {
        assert!(!is_loopback_host(host), "{host}");
        assert!(!is_permitted_authority(host), "{host}");
    }
    for host in [
        "localhost",
        "127.0.0.1",
        "127.0.0.2",
        "[::1]",
        "[::1]:8000",
        "localhost:8000",
    ] {
        assert!(is_permitted_authority(host), "{host}");
    }
    for host in ["localhost", "127.0.0.1", "127.0.0.2", "::1"] {
        assert!(is_loopback_host(host), "{host}");
    }
    for host in [
        "evil.example",
        "",
        "127.0.0.1.",
        "http://127.0.0.1",
        "127.0.0.1/path",
    ] {
        assert!(!is_loopback_host(host), "{host}");
        assert!(!is_permitted_authority(host), "{host}");
    }
    assert!(!is_permitted_authority("::1"));
    for authority in [
        "127.0.0.1 ",
        " 127.0.0.1",
        "[::1]x",
        "[::1]x:8000",
        ":8000",
        "1",
        "[::1",
    ] {
        assert!(!is_permitted_authority(authority), "{authority}");
    }
}

#[test]
fn the_launch_token_is_stored_never_returned_and_rotated() {
    let work = tempfile::tempdir().expect("a temporary directory");
    let data_dir = work.path();
    let first = issue_api_token(data_dir).expect("a launch token");
    let stored = read_api_token(data_dir).expect("the stored token");
    assert_eq!(stored, first);
    let path = data_dir.join(API_TOKEN_FILE_NAME);
    assert!(path.is_file());
    assert_eq!(validate_api_token(&stored).expect("a valid token"), stored);
    let second = issue_api_token(data_dir).expect("a launch token");
    let rotated = read_api_token(data_dir).expect("the stored token");
    assert_eq!(rotated, second);
    assert_ne!(rotated, stored);
    assert!(read_api_token(&work.path().join("missing")).is_none());
}

#[test]
#[cfg(unix)]
fn the_stored_token_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let work = tempfile::tempdir().expect("a temporary directory");
    let token = issue_api_token(work.path()).expect("a launch token");
    let path = work.path().join(API_TOKEN_FILE_NAME);
    let mode = std::fs::metadata(&path)
        .expect("the token file")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, API_TOKEN_FILE_MODE);
    assert_eq!(read_api_token(work.path()).as_deref(), Some(token.as_str()));
}

#[test]
fn a_missing_or_corrupt_token_store_reads_as_absent() {
    let work = tempfile::tempdir().expect("a temporary directory");
    let data_dir = work.path().join("empty");
    std::fs::create_dir_all(&data_dir).expect("the directory");
    assert!(read_api_token(&data_dir).is_none());
    let path = data_dir.join(API_TOKEN_FILE_NAME);
    std::fs::write(&path, b"short\n").expect("the token file");
    assert!(read_api_token(&data_dir).is_none());
    std::fs::write(&path, format!("{}\n", "x".repeat(40))).expect("the token file");
    assert_eq!(
        read_api_token(&data_dir).as_deref(),
        Some("x".repeat(40).as_str())
    );
    std::fs::write(&path, b"\xff\xfe not utf 8").expect("the token file");
    assert!(read_api_token(&data_dir).is_none());
    for unusable in ["", "   ", "a", &"a".repeat(15), &"a".repeat(129)] {
        assert!(validate_api_token(unusable).is_err(), "{unusable:?}");
    }
    assert!(validate_api_token(&"a".repeat(16)).is_ok());
    assert!(validate_api_token(&"a".repeat(128)).is_ok());
    assert!(validate_api_token("a-b_c.d~e01234567").is_ok());
    assert!(validate_api_token("has space in it and is long").is_err());
}

#[tokio::test]
async fn a_token_that_cannot_be_stored_still_guards_the_process() {
    let work = tempfile::tempdir().expect("a temporary directory");
    let data_dir = work.path().join("read-only");
    std::fs::create_dir_all(&data_dir).expect("the directory");
    std::fs::create_dir(data_dir.join(API_TOKEN_FILE_NAME)).expect("the blocked path");
    let token = issue_api_token(&data_dir).expect("a launch token");
    assert!(read_api_token(&data_dir).is_none());
    let state = ApiState::new(&token, 1024, Arc::new(ProbeRouter)).expect("a valid state");
    let (status, _, _) = answer(&state, Method::POST, &[("host", "127.0.0.1")], b"{}").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
