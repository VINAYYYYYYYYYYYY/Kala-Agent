//! The client is tested against an in-process mock bridge.
//!
//! The mock implements the bridge contract: it requires a bearer token, it
//! answers protocol v1 documents, and it maps its typed error envelope onto
//! HTTP statuses. It is a *mock*: it never claims a native Solid Edge build.

mod support;

use std::time::Duration;

use kala_solidedge_client::{
    Artifact, ArtifactFormat, BridgeClient, BridgeOp, BuildRequest, ErrorKind, JobState, Plane,
    build_request,
};
use support::{CannedResponse, MockBridge};

const TOKEN: &str = "test-token-0123456789abcdef";

fn client(bridge: &MockBridge) -> BridgeClient {
    BridgeClient::new(&bridge.url(), TOKEN, 5.0, 5.0).expect("a loopback client")
}

fn request() -> BuildRequest {
    BuildRequest::new(
        "req-1",
        Some("cal-1".to_owned()),
        "one part",
        vec![
            BridgeOp::PartNew {
                name: "plate".to_owned(),
            },
            BridgeOp::SketchRectangle {
                plane: Plane::XY,
                width: 20.0,
                height: 20.0,
                x: 0.0,
                y: 0.0,
            },
            BridgeOp::SolidExtrude {
                length: 10.0,
                reversed: false,
            },
            BridgeOp::PartMeasure,
        ],
        120.0,
        None,
    )
    .expect("a valid request")
}

fn artifact(data: &[u8], name: &str) -> Artifact {
    Artifact {
        name: name.to_owned(),
        format: ArtifactFormat::Step,
        media_type: "application/step".to_owned(),
        size_bytes: data.len() as u64,
        sha256: kala_solidedge_client::sha256_hex(data),
    }
}

fn health_body() -> String {
    r#"{"protocol_version":1,"bridge_version":"0.1.0","platform":"win32","supported":true,
        "available":true,"busy":false,"se_version":"2024","calibration_id":"cal-1",
        "capabilities":["protocol.v1"],"detail":""}"#
        .replace('\n', "")
        .replace("        ", "")
}

fn status_body(state: &str) -> String {
    format!(r#"{{"build_id":"build-1","state":"{state}","step_index":1,"total":1}}"#)
}

#[test]
fn health_is_authenticated_and_typed() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(200, &health_body()));
    let client = client(&bridge);

    let health = client.health().expect("health");

    assert_eq!(health.calibration_id.as_deref(), Some("cal-1"));
    assert_eq!(health.protocol_version, 1);
    let recorded = &bridge.requests()[0];
    assert_eq!(
        recorded.header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    assert_eq!(recorded.path, "/v1/health");
    assert!(recorded.header("origin").is_none());
}

#[test]
fn a_rejected_token_is_an_authentication_error_not_a_leak() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(
        401,
        r#"{"code":"invalid_request","message":"authentication failed"}"#,
    ));
    let client = client(&bridge);

    let error = client.health().expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Authentication);
    assert!(!error.message().contains(TOKEN), "the token leaked");
    assert!(error.message().contains("token"), "{}", error.message());
}

#[test]
fn a_forbidden_client_is_an_authentication_error() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(
        403,
        r#"{"code":"invalid_request","message":"forbidden"}"#,
    ));
    let client = client(&bridge);

    let error = client.health().expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Authentication);
}

#[test]
fn a_redirect_is_refused_not_followed() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(302, "").with_headers(vec![(
        "Location".to_owned(),
        "http://example.com/".to_owned(),
    )]));
    let client = client(&bridge);

    let error = client.health().expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Protocol);
    assert!(error.message().contains("redirect"), "{}", error.message());
    assert_eq!(bridge.requests().len(), 1);
}

#[test]
fn an_oversize_json_body_is_refused_before_it_is_parsed() {
    let bridge = MockBridge::start();
    let body = format!("{{\"padding\":\"{}\"}}", "x".repeat(1024 * 1024));
    bridge.queue(CannedResponse::json(200, &body));
    let client = client(&bridge);

    let error = client.health().expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Protocol);
    assert!(error.message().contains("1 MiB"), "{}", error.message());
}

#[test]
fn a_non_protocol_document_is_a_protocol_error() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(200, r#"{"unexpected":true}"#));
    let client = client(&bridge);

    let error = client.health().expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Protocol);
    assert!(
        error.message().contains("not protocol v1"),
        "{}",
        error.message()
    );
}

#[test]
fn a_version_mismatch_is_refused() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(
        200,
        r#"{"protocol_version":2,"bridge_version":"0.1.0","platform":"win32","supported":true,"available":true,"busy":false}"#,
    ));
    let client = client(&bridge);

    let error = client.health().expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Protocol);
    assert!(
        error.message().contains("protocol_version"),
        "{}",
        error.message()
    );
}

