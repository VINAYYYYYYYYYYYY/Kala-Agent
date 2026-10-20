//! Tamper evidence and concurrency for the native event journal.
//!
//! Written from the position of an attacker who can already write the database
//! file: the append-only triggers are dropped first, exactly as a file-level
//! attacker would, and the tampering must still be detected afterwards.

mod support;

use std::path::Path;

use kala_json::{EventChainFields, GENESIS_PREVIOUS_DIGEST, Sha256Digest, chain_digest};
use kala_persistence::{
    Database, EventFilter, EventJournal, JournalIntegrityError, PersistenceError, SnapshotStore,
};
use rusqlite::{params, params_from_iter, types::Value};
use support::{fixed_time, python_executable, repository_root, run_oracle};

fn seeded_database(database_path: &Path) -> Database {
    let database = Database::open(database_path).expect("open seeded database");
    let journal = EventJournal::new(&database).expect("seed journal");
    for index in 0..3_i64 {
        journal
            .append_with(
                &format!("step.{index}"),
                &format!("step-{index}"),
                &format!(r#"{{"index":{index}}}"#),
                &kala_persistence::AppendOptions::default()
                    .with_workflow_id("workflow-1")
                    .with_run_id("run-1")
                    .with_occurred_at(fixed_time(index)),
            )
            .expect("seed append");
    }
    database
}

fn tamper(database: &Database, statements: &[(&str, Vec<Value>)]) {
    database
        .serialized(|connection| {
            connection.execute_batch("DROP TRIGGER events_prevent_update")?;
            connection.execute_batch("DROP TRIGGER events_prevent_delete")?;
            Ok(())
        })
        .expect("drop triggers");
    for (statement, values) in statements {
        database
            .serialized(|connection| {
                connection.execute(statement, params_from_iter(values.iter()))?;
                Ok(())
            })
            .expect("tamper statement");
    }
}

fn expect_variant(actual: Result<(), PersistenceError>, expected: JournalIntegrityError) {
    match actual {
        Err(PersistenceError::JournalIntegrity(error)) => {
            assert_eq!(error, expected, "unexpected journal integrity variant");
        }
        other => panic!("expected {expected:?}, got {other:?}"),
    }
}

fn assert_every_read_fails(journal: &EventJournal<'_>, expected: &JournalIntegrityError) {
    expect_variant(journal.verify_chain().map(|_| ()), expected.clone());
    expect_variant(journal.read().map(|_| ()), expected.clone());
    expect_variant(journal.get(3).map(|_| ()), expected.clone());
    // A filter that matches nothing must still fail: verification always
    // covers the whole journal, not just the selected rows.
    expect_variant(
        journal
            .read_filtered(&EventFilter {
                event_type: Some("never.matches".to_owned()),
                ..EventFilter::default()
            })
            .map(|_| ()),
        expected.clone(),
    );
}

#[test]
fn every_covered_field_is_detected() {
    let tampered_payload = r#"{"a":1,"b":2}"#;
    let cases: Vec<(&str, &str, Vec<Value>, JournalIntegrityError)> = vec![
        (
            "sequence",
            "UPDATE events SET sequence = 4 WHERE sequence = 3",
            vec![],
            JournalIntegrityError::Continuity {
                expected: 3,
                actual: 4,
            },
        ),
        (
            "event_type",
            "UPDATE events SET event_type = 'step.tampered' WHERE sequence = 3",
            vec![],
            JournalIntegrityError::EventDigest { sequence: 3 },
        ),
        (
            "aggregate_id",
            "UPDATE events SET aggregate_id = 'step-tampered' WHERE sequence = 3",
            vec![],
            JournalIntegrityError::EventDigest { sequence: 3 },
        ),
        (
            "workflow_id",
            "UPDATE events SET workflow_id = 'workflow-tampered' WHERE sequence = 3",
            vec![],
            JournalIntegrityError::EventDigest { sequence: 3 },
        ),
        (
            "run_id",
            "UPDATE events SET run_id = 'run-tampered' WHERE sequence = 3",
            vec![],
            JournalIntegrityError::EventDigest { sequence: 3 },
        ),
        (
            "occurred_at",
            "UPDATE events SET occurred_at = '2020-01-01T00:00:00.000000Z' WHERE sequence = 3",
            vec![],
            JournalIntegrityError::EventDigest { sequence: 3 },
        ),
        (
            "occurred_at_invalid",
            "UPDATE events SET occurred_at = 'not-a-timestamp' WHERE sequence = 3",
            vec![],
            JournalIntegrityError::Timestamp { sequence: 3 },
        ),
        (
            "payload_and_digest",
            "UPDATE events SET payload_json = ?1, digest = ?2 WHERE sequence = 3",
            vec![
                Value::Text(tampered_payload.to_owned()),
                Value::Text(kala_persistence::sha256_text(tampered_payload).to_hex()),
            ],
            JournalIntegrityError::EventDigest { sequence: 3 },
        ),
        (
            "payload_without_digest",
            "UPDATE events SET payload_json = ?1 WHERE sequence = 3",
            vec![Value::Text(tampered_payload.to_owned())],
            JournalIntegrityError::Payload { sequence: 3 },
        ),
        (
            "previous_event_digest",
            "UPDATE events SET previous_event_digest = ?1 WHERE sequence = 3",
            vec![Value::Text(GENESIS_PREVIOUS_DIGEST.to_hex())],
            JournalIntegrityError::Link { sequence: 3 },
        ),
        (
            "event_digest_zero",
            "UPDATE events SET event_digest = ?1 WHERE sequence = 3",
            vec![Value::Text("0".repeat(64))],
            JournalIntegrityError::EventDigest { sequence: 3 },
        ),
        (
            "event_digest_malformed",
            "UPDATE events SET event_digest = 'not-a-digest' WHERE sequence = 3",
            vec![],
            JournalIntegrityError::StoredDigest { sequence: 3 },
        ),
        (
            "event_digest_uppercase",
            "UPDATE events SET event_digest = upper(event_digest) WHERE sequence = 3",
            vec![],
            JournalIntegrityError::StoredDigest { sequence: 3 },
        ),
        (
            "previous_digest_uppercase",
            "UPDATE events SET previous_event_digest = upper(previous_event_digest) \
             WHERE sequence = 3",
            vec![],
            JournalIntegrityError::StoredDigest { sequence: 3 },
        ),
        (
            "payload_digest_uppercase",
            "UPDATE events SET digest = upper(digest) WHERE sequence = 3",
            vec![],
            JournalIntegrityError::Payload { sequence: 3 },
        ),
    ];

    for (name, statement, values, expected) in cases {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let path = directory.path().join(format!("tampered-{name}.sqlite3"));
        let database = seeded_database(&path);
        let journal = EventJournal::new(&database).expect("journal");
        tamper(&database, &[(statement, values)]);
        assert_every_read_fails(&journal, &expected);
    }
}

#[test]
fn whitespace_padding_of_stored_identifiers_is_tampering() {
    let directory = tempfile::TempDir::new().expect("temp dir");

    for field in ["event_type", "aggregate_id", "workflow_id", "run_id"] {
        let path = directory
            .path()
            .join(format!("padded-middle-{field}.sqlite3"));
        {
            let database = seeded_database(&path);
            let journal = EventJournal::new(&database).expect("journal");
            tamper(
                &database,
                &[(
                    &format!(
                        "UPDATE events SET {field} = ' ' || {field} || ' ' WHERE sequence = 2"
                    ),
                    vec![],
                )],
            );
            // The digest was computed over the unpadded identifier, so the raw
            // padded bytes must fail even though trimming would make the value
            // look valid again.
            assert_every_read_fails(
                &journal,
                &JournalIntegrityError::EventDigest { sequence: 2 },
            );
        }

        // Startup verification must see the same raw-byte mismatch.
        let reopened = Database::open(&path).expect("reopen");
        match EventJournal::new(&reopened) {
            Err(PersistenceError::JournalIntegrity(JournalIntegrityError::EventDigest {
                sequence: 2,
            })) => {}
            other => panic!("expected startup padding refusal for {field}, got {other:?}"),
        }

        // Padding the verified tail must also refuse an append, not extend a
        // chain whose stored bytes no longer match their digest.
        let head_path = directory
            .path()
            .join(format!("padded-head-{field}.sqlite3"));
        let head_database = seeded_database(&head_path);
        let head_journal = EventJournal::new(&head_database).expect("head journal");
        tamper(
            &head_database,
            &[(
                &format!("UPDATE events SET {field} = ' ' || {field} || ' ' WHERE sequence = 3"),
                vec![],
            )],
        );
        expect_variant(
            head_journal
                .append("step.3", "step-3", r#"{"index":3}"#)
                .map(|_| ()),
            JournalIntegrityError::EventDigest { sequence: 3 },
        );
    }
}

#[test]
fn correctly_rehashed_raw_padded_identifiers_match_python() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("padded-rehashed.sqlite3");
    let database = seeded_database(&path);
    let journal = EventJournal::new(&database).expect("journal");
    tamper(
        &database,
        &[(
            "UPDATE events SET \
             event_type = ' ' || event_type || ' ', \
             aggregate_id = ' ' || aggregate_id || ' ', \
             workflow_id = ' ' || workflow_id || ' ', \
             run_id = ' ' || run_id || ' ' \
             WHERE sequence = 2",
            vec![],
        )],
    );
    // Recompute the chain over the raw stored bytes, as Python does. This is
    // the compatibility half of the repair: a padded row whose digest really
    // covers the padding is accepted and only normalized for the typed API.
    rechain(&database);

    let verification = journal.verify_chain().expect("verify raw padded chain");
    assert_eq!(verification.event_count, 3);
    let events = journal.read().expect("read raw padded chain");
    assert_eq!(events[1].event_type, "step.1");
    assert_eq!(events[1].aggregate_id, "step-1");
    assert_eq!(events[1].workflow_id.as_deref(), Some("workflow-1"));
    assert_eq!(events[1].run_id.as_deref(), Some("run-1"));
    assert_eq!(
        journal.get(2).expect("get").expect("event 2").aggregate_id,
        "step-1"
    );

    // Python's verify_chain hashes the raw values and its Event API trims the
    // same fields, so it must accept this row and report the same head.
    let inspected = run_oracle(
        &["inspect", path.to_str().expect("path"), "workflow-1"],
        b"",
    );
    assert_eq!(inspected["verification"]["event_count"].as_i64(), Some(3));
    assert_eq!(
        inspected["verification"]["head_event_digest"].as_str(),
        verification
            .head_event_digest
            .map(|digest| digest.to_hex())
            .as_deref()
    );
    assert_eq!(
        inspected["events"][1]["event_type"].as_str(),
        Some("step.1")
    );
    assert_eq!(
        inspected["events"][1]["aggregate_id"].as_str(),
        Some("step-1")
    );
    assert_eq!(
        inspected["events"][1]["workflow_id"].as_str(),
        Some("workflow-1")
    );
    assert_eq!(inspected["events"][1]["run_id"].as_str(), Some("run-1"));
}

#[test]
fn stored_timestamps_use_python_reader_semantics() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("python-timestamps.sqlite3");
    let database = seeded_database(&path);
    let journal = EventJournal::new(&database).expect("journal");
    tamper(
        &database,
        &[
            (
                "UPDATE events SET occurred_at = '2026-01-01T00:00:01.123456789Z' \
                 WHERE sequence = 2",
                vec![],
            ),
            (
                "UPDATE events SET occurred_at = '2026-W01-4T00:00:02+05:30:15' \
                 WHERE sequence = 3",
                vec![],
            ),
        ],
    );
    rechain(&database);

    let verification = journal.verify_chain().expect("verify");
    assert_eq!(verification.event_count, 3);
    let events = journal.read().expect("read");
    assert_eq!(
        kala_persistence::timestamp_to_iso(&events[1].timestamp).expect("event 2 timestamp"),
        "2026-01-01T00:00:01.123456Z"
    );
    assert_eq!(
        kala_persistence::timestamp_to_iso(&events[2].timestamp).expect("event 3 timestamp"),
        "2025-12-31T18:29:47.000000Z"
    );

    let inspected = run_oracle(
        &["inspect", path.to_str().expect("path"), "workflow-1"],
        b"",
    );
    assert_eq!(inspected["verification"]["event_count"].as_i64(), Some(3));
    assert_eq!(
        inspected["events"][1]["occurred_at"].as_str(),
        Some("2026-01-01T00:00:01.123456Z")
    );
    assert_eq!(
        inspected["events"][2]["occurred_at"].as_str(),
        Some("2025-12-31T18:29:47.000000Z")
    );
}

