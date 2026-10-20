//! The live Python differential for the connection layer.
//!
//! The oracle (`tests/oracle_http.py`) builds the **live** Python FastAPI
//! application of `kala_agent.api` -- the real boundary middleware stack and
//! the real error handlers -- replays a deterministic corpus through it, and
//! writes both the corpus and the answers to disk. This test replays the very
//! same corpus through the native connection layer and compares the status,
//! every response header, and the exact body bytes.
//!
//! Python is a test oracle only. The crate itself never starts an
//! interpreter.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::header::{HeaderName, HeaderValue};
use hyper::{Method, Request};
use kala_api::errors::validation_details;
use kala_api::errors::{self, ApplicationErrorKind, ValidationFailure};
use kala_api::probe::ProbeRouter;
use kala_api::server::{ApiState, handle_request};
use serde_json::Value;

const REPO: &str = "/home/v9i/Projects/Kala-Agent";

fn python() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("KALA_PYTHON") {
        let path = PathBuf::from(explicit);
        return path.exists().then_some(path);
    }
    let path = Path::new(REPO).join(".venv/bin/python");
    path.exists().then_some(path)
}

fn base64_decode(value: &str) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out: Vec<u8> = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for character in value.bytes() {
        if character == b'=' {
            break;
        }
        let Some(index) = ALPHABET
            .iter()
            .position(|candidate| *candidate == character)
        else {
            continue;
        };
        buffer = (buffer << 6) | u32::from(index as u8);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    out
}

fn header_pairs(headers: &hyper::HeaderMap) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();
    pairs.sort();
    pairs
}

