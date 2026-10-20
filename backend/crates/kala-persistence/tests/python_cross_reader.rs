//! Real cross-reader tests: Rust and Python each open, verify and extend the
//! other's databases. Python is the live product oracle; the native library
//! issues every write through its own SQLite connection.

mod support;

use kala_persistence::{Database, Event, EventJournal, SnapshotStore, timestamp_to_iso};
use serde_json::{Value, json};
use support::{copy_fixture_database, fixed_time, fixture_json, run_oracle};

fn event_json(event: &Event) -> Value {
    json!({
        "sequence": event.sequence,
        "event_type": event.event_type,
        "aggregate_id": event.aggregate_id,
        "workflow_id": event.workflow_id,
        "run_id": event.run_id,
        "payload_json": event.payload_json,
        "digest": event.digest.to_hex(),
        "occurred_at": timestamp_to_iso(&event.timestamp).expect("timestamp"),
        "previous_event_digest": event.previous_event_digest.to_hex(),
        "event_digest": event.event_digest.to_hex(),
    })
}

fn assert_event_matches(actual: &Value, expected: &Value) {
    for field in [
        "sequence",
        "event_type",
        "aggregate_id",
        "workflow_id",
        "run_id",
        "payload_json",
        "digest",
        "occurred_at",
        "previous_event_digest",
        "event_digest",
    ] {
        assert_eq!(actual[field], expected[field], "event field {field}");
    }
}