#[test]
fn rechain_detects_payload_identity_identifier_and_timestamp_tampering() {
    let directory = tempfile::TempDir::new().expect("temp dir");

    for (name, mutation, expected) in [
        (
            "noncanonical_payload",
            r#"UPDATE events SET payload_json = '{"index": 1}' WHERE sequence = 2"#,
            JournalIntegrityError::PayloadNotCanonical { sequence: 2 },
        ),
        (
            "invalid_event_type",
            "UPDATE events SET event_type = 'bad type' WHERE sequence = 2",
            JournalIntegrityError::Identifier {
                sequence: 2,
                field: "event_type",
            },
        ),
        (
            "invalid_run_id",
            "UPDATE events SET run_id = '../run' WHERE sequence = 2",
            JournalIntegrityError::Identifier {
                sequence: 2,
                field: "run_id",
            },
        ),
        (
            "invalid_timestamp",
            "UPDATE events SET occurred_at = '2026-13-45T99:99:99Z' WHERE sequence = 2",
            JournalIntegrityError::Timestamp { sequence: 2 },
        ),
    ] {
        let path = directory.path().join(format!("rechain-{name}.sqlite3"));
        let database = seeded_database(&path);
        let journal = EventJournal::new(&database).expect("journal");
        tamper(&database, &[(mutation, vec![])]);
        let stored_payload: String = database
            .serialized(|connection| {
                let payload: String = connection.query_row(
                    "SELECT payload_json FROM events WHERE sequence = 2",
                    [],
                    |row| row.get(0),
                )?;
                Ok(payload)
            })
            .expect("stored payload");
        let payload_digest = kala_persistence::sha256_text(&stored_payload);
        database
            .serialized(|connection| {
                connection.execute(
                    "UPDATE events SET digest = ?1 WHERE sequence = 2",
                    params![payload_digest.to_hex()],
                )?;
                Ok(())
            })
            .expect("refresh payload digest");
        rechain(&database);
        expect_variant(journal.verify_chain().map(|_| ()), expected.clone());
        expect_variant(journal.read().map(|_| ()), expected);
    }
}

