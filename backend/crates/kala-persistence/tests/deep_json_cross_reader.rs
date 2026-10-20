//! Real deep-JSON cross-reader tests.
//!
//! Rust and the live Python product each open, fully verify and extend the
//! other's databases with payloads and snapshots nested far beyond the old
//! 1,024-level native ceiling. Both sides compare the canonical bytes, payload
//! digests and chain heads, including databases created from the accepted
//! schema-v2 fixture copy.

mod support;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use kala_persistence::{
    AppendOptions, Database, EventJournal, SnapshotStore, canonicalize_json_object, sha256_text,
};
use serde_json::Value;
use support::{copy_fixture_database, fixed_time, python_executable, repository_root};

const LEAF: &str = r#"{"big":1180591620717411303424,"f":-0.0,"n":null,"u":"\u03ba \ud83d\ude00"}"#;

fn deep_oracle_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/deep_json_oracle.py")
}

fn run_deep_oracle(args: &[&str], stdin: &[u8]) -> Vec<u8> {
    let mut child = Command::new(python_executable())
        .arg(deep_oracle_script())
        .args(args)
        .env("PYTHONPATH", repository_root().join("src"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn deep JSON oracle");
    child
        .stdin
        .as_mut()
        .expect("oracle stdin")
        .write_all(stdin)
        .expect("write oracle input");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for oracle");
    assert!(
        output.status.success(),
        "deep JSON oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn run_deep_oracle_json(args: &[&str]) -> Value {
    serde_json::from_slice(&run_deep_oracle(args, b"")).expect("oracle JSON")
}

fn deep_source(depth: usize, tag: &str) -> String {
    let tag = serde_json::to_string(tag).expect("serialize tag");
    let mut source = format!(r#"{{"tag":{tag},"x":"#);
    let mut closers: Vec<char> = Vec::with_capacity(depth);
    for level in 0..depth {
        if level % 2 == 0 {
            source.push('[');
            closers.push(']');
        } else {
            source.push_str(r#"{"k":"#);
            closers.push('}');
        }
    }
    source.push_str(LEAF);
    for closer in closers.into_iter().rev() {
        source.push(closer);
    }
    source.push('}');
    source
}

fn canonical_deep(depth: usize, tag: &str) -> String {
    let source = deep_source(depth, tag);
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || canonicalize_json_object(&source).expect("canonicalize deep source"))
        .expect("spawn 2 MiB thread")
        .join()
        .expect("deep canonicalization")
}

#[test]
fn rust_created_deep_database_round_trips_through_python() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("rust-deep.sqlite3");
    let python_path = path.to_str().expect("path");

    let first_canonical = canonical_deep(5_000, "rust-first");
    let first_snapshot = canonical_deep(2_500, "rust-first-snapshot");
    let (rust_head, first_digest) = {
        let database = Database::open(&path).expect("open");
        let journal = EventJournal::new(&database).expect("journal");
        let event = journal
            .append_with(
                "domain.deep.rust",
                "deep-rust",
                &deep_source(5_000, "rust-first"),
                &AppendOptions::default()
                    .with_workflow_id("flow-deep")
                    .with_run_id("run-deep")
                    .with_occurred_at(fixed_time(0)),
            )
            .expect("append deep event");
        assert_eq!(event.payload_json, first_canonical);
        SnapshotStore::new(&database)
            .save(
                "snapshot-deep-rust-000001",
                "flow-deep",
                &deep_source(2_500, "rust-first-snapshot"),
                Some(fixed_time(1)),
            )
            .expect("save deep snapshot");
        (
            journal.verify_chain().expect("verify").head_event_digest,
            event.digest,
        )
    };
    let rust_head = rust_head.expect("head").to_hex();
    assert_eq!(first_digest, sha256_text(&first_canonical));

    // Python fully verifies the Rust database, including deep canonical
    // bytes and the snapshot digest.
    let inspected = run_deep_oracle_json(&["inspect-deep", python_path]);
    assert_eq!(inspected["verification"]["event_count"].as_i64(), Some(1));
    assert_eq!(
        inspected["verification"]["head_event_digest"].as_str(),
        Some(rust_head.as_str())
    );
    let events = inspected["events"].as_array().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]["payload_json"].as_str(),
        Some(first_canonical.as_str())
    );
    assert_eq!(
        events[0]["digest"].as_str(),
        Some(first_digest.to_hex().as_str())
    );
    assert_eq!(
        inspected["snapshot"]["state_json"].as_str(),
        Some(first_snapshot.as_str())
    );
    assert_eq!(
        inspected["snapshot"]["digest"].as_str(),
        Some(sha256_text(&first_snapshot).to_hex().as_str())
    );

    // Python appends a deeper event and snapshot to the Rust database.
    let appended = run_deep_oracle_json(&["append-deep", python_path, "10000", "python-on-rust"]);
    let python_payload = appended["event"]["payload_json"]
        .as_str()
        .expect("python payload")
        .to_owned();
    let python_head = appended["verification"]["head_event_digest"]
        .as_str()
        .expect("python head")
        .to_owned();
    let python_snapshot = appended["snapshot"]["state_json"]
        .as_str()
        .expect("python snapshot")
        .to_owned();
    assert_eq!(appended["verification"]["event_count"].as_i64(), Some(2));
    assert_eq!(
        appended["event"]["previous_event_digest"].as_str(),
        Some(rust_head.as_str())
    );
    assert_eq!(python_payload, canonical_deep(10_000, "python-on-rust"));

    // Rust reopens and verifies Python's deep bytes and head.
    let rust_third_canonical = canonical_deep(20_000, "rust-after-python");
    let rust_third_snapshot = canonical_deep(10_000, "rust-after-python-snapshot");
    let final_head = {
        let database = Database::open(&path).expect("reopen");
        let journal = EventJournal::new(&database).expect("reopened journal");
        let verification = journal.verify_chain().expect("verify");
        assert_eq!(verification.event_count, 2);
        assert_eq!(
            verification.head_event_digest.map(|digest| digest.to_hex()),
            Some(python_head)
        );
        let read = journal.read().expect("read");
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].payload_json, first_canonical);
        assert_eq!(read[1].payload_json, python_payload);
        assert_eq!(read[1].digest, sha256_text(&python_payload));
        assert_eq!(
            SnapshotStore::new(&database)
                .restore_latest("flow-deep")
                .expect("restore"),
            Some(python_snapshot)
        );

        let third = journal
            .append_with(
                "domain.deep.rust",
                "deep-rust",
                &deep_source(20_000, "rust-after-python"),
                &AppendOptions::default()
                    .with_workflow_id("flow-deep")
                    .with_run_id("run-deep")
                    .with_occurred_at(fixed_time(60)),
            )
            .expect("append after python");
        assert_eq!(third.payload_json, rust_third_canonical);
        SnapshotStore::new(&database)
            .save(
                "snapshot-deep-rust-000003",
                "flow-deep",
                &deep_source(10_000, "rust-after-python-snapshot"),
                Some(fixed_time(61)),
            )
            .expect("save after python");
        journal
            .verify_chain()
            .expect("verify after append")
            .head_event_digest
            .expect("head")
            .to_hex()
    };

    // Python verifies the Rust extension as the final cross-reader step.
    let final_inspection = run_deep_oracle_json(&["inspect-deep", python_path]);
    assert_eq!(
        final_inspection["verification"]["event_count"].as_i64(),
        Some(3)
    );
    assert_eq!(
        final_inspection["verification"]["head_event_digest"].as_str(),
        Some(final_head.as_str())
    );
    let final_events = final_inspection["events"].as_array().expect("events");
    assert_eq!(final_events.len(), 3);
    assert_eq!(
        final_events[2]["payload_json"].as_str(),
        Some(rust_third_canonical.as_str())
    );
    assert_eq!(
        final_inspection["snapshot"]["state_json"].as_str(),
        Some(rust_third_snapshot.as_str())
    );
}