#[test]
fn submit_posts_the_typed_request() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(
        202,
        r#"{"build_id":"build-1","state":"queued"}"#,
    ));
    let client = client(&bridge);

    let accepted = client.submit(&request()).expect("accepted");

    assert_eq!(accepted.build_id, "build-1");
    assert_eq!(accepted.state, JobState::Queued);
    let recorded = &bridge.requests()[0];
    assert_eq!(recorded.method, "POST");
    assert_eq!(recorded.path, "/v1/builds");
    assert_eq!(recorded.header("content-type"), Some("application/json"));
    let parsed = BuildRequest::from_json_bytes(&recorded.body).expect("a protocol request");
    assert_eq!(parsed.request_id, "req-1");
    assert_eq!(parsed.calibration_id.as_deref(), Some("cal-1"));
}

#[test]
fn a_lost_submission_is_reported_as_ambiguous() {
    let bridge = MockBridge::start();
    let url = bridge.url();
    drop(bridge);
    let client = BridgeClient::new(&url, TOKEN, 2.0, 2.0).expect("a loopback client");

    let error = client.submit(&request()).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Network);
    assert!(
        error.message().contains("may or may not"),
        "{}",
        error.message()
    );
    assert!(
        error.message().contains("request id"),
        "{}",
        error.message()
    );
}

#[test]
fn a_slow_bridge_is_a_network_error_within_the_deadline() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::Delayed {
        status: 200,
        body: health_body().into_bytes(),
        delay: Duration::from_millis(600),
    });
    let client = BridgeClient::new(&bridge.url(), TOKEN, 0.2, 5.0).expect("a loopback client");

    let error = client.health().expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Network);
    assert!(error.message().contains("timed out"), "{}", error.message());
}

#[test]
fn status_and_cancel_use_the_build_id() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(200, &status_body("running")));
    bridge.queue(CannedResponse::json(200, &status_body("cancelled")));
    let client = client(&bridge);

    assert_eq!(
        client.status("build-1").expect("status").state,
        JobState::Running
    );
    assert_eq!(
        client.cancel("build-1").expect("cancel").state,
        JobState::Cancelled
    );
    let requests = bridge.requests();
    assert_eq!(requests[0].path, "/v1/builds/build-1");
    assert_eq!(requests[1].path, "/v1/builds/build-1/cancel");
    assert_eq!(requests[1].method, "POST");
}

#[test]
fn a_build_id_with_a_separator_is_refused_locally() {
    let bridge = MockBridge::start();
    let client = client(&bridge);

    let error = client.status("../etc/passwd").expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Client);
    assert!(
        error.message().contains("safe path segment"),
        "{}",
        error.message()
    );
    assert!(bridge.requests().is_empty());
}

#[test]
fn a_busy_bridge_is_mapped_from_the_error_envelope() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(
        409,
        r#"{"code":"busy","message":"another build owns the worker"}"#,
    ));
    let client = client(&bridge);

    let error = client.submit(&request()).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Client);
    assert!(
        error.message().contains("busy or conflicting"),
        "{}",
        error.message()
    );
    assert!(
        error.message().contains("another build owns the worker"),
        "{}",
        error.message()
    );
}

#[test]
fn an_unready_bridge_is_mapped_from_the_error_envelope() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(
        503,
        r#"{"code":"unavailable","message":"no Solid Edge in this session"}"#,
    ));
    let client = client(&bridge);

    let error = client.submit(&request()).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Client);
    assert!(error.message().contains("not ready"), "{}", error.message());
}

#[test]
fn an_uncalibrated_host_is_mapped_from_the_error_envelope() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(
        400,
        r#"{"code":"calibration_required","message":"the host is not calibrated"}"#,
    ));
    let client = client(&bridge);

    let error = client.submit(&request()).expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Client);
    assert!(
        error.message().contains("not calibrated"),
        "{}",
        error.message()
    );
}

#[test]
fn an_unknown_build_is_mapped_from_the_error_envelope() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(
        404,
        r#"{"code":"invalid_request","message":"no such build"}"#,
    ));
    let client = client(&bridge);

    let error = client.status("build-9").expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Client);
    assert!(
        error.message().contains("no such build"),
        "{}",
        error.message()
    );
}

#[test]
fn an_untyped_error_document_is_still_a_typed_refusal() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(500, "not json at all"));
    let client = client(&bridge);

    let error = client.health().expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Client);
    assert!(
        error.message().contains("untyped error"),
        "{}",
        error.message()
    );
}

#[test]
fn download_verifies_and_publishes_atomically() {
    let bridge = MockBridge::start();
    let data = b"ISO-10303-21;\nHEADER;\n";
    bridge.queue(CannedResponse::json(200, "ISO-10303-21;\nHEADER;\n"));
    let client = client(&bridge);
    let directory = tempfile::tempdir().expect("a temporary directory");

    let download = client
        .download("build-1", &artifact(data, "plate.step"), directory.path())
        .expect("a verified artifact");

    assert_eq!(std::fs::read(&download.path).expect("read"), data);
    assert_eq!(download.path.parent(), Some(directory.path()));
    let names: Vec<String> = std::fs::read_dir(directory.path())
        .expect("list")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, vec!["plate.step".to_owned()]);
    assert_eq!(
        bridge.requests()[0].path,
        "/v1/builds/build-1/artifacts/plate.step"
    );
}