#[test]
fn omitted_and_reordered_rows_are_detected() {
    let directory = tempfile::TempDir::new().expect("temp dir");

    let omitted_path = directory.path().join("omitted.sqlite3");
    let database = seeded_database(&omitted_path);
    let journal = EventJournal::new(&database).expect("journal");
    tamper(
        &database,
        &[("DELETE FROM events WHERE sequence = 2", vec![])],
    );
    expect_variant(
        journal.read().map(|_| ()),
        JournalIntegrityError::Continuity {
            expected: 2,
            actual: 3,
        },
    );

    let reordered_path = directory.path().join("reordered.sqlite3");
    let database = seeded_database(&reordered_path);
    let journal = EventJournal::new(&database).expect("journal");
    tamper(
        &database,
        &[
            (
                "CREATE TABLE swapped AS SELECT event_type, aggregate_id, payload_json, digest, \
                 occurred_at FROM events WHERE sequence = 2",
                vec![],
            ),
            (
                "UPDATE events SET event_type = (SELECT event_type FROM swapped), \
                 aggregate_id = (SELECT aggregate_id FROM swapped), \
                 payload_json = (SELECT payload_json FROM swapped), \
                 digest = (SELECT digest FROM swapped), \
                 occurred_at = (SELECT occurred_at FROM swapped) WHERE sequence = 3",
                vec![],
            ),
            ("DROP TABLE swapped", vec![]),
        ],
    );
    expect_variant(
        journal.read().map(|_| ()),
        JournalIntegrityError::EventDigest { sequence: 3 },
    );
}

