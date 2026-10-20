//! The journal is readable by the Python client and by the Rust client.
//!
//! The records are the same bytes in both directions, so a submission that
//! one client started can be inspected, resumed, or reconciled by the other.

use std::path::PathBuf;
use std::process::Command;

use kala_json::{CanonicalValue, JsonText, parse_strict};
use kala_solidedge_client::{Journal, JournalEntry};

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

fn run_oracle(mode: &str, path: &std::path::Path) -> Vec<u8> {
    let root = repository_root();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/journal_oracle.py");
    let search = std::env::join_paths([
        root.join("adapters/solidedge/src"),
        root.join("adapters/solidedge-bridge/src"),
        root.join("src"),
    ])
    .expect("a search path");
    let output = Command::new(python_executable())
        .arg("-I")
        .arg(&script)
        .arg(mode)
        .arg(path)
        .arg(&root)
        .env("PYTHONPATH", search)
        .output()
        .expect("run the Python journal oracle");
    assert!(
        output.status.success(),
        "the Python journal oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn full_entry() -> JournalEntry {
    JournalEntry {
        script_digest: "a".repeat(64),
        request_id: "req-1".to_owned(),
        build_id: Some("build-1".to_owned()),
        status: "pending".to_owned(),
        summary: "a plate".to_owned(),
        request_digest: Some("b".repeat(64)),
        bridge_target: Some("http://127.0.0.1:8797|cal-1".to_owned()),
        script: Some("{\"summary\":\"plate\"}".to_owned()),
    }
}

fn minimal_entry() -> JournalEntry {
    JournalEntry {
        script_digest: "c".repeat(64),
        request_id: "req-2".to_owned(),
        build_id: None,
        status: "succeeded".to_owned(),
        summary: String::new(),
        request_digest: None,
        bridge_target: None,
        script: None,
    }
}

#[test]
fn the_rust_client_reads_the_python_records() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("journal.jsonl");
    run_oracle("write", &path);

    let entries = Journal::new(&path).read();

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].request_id, "req-1");
    assert_eq!(entries[0].build_id.as_deref(), Some("build-2"));
    assert_eq!(entries[0].status, "succeeded");
    assert_eq!(entries[0].summary, "a plate");
    assert_eq!(
        entries[0].request_digest.as_deref(),
        Some("b".repeat(64).as_str())
    );
    assert_eq!(
        entries[0].bridge_target.as_deref(),
        Some("http://127.0.0.1:8797|cal-1")
    );
    assert_eq!(
        entries[0].script.as_deref(),
        Some("{\"summary\":\"plate\"}")
    );
    assert_eq!(entries[1].request_id, "req-2");
    assert_eq!(entries[1].build_id, None);
    assert!(entries[1].request_digest.is_none());
}

#[test]
fn the_record_bytes_are_the_same_in_both_clients() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let python_path = directory.path().join("python.jsonl");
    let rust_path = directory.path().join("rust.jsonl");
    run_oracle("write", &python_path);

    let journal = Journal::new(&rust_path);
    journal.append(&full_entry()).expect("append");
    journal.append(&minimal_entry()).expect("append");
    journal
        .update("req-1", Some("build-2"), "succeeded")
        .expect("update");

    let python_bytes = std::fs::read(&python_path).expect("read the Python journal");
    let rust_bytes = std::fs::read(&rust_path).expect("read the Rust journal");
    assert_eq!(
        String::from_utf8_lossy(&rust_bytes),
        String::from_utf8_lossy(&python_bytes),
        "the record bytes differ"
    );
}

#[test]
fn the_python_client_reads_the_rust_records() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("journal.jsonl");
    let journal = Journal::new(&path);
    journal.append(&full_entry()).expect("append");
    journal.append(&minimal_entry()).expect("append");
    journal
        .update("req-1", Some("build-2"), "succeeded")
        .expect("update");

    let output = run_oracle("read", &path);
    let value = parse_strict(&output, &kala_contracts::model_json_limits())
        .expect("the oracle prints JSON");
    let CanonicalValue::Array(entries) = &value else {
        panic!("the oracle printed no entries");
    };
    assert_eq!(entries.len(), 2);

    let CanonicalValue::Object(first) = &entries[0] else {
        panic!("an entry is not an object");
    };
    let text = |name: &str| -> String {
        match first.get(&JsonText::new(name)) {
            Some(CanonicalValue::Text(value)) => value.to_utf8().expect("utf-8"),
            _ => panic!("{name} is not text"),
        }
    };
    assert_eq!(text("request_id"), "req-1");
    assert_eq!(text("build_id"), "build-2");
    assert_eq!(text("status"), "succeeded");
    assert_eq!(text("summary"), "a plate");
    assert_eq!(text("request_digest"), "b".repeat(64));
    assert_eq!(text("bridge_target"), "http://127.0.0.1:8797|cal-1");
    assert_eq!(text("script"), "{\"summary\":\"plate\"}");

    let CanonicalValue::Object(second) = &entries[1] else {
        panic!("an entry is not an object");
    };
    assert!(matches!(
        second.get(&JsonText::new("build_id")),
        Some(CanonicalValue::Null)
    ));
    assert!(matches!(
        second.get(&JsonText::new("request_digest")),
        Some(CanonicalValue::Null)
    ));
}

#[test]
fn an_exact_replay_is_found_by_identity() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("journal.jsonl");
    let journal = Journal::new(&path);
    journal.append(&full_entry()).expect("append");

    let found = journal
        .find_exact(
            &"a".repeat(64),
            &"b".repeat(64),
            "http://127.0.0.1:8797|cal-1",
        )
        .expect("an exact entry");
    assert_eq!(found.request_id, "req-1");
    assert!(
        journal
            .find_exact(
                &"a".repeat(64),
                &"b".repeat(64),
                "http://127.0.0.1:8797|other"
            )
            .is_none()
    );
    assert!(
        journal
            .find_by_digest(&"a".repeat(64))
            .is_some_and(|entry| entry.request_id == "req-1")
    );
    assert!(journal.find_by_request("req-2").is_none());
}
