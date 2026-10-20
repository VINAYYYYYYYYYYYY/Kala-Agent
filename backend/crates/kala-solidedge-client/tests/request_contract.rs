//! The build request bytes, the canonical digest, and the wire body agree
//! with the Python bridge contract.
//!
//! The request digest is what the journal uses to recognise one semantic
//! submission, so a byte difference here would let one client resubmit a
//! build the other client already started.

use std::path::PathBuf;
use std::process::Command;

use kala_contracts::load_construction_json_str;
use kala_json::{CanonicalValue, JsonText, parse_strict};
use kala_solidedge_client::{BridgeOp, BuildRequest, Plane, build_request};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn python_executable() -> PathBuf {
    if let Ok(path) = std::env::var("KALA_PYTHON") {
        return PathBuf::from(path);
    }
    let venv = repository_root().join(".venv/bin/python");
    if venv.is_file() {
        return venv;
    }
    PathBuf::from("python3")
}

fn oracle(path: &std::path::Path) -> CanonicalValue {
    let root = repository_root();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/request_oracle.py");
    let search = std::env::join_paths([
        root.join("adapters/solidedge/src"),
        root.join("adapters/solidedge-bridge/src"),
        root.join("src"),
    ])
    .expect("a search path");
    let output = Command::new(python_executable())
        .arg("-I")
        .arg(&script)
        .arg(path)
        .arg(&root)
        .env("PYTHONPATH", search)
        .output()
        .expect("run the Python request oracle");
    assert!(
        output.status.success(),
        "the Python request oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_strict(&output.stdout, &kala_contracts::model_json_limits())
        .expect("the oracle prints JSON")
}

fn text(value: &CanonicalValue, name: &str) -> String {
    let CanonicalValue::Object(object) = value else {
        panic!("the oracle printed no object");
    };
    match object.get(&JsonText::new(name)) {
        Some(CanonicalValue::Text(text)) => text.to_utf8().expect("utf-8"),
        _ => panic!("{name} is not text"),
    }
}

#[test]
fn the_canonical_bytes_and_the_digest_match_the_python_contract() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("request.json");

    let script = load_construction_json_str(
        r#"{"summary":"a plate","components":[{"name":"plate","steps":[
            {"op":"sketch","profile":"rectangle","width":20,"height":20,"x":2.5,"y":-3.5},
            {"op":"extrude","length":10}]}]}"#,
    )
    .expect("a valid script");
    let request = build_request(&script, "req-1", Some("cal-1"), 120.0, None).expect("a request");
    let body = request.to_wire_json().expect("a wire body");
    std::fs::write(&path, body.as_bytes()).expect("write the request");

    let oracle = oracle(&path);

    assert_eq!(
        request
            .canonical_request_bytes()
            .expect("canonical bytes")
            .iter()
            .map(|byte| *byte as char)
            .collect::<String>(),
        text(&oracle, "canonical"),
        "the canonical request bytes differ"
    );
    assert_eq!(
        request.request_digest().expect("a digest"),
        text(&oracle, "digest"),
        "the request digest differs"
    );
    assert_eq!(body, text(&oracle, "wire"), "the wire body differs");
}

#[test]
fn the_wire_body_is_accepted_by_the_python_contract() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("request.json");

    let request = BuildRequest::new(
        "req-2",
        None,
        "one part",
        vec![
            BridgeOp::PartNew {
                name: "plate".to_owned(),
            },
            BridgeOp::SketchRectangle {
                plane: Plane::XZ,
                width: 4.0,
                height: 5.0,
                x: 0.0,
                y: 0.0,
            },
            BridgeOp::SolidExtrude {
                length: 6.0,
                reversed: true,
            },
            BridgeOp::PartMeasure,
            BridgeOp::PartSave,
            BridgeOp::PartExportStep,
        ],
        300.0,
        Some(7800.0),
    )
    .expect("a valid request");
    let body = request.to_wire_json().expect("a wire body");
    std::fs::write(&path, body.as_bytes()).expect("write the request");

    let oracle = oracle(&path);

    assert_eq!(
        request.request_digest().expect("a digest"),
        text(&oracle, "digest"),
        "the request digest differs"
    );
    assert_eq!(body, text(&oracle, "wire"), "the wire body differs");
}
