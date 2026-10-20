//! Persistence lifecycle coverage for deep `CanonicalJsonObject` payloads.
//!
//! A stored event or snapshot may be nested far beyond the old 1,024-level
//! parser ceiling. These tests exercise append, get, filtered reads, chain
//! verification, snapshots, restore/reopen and a copied fixture database at
//! depths that the previous implementation rejected. Deep values are parsed,
//! cloned, encoded and dropped on ordinary 2 MiB thread stacks.

mod support;

use kala_persistence::{
    AppendOptions, Database, EventFilter, EventJournal, SnapshotStore, canonicalize_json_object,
    canonicalize_object_value, parse_json_object, sha256_text,
};
use support::{copy_fixture_database, fixed_time};

const LEAF: &str = r#"{"big":1180591620717411303424,"f":-0.0,"n":null,"u":"\u03ba \ud83d\ude00"}"#;

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

fn array_chain_source(depth: usize) -> String {
    let mut source = String::from(r#"{"x":"#);
    source.push_str(&"[".repeat(depth));
    source.push('0');
    source.push_str(&"]".repeat(depth));
    source.push('}');
    source
}

fn on_two_mib_stack<F, T>(operation: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(operation)
        .expect("spawn 2 MiB thread")
        .join()
        .expect("deep operation must not overflow a 2 MiB stack")
}

fn canonical_on_small_stack(source: String) -> String {
    on_two_mib_stack(move || canonicalize_json_object(&source).expect("canonicalize deep object"))
}

fn parsed_on_small_stack(canonical: String) -> kala_persistence::CanonicalValue {
    on_two_mib_stack(move || parse_json_object(&canonical).expect("parse deep object"))
}

#[test]
fn deep_journal_and_snapshot_lifecycle() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("deep-lifecycle.sqlite3");

    let first_source = deep_source(20_000, "persist-first");
    let first_canonical = canonical_on_small_stack(first_source.clone());
    assert_eq!(first_canonical, first_source);
    let first_value = parsed_on_small_stack(first_canonical.clone());
    let first_typed_canonical = on_two_mib_stack({
        let value = first_value.clone();
        move || canonicalize_object_value(&value).expect("encode typed value")
    });
    assert_eq!(first_typed_canonical, first_canonical);

    let second_source = array_chain_source(15_000);
    let second_canonical = canonical_on_small_stack(second_source.clone());
    let snapshot_source = deep_source(10_000, "persist-snapshot");
    let snapshot_canonical = canonical_on_small_stack(snapshot_source.clone());
    let snapshot_value = parsed_on_small_stack(snapshot_canonical.clone());

    let (head, first_sequence, second_sequence, snapshot_id) = {
        let database = Database::open(&path).expect("open");
        let journal = EventJournal::new(&database).expect("journal");
        let first = journal
            .append_value(
                "domain.deep",
                "deep-rust",
                &first_value,
                &AppendOptions::default()
                    .with_workflow_id("flow-deep")
                    .with_run_id("run-deep")
                    .with_occurred_at(fixed_time(0)),
            )
            .expect("append deep typed event");
        assert_eq!(first.payload_json, first_canonical);
        assert_eq!(first.digest, sha256_text(&first_canonical));

        let second = journal
            .append_with(
                "domain.deep",
                "deep-rust",
                &second_source,
                &AppendOptions::default()
                    .with_workflow_id("flow-deep")
                    .with_occurred_at(fixed_time(1)),
            )
            .expect("append deep array event");
        assert_eq!(second.payload_json, second_canonical);

        let snapshot = SnapshotStore::new(&database)
            .save_value(
                "snapshot-deep-rust-000001",
                "flow-deep",
                &snapshot_value,
                Some(fixed_time(2)),
            )
            .expect("save deep snapshot");
        assert_eq!(snapshot.state_json, snapshot_canonical);
        assert_eq!(snapshot.digest, sha256_text(&snapshot_canonical));

        // get / read / filtered reads all re-verify the complete chain and
        // return the deep canonical bytes unchanged.
        let fetched = journal.get(1).expect("get first").expect("first exists");
        assert_eq!(fetched.payload_json, first_canonical);
        let read = journal.read().expect("read");
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].payload_json, first_canonical);
        assert_eq!(read[1].payload_json, second_canonical);

        let filtered = journal
            .read_filtered(&EventFilter {
                event_type: Some("domain.deep".to_owned()),
                aggregate_id: Some("deep-rust".to_owned()),
                workflow_id: Some("flow-deep".to_owned()),
                run_id: Some("run-deep".to_owned()),
                after_sequence: 0,
                limit: Some(10),
            })
            .expect("filtered read");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].sequence, first.sequence);
        assert_eq!(filtered[0].payload_json, first_canonical);

        // Parsing the returned payload exercises the deep lifecycle again.
        let payload = on_two_mib_stack({
            let event = fetched.clone();
            move || event.payload().expect("parse stored deep payload")
        });
        let payload_canonical = on_two_mib_stack(move || {
            canonicalize_object_value(&payload).expect("re-encode stored deep payload")
        });
        assert_eq!(payload_canonical, first_canonical);

        let verification = journal.verify_chain().expect("verify");
        assert_eq!(verification.event_count, 2);
        assert_eq!(verification.head_sequence, Some(2));
        let head = verification.head_event_digest.expect("head");

        let stored_snapshot = SnapshotStore::new(&database)
            .get("snapshot-deep-rust-000001")
            .expect("get snapshot")
            .expect("snapshot exists");
        assert_eq!(stored_snapshot.state_json, snapshot_canonical);
        let latest = SnapshotStore::new(&database)
            .latest("flow-deep")
            .expect("latest")
            .expect("latest exists");
        assert_eq!(latest.state_json, snapshot_canonical);
        assert_eq!(
            SnapshotStore::new(&database)
                .restore("snapshot-deep-rust-000001")
                .expect("restore"),
            Some(snapshot_canonical.clone())
        );
        assert_eq!(
            SnapshotStore::new(&database)
                .restore_latest("flow-deep")
                .expect("restore latest"),
            Some(snapshot_canonical.clone())
        );
        (
            head,
            first.sequence,
            second.sequence,
            "snapshot-deep-rust-000001",
        )
    };

    // Drop the deep typed values on the ordinary stack boundary, then reopen
    // the database and re-verify every deep byte.
    on_two_mib_stack(move || {
        drop(first_value);
        drop(snapshot_value);
    });

    let database = Database::open(&path).expect("reopen");
    let journal = EventJournal::new(&database).expect("reopened journal");
    let verification = journal.verify_chain().expect("verify reopened");
    assert_eq!(verification.event_count, 2);
    assert_eq!(verification.head_event_digest, Some(head));
    assert_eq!(
        journal
            .get(first_sequence)
            .expect("get")
            .unwrap()
            .payload_json,
        first_canonical
    );
    assert_eq!(
        journal
            .get(second_sequence)
            .expect("get")
            .unwrap()
            .payload_json,
        second_canonical
    );
    assert_eq!(
        SnapshotStore::new(&database)
            .restore_latest("flow-deep")
            .expect("restore reopened"),
        Some(snapshot_canonical)
    );
    assert_eq!(
        SnapshotStore::new(&database)
            .get(snapshot_id)
            .expect("snapshot")
            .expect("snapshot exists")
            .sequence,
        1
    );
}

