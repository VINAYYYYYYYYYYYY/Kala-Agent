//! Identifier whitespace normalization compatibility.
//!
//! The live product's `_normalize_identifier` uses Python `str.strip()`, whose
//! whitespace set includes `U+001C..=U+001F` in addition to Rust's
//! `char::is_whitespace` set. The journal hashes raw stored identifier bytes
//! (the accepted round-2 repair) but its typed view and row-shape validation
//! must strip exactly what Python strips. These tests compare against the live
//! `_validate_identifier` oracle and exercise correctly rehashed raw rows in
//! both Rust -> Python and Python -> Rust directions.

mod support;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use kala_json::{EventChainFields, Sha256Digest, chain_digest};
use kala_persistence::{AppendOptions, Database, EventJournal, PersistenceError, timestamp_to_iso};
use rusqlite::params;
use serde_json::Value;
use support::{fixed_time, python_executable, repository_root};

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
        .expect("spawn identifier oracle");
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
        "identifier oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn identifier_cases() -> Vec<&'static str> {
    vec![
        "\u{1c}foo\u{1c}",
        "\u{1d}foo\u{1d}",
        "\u{1e}foo\u{1e}",
        "\u{1f}foo\u{1f}",
        "\u{1c}foo\u{1f}",
        "\u{1c} foo \u{1f}",
        "\u{1c}foo\u{1f} bar",
        "\u{1c}\u{1f}",
        "\u{85}foo\u{3000}",
        "\u{2000}foo\u{202f}",
        "\u{a0}foo\u{a0}",
        " foo ",
        "\u{200b}foo",
        "\u{1c}foo-bar.baz:qux_1\u{1f}",
        "foo",
    ]
}

#[test]
fn append_normalization_matches_live_python_for_every_field() {
    let cases = identifier_cases();
    let request = serde_json::to_vec(&cases).expect("serialize identifier cases");
    let results: Value = serde_json::from_slice(&run_deep_oracle(&["identifiers"], &request))
        .expect("identifier oracle JSON");
    let results = results.as_array().expect("result array");
    assert_eq!(results.len(), cases.len());

    for (case, result) in cases.iter().zip(results) {
        let python_ok = result["ok"].as_bool().expect("ok flag");
        let expected = result["output"].as_str();
        let database = Database::in_memory().expect("open");
        let journal = EventJournal::new(&database).expect("journal");

        let event = journal.append_with(
            case,
            "aggregate",
            "{}",
            &AppendOptions::default().with_occurred_at(fixed_time(0)),
        );
        if python_ok {
            let event = event.unwrap_or_else(|error| {
                panic!("Rust rejected Python-accepted event_type {case:?}: {error:?}")
            });
            assert_eq!(
                event.event_type,
                expected.expect("python normalized value"),
                "event_type normalization for {case:?}"
            );

            let event = journal
                .append_with(
                    "event.type",
                    case,
                    "{}",
                    &AppendOptions::default()
                        .with_workflow_id(*case)
                        .with_run_id(*case)
                        .with_occurred_at(fixed_time(1)),
                )
                .unwrap_or_else(|error| {
                    panic!("Rust rejected Python-accepted identifier fields {case:?}: {error:?}")
                });
            assert_eq!(
                event.aggregate_id,
                expected.expect("python normalized value"),
                "aggregate_id normalization for {case:?}"
            );
            assert_eq!(
                event.workflow_id.as_deref(),
                expected,
                "workflow_id normalization for {case:?}"
            );
            assert_eq!(
                event.run_id.as_deref(),
                expected,
                "run_id normalization for {case:?}"
            );
        } else {
            match event {
                Err(PersistenceError::InvalidIdentifier {
                    field: "event_type",
                }) => {}
                other => panic!("expected event_type refusal for {case:?}, got {other:?}"),
            }
            match journal.append_with(
                "event.type",
                case,
                "{}",
                &AppendOptions::default().with_occurred_at(fixed_time(2)),
            ) {
                Err(PersistenceError::InvalidIdentifier {
                    field: "aggregate_id",
                }) => {}
                other => panic!("expected aggregate_id refusal for {case:?}, got {other:?}"),
            }
        }
    }
}

/// Rewrite one event's identifiers with raw `U+001C` padding and rehash the
/// chain over those raw bytes, exactly as a legitimate Python writer that did
/// not normalize would have committed them.
fn raw_padded_database(database: &Database) {
    let padded = |value: &str| format!("\u{1c}{value}\u{1c}");
    database
        .serialized(|connection| {
            connection.execute_batch(
                "DROP TRIGGER events_prevent_update; DROP TRIGGER events_prevent_delete",
            )?;
            connection.execute(
                "UPDATE events SET event_type = ?1, aggregate_id = ?2, \
                 workflow_id = ?3, run_id = ?4 WHERE sequence = 1",
                params![
                    padded("step.0"),
                    padded("step-0"),
                    padded("flow-deep"),
                    padded("run-deep"),
                ],
            )?;
            Ok(())
        })
        .expect("pad raw identifiers");

    let fields = database
        .serialized(|connection| {
            let (event_type, aggregate_id, workflow_id, run_id, digest, occurred_at, previous) =
                connection.query_row(
                    "SELECT event_type, aggregate_id, workflow_id, run_id, digest, \
                     occurred_at, previous_event_digest FROM events WHERE sequence = 1",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                        ))
                    },
                )?;
            Ok(EventChainFields {
                sequence: 1,
                event_type,
                aggregate_id,
                workflow_id,
                run_id,
                occurred_at,
                payload_digest: Sha256Digest::from_hex(&digest).expect("payload digest"),
                previous_event_digest: Sha256Digest::from_hex(&previous).expect("previous"),
            })
        })
        .expect("read raw fields");
    let digest = chain_digest(&fields);
    database
        .serialized(|connection| {
            connection.execute(
                "UPDATE events SET event_digest = ?1 WHERE sequence = 1",
                params![digest.to_hex()],
            )?;
            Ok(())
        })
        .expect("rechain raw identifiers");
}

