//! Fixture parity: read the Python-created schema-v2 database and its
//! recorded event/snapshot rows, byte for byte, without modifying the fixture.

mod support;

use std::fs;

use kala_persistence::{
    Database, EventFilter, EventJournal, PersistenceError, SnapshotStore, timestamp_to_iso,
};
use rusqlite::Error as SqliteError;
use support::{copy_fixture_database, fixed_time, fixture_dir, fixture_json};

fn fixture_database_bytes() -> Vec<u8> {
    fs::read(fixture_dir().join("schema_v2.sqlite3")).expect("read fixture database")
}

#[test]
fn fixture_database_is_consumed_read_only() {
    let before = fixture_database_bytes();
    let copy = copy_fixture_database();
    let database = Database::open(&copy.path).expect("open fixture copy");
    let journal = EventJournal::new(&database).expect("journal");
    assert_eq!(journal.verify_chain().expect("verify").event_count, 5);
    drop(database);
    let after = fixture_database_bytes();
    assert_eq!(before, after, "fixture database bytes were modified");
}

#[test]
fn fixture_events_chain_and_snapshot_match_python_rows() {
    let copy = copy_fixture_database();
    let database = Database::open(&copy.path).expect("open fixture copy");
    let journal = EventJournal::new(&database).expect("journal");
    let fixture = fixture_json("event_chain.json");
    let database_record = &fixture["database"];

    let events = journal.read().expect("read events");
    let expected_events = database_record["events"].as_array().expect("events");
    assert_eq!(events.len(), expected_events.len());
    for (event, expected) in events.iter().zip(expected_events) {
        assert_eq!(
            event.sequence,
            expected["sequence"].as_i64().expect("sequence")
        );
        assert_eq!(
            event.event_type,
            expected["event_type"].as_str().expect("type")
        );
        assert_eq!(
            event.aggregate_id,
            expected["aggregate_id"].as_str().expect("aggregate")
        );
        assert_eq!(
            event.workflow_id.as_deref(),
            expected["workflow_id"].as_str()
        );
        assert_eq!(event.run_id.as_deref(), expected["run_id"].as_str());
        assert_eq!(
            event.payload_json,
            expected["payload_json"].as_str().expect("payload")
        );
        assert_eq!(
            event.digest.to_hex(),
            expected["digest"].as_str().expect("digest")
        );
        assert_eq!(
            timestamp_to_iso(&event.timestamp).expect("timestamp"),
            expected["occurred_at"].as_str().expect("occurred_at")
        );
        assert_eq!(
            event.previous_event_digest.to_hex(),
            expected["previous_event_digest"]
                .as_str()
                .expect("previous digest")
        );
        assert_eq!(
            event.event_digest.to_hex(),
            expected["event_digest"].as_str().expect("event digest")
        );
    }

    let verification = journal.verify_chain().expect("verify");
    assert_eq!(
        verification.event_count,
        database_record["verified_event_count"]
            .as_i64()
            .expect("count") as u64
    );
    assert_eq!(
        verification.head_sequence,
        Some(
            database_record["verified_head_sequence"]
                .as_i64()
                .expect("head sequence")
        )
    );
    assert_eq!(
        verification.head_event_digest.map(|digest| digest.to_hex()),
        Some(
            database_record["verified_head_event_digest"]
                .as_str()
                .expect("head digest")
                .to_owned()
        )
    );

    assert_eq!(journal.get(3).expect("get").expect("event 3").sequence, 3);
    assert!(journal.get(0).expect("get zero").is_none());
    assert!(journal.get(99).expect("get missing").is_none());
    assert!(matches!(
        journal.get(-1),
        Err(PersistenceError::NegativeSequence { field: "sequence" })
    ));

    let completed = journal
        .read_filtered(&EventFilter {
            event_type: Some("workflow.completed".to_owned()),
            ..EventFilter::default()
        })
        .expect("filtered read");
    assert_eq!(
        completed
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![5]
    );
    let run_events = journal
        .read_filtered(&EventFilter {
            workflow_id: Some("flow-corpus".to_owned()),
            run_id: Some("run-1".to_owned()),
            after_sequence: 3,
            ..EventFilter::default()
        })
        .expect("run filter");
    assert_eq!(
        run_events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![4, 5]
    );

    let snapshots = SnapshotStore::new(&database);
    let snapshot = snapshots
        .get("snapshot-flow-corpus-000001")
        .expect("get snapshot")
        .expect("snapshot exists");
    let expected_snapshot = &database_record["snapshot"];
    assert_eq!(
        snapshot.sequence,
        expected_snapshot["sequence"].as_i64().expect("sequence")
    );
    assert_eq!(
        snapshot.snapshot_id,
        expected_snapshot["snapshot_id"].as_str().expect("id")
    );
    assert_eq!(
        snapshot.workflow_id,
        expected_snapshot["workflow_id"].as_str().expect("workflow")
    );
    assert_eq!(
        snapshot.state_json,
        expected_snapshot["state_json"].as_str().expect("state")
    );
    assert_eq!(
        snapshot.digest.to_hex(),
        expected_snapshot["digest"].as_str().expect("digest")
    );
    assert_eq!(
        timestamp_to_iso(&snapshot.created_at).expect("created_at"),
        expected_snapshot["created_at"]
            .as_str()
            .expect("created at")
    );
    assert_eq!(
        snapshots
            .restore("snapshot-flow-corpus-000001")
            .expect("restore"),
        Some(snapshot.state_json.clone())
    );
    assert_eq!(
        snapshots
            .restore_latest("flow-corpus")
            .expect("restore latest"),
        Some(snapshot.state_json.clone())
    );
    assert_eq!(
        snapshots.latest("flow-corpus").expect("latest"),
        Some(snapshot)
    );
    assert_eq!(snapshots.get("missing").expect("missing"), None);
    assert_eq!(snapshots.latest("missing").expect("missing latest"), None);
}