#[test]
fn deep_payload_on_a_copied_fixture_database() {
    let copy = copy_fixture_database();
    let source = deep_source(20_000, "fixture-deep");
    let canonical = canonical_on_small_stack(source.clone());
    let snapshot_source = deep_source(5_000, "fixture-snapshot");
    let snapshot_canonical = canonical_on_small_stack(snapshot_source.clone());

    let head = {
        let database = Database::open(&copy.path).expect("open fixture copy");
        let journal = EventJournal::new(&database).expect("fixture journal");
        assert_eq!(
            journal.verify_chain().expect("verify fixture").event_count,
            5
        );
        let event = journal
            .append_with(
                "domain.deep.fixture",
                "deep-fixture",
                &source,
                &AppendOptions::default()
                    .with_workflow_id("flow-deep")
                    .with_run_id("run-deep")
                    .with_occurred_at(fixed_time(100)),
            )
            .expect("append deep event to fixture copy");
        assert_eq!(event.sequence, 6);
        assert_eq!(event.payload_json, canonical);
        let snapshot = SnapshotStore::new(&database)
            .save(
                "snapshot-deep-fixture-000001",
                "flow-deep",
                &snapshot_source,
                Some(fixed_time(101)),
            )
            .expect("save deep snapshot on fixture copy");
        assert_eq!(snapshot.state_json, snapshot_canonical);
        let verification = journal.verify_chain().expect("verify extended fixture");
        assert_eq!(verification.event_count, 6);
        assert_eq!(journal.read().expect("read extended fixture").len(), 6);
        verification.head_event_digest.expect("head")
    };

    let database = Database::open(&copy.path).expect("reopen fixture copy");
    let journal = EventJournal::new(&database).expect("reopened fixture journal");
    assert_eq!(
        journal.verify_chain().expect("verify").head_event_digest,
        Some(head)
    );
    assert_eq!(
        journal
            .get(6)
            .expect("get deep event")
            .unwrap()
            .payload_json,
        canonical
    );
    assert_eq!(
        SnapshotStore::new(&database)
            .restore_latest("flow-deep")
            .expect("restore"),
        Some(snapshot_canonical)
    );
}