#[test]
fn appending_to_a_tampered_journal_fails_closed() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("append-tampered.sqlite3");
    let database = seeded_database(&path);
    let journal = EventJournal::new(&database).expect("journal");
    tamper(
        &database,
        &[(
            "UPDATE events SET occurred_at = '2020-01-01T00:00:00.000000Z' WHERE sequence = 3",
            vec![],
        )],
    );
    expect_variant(
        journal
            .append("step.3", "step-3", r#"{"index":3}"#)
            .map(|_| ()),
        JournalIntegrityError::EventDigest { sequence: 3 },
    );
    expect_variant(
        journal.verify_chain().map(|_| ()),
        JournalIntegrityError::EventDigest { sequence: 3 },
    );
}

#[test]
fn startup_refuses_a_journal_tampered_while_closed() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("startup.sqlite3");
    {
        let database = seeded_database(&path);
        let journal = EventJournal::new(&database).expect("journal");
        assert!(
            journal
                .verify_chain()
                .expect("verify")
                .head_event_digest
                .is_some()
        );
    }
    let attacker = Database::open(&path).expect("attacker opens");
    tamper(
        &attacker,
        &[(
            "UPDATE events SET run_id = 'run-x' WHERE sequence = 1",
            vec![],
        )],
    );
    match EventJournal::new(&attacker) {
        Err(PersistenceError::JournalIntegrity(JournalIntegrityError::EventDigest {
            sequence: 1,
        })) => {}
        other => panic!("expected startup refusal, got {other:?}"),
    }
}

