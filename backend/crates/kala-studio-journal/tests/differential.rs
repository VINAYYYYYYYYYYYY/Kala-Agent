//! The live differential against the repository's Python journal.
//!
//! The Python module is the oracle: the same file is written by both
//! implementations, alternately, while both hold the file open. Every event
//! one implementation writes is read back by the other and compared field by
//! field, and the raw stored rows are compared column by column, so the two
//! files are the same bytes.
//!
//! The cases are the ones the cell names: interleaved appends in both
//! directions, tamper cases, concurrent appends and the O(1) append shape.

mod support;

use std::path::Path;

use kala_persistence::timestamp_from_iso;
use kala_studio_journal::{AppendOptions, StudioSessionJournal};
use rusqlite::Connection;
use serde_json::{Value, json};
use support::{PythonOracle, base64_decode, base64_encode};
use tempfile::TempDir;

const TIMESTAMPS: [&str; 10] = [
    "2026-10-20T12:00:00.000000Z",
    "2026-10-20T12:00:00.500000Z",
    "2026-10-20T14:00:01.250000+02:00",
    "2026-10-20T12:00:02.750000Z",
    "2026-10-20T12:00:03.000000Z",
    "2026-10-20T12:00:04.125000Z",
    "2026-10-20T12:00:05.999999Z",
    "2026-10-20T12:00:06.000000Z",
    // An offset that is not a whole hour, and a negative offset.
    "2026-10-20T17:30:07.125000+05:30",
    "2026-10-20T07:00:08.875000-05:00",
];

fn native_event(event: &kala_studio_journal::SessionEvent) -> Value {
    let canonical = event
        .payload
        .as_ref()
        .map(|value| kala_persistence::canonicalize_object_value(value).expect("canonical"));
    json!({
        "sequence": event.sequence,
        "event_type": event.event_type,
        "aggregate_id": event.aggregate_id,
        "payload": canonical,
        "payload_canonical": canonical,
        "payload_json": event.payload_json,
        "digest": event.digest,
        "occurred_at": event.occurred_at,
        "previous_event_digest": event.previous_event_digest,
        "event_digest": event.event_digest,
    })
}

/// Compare one event the Python implementation produced with the native one.
fn assert_event_matches(python: &Value, native: &Value, context: &str) {
    for field in [
        "sequence",
        "event_type",
        "aggregate_id",
        "payload_json",
        "digest",
        "occurred_at",
        "previous_event_digest",
        "event_digest",
        "payload_canonical",
    ] {
        assert_eq!(
            python[field], native[field],
            "{context}: the field {field} differs\npython={python}\nnative={native}"
        );
    }
    assert_eq!(
        python["payload"].is_null(),
        native["payload"].is_null(),
        "{context}: the payload presence differs\npython={python}\nnative={native}"
    );
}

/// Read the stored rows the way the oracle does.
fn native_rows(path: &Path) -> Vec<Value> {
    let connection = Connection::open(path).expect("open the journal");
    let mut statement = connection
        .prepare(
            "SELECT sequence, event_type, aggregate_id, workflow_id, run_id, payload_json, \
             digest, occurred_at, previous_event_digest, event_digest \
             FROM studio_events ORDER BY sequence ASC",
        )
        .expect("prepare");
    let mut rows = statement.query([]).expect("query");
    let mut out = Vec::new();
    while let Some(row) = rows.next().expect("next") {
        out.push(json!({
            "sequence": row.get::<_, i64>("sequence").expect("sequence"),
            "event_type": row.get::<_, String>("event_type").expect("event_type"),
            "aggregate_id": row.get::<_, String>("aggregate_id").expect("aggregate_id"),
            "workflow_id": row.get::<_, Option<String>>("workflow_id").expect("workflow_id"),
            "run_id": row.get::<_, Option<String>>("run_id").expect("run_id"),
            "payload_json": row.get::<_, String>("payload_json").expect("payload_json"),
            "digest": row.get::<_, String>("digest").expect("digest"),
            "occurred_at": row.get::<_, String>("occurred_at").expect("occurred_at"),
            "previous_event_digest": row
                .get::<_, String>("previous_event_digest")
                .expect("previous_event_digest"),
            "event_digest": row.get::<_, String>("event_digest").expect("event_digest"),
        }));
    }
    out
}