#[test]
fn a_hash_mismatch_discards_the_file() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(200, "evil bytes"));
    let client = client(&bridge);
    let directory = tempfile::tempdir().expect("a temporary directory");

    let error = client
        .download(
            "build-1",
            &artifact(b"good bytes", "plate.step"),
            directory.path(),
        )
        .expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Artifact);
    assert!(error.message().contains("digest"), "{}", error.message());
    assert_eq!(
        std::fs::read_dir(directory.path()).expect("list").count(),
        0
    );
}

#[test]
fn a_size_mismatch_discards_the_file() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(200, "12345"));
    let client = client(&bridge);
    let directory = tempfile::tempdir().expect("a temporary directory");

    let error = client
        .download(
            "build-1",
            &artifact(b"1234", "plate.step"),
            directory.path(),
        )
        .expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Artifact);
    assert!(error.message().contains("bytes"), "{}", error.message());
    assert_eq!(
        std::fs::read_dir(directory.path()).expect("list").count(),
        0
    );
}

#[test]
fn an_unsafe_artifact_name_is_refused_locally() {
    let bridge = MockBridge::start();
    let client = client(&bridge);
    let directory = tempfile::tempdir().expect("a temporary directory");
    let mut declared = artifact(b"data", "plate.step");
    declared.name = "../escape.step".to_owned();

    let error = client
        .download("build-1", &declared, directory.path())
        .expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Artifact);
    assert!(bridge.requests().is_empty());
}

#[test]
fn an_oversize_declared_artifact_is_refused_before_it_is_written() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(200, "small").with_headers(vec![(
        "Content-Length".to_owned(),
        (512_u64 * 1024 * 1024).to_string(),
    )]));
    let client = client(&bridge);
    let directory = tempfile::tempdir().expect("a temporary directory");

    let error = client
        .download(
            "build-1",
            &artifact(b"small", "plate.step"),
            directory.path(),
        )
        .expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Artifact);
    assert!(
        error.message().contains("larger than the 256 MiB"),
        "{}",
        error.message()
    );
    assert_eq!(
        std::fs::read_dir(directory.path()).expect("list").count(),
        0
    );
}

#[test]
fn an_artifact_download_that_times_out_is_discarded() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::Delayed {
        status: 200,
        body: b"abcdefgh".to_vec(),
        delay: Duration::from_millis(600),
    });
    let client = BridgeClient::new(&bridge.url(), TOKEN, 5.0, 0.2).expect("a loopback client");
    let directory = tempfile::tempdir().expect("a temporary directory");

    let error = client
        .download(
            "build-1",
            &artifact(b"abcdefgh", "plate.step"),
            directory.path(),
        )
        .expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Network);
    assert!(
        error.message().contains("overall timeout"),
        "{}",
        error.message()
    );
    assert_eq!(
        std::fs::read_dir(directory.path()).expect("list").count(),
        0
    );
}

#[test]
fn wait_returns_a_terminal_status_without_unbounded_polling() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(200, &status_body("queued")));
    bridge.queue(CannedResponse::json(200, &status_body("succeeded")));
    let client = client(&bridge);

    let status = client
        .wait_with("build-1", 5.0, 0.0, &mut || 0.0, &mut |_| {})
        .expect("a terminal status");

    assert_eq!(status.state, JobState::Succeeded);
    assert_eq!(bridge.requests().len(), 2);
}

#[test]
fn wait_times_out_with_a_message_that_keeps_the_build_id() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(200, &status_body("running")));
    bridge.queue(CannedResponse::json(200, &status_body("running")));
    let client = client(&bridge);
    let mut ticks = vec![0.0, 0.0, 5.0];
    let mut clock = || {
        if ticks.is_empty() {
            return 5.0;
        }
        ticks.remove(0)
    };

    let error = client
        .wait_with("build-1", 1.0, 0.0, &mut clock, &mut |_| {})
        .expect_err("refused");

    assert_eq!(error.kind(), ErrorKind::Network);
    assert!(
        error.message().contains("keep the build id"),
        "{}",
        error.message()
    );
}

#[test]
fn a_compiled_request_is_the_one_the_bridge_receives() {
    let bridge = MockBridge::start();
    bridge.queue(CannedResponse::json(
        202,
        r#"{"build_id":"build-1","state":"queued"}"#,
    ));
    let client = client(&bridge);
    let script = kala_contracts::load_construction_json_str(
        r#"{"summary":"plate","components":[{"name":"plate","steps":[
            {"op":"sketch","profile":"rectangle","width":20,"height":20},
            {"op":"extrude","length":10}]}]}"#,
    )
    .expect("a valid script");
    let request = build_request(&script, "req-2", Some("cal-1"), 120.0, None).expect("a request");

    client.submit(&request).expect("accepted");

    let recorded = &bridge.requests()[0];
    let parsed = BuildRequest::from_json_bytes(&recorded.body).expect("a protocol request");
    assert_eq!(parsed, request);
    assert_eq!(
        parsed.request_digest().expect("a digest"),
        request.request_digest().expect("a digest")
    );
}