#[test]
fn python_created_deep_database_round_trips_through_rust() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("python-deep.sqlite3");
    let python_path = path.to_str().expect("path");

    let created = run_deep_oracle_json(&["create-deep", python_path, "10000", "python-created"]);
    let python_payload = created["event"]["payload_json"]
        .as_str()
        .expect("payload")
        .to_owned();
    let python_snapshot = created["snapshot"]["state_json"]
        .as_str()
        .expect("snapshot")
        .to_owned();
    let python_head = created["verification"]["head_event_digest"]
        .as_str()
        .expect("head")
        .to_owned();
    assert_eq!(created["verification"]["event_count"].as_i64(), Some(1));
    assert_eq!(python_payload, canonical_deep(10_000, "python-created"));

    let rust_second_canonical = canonical_deep(20_000, "rust-on-python");
    let rust_second_snapshot = canonical_deep(10_000, "rust-on-python-snapshot");
    let rust_head = {
        let database = Database::open(&path).expect("open python database");
        let journal = EventJournal::new(&database).expect("python journal");
        let verification = journal.verify_chain().expect("verify python chain");
        assert_eq!(verification.event_count, 1);
        assert_eq!(
            verification.head_event_digest.map(|digest| digest.to_hex()),
            Some(python_head.clone())
        );
        let first = journal.get(1).expect("get").expect("first event");
        assert_eq!(first.payload_json, python_payload);
        assert_eq!(first.digest, sha256_text(&python_payload));
        let first_value = first.payload().expect("parse python payload");
        let first_value_canonical = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                kala_persistence::canonicalize_object_value(&first_value)
                    .expect("re-encode python payload")
            })
            .expect("spawn")
            .join()
            .expect("deep value");
        assert_eq!(first_value_canonical, python_payload);

        let snapshot = SnapshotStore::new(&database)
            .get("snapshot-deep-python-000001")
            .expect("get snapshot")
            .expect("snapshot exists");
        assert_eq!(snapshot.state_json, python_snapshot);
        assert_eq!(snapshot.digest, sha256_text(&python_snapshot));
        assert_eq!(
            SnapshotStore::new(&database)
                .latest("flow-deep")
                .expect("latest")
                .expect("latest exists")
                .state_json,
            python_snapshot
        );

        let second = journal
            .append_with(
                "domain.deep.rust",
                "deep-rust",
                &deep_source(20_000, "rust-on-python"),
                &AppendOptions::default()
                    .with_workflow_id("flow-deep")
                    .with_run_id("run-deep")
                    .with_occurred_at(fixed_time(30)),
            )
            .expect("append on python database");
        assert_eq!(second.payload_json, rust_second_canonical);
        assert_eq!(second.previous_event_digest.to_hex(), python_head);
        SnapshotStore::new(&database)
            .save(
                "snapshot-deep-rust-000002",
                "flow-deep",
                &deep_source(10_000, "rust-on-python-snapshot"),
                Some(fixed_time(31)),
            )
            .expect("save on python database");
        journal
            .verify_chain()
            .expect("verify extended")
            .head_event_digest
            .expect("head")
            .to_hex()
    };

    // Python reopens the Rust-extended database and verifies every deep byte
    // and the new head.
    let inspected = run_deep_oracle_json(&["inspect-deep", python_path]);
    assert_eq!(inspected["verification"]["event_count"].as_i64(), Some(2));
    assert_eq!(
        inspected["verification"]["head_event_digest"].as_str(),
        Some(rust_head.as_str())
    );
    let events = inspected["events"].as_array().expect("events");
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0]["payload_json"].as_str(),
        Some(python_payload.as_str())
    );
    assert_eq!(
        events[1]["payload_json"].as_str(),
        Some(rust_second_canonical.as_str())
    );
    assert_eq!(
        inspected["snapshot"]["state_json"].as_str(),
        Some(rust_second_snapshot.as_str())
    );
}