fn options(timestamp: &str) -> AppendOptions {
    AppendOptions::default().with_occurred_at(timestamp_from_iso(timestamp).expect("a timestamp"))
}

fn append_request(index: usize, payload: &str) -> Value {
    json!({
        "op": "append",
        "event_type": "chat.turn",
        "payload_text": payload,
        "occurred_at": TIMESTAMPS[index % TIMESTAMPS.len()],
    })
}

#[test]
fn python_and_rust_write_one_file_with_the_same_bytes() {
    let directory = TempDir::new().expect("temp dir");
    let data_dir = directory.path().join("studio-data");
    let mut oracle = PythonOracle::spawn();

    // The Python journal opens the directory first, so the schema, the
    // pragmas and the file are created by the Python implementation.
    let python_open = oracle.ok(json!({"op": "open", "data_dir": data_dir.to_str().unwrap()}));
    let journal = StudioSessionJournal::open(&data_dir).expect("open the native journal");
    assert_eq!(
        python_open["database_path"].as_str().expect("a path"),
        journal.database_path().to_str().expect("a path"),
        "the two implementations must use the same journal file"
    );
    assert_eq!(
        python_open["artifact_root"].as_str().expect("a path"),
        journal.artifact_root().to_str().expect("a path"),
        "the two implementations must use the same artifact root"
    );

    // --- three native events ---------------------------------------------
    let mut native_events = Vec::new();
    for (index, timestamp) in TIMESTAMPS.iter().enumerate().take(3) {
        let payload = format!(r#"{{"index":{index},"source":"rust","text":"hello"}}"#);
        native_events.push(
            journal
                .append_json_with("chat.turn", &payload, &options(timestamp))
                .expect("native append"),
        );
    }

    // --- Python reads them back ------------------------------------------
    let python_page = oracle.ok(json!({"op": "page", "after_sequence": 0, "limit": 100}));
    let python_events = python_page.as_array().expect("a page");
    assert_eq!(python_events.len(), native_events.len());
    for (python, native) in python_events.iter().zip(&native_events) {
        assert_event_matches(
            python,
            &native_event(native),
            "a native event read by Python",
        );
    }

    // --- two Python events ------------------------------------------------
    let mut python_written = Vec::new();
    for index in 3..5 {
        let payload = format!(r#"{{"index":{index},"source":"python","text":"world"}}"#);
        python_written.push(oracle.ok(append_request(index, &payload)));
    }

    // --- the native journal reads them back -------------------------------
    let native_page = journal.page(0, 100).expect("native page");
    assert_eq!(native_page.len(), 5);
    for (python, native) in python_written.iter().zip(&native_page[3..]) {
        assert_event_matches(python, &native_event(native), "a Python event read by Rust");
    }

    // --- one more native event, linked to the Python head -----------------
    let sixth = journal
        .append_json_with(
            "chat.turn",
            r#"{"index":5,"source":"rust","text":"again"}"#,
            &options(TIMESTAMPS[5]),
        )
        .expect("native append");
    assert_eq!(
        sixth.previous_event_digest,
        python_written[1]["event_digest"]
            .as_str()
            .expect("a digest"),
        "the native append must link to the Python head"
    );

    // --- both read the whole file identically -----------------------------
    let python_rows = oracle.ok(json!({"op": "rows"}));
    let python_rows = python_rows.as_array().expect("rows");
    let native_rows = native_rows(journal.database_path());
    assert_eq!(python_rows.len(), native_rows.len());
    for (python, native) in python_rows.iter().zip(&native_rows) {
        assert_eq!(python, native, "the stored rows must be identical");
    }

    // --- both agree on the verdict ----------------------------------------
    let python_verify = oracle.ok(json!({"op": "verify"}));
    let native_verify = journal.verify().expect("native verify");
    assert_eq!(python_verify["ok"], json!(native_verify.ok));
    assert_eq!(
        python_verify["event_count"],
        json!(native_verify.event_count)
    );
    assert_eq!(
        python_verify["head_digest"],
        json!(native_verify.head_digest)
    );
    assert_eq!(
        python_verify["first_bad_sequence"],
        json!(native_verify.first_bad_sequence)
    );
    assert!(native_verify.ok);
    assert_eq!(native_verify.event_count, 6);

    let python_head = oracle.ok(json!({"op": "head_digest"}));
    assert_eq!(python_head, json!(native_verify.head_digest));

    // --- both wrote the same schema ---------------------------------------
    let python_schema = oracle.ok(json!({"op": "schema"}));
    let native_schema = native_schema(journal.database_path());
    assert_eq!(
        python_schema, native_schema,
        "the two implementations must write the same schema"
    );
    let python_pragmas = oracle.ok(json!({"op": "pragmas"}));
    assert_eq!(python_pragmas["journal_mode"], json!("wal"));
}

/// Read the stored schema the way the oracle does.
fn native_schema(path: &Path) -> Value {
    let connection = Connection::open(path).expect("open the journal");
    let mut statement = connection
        .prepare("SELECT type, name, sql FROM sqlite_master ORDER BY type, name")
        .expect("prepare");
    let mut rows = statement.query([]).expect("query");
    let mut out = Vec::new();
    while let Some(row) = rows.next().expect("next") {
        out.push(json!([
            row.get::<_, String>("type").expect("type"),
            row.get::<_, String>("name").expect("name"),
            row.get::<_, Option<String>>("sql").expect("sql"),
        ]));
    }
    Value::Array(out)
}

#[test]
fn both_implementations_detect_the_same_tampering() {
    let directory = TempDir::new().expect("temp dir");
    let data_dir = directory.path().join("studio-data");
    let mut oracle = PythonOracle::spawn();
    oracle.ok(json!({"op": "open", "data_dir": data_dir.to_str().unwrap()}));
    let journal = StudioSessionJournal::open(&data_dir).expect("open the native journal");

    for (index, timestamp) in TIMESTAMPS.iter().enumerate().take(4) {
        journal
            .append_json_with(
                "chat.turn",
                &format!(r#"{{"index":{index}}}"#),
                &options(timestamp),
            )
            .expect("native append");
    }

    // --- Python tampers with a payload ------------------------------------
    oracle.ok(json!({
        "op": "tamper_payload",
        "sequence": 2,
        "payload_json": "{\"index\":999}",
    }));
    let python_verify = oracle.ok(json!({"op": "verify"}));
    let native_verify = journal.verify().expect("native verify");
    assert_eq!(
        python_verify["first_bad_sequence"],
        json!(native_verify.first_bad_sequence)
    );
    assert_eq!(python_verify["first_bad_sequence"], json!(2));
    assert_eq!(python_verify["ok"], json!(native_verify.ok));
    assert_eq!(
        python_verify["event_count"],
        json!(native_verify.event_count)
    );

    // Both refuse to extend a broken chain.
    let python_error = oracle.err(append_request(4, r#"{"index":4}"#));
    assert_eq!(python_error["kind"], json!("integrity"));
    let native_error = journal
        .append_json_with("chat.turn", r#"{"index":4}"#, &options(TIMESTAMPS[4]))
        .expect_err("a broken chain must not be extended");
    assert_eq!(
        native_error.kind(),
        kala_studio_journal::ErrorKind::Integrity
    );
    assert_eq!(
        native_error.to_string(),
        "the studio session journal is broken at sequence 2; refusing to extend it"
    );

    // --- the history stays readable ---------------------------------------
    let python_page = oracle.ok(json!({"op": "page", "after_sequence": 0, "limit": 100}));
    let native_page = journal.page(0, 100).expect("native page");
    assert_eq!(
        python_page.as_array().expect("a page").len(),
        native_page.len()
    );
    assert_eq!(
        python_page.as_array().expect("a page")[1]["payload_json"],
        json!(native_page[1].payload_json)
    );

    // --- repair it, then break the chain digest ---------------------------
    oracle.ok(json!({
        "op": "tamper_payload",
        "sequence": 2,
        "payload_json": "{\"index\":1}",
    }));
    assert!(journal.verify().expect("native verify").ok);

    oracle.ok(json!({
        "op": "tamper_digest",
        "sequence": 3,
        "event_digest": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    }));
    let python_verify = oracle.ok(json!({"op": "verify"}));
    let native_verify = journal.verify().expect("native verify");
    assert_eq!(
        python_verify["first_bad_sequence"],
        json!(native_verify.first_bad_sequence)
    );
    assert_eq!(python_verify["first_bad_sequence"], json!(3));

    // --- drop an event ----------------------------------------------------
    oracle.ok(json!({"op": "drop", "sequence": 2}));
    let python_verify = oracle.ok(json!({"op": "verify"}));
    let native_verify = journal.verify().expect("native verify");
    assert_eq!(
        python_verify["first_bad_sequence"],
        json!(native_verify.first_bad_sequence)
    );
    assert_eq!(python_verify["first_bad_sequence"], json!(2));
    assert_eq!(
        python_verify["event_count"],
        json!(native_verify.event_count)
    );
    assert_eq!(python_verify["event_count"], json!(3));
}

#[test]
fn both_implementations_refuse_the_same_invalid_input() {
    let directory = TempDir::new().expect("temp dir");
    let data_dir = directory.path().join("studio-data");
    let mut oracle = PythonOracle::spawn();
    oracle.ok(json!({"op": "open", "data_dir": data_dir.to_str().unwrap()}));
    let journal = StudioSessionJournal::open(&data_dir).expect("open the native journal");

    // An unsafe event type.
    let python_error = oracle.err(json!({
        "op": "append",
        "event_type": "not a safe event type",
        "payload_text": "{}",
    }));
    let native_error = journal
        .append_json("not a safe event type", "{}")
        .expect_err("an unsafe event type must be refused");
    assert_eq!(python_error["kind"], json!("value"));
    assert_eq!(native_error.kind(), kala_studio_journal::ErrorKind::Value);
    assert_eq!(python_error["message"], json!(native_error.to_string()));

    // A payload that is not a mapping.
    let python_error = oracle.err(json!({
        "op": "append",
        "event_type": "chat.turn",
        "payload_text": "\"hello\"",
    }));
    let native_error = journal
        .append_json("chat.turn", "\"hello\"")
        .expect_err("a non-object payload must be refused");
    assert_eq!(python_error["kind"], json!("type"));
    assert_eq!(native_error.kind(), kala_studio_journal::ErrorKind::Type);
    assert_eq!(python_error["message"], json!(native_error.to_string()));

    // A payload that is not JSON at all.
    let python_error = oracle.err(json!({
        "op": "append",
        "event_type": "chat.turn",
        "payload_text": "{not json",
    }));
    let native_error = journal
        .append_json("chat.turn", "{not json")
        .expect_err("an invalid payload must be refused");
    assert_eq!(python_error["kind"], json!("value"));
    assert_eq!(native_error.kind(), kala_studio_journal::ErrorKind::Value);
    assert_eq!(python_error["message"], json!(native_error.to_string()));

    // An unknown artifact.
    let python_error = oracle.err(json!({"op": "get_artifact", "digest": "0".repeat(64)}));
    let native_error = journal
        .get_artifact(&"0".repeat(64))
        .expect_err("an unknown artifact must be refused");
    assert_eq!(python_error["kind"], json!("not_found"));
    assert_eq!(
        native_error.kind(),
        kala_studio_journal::ErrorKind::NotFound
    );

    // The payload type is checked before the identifiers.
    let python_error = oracle.err(json!({
        "op": "append",
        "event_type": "not a safe event type",
        "payload_text": "\"hello\"",
    }));
    let native_error = journal
        .append_json("not a safe event type", "\"hello\"")
        .expect_err("a non-object payload must be refused");
    assert_eq!(python_error["kind"], json!("type"));
    assert_eq!(native_error.kind(), kala_studio_journal::ErrorKind::Type);
    assert_eq!(python_error["message"], json!(native_error.to_string()));
}

#[test]
fn both_implementations_store_the_same_artifacts() {
    let directory = TempDir::new().expect("temp dir");
    let data_dir = directory.path().join("studio-data");
    let mut oracle = PythonOracle::spawn();
    oracle.ok(json!({"op": "open", "data_dir": data_dir.to_str().unwrap()}));
    let journal = StudioSessionJournal::open(&data_dir).expect("open the native journal");

    let payloads: [&[u8]; 3] = [
        b"ISO-10303-21;\nEND-ISO-10303-21;\n",
        b"",
        &[0u8, 1, 2, 255, 254],
    ];
    for payload in payloads {
        let python_digest = oracle.ok(json!({
            "op": "put_artifact",
            "data_base64": base64_encode(payload),
        }));
        let native_digest = journal.put_artifact(payload).expect("store");
        assert_eq!(python_digest, json!(native_digest));
        assert_eq!(
            python_digest.as_str().expect("a digest").len(),
            64,
            "the digest must be a SHA-256 hex digest"
        );

        let python_bytes = oracle.ok(json!({"op": "get_artifact", "digest": python_digest}));
        let decoded = base64_decode(python_bytes["base64"].as_str().expect("base64"));
        assert_eq!(decoded, payload);
        assert_eq!(journal.get_artifact(&native_digest).expect("read"), payload);

        let python_has = oracle.ok(json!({"op": "has_artifact", "digest": python_digest}));
        assert_eq!(
            python_has,
            json!(journal.has_artifact(&native_digest).expect("has"))
        );
    }

    // The artifact files themselves are the same layout.
    let artifact_root = journal.artifact_root();
    for payload in payloads {
        let digest = journal.put_artifact(payload).expect("store");
        let bucket = &digest[..2];
        let object = artifact_root.join("objects").join(bucket).join(&digest);
        assert!(
            object.is_file(),
            "the artifact must live at objects/{bucket}/{digest}"
        );
        assert_eq!(std::fs::read(&object).expect("read"), payload);
    }
}

#[test]
fn the_two_implementations_append_concurrently_to_one_file() {
    let directory = TempDir::new().expect("temp dir");
    let data_dir = directory.path().join("studio-data");
    let mut oracle = PythonOracle::spawn();
    oracle.ok(json!({"op": "open", "data_dir": data_dir.to_str().unwrap()}));
    let journal = StudioSessionJournal::open(&data_dir).expect("open the native journal");

    let count = 40;
    let python_events = std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            oracle.ok(json!({
                "op": "append_many",
                "count": count,
                "event_type": "chat.turn",
                "payload_text": r#"{"index":{{index}},"source":"python"}"#,
                "occurred_at": TIMESTAMPS[0],
            }))
        });
        for index in 0..count {
            journal
                .append_json_with(
                    "chat.turn",
                    &format!(r#"{{"index":{index},"source":"rust"}}"#),
                    &options(TIMESTAMPS[index % TIMESTAMPS.len()]),
                )
                .expect("native append");
        }
        handle.join().expect("the oracle thread")
    });
    assert_eq!(python_events.as_array().expect("events").len(), count);

    // The chain is intact and holds every event, whoever wrote it.
    let native_verify = journal.verify().expect("native verify");
    assert!(native_verify.ok, "the interleaved chain must verify");
    assert_eq!(native_verify.event_count, (count * 2) as i64);

    let python_verify = oracle.ok(json!({"op": "verify"}));
    assert_eq!(python_verify["ok"], json!(native_verify.ok));
    assert_eq!(
        python_verify["event_count"],
        json!(native_verify.event_count)
    );
    assert_eq!(
        python_verify["head_digest"],
        json!(native_verify.head_digest)
    );

    // Every event links to the one before it.
    let events = journal.page(0, (count * 2) as i64).expect("page");
    assert_eq!(events.len(), count * 2);
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event.sequence, index as i64 + 1);
        if index > 0 {
            assert_eq!(
                event.previous_event_digest,
                events[index - 1].event_digest,
                "event {index} must link to the event before it"
            );
        }
    }
}

#[test]
fn a_long_native_history_is_still_written_by_python() {
    let directory = TempDir::new().expect("temp dir");
    let data_dir = directory.path().join("studio-data");
    let mut oracle = PythonOracle::spawn();
    oracle.ok(json!({"op": "open", "data_dir": data_dir.to_str().unwrap()}));
    let journal = StudioSessionJournal::open(&data_dir).expect("open the native journal");

    for index in 0..50 {
        journal
            .append_json_with(
                "chat.turn",
                &format!(r#"{{"index":{index}}}"#),
                &options(TIMESTAMPS[index % TIMESTAMPS.len()]),
            )
            .expect("native append");
    }
    let python_event = oracle.ok(append_request(0, r#"{"index":50,"source":"python"}"#));

    let native_page = journal.page(0, 100).expect("native page");
    assert_eq!(native_page.len(), 51);
    assert_event_matches(
        &python_event,
        &native_event(&native_page[50]),
        "a Python event read by Rust after a long native history",
    );

    let python_verify = oracle.ok(json!({"op": "verify"}));
    let native_verify = journal.verify().expect("native verify");
    assert_eq!(python_verify["ok"], json!(native_verify.ok));
    assert_eq!(
        python_verify["event_count"],
        json!(native_verify.event_count)
    );
    assert_eq!(python_verify["event_count"], json!(51));
}