#[test]
fn rewritten_history_verifies_but_changes_the_head() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("rewritten.sqlite3");
    let database = seeded_database(&path);
    let journal = EventJournal::new(&database).expect("journal");
    let original_head = journal.verify_chain().expect("verify").head_event_digest;
    let tampered_payload = r#"{"a":1,"b":2}"#;
    tamper(
        &database,
        &[(
            "UPDATE events SET payload_json = ?1, digest = ?2 WHERE sequence = 2",
            vec![
                Value::Text(tampered_payload.to_owned()),
                Value::Text(kala_persistence::sha256_text(tampered_payload).to_hex()),
            ],
        )],
    );
    rechain(&database);
    let rewritten = EventJournal::new(&database).expect("rewritten journal");
    let verification = rewritten.verify_chain().expect("verify rewritten");
    assert_eq!(verification.event_count, 3);
    assert_ne!(verification.head_event_digest, original_head);
    let edited = rewritten.get(2).expect("get").expect("event 2");
    assert_eq!(edited.payload_json, tampered_payload);
}

fn rechain(database: &Database) {
    database
        .serialized(|connection| {
            let mut statement = connection.prepare(
                "SELECT sequence, event_type, aggregate_id, workflow_id, run_id, \
                 payload_json, digest, occurred_at FROM events ORDER BY sequence ASC",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(statement);
            let mut previous = GENESIS_PREVIOUS_DIGEST;
            for (sequence, event_type, aggregate_id, workflow_id, run_id, digest, occurred_at) in
                rows
            {
                let fields = EventChainFields {
                    sequence: sequence as u64,
                    event_type,
                    aggregate_id,
                    workflow_id,
                    run_id,
                    occurred_at,
                    payload_digest: Sha256Digest::from_hex(&digest).expect("lowercase digest"),
                    previous_event_digest: previous,
                };
                let event_digest = chain_digest(&fields);
                connection.execute(
                    "UPDATE events SET previous_event_digest = ?1, event_digest = ?2 \
                     WHERE sequence = ?3",
                    params![previous.to_hex(), event_digest.to_hex(), sequence],
                )?;
                previous = event_digest;
            }
            Ok(())
        })
        .expect("rechain");
}

#[test]
fn concurrent_appends_lose_no_acknowledged_event() {
    let database = Database::in_memory().expect("open");
    let journal = EventJournal::new(&database).expect("journal");
    const THREADS: i64 = 4;
    const PER_THREAD: i64 = 25;

    let acknowledged = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        let journal = &journal;
        let acknowledged = &acknowledged;
        for thread_index in 0..THREADS {
            scope.spawn(move || {
                let mut local = Vec::new();
                for append_index in 0..PER_THREAD {
                    let event = journal
                        .append(
                            "step.started",
                            &format!("step-{thread_index}"),
                            &format!(r#"{{"thread":{thread_index},"append":{append_index}}}"#),
                        )
                        .expect("append");
                    local.push(event.sequence);
                }
                acknowledged.lock().expect("lock").extend(local);
            });
        }
    });

    let mut sequences = acknowledged.into_inner().expect("lock");
    sequences.sort_unstable();
    assert_eq!(sequences, (1..=THREADS * PER_THREAD).collect::<Vec<_>>());

    let verification = journal.verify_chain().expect("verify");
    assert_eq!(verification.event_count, (THREADS * PER_THREAD) as u64);
    let events = journal
        .read_filtered(&EventFilter {
            limit: None,
            ..EventFilter::default()
        })
        .expect("read all");
    assert_eq!(events.len() as i64, THREADS * PER_THREAD);
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event.sequence, index as i64 + 1);
    }
    assert_eq!(
        verification.head_event_digest,
        events.last().map(|event| event.event_digest)
    );
}