#[test]
fn rust_deep_extension_of_the_fixture_copy_is_verified_by_python() {
    let copy = copy_fixture_database();
    let path = copy.path.to_str().expect("path").to_owned();
    let canonical = canonical_deep(20_000, "fixture-cross");
    let snapshot_canonical = canonical_deep(5_000, "fixture-cross-snapshot");
    let head = {
        let database = Database::open(&copy.path).expect("open fixture copy");
        let journal = EventJournal::new(&database).expect("journal");
        assert_eq!(journal.verify_chain().expect("verify").event_count, 5);
        let event = journal
            .append_with(
                "domain.deep.fixture",
                "deep-fixture",
                &deep_source(20_000, "fixture-cross"),
                &AppendOptions::default()
                    .with_workflow_id("flow-deep")
                    .with_occurred_at(fixed_time(80)),
            )
            .expect("append deep fixture event");
        assert_eq!(event.sequence, 6);
        assert_eq!(event.payload_json, canonical);
        SnapshotStore::new(&database)
            .save(
                "snapshot-deep-fixture-000001",
                "flow-deep",
                &deep_source(5_000, "fixture-cross-snapshot"),
                Some(fixed_time(81)),
            )
            .expect("snapshot");
        journal
            .verify_chain()
            .expect("verify extended")
            .head_event_digest
            .expect("head")
            .to_hex()
    };

    let inspected = run_deep_oracle_json(&["inspect-deep", &path]);
    assert_eq!(inspected["verification"]["event_count"].as_i64(), Some(6));
    assert_eq!(
        inspected["verification"]["head_event_digest"].as_str(),
        Some(head.as_str())
    );
    let events = inspected["events"].as_array().expect("events");
    assert_eq!(events.len(), 6);
    assert_eq!(events[5]["payload_json"].as_str(), Some(canonical.as_str()));
    assert_eq!(
        inspected["snapshot"]["state_json"].as_str(),
        Some(snapshot_canonical.as_str())
    );
}