#[tokio::test]
async fn the_native_layer_matches_the_live_python_api() {
    let Some(interpreter) = python() else {
        eprintln!(
            "SKIPPED: the repository Python interpreter was not found; the live differential \
             did not run. Set KALA_PYTHON to run it."
        );
        return;
    };
    let work = tempfile::tempdir().expect("a temporary directory");
    let corpus_path = work.path().join("corpus.json");
    let results_path = work.path().join("results.json");
    let oracle = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle_http.py");
    let output = Command::new(&interpreter)
        .arg("-I")
        .arg(&oracle)
        .arg(&corpus_path)
        .arg(&results_path)
        .output()
        .expect("the oracle runs");
    assert!(
        output.status.success(),
        "the oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let corpus_text = std::fs::read_to_string(&corpus_path).expect("the corpus is written");
    let corpus: Value = serde_json::from_str(&corpus_text).expect("the corpus is JSON");
    let results_text = std::fs::read_to_string(&results_path).expect("the results are written");
    let results: Value = serde_json::from_str(&results_text).expect("the results are JSON");

    let token = corpus["token"]
        .as_str()
        .expect("the corpus carries a token");
    let max_body_bytes = corpus["max_body_bytes"]
        .as_i64()
        .expect("the corpus carries a body limit");
    let state = ApiState::new(token, max_body_bytes, Arc::new(ProbeRouter)).expect("a valid state");
    let cases = corpus["cases"]
        .as_array()
        .expect("the corpus carries cases");
    let answers = results["http"]
        .as_array()
        .expect("the answers are an array");
    assert_eq!(cases.len(), answers.len(), "every case has an answer");

    let mut mismatches: Vec<String> = Vec::new();
    let mut compared = 0usize;
    for (item, expected) in cases.iter().zip(answers.iter()) {
        let name = item["name"].as_str().unwrap_or("<unnamed>");
        let method = item["method"].as_str().unwrap_or("GET");
        let path = item["path"].as_str().unwrap_or("/probe");
        let body = base64_decode(item["body"].as_str().unwrap_or(""));

        let mut request = Request::builder()
            .method(Method::from_bytes(method.as_bytes()).expect("a valid method"))
            .uri(path)
            .body(Full::new(Bytes::from(body)))
            .expect("a valid request");
        for header in item["headers"]
            .as_array()
            .expect("the headers are an array")
        {
            let name = header[0].as_str().expect("a header name");
            let value = header[1].as_str().expect("a header value");
            request.headers_mut().append(
                HeaderName::from_bytes(name.as_bytes()).expect("a valid header name"),
                HeaderValue::from_bytes(value.as_bytes()).expect("a valid header value"),
            );
        }

        let response = handle_request(&state, request).await;
        let status = response.status().as_u16();
        let headers = header_pairs(response.headers());
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("the body is readable")
            .to_bytes();

        let expected_status = expected["status"].as_u64().expect("a status") as u16;
        let mut expected_headers: Vec<(String, String)> = expected["headers"]
            .as_array()
            .expect("the headers are an array")
            .iter()
            .map(|pair| {
                (
                    pair[0].as_str().expect("a header name").to_owned(),
                    pair[1].as_str().expect("a header value").to_owned(),
                )
            })
            .collect();
        expected_headers.sort();
        let expected_body = base64_decode(expected["body"].as_str().unwrap_or(""));

        compared += 1;
        if status != expected_status
            || headers != expected_headers
            || bytes.as_ref() != expected_body.as_slice()
        {
            mismatches.push(format!(
                "{name}: python={expected_status} {expected_headers:?} {:?}\n\
                 native={status} {headers:?} {:?}",
                String::from_utf8_lossy(&expected_body),
                String::from_utf8_lossy(&bytes)
            ));
        }
    }
    eprintln!("differential cases: {compared}");

    // The validation details of the error handlers, formatted from the real
    // Pydantic failures the live application produced.
    let mut validation_compared = 0usize;
    for expected in results["validation"].as_array().expect("the answers") {
        let name = expected["name"].as_str().unwrap_or("<unnamed>");
        let failures: Vec<ValidationFailure> = expected["failures"]
            .as_array()
            .expect("the failures are an array")
            .iter()
            .map(|failure| ValidationFailure {
                location: failure["location"]
                    .as_array()
                    .expect("the location is an array")
                    .iter()
                    .map(|part| part.as_str().unwrap_or("").to_owned())
                    .collect(),
                message: failure["message"].as_str().map(str::to_owned),
                kind: failure["kind"].as_str().map(str::to_owned),
            })
            .collect();
        let live_details: Vec<String> = expected["details"]
            .as_array()
            .expect("the details are an array")
            .iter()
            .map(|detail| detail.as_str().unwrap_or("").to_owned())
            .collect();
        let native_details = validation_details(&failures);
        validation_compared += 1;
        if native_details != live_details {
            mismatches.push(format!(
                "{name}: python details={live_details:?} native details={native_details:?}"
            ));
            continue;
        }
        let body = errors::handle_request_validation(Some(&failures));
        let native_body = body.ordered_bytes();
        let live_body = base64_decode(expected["body"].as_str().unwrap_or(""));
        if native_body != live_body {
            mismatches.push(format!(
                "{name}: python body={:?} native body={:?}",
                String::from_utf8_lossy(&live_body),
                String::from_utf8_lossy(&native_body)
            ));
        }
    }
    eprintln!("validation differential cases: {validation_compared}");

    // The status every composition root failure answers with.
    let mut error_compared = 0usize;
    for expected in results["application_errors"]
        .as_array()
        .expect("the answers")
    {
        let name = expected["name"].as_str().unwrap_or("<unnamed>");
        let kind = kind_for(name).unwrap_or_else(|| panic!("the taxonomy knows {name}"));
        let status = kind.status();
        let code = kind.code();
        error_compared += 1;
        if status as u64 != expected["status"].as_u64().unwrap_or_default()
            || code != expected["code"].as_str().unwrap_or("")
        {
            mismatches.push(format!(
                "{name}: python=({}, {}) native=({status}, {code})",
                expected["status"], expected["code"]
            ));
        }
    }
    eprintln!("application error differential cases: {error_compared}");

    // The framework errors the error handlers answer.
    let mut exception_compared = 0usize;
    for expected in results["http_exceptions"].as_array().expect("the answers") {
        let name = expected["name"].as_str().unwrap_or("<unnamed>");
        let case = corpus["http_exception_cases"]
            .as_array()
            .expect("the cases")
            .iter()
            .find(|item| item["name"].as_str() == Some(name))
            .expect("every case has an entry");
        let status = case["status"].as_u64().unwrap_or_default() as u16;
        let detail = case["detail"].as_str().unwrap_or("");
        exception_compared += 1;
        if status == 0 {
            // An unexpected failure answers through the unexpected handler.
            let (answered, body) = errors::handle_unexpected_error();
            let native_body = body.ordered_bytes();
            let live_body = base64_decode(expected["body"].as_str().unwrap_or(""));
            if answered as u64 != expected["status"].as_u64().unwrap_or_default()
                || native_body != live_body
            {
                mismatches.push(format!(
                    "{name}: python=({}, {:?}) native=({answered}, {:?})",
                    expected["status"],
                    String::from_utf8_lossy(&live_body),
                    String::from_utf8_lossy(&native_body)
                ));
            }
            continue;
        }
        let (answered, body) = errors::handle_http_exception(status, detail);
        let native_body = body.ordered_bytes();
        let live_body = base64_decode(expected["body"].as_str().unwrap_or(""));
        if answered as u64 != expected["status"].as_u64().unwrap_or_default()
            || native_body != live_body
        {
            mismatches.push(format!(
                "{name}: python=({}, {:?}) native=({answered}, {:?})",
                expected["status"],
                String::from_utf8_lossy(&live_body),
                String::from_utf8_lossy(&native_body)
            ));
        }
    }
    eprintln!("http exception differential cases: {exception_compared}");

    assert!(
        mismatches.is_empty(),
        "{} mismatches out of {compared} cases:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

/// The taxonomy entry of a Python failure class.
fn kind_for(name: &str) -> Option<ApplicationErrorKind> {
    Some(match name {
        "UnknownDomainError" => ApplicationErrorKind::UnknownDomain,
        "UnknownRevisionError" => ApplicationErrorKind::UnknownRevision,
        "UnknownPreparedRevisionError" => ApplicationErrorKind::UnknownPreparedRevision,
        "UnknownWorkflowError" => ApplicationErrorKind::UnknownWorkflow,
        "ProvenanceUnavailableError" => ApplicationErrorKind::ProvenanceUnavailable,
        "CheckpointNotPendingError" => ApplicationErrorKind::CheckpointNotPending,
        "RevisionCommitRejectedError" => ApplicationErrorKind::RevisionCommitRejected,
        "RevisionForkRefusedError" => ApplicationErrorKind::RevisionForkRefused,
        "RevisionHeadMovedError" => ApplicationErrorKind::RevisionHeadMoved,
        "CheckpointDecisionDeniedError" => ApplicationErrorKind::CheckpointDecisionDenied,
        "CheckpointDigestMismatchError" => ApplicationErrorKind::CheckpointDigestMismatch,
        "WorkflowFinishedError" => ApplicationErrorKind::WorkflowFinished,
        "CorruptKernelStateError" => ApplicationErrorKind::CorruptKernelState,
        "ClosedApplicationError" => ApplicationErrorKind::ClosedApplication,
        "InvalidIdentifierError" => ApplicationErrorKind::InvalidIdentifier,
        "ReservedIdentifierError" => ApplicationErrorKind::ReservedIdentifier,
        "InvalidParametersError" => ApplicationErrorKind::InvalidParameters,
        _ => return None,
    })
}

#[tokio::test]
async fn the_native_layer_answers_the_boundary_cases_on_its_own() {
    let state = ApiState::new(
        "unit-test-launch-token-0123456789abcdef",
        4096,
        Arc::new(ProbeRouter),
    )
    .expect("a valid state");
    let request = |method: Method, host: Option<&str>, origin: Option<&str>| {
        let mut request = Request::builder()
            .method(method)
            .uri("/probe")
            .body(Full::new(Bytes::new()))
            .expect("a valid request");
        if let Some(host) = host {
            request
                .headers_mut()
                .insert("host", HeaderValue::from_str(host).expect("a host"));
        }
        if let Some(origin) = origin {
            request
                .headers_mut()
                .insert("origin", HeaderValue::from_str(origin).expect("an origin"));
        }
        request
    };
    let response = handle_request(&state, request(Method::GET, Some("127.0.0.1"), None)).await;
    assert_eq!(response.status(), 200);
    let response = handle_request(&state, request(Method::GET, Some("evil.example"), None)).await;
    assert_eq!(response.status(), 421);
    let response = handle_request(
        &state,
        request(Method::GET, Some("127.0.0.1"), Some("null")),
    )
    .await;
    assert_eq!(response.status(), 403);
    let response = handle_request(&state, request(Method::POST, Some("127.0.0.1"), None)).await;
    assert_eq!(response.status(), 401);
}