#[test]
fn append_to_fixture_copy_extends_chain_and_reopens() {
    let copy = copy_fixture_database();
    let database = Database::open(&copy.path).expect("open fixture copy");
    let journal = EventJournal::new(&database).expect("journal");
    let before = journal.verify_chain().expect("verify before");
    let appended = journal
        .append_with(
            "workflow.snapshot",
            "flow-corpus",
            r#"{"note":"native append"}"#,
            &kala_persistence::AppendOptions::default()
                .with_workflow_id("flow-corpus")
                .with_run_id("run-2")
                .with_occurred_at(fixed_time(30)),
        )
        .expect("append");
    assert_eq!(appended.sequence, 6);
    assert_eq!(
        appended.previous_event_digest,
        before.head_event_digest.expect("head")
    );
    assert_eq!(appended.payload_json, r#"{"note":"native append"}"#);
    let snapshot = SnapshotStore::new(&database)
        .save(
            "snapshot-flow-corpus-000002",
            "flow-corpus",
            r#"{"state":"final"}"#,
            Some(fixed_time(31)),
        )
        .expect("save snapshot");
    assert_eq!(snapshot.state_json, r#"{"state":"final"}"#);
    database.close().expect("close");

    let reopened = Database::open(&copy.path).expect("reopen");
    let reopened_journal = EventJournal::new(&reopened).expect("reopen journal");
    let verification = reopened_journal.verify_chain().expect("verify after");
    assert_eq!(verification.event_count, 6);
    assert_eq!(verification.head_event_digest, Some(appended.event_digest));
    let read = reopened_journal.read().expect("read");
    assert_eq!(
        read.last().expect("last").event_digest,
        appended.event_digest
    );
    let snapshots = SnapshotStore::new(&reopened);
    assert_eq!(
        snapshots
            .restore_latest("flow-corpus")
            .expect("restore latest"),
        Some(r#"{"state":"final"}"#.to_owned())
    );
}

#[test]
fn append_only_triggers_and_chain_insert_guard_hold() {
    let database = Database::in_memory().expect("open");
    let journal = EventJournal::new(&database).expect("journal");
    let event = journal
        .append("step.started", "step-1", r#"{"value":1}"#)
        .expect("append");

    let update = database.serialized(|connection| {
        connection.execute(
            "UPDATE events SET event_type = 'changed' WHERE sequence = ?1",
            rusqlite::params![event.sequence],
        )?;
        Ok(())
    });
    assert_sqlite_message(update, "append-only");

    let delete = database.serialized(|connection| {
        connection.execute(
            "DELETE FROM events WHERE sequence = ?1",
            rusqlite::params![event.sequence],
        )?;
        Ok(())
    });
    assert_sqlite_message(delete, "append-only");

    let unchained = database.serialized(|connection| {
        connection.execute(
            "INSERT INTO events(
                event_type, aggregate_id, workflow_id, run_id, payload_json,
                digest, occurred_at
            ) VALUES (?1, ?2, NULL, NULL, ?3, ?4, ?5)",
            rusqlite::params![
                "step.started",
                "step-1",
                "{}",
                kala_persistence::sha256_text("{}").to_hex(),
                "2026-01-02T03:04:05.000000Z",
            ],
        )?;
        Ok(())
    });
    assert_sqlite_message(unchained, "hash chain");

    assert_eq!(journal.get(event.sequence).expect("get"), Some(event));
}

fn assert_sqlite_message(result: Result<(), PersistenceError>, needle: &str) {
    match result {
        Err(PersistenceError::Sqlite(SqliteError::SqliteFailure(_, Some(message)))) => {
            assert!(
                message.contains(needle),
                "expected {needle:?} in {message:?}"
            );
        }
        other => panic!("expected SQLite failure containing {needle:?}, got {other:?}"),
    }
}