#[test]
fn rust_raw_padded_row_is_accepted_by_rust_and_python() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("rust-padded.sqlite3");
    let head = {
        let database = Database::open(&path).expect("open");
        let journal = EventJournal::new(&database).expect("journal");
        journal
            .append_with(
                "step.0",
                "step-0",
                "{}",
                &AppendOptions::default()
                    .with_workflow_id("flow-deep")
                    .with_run_id("run-deep")
                    .with_occurred_at(fixed_time(0)),
            )
            .expect("append");
        raw_padded_database(&database);

        // The raw bytes are what the chain commits to; the typed view trims
        // Python's wider whitespace set without rehashing.
        let verification = journal.verify_chain().expect("verify padded chain");
        assert_eq!(verification.event_count, 1);
        let event = journal.get(1).expect("get").expect("event");
        assert_eq!(event.event_type, "step.0");
        assert_eq!(event.aggregate_id, "step-0");
        assert_eq!(event.workflow_id.as_deref(), Some("flow-deep"));
        assert_eq!(event.run_id.as_deref(), Some("run-deep"));
        let read = journal.read().expect("read");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0], event);

        // A new append extends the raw-padded head rather than rejecting it.
        let appended = journal
            .append_with(
                "step.1",
                "step-1",
                "{}",
                &AppendOptions::default()
                    .with_workflow_id("flow-deep")
                    .with_occurred_at(fixed_time(1)),
            )
            .expect("append after padded head");
        assert_eq!(appended.sequence, 2);
        assert_eq!(
            appended.previous_event_digest,
            verification.head_event_digest.unwrap()
        );
        journal
            .verify_chain()
            .expect("verify extended")
            .head_event_digest
            .expect("head")
            .to_hex()
    };

    let inspected: Value = serde_json::from_slice(&run_deep_oracle(
        &["inspect-deep", path.to_str().expect("path")],
        b"",
    ))
    .expect("inspect JSON");
    assert_eq!(inspected["verification"]["event_count"].as_i64(), Some(2));
    assert_eq!(
        inspected["verification"]["head_event_digest"].as_str(),
        Some(head.as_str())
    );
    let events = inspected["events"].as_array().expect("events");
    assert_eq!(events[0]["event_type"].as_str(), Some("step.0"));
    assert_eq!(events[0]["aggregate_id"].as_str(), Some("step-0"));
    assert_eq!(events[0]["workflow_id"].as_str(), Some("flow-deep"));
    assert_eq!(events[0]["run_id"].as_str(), Some("run-deep"));
    assert_eq!(events[1]["event_type"].as_str(), Some("step.1"));
}

#[test]
fn python_raw_padded_row_is_accepted_by_rust_and_python() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("python-padded.sqlite3");
    let created: Value = serde_json::from_slice(&run_deep_oracle(
        &["padded-row", path.to_str().expect("path")],
        b"",
    ))
    .expect("padded row JSON");
    assert_eq!(created["events"][0]["event_type"].as_str(), Some("step.0"));
    let python_head = created["verification"]["head_event_digest"]
        .as_str()
        .expect("head")
        .to_owned();

    let (rust_head, previous) = {
        let database = Database::open(&path).expect("open python padded database");
        let journal = EventJournal::new(&database).expect("journal");
        let verification = journal.verify_chain().expect("verify python padded row");
        assert_eq!(verification.event_count, 1);
        assert_eq!(
            verification.head_event_digest.map(|digest| digest.to_hex()),
            Some(python_head.clone())
        );
        let event = journal.get(1).expect("get").expect("event");
        assert_eq!(event.event_type, "step.0");
        assert_eq!(event.aggregate_id, "step-0");
        assert_eq!(event.workflow_id.as_deref(), Some("flow-deep"));
        assert_eq!(event.run_id.as_deref(), Some("run-deep"));
        assert_eq!(
            timestamp_to_iso(&event.timestamp).expect("timestamp"),
            "2026-01-01T00:00:00.000000Z"
        );

        let appended = journal
            .append_with(
                "step.1",
                "step-1",
                "{}",
                &AppendOptions::default()
                    .with_workflow_id("flow-deep")
                    .with_occurred_at(fixed_time(1)),
            )
            .expect("append after python padded row");
        assert_eq!(appended.previous_event_digest.to_hex(), python_head);
        (
            journal
                .verify_chain()
                .expect("verify extended")
                .head_event_digest
                .expect("head")
                .to_hex(),
            appended.previous_event_digest.to_hex(),
        )
    };
    assert_eq!(previous, python_head);

    let inspected: Value = serde_json::from_slice(&run_deep_oracle(
        &["inspect-deep", path.to_str().expect("path")],
        b"",
    ))
    .expect("inspect JSON");
    assert_eq!(inspected["verification"]["event_count"].as_i64(), Some(2));
    assert_eq!(
        inspected["verification"]["head_event_digest"].as_str(),
        Some(rust_head.as_str())
    );
    assert_eq!(
        inspected["events"][0]["event_type"].as_str(),
        Some("step.0")
    );
    assert_eq!(
        inspected["events"][1]["event_type"].as_str(),
        Some("step.1")
    );
}