#[test]
fn read_defaults_pagination_and_invalid_arguments() {
    let database = Database::in_memory().expect("open");
    let journal = EventJournal::new(&database).expect("journal");
    const COUNT: i64 = 1_005;
    for index in 0..COUNT {
        journal
            .append("step.started", "step-1", &format!(r#"{{"i":{index}}}"#))
            .expect("append");
    }

    let default_read = journal.read().expect("default read");
    assert_eq!(default_read.len(), 1_000);
    assert_eq!(default_read.first().expect("first").sequence, 1);
    assert_eq!(default_read.last().expect("last").sequence, 1_000);

    let all = journal
        .read_filtered(&EventFilter {
            limit: None,
            ..EventFilter::default()
        })
        .expect("read all");
    assert_eq!(all.len() as i64, COUNT);

    let tail = journal
        .read_filtered(&EventFilter {
            after_sequence: 1_000,
            ..EventFilter::default()
        })
        .expect("tail");
    assert_eq!(
        tail.iter().map(|event| event.sequence).collect::<Vec<_>>(),
        vec![1_001, 1_002, 1_003, 1_004, 1_005]
    );

    assert!(matches!(
        journal.read_filtered(&EventFilter {
            limit: Some(0),
            ..EventFilter::default()
        }),
        Err(PersistenceError::InvalidLimit)
    ));
    assert!(matches!(
        journal.read_filtered(&EventFilter {
            after_sequence: -1,
            ..EventFilter::default()
        }),
        Err(PersistenceError::NegativeSequence {
            field: "after_sequence"
        })
    ));
    assert!(matches!(
        journal.read_filtered(&EventFilter {
            event_type: Some("bad type".to_owned()),
            ..EventFilter::default()
        }),
        Err(PersistenceError::InvalidIdentifier {
            field: "event_type"
        })
    ));
    assert!(matches!(
        journal.get(-1),
        Err(PersistenceError::NegativeSequence { field: "sequence" })
    ));
}

#[test]
fn append_and_snapshot_reject_invalid_input() {
    let database = Database::in_memory().expect("open");
    let journal = EventJournal::new(&database).expect("journal");
    assert!(matches!(
        journal.append("bad type;DROP TABLE events", "step-1", r#"{}"#),
        Err(PersistenceError::InvalidIdentifier {
            field: "event_type"
        })
    ));
    assert!(matches!(
        journal.append("step.started", "../step", r#"{}"#),
        Err(PersistenceError::InvalidIdentifier {
            field: "aggregate_id"
        })
    ));
    assert!(matches!(
        journal.append_with(
            "step.started",
            "step-1",
            r#"{}"#,
            &kala_persistence::AppendOptions::default().with_workflow_id("../workflow"),
        ),
        Err(PersistenceError::InvalidIdentifier {
            field: "workflow_id"
        })
    ));
    assert!(matches!(
        journal.append("step.started", "step-1", "[]"),
        Err(PersistenceError::JsonObjectRequired)
    ));
    assert!(matches!(
        journal.append("step.started", "step-1", r#"{"a":NaN}"#),
        Err(PersistenceError::CanonicalJson(_))
    ));
    assert!(matches!(
        journal.append("step.started", "step-1", r#"{"a":1,"a":1}"#),
        Err(PersistenceError::CanonicalJson(_))
    ));
    assert_eq!(journal.verify_chain().expect("empty verify").event_count, 0);

    let snapshots = SnapshotStore::new(&database);
    let first = snapshots
        .save(
            "snapshot-1",
            "workflow-1",
            r#"{"z":2,"a":1}"#,
            Some(fixed_time(0)),
        )
        .expect("save");
    assert_eq!(first.state_json, r#"{"a":1,"z":2}"#);
    assert_eq!(
        first.digest,
        kala_persistence::sha256_text(r#"{"a":1,"z":2}"#)
    );
    snapshots
        .save(
            "snapshot-2",
            "workflow-1",
            r#"{"state":"running"}"#,
            Some(fixed_time(60)),
        )
        .expect("save second");
    assert_eq!(
        snapshots.restore_latest("workflow-1").expect("latest"),
        Some(r#"{"state":"running"}"#.to_owned())
    );
    assert!(matches!(
        snapshots.save("snapshot-3", "workflow-1", "[]", None),
        Err(PersistenceError::JsonObjectRequired)
    ));
    assert!(matches!(
        snapshots.save("snapshot-3", "../workflow", r#"{}"#, None),
        Err(PersistenceError::InvalidIdentifier {
            field: "workflow_id"
        })
    ));
    let duplicate = snapshots.save("snapshot-1", "workflow-1", r#"{}"#, None);
    match duplicate {
        Err(PersistenceError::Sqlite(error)) => {
            assert!(
                error.to_string().contains("UNIQUE"),
                "expected unique violation, got {error}"
            );
        }
        other => panic!("expected duplicate refusal, got {other:?}"),
    }
    assert_eq!(
        snapshots.restore("snapshot-1").expect("restore"),
        Some(r#"{"a":1,"z":2}"#.to_owned())
    );

    // Snapshot tamper detection: canonical bytes and digest are both checked.
    database
        .serialized(|connection| {
            connection.execute(
                "UPDATE workflow_snapshots SET state_json = ?1 WHERE snapshot_id = 'snapshot-2'",
                params![r#"{"state":"tampered"}"#],
            )?;
            Ok(())
        })
        .expect("tamper snapshot state");
    assert!(matches!(
        snapshots.get("snapshot-2"),
        Err(PersistenceError::SnapshotDigestVerification)
    ));
    database
        .serialized(|connection| {
            connection.execute(
                "UPDATE workflow_snapshots SET state_json = ?1, digest = upper(digest) \
                 WHERE snapshot_id = 'snapshot-2'",
                params![r#"{"state":"running"}"#],
            )?;
            Ok(())
        })
        .expect("tamper snapshot digest");
    assert!(matches!(
        snapshots.get("snapshot-2"),
        Err(PersistenceError::SnapshotDigestVerification)
    ));
}

#[test]
fn python_oracle_environment_is_available() {
    // The cross-reader tests spawn the repository Python oracle. Make the
    // dependency explicit here so a missing oracle fails loudly, not silently.
    let python = python_executable();
    let output = std::process::Command::new(python)
        .arg("-c")
        .arg("import kala_agent.persistence.database, kala_agent.persistence.journal")
        .env("PYTHONPATH", repository_root().join("src"))
        .output()
        .expect("spawn python");
    assert!(
        output.status.success(),
        "Python oracle unavailable: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