#[test]
fn rust_created_database_round_trips_through_python() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("rust-created.sqlite3");

    let (rust_events, rust_head) = {
        let database = Database::open(&path).expect("open");
        let journal = EventJournal::new(&database).expect("journal");
        let first = journal
            .append_with(
                "design.created",
                "design-rust",
                r#"{"big_integer":1180591620717411303424,"ratio":1e-05}"#,
                &kala_persistence::AppendOptions::default().with_occurred_at(fixed_time(0)),
            )
            .expect("append first");
        let second = journal
            .append_with(
                "workflow.started",
                "flow-rust",
                r#"{"offline":true,"step_ids":["one","two"]}"#,
                &kala_persistence::AppendOptions::default()
                    .with_workflow_id("flow-rust")
                    .with_run_id("run-rust")
                    .with_occurred_at(fixed_time(1)),
            )
            .expect("append second");
        // A payload beyond every tools.protocol bound, so Python must use the
        // larger CanonicalJsonObject domain to verify the native bytes.
        let large_key = "k".repeat(300);
        let large_payload = format!(
            r#"{{"deep":{}0{},"large_array":[{}],"{large_key}":"{}"}}"#,
            "[".repeat(64),
            "]".repeat(64),
            std::iter::repeat_n("1", 2_100)
                .collect::<Vec<_>>()
                .join(","),
            "a".repeat(33_000),
        );
        let large_canonical =
            kala_persistence::canonicalize_json_object(&large_payload).expect("large object");
        let third = journal
            .append_with(
                "domain.large",
                "design-rust",
                &large_payload,
                &kala_persistence::AppendOptions::default().with_occurred_at(fixed_time(2)),
            )
            .expect("append large");
        assert_eq!(third.payload_json, large_canonical);
        SnapshotStore::new(&database)
            .save(
                "snapshot-rust-000001",
                "flow-rust",
                r#"{"state":"running","step":1}"#,
                Some(fixed_time(3)),
            )
            .expect("snapshot");
        let head = journal.verify_chain().expect("verify").head_event_digest;
        assert_eq!(head, Some(third.event_digest));
        (vec![first, second, third], head.expect("head"))
    };

    // Python reopens the Rust database and fully verifies it.
    let inspected = run_oracle(&["inspect", path.to_str().expect("path"), "flow-rust"], b"");
    let events = inspected["events"].as_array().expect("events");
    assert_eq!(events.len(), rust_events.len());
    for (actual, expected) in events.iter().zip(&rust_events) {
        assert_event_matches(actual, &event_json(expected));
    }
    assert_eq!(
        inspected["verification"]["head_event_digest"].as_str(),
        Some(rust_head.to_hex().as_str())
    );
    assert_eq!(inspected["verification"]["event_count"].as_i64(), Some(3));
    assert_eq!(
        inspected["snapshot"]["state_json"].as_str(),
        Some(r#"{"state":"running","step":1}"#)
    );

    // Python appends a large CanonicalJsonObject payload to the Rust database.
    let appended = run_oracle(&["append", path.to_str().expect("path")], b"");
    let python_head = appended["verification"]["head_event_digest"]
        .as_str()
        .expect("head");
    let python_payload = appended["event"]["payload_json"]
        .as_str()
        .expect("payload")
        .to_owned();
    assert_eq!(appended["verification"]["event_count"].as_i64(), Some(4));
    assert_eq!(appended["event"]["sequence"].as_i64(), Some(4));
    assert_eq!(
        appended["event"]["previous_event_digest"].as_str(),
        Some(rust_head.to_hex().as_str())
    );
    assert!(python_payload.contains(r#""source":"python""#));
    assert!(python_payload.len() > 33_000);

    // Rust reopens and verifies Python's bytes.
    let database = Database::open(&path).expect("reopen");
    let journal = EventJournal::new(&database).expect("reopened journal");
    let verification = journal.verify_chain().expect("verify");
    assert_eq!(verification.event_count, 4);
    assert_eq!(
        verification.head_event_digest.map(|digest| digest.to_hex()),
        Some(python_head.to_owned())
    );
    let events = journal.read().expect("read");
    assert_eq!(events.len(), 4);
    assert_eq!(events[3].payload_json, python_payload);
    assert_eq!(
        events[3].digest,
        kala_persistence::sha256_text(&python_payload)
    );
    assert_eq!(
        SnapshotStore::new(&database)
            .restore_latest("flow-oracle")
            .expect("restore"),
        Some(r#"{"state":"appended"}"#.to_owned())
    );
}

#[test]
fn python_created_database_round_trips_through_rust() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("python-created.sqlite3");

    let created = run_oracle(&["create", path.to_str().expect("path")], b"");
    let python_events = created["events"].as_array().expect("events");
    assert_eq!(python_events.len(), 2);
    let python_head = created["verification"]["head_event_digest"]
        .as_str()
        .expect("head")
        .to_owned();

    let rust_head = {
        let database = Database::open(&path).expect("open python database");
        let journal = EventJournal::new(&database).expect("journal");
        let events = journal.read().expect("read");
        assert_eq!(events.len(), python_events.len());
        for (event, expected) in events.iter().zip(python_events) {
            assert_event_matches(&event_json(event), expected);
        }
        let snapshot = SnapshotStore::new(&database)
            .get("snapshot-oracle-000001")
            .expect("snapshot")
            .expect("snapshot exists");
        assert_eq!(
            snapshot.state_json,
            r#"{"state":"created","steps":["one","two"]}"#
        );
        let head = journal.verify_chain().expect("verify").head_event_digest;
        assert_eq!(
            head.map(|digest| digest.to_hex()),
            Some(python_head.clone())
        );

        let appended = journal
            .append_with(
                "workflow.completed",
                "flow-oracle",
                r#"{"status":"completed"}"#,
                &kala_persistence::AppendOptions::default()
                    .with_workflow_id("flow-oracle")
                    .with_occurred_at(fixed_time(20)),
            )
            .expect("rust append");
        assert_eq!(appended.sequence, 3);
        assert_eq!(appended.previous_event_digest.to_hex(), python_head);
        SnapshotStore::new(&database)
            .save(
                "snapshot-rust-000003",
                "flow-oracle",
                r#"{"state":"completed"}"#,
                Some(fixed_time(21)),
            )
            .expect("rust snapshot");
        journal
            .verify_chain()
            .expect("verify")
            .head_event_digest
            .expect("head")
    };

    // Python reopens the Rust-extended database and verifies every byte.
    let inspected = run_oracle(&["inspect", path.to_str().expect("path")], b"");
    assert_eq!(inspected["verification"]["event_count"].as_i64(), Some(3));
    assert_eq!(
        inspected["verification"]["head_event_digest"].as_str(),
        Some(rust_head.to_hex().as_str())
    );
    let events = inspected["events"].as_array().expect("events");
    assert_eq!(events.len(), 3);
    assert_eq!(
        events[2]["payload_json"].as_str(),
        Some(r#"{"status":"completed"}"#)
    );
    assert_eq!(
        inspected["snapshot"]["state_json"].as_str(),
        Some(r#"{"state":"completed"}"#)
    );
}

#[test]
fn copied_fixture_rust_extension_is_verified_by_python() {
    let copy = copy_fixture_database();
    let fixture = fixture_json("event_chain.json");
    let fixture_events = fixture["database"]["events"]
        .as_array()
        .expect("fixture events");

    let (rust_sequence, rust_head) = {
        let database = Database::open(&copy.path).expect("open fixture copy");
        let journal = EventJournal::new(&database).expect("journal");
        let events = journal.read().expect("read fixture");
        assert_eq!(events.len(), 5);
        for (event, expected) in events.iter().zip(fixture_events) {
            assert_event_matches(&event_json(event), expected);
        }
        let appended = journal
            .append_with(
                "workflow.extended",
                "flow-corpus",
                r#"{"source":"rust","verified_fixture":true}"#,
                &kala_persistence::AppendOptions::default()
                    .with_workflow_id("flow-corpus")
                    .with_run_id("run-rust")
                    .with_occurred_at(fixed_time(10)),
            )
            .expect("append");
        SnapshotStore::new(&database)
            .save(
                "snapshot-rust-fixture-000001",
                "flow-corpus",
                r#"{"state":"native","sequence":6}"#,
                Some(fixed_time(11)),
            )
            .expect("snapshot");
        let head = journal.verify_chain().expect("verify").head_event_digest;
        (appended.sequence, head.expect("head"))
    };
    assert_eq!(rust_sequence, 6);

    let inspected = run_oracle(
        &["inspect", copy.path.to_str().expect("path"), "flow-corpus"],
        b"",
    );
    assert_eq!(inspected["verification"]["event_count"].as_i64(), Some(6));
    assert_eq!(
        inspected["verification"]["head_event_digest"].as_str(),
        Some(rust_head.to_hex().as_str())
    );
    let events = inspected["events"].as_array().expect("events");
    for (actual, expected) in events.iter().take(5).zip(fixture_events) {
        assert_event_matches(actual, expected);
    }
    assert_eq!(
        events[5]["payload_json"].as_str(),
        Some(r#"{"source":"rust","verified_fixture":true}"#)
    );
    assert_eq!(
        inspected["snapshot"]["state_json"].as_str(),
        Some(r#"{"sequence":6,"state":"native"}"#)
    );
    assert_eq!(
        inspected["snapshot"]["digest"].as_str(),
        Some(
            kala_persistence::sha256_text(r#"{"sequence":6,"state":"native"}"#)
                .to_hex()
                .as_str()
        )
    );
}
