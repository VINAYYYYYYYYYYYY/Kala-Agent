//! Windows-style JSON encodings and CPython-compatible malformed offsets.
//!
//! `json.loads(bytes)` in FastAPI performs encoding detection, so the live
//! Python Studio accepts UTF-8 BOM, UTF-16 and UTF-32 request bodies. These
//! cases fail against the pre-repair native parser and must stay exact once
//! fixed. Every request goes through the real socket and middleware stack.
//!
//! The body parsing under test is the parked native parser, so this file
//! runs in the `native-primitives` build. The default build forwards the
//! body to the geometry worker, which parses it with the source's own
//! parser; the live Python differential covers that path.

#![cfg(feature = "native-primitives")]

mod common;

use std::collections::HashMap;

use common::{RequestSpec, TOKEN, TestServer, empty_environ};

fn utf16(text: &str, little: bool, bom: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    if bom {
        bytes.extend_from_slice(if little { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
    }
    for unit in text.encode_utf16() {
        let encoded = if little {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        };
        bytes.extend_from_slice(&encoded);
    }
    bytes
}

fn utf32(text: &str, little: bool, bom: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    if bom {
        bytes.extend_from_slice(if little {
            &[0xFF, 0xFE, 0x00, 0x00]
        } else {
            &[0x00, 0x00, 0xFE, 0xFF]
        });
    }
    for character in text.chars() {
        let value = character as u32;
        let encoded = if little {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        };
        bytes.extend_from_slice(&encoded);
    }
    bytes
}

fn preview_spec(body: Vec<u8>, content_type: &str) -> RequestSpec {
    let mut spec = RequestSpec::default();
    spec.headers
        .push(("Authorization".to_owned(), format!("Bearer {TOKEN}")));
    spec.headers
        .push(("content-type".to_owned(), content_type.to_owned()));
    spec.body = Some(body);
    spec
}

async fn post_preview(server: &TestServer, body: Vec<u8>) -> common::TestResponse {
    common::request(
        server.addr,
        "POST",
        "/api/studio/preview",
        preview_spec(body, "application/json"),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn python_accepted_encodings_reach_the_route() {
    let work = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_backend(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        HashMap::new(),
        kala_studio::geometry::default_step_exporter(),
        None,
        std::sync::Arc::new(kala_studio::native_backend::NativePrimitivesBackend::new(
            kala_studio::geometry::default_step_exporter(),
            std::env::temp_dir().join("kala-studio-work"),
        )),
    )
    .await;

    let text = r#"{"kind":"box"}"#;
    let baseline = post_preview(&server, text.as_bytes().to_vec()).await;
    assert_eq!(baseline.status, 200);
    assert_eq!(baseline.body.len(), 925);

    let mut encoded: Vec<(&str, Vec<u8>)> = vec![
        (
            "utf8bom",
            [b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat(),
        ),
        ("utf16le", utf16(text, true, false)),
        ("utf16be", utf16(text, false, false)),
        ("utf16le-bom", utf16(text, true, true)),
        ("utf16be-bom", utf16(text, false, true)),
        ("utf32le", utf32(text, true, false)),
        ("utf32be", utf32(text, false, false)),
        ("utf32le-bom", utf32(text, true, true)),
        ("utf32be-bom", utf32(text, false, true)),
    ];
    encoded.push(("utf8", text.as_bytes().to_vec()));
    for (name, body) in encoded {
        let response = post_preview(&server, body).await;
        assert_eq!(response.status, 200, "{name}: {}", response.body_text());
        assert_eq!(response.body, baseline.body, "{name} body bytes");
        assert_eq!(
            response.header("content-type"),
            Some("application/json"),
            "{name}"
        );
        assert_eq!(response.header("cache-control"), Some("no-store"), "{name}");
    }
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_input_status_and_offsets_match_cpython() {
    let work = tempfile::tempdir().unwrap();
    let server = TestServer::start_with_backend(
        work.path().join("data"),
        TOKEN,
        64 * 1024,
        None,
        empty_environ(),
        kala_studio::geometry::default_step_exporter(),
        None,
        std::sync::Arc::new(kala_studio::native_backend::NativePrimitivesBackend::new(
            kala_studio::geometry::default_step_exporter(),
            std::env::temp_dir().join("kala-studio-work"),
        )),
    )
    .await;

    for (name, body, offset) in [
        ("utf8-truncated", b"{\"kind\":".to_vec(), 8usize),
        ("utf16-truncated", utf16(r#"{"kind":"#, true, true), 8),
        ("utf16-nan123", utf16(r#"{"kind":NaN123}"#, true, true), 11),
        ("nan123", b"{\"kind\":NaN123}".to_vec(), 11),
        ("infinity1", b"{\"kind\":Infinity1}".to_vec(), 16),
        ("negative-infinity", b"{\"kind\":-Infinityx}".to_vec(), 17),
        (
            "constant-before-later-error",
            b"{\"kind\":NaN,\"parameters\":}".to_vec(),
            25,
        ),
    ] {
        let response = post_preview(&server, body).await;
        assert_eq!(response.status, 422, "{name}: {}", response.body_text());
        assert_eq!(
            response.json()["details"][0],
            format!("body.{offset}: json_invalid: JSON decode error"),
            "{name}"
        );
    }

    // Non-UTF-8 bytes are a `UnicodeDecodeError`, which FastAPI answers with
    // its generic 400 body.
    let response = post_preview(&server, b"\xff\xfe\x02".to_vec()).await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["code"], "request_rejected");

    // A BOM-only document decodes to an empty string and reports position 0.
    let response = post_preview(&server, b"\xef\xbb\xbf".to_vec()).await;
    assert_eq!(response.status, 422);
    assert_eq!(
        response.json()["details"][0],
        "body.0: json_invalid: JSON decode error"
    );

    // A sanctioned non-finite constant in a dimension keeps the source's
    // finite-value validation branch instead of becoming a huge number.
    let response = post_preview(
        &server,
        b"{\"kind\":\"box\",\"parameters\":{\"size_x\":NaN}}".to_vec(),
    )
    .await;
    assert_eq!(response.status, 422);
    assert_eq!(response.json()["code"], "invalid_request");
    assert!(
        response.body_text().contains("finite"),
        "{}",
        response.body_text()
    );
    server.stop().await;
}
