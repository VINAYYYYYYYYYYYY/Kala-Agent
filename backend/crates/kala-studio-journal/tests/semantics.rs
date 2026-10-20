//! The behaviour of the native Studio session journal.
//!
//! The cases are the ones the Python module's own test suite pins, ported one
//! for one: the append contract, the page contract, the verification
//! contract, the tamper detection, the artifact store, the limits, the
//! placement and the cached head.

use std::fs;
use std::path::Path;

use kala_json::{EventChainFields, GENESIS_PREVIOUS_DIGEST, chain_digest};
use kala_persistence::sha256_text;
use kala_studio_journal::{
    ARTIFACT_DIRECTORY_NAME, AppendOptions, DEFAULT_AGGREGATE_ID, JOURNAL_FILE_NAME, JournalConfig,
    JournalEventType, StudioSessionJournal,
};
use rusqlite::Connection;
use tempfile::TempDir;

fn sha256_hex(data: &[u8]) -> String {
    sha256_text(std::str::from_utf8(data).expect("utf-8")).to_hex()
}

fn open(directory: &Path) -> StudioSessionJournal {
    StudioSessionJournal::open(directory).expect("open the journal")
}

fn tamper(path: &Path, statement: &str) {
    let connection = Connection::open(path).expect("open the tampered database");
    connection.execute(statement, []).expect("tamper");
    connection.close().expect("close the tampered database");
}

// --- append ---

#[test]
fn append_returns_the_sequence_and_the_payload_digest() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    let event = journal
        .append_json(JournalEventType::ChatTurn.as_str(), r#"{"text":"hello"}"#)
        .expect("append");

    assert_eq!(event.sequence, 1);
    assert_eq!(event.event_type, "chat.turn");
    assert_eq!(event.digest, sha256_hex(event.payload_json.as_bytes()));
    assert_eq!(
        event.previous_event_digest,
        GENESIS_PREVIOUS_DIGEST.to_hex()
    );
    let payload = event.payload.as_ref().expect("a decoded payload");
    assert_eq!(
        kala_persistence::canonicalize_object_value(payload).expect("canonical"),
        r#"{"text":"hello"}"#
    );
}

#[test]
fn append_uses_the_core_chain_preimage() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    let event = journal
        .append_json("design.intent", r#"{"template":"l_bracket","width_mm":60}"#)
        .expect("append");

    let expected = chain_digest(&EventChainFields {
        sequence: 1,
        event_type: "design.intent".to_owned(),
        aggregate_id: DEFAULT_AGGREGATE_ID.to_owned(),
        workflow_id: None,
        run_id: None,
        occurred_at: event.occurred_at.clone(),
        payload_digest: kala_json::Sha256Digest::from_hex(&event.digest).expect("digest"),
        previous_event_digest: GENESIS_PREVIOUS_DIGEST,
    });
    assert_eq!(event.event_digest, expected.to_hex());
}

#[test]
fn append_stores_canonical_payload_json() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    let event = journal
        .append_json("chat.turn", r#"{"b":1,"a":[2,{"d":4,"c":3}]}"#)
        .expect("append");

    assert_eq!(event.payload_json, r#"{"a":[2,{"c":3,"d":4}],"b":1}"#);
}

#[test]
fn append_refuses_a_payload_that_is_not_json() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    let error = journal
        .append_json("chat.turn", "{not json")
        .expect_err("invalid JSON must be refused");
    assert_eq!(error.kind(), kala_studio_journal::ErrorKind::Value);
    assert_eq!(error.to_string(), "value must be a canonical JSON object");

    assert_eq!(journal.verify().expect("verify").event_count, 0);
}

#[test]
fn append_refuses_a_payload_that_is_not_a_mapping() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    let error = journal
        .append_json("chat.turn", r#""hello""#)
        .expect_err("a non-object payload must be refused");
    assert_eq!(error.kind(), kala_studio_journal::ErrorKind::Type);
    assert_eq!(error.to_string(), "event payload must be a mapping");

    assert_eq!(journal.verify().expect("verify").event_count, 0);
}

#[test]
fn append_refuses_an_unsafe_event_type() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    let error = journal
        .append_json("not a safe event type", r#"{"text":"hello"}"#)
        .expect_err("an unsafe event type must be refused");
    assert_eq!(error.kind(), kala_studio_journal::ErrorKind::Value);
    assert_eq!(error.to_string(), "event_type must be a safe identifier");
}

// --- page ---

#[test]
fn page_returns_events_in_sequence_order_after_the_cursor() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    for index in 0..5 {
        journal
            .append_json("chat.turn", &format!(r#"{{"index":{index}}}"#))
            .expect("append");
    }

    let first = journal.page(0, 3).expect("first page");
    let second = journal.page(3, 10).expect("second page");
    let third = journal.page(5, 10).expect("third page");

    assert_eq!(
        first.iter().map(|event| event.sequence).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(
        second
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![4, 5]
    );
    assert!(third.is_empty());
    assert_eq!(
        first
            .iter()
            .map(|event| event.payload_json.clone())
            .collect::<Vec<_>>(),
        vec![r#"{"index":0}"#, r#"{"index":1}"#, r#"{"index":2}"#]
    );
}

#[test]
fn page_is_bounded_by_the_maximum_page_size() {
    let directory = TempDir::new().expect("temp dir");
    let journal = StudioSessionJournal::open_with_config(
        directory.path(),
        JournalConfig {
            max_page_size: 4,
            ..JournalConfig::default()
        },
    )
    .expect("open the journal");
    for index in 0..10 {
        journal
            .append_json("chat.turn", &format!(r#"{{"index":{index}}}"#))
            .expect("append");
    }

    let page = journal.page(0, 1_000).expect("page");

    assert_eq!(page.len(), 4);
    assert_eq!(
        page.iter().map(|event| event.sequence).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn page_rejects_an_invalid_cursor_or_limit() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    let error = journal
        .page(-1, 10)
        .expect_err("a negative cursor must be refused");
    assert_eq!(error.to_string(), "after_sequence must be nonnegative");
    let error = journal
        .page(0, 0)
        .expect_err("a zero limit must be refused");
    assert_eq!(error.to_string(), "limit must be positive");
}

#[test]
fn get_returns_one_event_or_nothing() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    journal
        .append_json("chat.turn", r#"{"text":"hello"}"#)
        .expect("append");

    let event = journal.get(1).expect("get").expect("the event exists");
    assert_eq!(event.sequence, 1);
    assert!(journal.get(2).expect("get").is_none());
    let error = journal
        .get(-1)
        .expect_err("a negative sequence must be refused");
    assert_eq!(error.to_string(), "sequence must be nonnegative");
}

// --- verify ---

#[test]
fn verify_reports_an_intact_chain() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    let mut events = Vec::new();
    for index in 0..3 {
        events.push(
            journal
                .append_json("chat.turn", &format!(r#"{{"index":{index}}}"#))
                .expect("append"),
        );
    }

    let verification = journal.verify().expect("verify");

    assert!(verification.ok);
    assert_eq!(verification.event_count, 3);
    assert_eq!(
        verification.head_digest,
        Some(events.last().expect("an event").event_digest.clone())
    );
    assert_eq!(verification.first_bad_sequence, None);
}

#[test]
fn an_empty_journal_verifies() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    let verification = journal.verify().expect("verify");

    assert!(verification.ok);
    assert_eq!(verification.event_count, 0);
    assert_eq!(verification.head_digest, None);
    assert_eq!(verification.first_bad_sequence, None);
    assert_eq!(journal.head_digest().expect("head"), None);
}

#[test]
fn verify_detects_a_tampered_payload() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    journal
        .append_json("chat.turn", r#"{"text":"hello"}"#)
        .expect("append");
    journal
        .append_json("design.measured", r#"{"volume_mm3":35571.416047}"#)
        .expect("append");
    journal
        .append_json("design.checked", r#"{"verified":true}"#)
        .expect("append");

    tamper(
        journal.database_path(),
        "UPDATE studio_events SET payload_json = '{\"volume_mm3\":35571.416048}' WHERE sequence = 2",
    );

    let verification = journal.verify().expect("verify");

    assert!(!verification.ok);
    assert_eq!(verification.first_bad_sequence, Some(2));
    assert_eq!(verification.event_count, 3);
    assert_eq!(
        verification.head_digest,
        Some(
            journal
                .get(1)
                .expect("get")
                .expect("an event")
                .event_digest
                .clone()
        )
    );
}

#[test]
fn verify_detects_a_tampered_event_digest() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    journal
        .append_json("chat.turn", r#"{"text":"hello"}"#)
        .expect("append");
    journal
        .append_json("design.measured", r#"{"volume_mm3":1.0}"#)
        .expect("append");

    tamper(
        journal.database_path(),
        "UPDATE studio_events SET event_digest = 'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' WHERE sequence = 2",
    );

    let verification = journal.verify().expect("verify");

    assert!(!verification.ok);
    assert_eq!(verification.first_bad_sequence, Some(2));
}

#[test]
fn verify_detects_a_dropped_event() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    journal
        .append_json("chat.turn", r#"{"text":"hello"}"#)
        .expect("append");
    journal
        .append_json("design.measured", r#"{"volume_mm3":1.0}"#)
        .expect("append");
    journal
        .append_json("design.checked", r#"{"verified":true}"#)
        .expect("append");

    tamper(
        journal.database_path(),
        "DELETE FROM studio_events WHERE sequence = 2",
    );

    let verification = journal.verify().expect("verify");

    assert!(!verification.ok);
    assert_eq!(verification.first_bad_sequence, Some(2));
    assert_eq!(verification.event_count, 2);
}

#[test]
fn a_tampered_journal_is_not_extended() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    journal
        .append_json("chat.turn", r#"{"text":"hello"}"#)
        .expect("append");

    tamper(
        journal.database_path(),
        "UPDATE studio_events SET payload_json = '{\"text\":\"hellp\"}' WHERE sequence = 1",
    );

    let error = journal
        .append_json("chat.turn", r#"{"text":"again"}"#)
        .expect_err("a broken chain must not be extended");
    assert_eq!(error.kind(), kala_studio_journal::ErrorKind::Integrity);
    assert_eq!(
        error.to_string(),
        "the studio session journal is broken at sequence 1; refusing to extend it"
    );

    assert_eq!(journal.verify().expect("verify").event_count, 1);
}

#[test]
fn a_tampered_journal_is_still_readable() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    journal
        .append_json("chat.turn", r#"{"text":"hello"}"#)
        .expect("append");

    tamper(
        journal.database_path(),
        "UPDATE studio_events SET payload_json = 'not json at all' WHERE sequence = 1",
    );

    let events = journal.page(0, 10).expect("page");

    assert_eq!(events.len(), 1);
    assert!(events[0].payload.is_none());
    assert_eq!(events[0].payload_json, "not json at all");
}

// --- artifacts ---

#[test]
fn artifact_round_trip_returns_the_sha256() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    let data = b"ISO-10303-21;\nEND-ISO-10303-21;\n";

    let digest = journal.put_artifact(data).expect("store the artifact");

    assert_eq!(digest, sha256_hex(data));
    assert_eq!(digest.len(), 64);
    assert_eq!(journal.get_artifact(&digest).expect("read"), data);
    assert_eq!(
        journal.get_artifact_text(&digest).expect("read"),
        String::from_utf8(data.to_vec()).expect("utf-8")
    );
    assert!(journal.has_artifact(&digest).expect("contains"));
}

#[test]
fn the_same_bytes_are_stored_once() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    let first = journal.put_artifact(b"same bytes").expect("store");
    let second = journal.put_artifact(b"same bytes").expect("store");

    assert_eq!(first, second);
    assert_eq!(journal.get_artifact(&first).expect("read"), b"same bytes");
}

#[test]
fn an_unknown_artifact_is_refused() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    let error = journal
        .get_artifact(&"0".repeat(64))
        .expect_err("an unknown artifact must be refused");
    assert_eq!(error.kind(), kala_studio_journal::ErrorKind::NotFound);
    assert!(!journal.has_artifact(&"0".repeat(64)).expect("contains"));
}

#[test]
fn the_artifact_size_limit_is_enforced() {
    let directory = TempDir::new().expect("temp dir");
    let journal = StudioSessionJournal::open_with_config(
        directory.path(),
        JournalConfig {
            max_artifact_bytes: 64,
            ..JournalConfig::default()
        },
    )
    .expect("open the journal");

    let error = journal
        .put_artifact(&[b'x'; 65])
        .expect_err("an oversized artifact must be refused");
    assert_eq!(error.kind(), kala_studio_journal::ErrorKind::Value);
    assert_eq!(
        error.to_string(),
        "artifact exceeds the configured size limit"
    );

    assert!(
        !journal
            .has_artifact(&sha256_hex(&[b'x'; 64]))
            .expect("contains")
    );
}

// --- limits, placement and concurrency ---

#[test]
fn the_payload_size_limit_is_enforced_and_writes_nothing() {
    let directory = TempDir::new().expect("temp dir");
    let journal = StudioSessionJournal::open_with_config(
        directory.path(),
        JournalConfig {
            max_payload_bytes: 128,
            ..JournalConfig::default()
        },
    )
    .expect("open the journal");

    let error = journal
        .append_json("chat.turn", &format!(r#"{{"text":"{}"}}"#, "x".repeat(512)))
        .expect_err("an oversized payload must be refused");
    assert_eq!(
        error.to_string(),
        "event payload exceeds the configured size limit"
    );

    let verification = journal.verify().expect("verify");
    assert!(verification.ok);
    assert_eq!(verification.event_count, 0);
}

#[test]
fn the_journal_lives_in_the_supplied_data_directory() {
    let directory = TempDir::new().expect("temp dir");
    let data_dir = directory.path().join("studio-data");
    let journal = open(&data_dir);
    journal
        .append_json("chat.turn", r#"{"text":"hello"}"#)
        .expect("append");
    let resolved = data_dir.canonicalize().expect("resolve");

    assert_eq!(journal.database_path(), resolved.join(JOURNAL_FILE_NAME));
    assert!(journal.database_path().is_file());
    assert_eq!(
        journal.artifact_root(),
        resolved.join(ARTIFACT_DIRECTORY_NAME)
    );
    assert_eq!(journal.data_dir(), resolved.as_path());
}

#[test]
fn the_journal_never_reaches_the_core_application() {
    // The Studio journal is standalone: it composes no core application. The
    // Python module pins this with an AST check over its imports; the native
    // equivalent is the dependency list of this crate.
    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("read the manifest");
    for forbidden in [
        "kala-application =",
        "kala-runtime =",
        "kala-tools =",
        "kala-studio =",
        "kala-repositories =",
        "kala-kernel =",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "the journal must not depend on {forbidden}"
        );
    }
}

#[test]
fn concurrent_appends_keep_the_chain_consistent() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    let workers = 8;
    let per_worker = 10;

    std::thread::scope(|scope| {
        for worker in 0..workers {
            let journal = &journal;
            scope.spawn(move || {
                for step in 0..per_worker {
                    journal
                        .append_json(
                            "chat.turn",
                            &format!(r#"{{"worker":{worker},"step":{step}}}"#),
                        )
                        .expect("append");
                }
            });
        }
    });

    let verification = journal.verify().expect("verify");
    assert!(verification.ok);
    assert_eq!(verification.event_count, (workers * per_worker) as i64);
    let events = journal
        .page(0, (workers * per_worker) as i64)
        .expect("page");
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        (1..=(workers * per_worker) as i64).collect::<Vec<_>>()
    );
}

#[test]
fn readers_and_writers_share_the_journal_safely() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    std::thread::scope(|scope| {
        for writer in 0..4 {
            let journal = &journal;
            scope.spawn(move || {
                for step in 0..10 {
                    journal
                        .append_json(
                            "chat.turn",
                            &format!(r#"{{"writer":{writer},"step":{step}}}"#),
                        )
                        .expect("append");
                }
            });
        }
        for _ in 0..2 {
            let journal = &journal;
            scope.spawn(move || {
                for _ in 0..10 {
                    journal.page(0, 5).expect("page");
                    journal.verify().expect("verify");
                }
            });
        }
    });

    let verification = journal.verify().expect("verify");
    assert!(verification.ok);
    assert_eq!(verification.event_count, 40);
}

#[test]
fn the_stored_payload_is_the_canonical_bytes_that_were_hashed() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    let event = journal
        .append_json("chat.turn", r#"{"z":1,"a":{"n":null,"m":true}}"#)
        .expect("append");

    assert_eq!(event.payload_json, r#"{"a":{"m":true,"n":null},"z":1}"#);
    assert_eq!(event.digest, sha256_hex(event.payload_json.as_bytes()));
}

// --- the cached head: what an append verifies, and what it costs ---

#[test]
fn appends_after_a_long_history_do_not_walk_the_whole_chain() {
    // The chain is walked once, when the journal is opened, and the verified
    // head is cached from then on. A steady stream of appends must not walk
    // it again: a journal that persists on disk across sessions would
    // otherwise cost more and more per event.
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    for index in 0..2_000 {
        journal
            .append_json("chat.turn", &format!(r#"{{"index":{index}}}"#))
            .expect("append");
    }

    let before = journal.diagnostics();
    for index in 0..100 {
        journal
            .append_json("chat.turn", &format!(r#"{{"index":{index}}}"#))
            .expect("append");
    }
    let after = journal.diagnostics();

    assert_eq!(after.chain_walks, before.chain_walks);
    assert_eq!(after.fast_path_appends, before.fast_path_appends + 100);
    assert_eq!(after.fallback_appends, before.fallback_appends);
    let verification = journal.verify().expect("verify");
    assert_eq!(verification.event_count, 2_100);
    assert!(verification.ok);
}

#[test]
fn a_tampered_head_is_detected_at_the_next_append() {
    // Editing the newest event is caught by the next write.
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    journal
        .append_json("chat.turn", r#"{"text":"hello"}"#)
        .expect("append");
    journal
        .append_json("design.checked", r#"{"verified":true}"#)
        .expect("append");

    tamper(
        journal.database_path(),
        "UPDATE studio_events SET payload_json = '{\"verified\":false}' WHERE sequence = 2",
    );

    let error = journal
        .append_json("chat.turn", r#"{"text":"again"}"#)
        .expect_err("a tampered head must be refused");
    assert_eq!(error.kind(), kala_studio_journal::ErrorKind::Integrity);

    let verification = journal.verify().expect("verify");
    assert!(!verification.ok);
    assert_eq!(verification.first_bad_sequence, Some(2));
    assert_eq!(verification.event_count, 2);
}

#[test]
fn a_tampered_middle_is_detected_by_verify_and_on_reopen() {
    // An edit in the middle needs a full walk: verify(), then the next open.
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    for index in 0..5 {
        journal
            .append_json("chat.turn", &format!(r#"{{"index":{index}}}"#))
            .expect("append");
    }

    tamper(
        journal.database_path(),
        "UPDATE studio_events SET payload_json = '{\"index\":999}' WHERE sequence = 3",
    );

    let verification = journal.verify().expect("verify");
    assert!(!verification.ok);
    assert_eq!(verification.first_bad_sequence, Some(3));
    assert_eq!(verification.event_count, 5);
    journal.close().expect("close");

    let reopened = open(directory.path());
    let reopened_verification = reopened.verify().expect("verify");
    assert!(!reopened_verification.ok);
    assert_eq!(reopened_verification.first_bad_sequence, Some(3));
    let error = reopened
        .append_json("chat.turn", r#"{"index":5}"#)
        .expect_err("a broken chain must not be extended");
    assert_eq!(error.kind(), kala_studio_journal::ErrorKind::Integrity);
}

#[test]
fn a_second_instance_on_the_same_file_extends_the_chain() {
    // Two journals on one file extend one chain; they never fork it.
    let directory = TempDir::new().expect("temp dir");
    let first = open(directory.path());
    first
        .append_json("chat.turn", r#"{"text":"one"}"#)
        .expect("append");

    let second = open(directory.path());
    second
        .append_json("chat.turn", r#"{"text":"two"}"#)
        .expect("append");

    let before = first.diagnostics();
    let event = first
        .append_json("chat.turn", r#"{"text":"three"}"#)
        .expect("append");
    let after = first.diagnostics();

    // The cached head no longer matched the file, so the chain was re-walked.
    assert!(after.chain_walks > before.chain_walks);
    assert_eq!(event.sequence, 3);

    let events = first.page(0, 10).expect("page");
    assert_eq!(
        events
            .iter()
            .map(|event| event.payload_json.clone())
            .collect::<Vec<_>>(),
        vec![
            r#"{"text":"one"}"#,
            r#"{"text":"two"}"#,
            r#"{"text":"three"}"#
        ]
    );
    assert_eq!(events[2].previous_event_digest, events[1].event_digest);

    let verification = first.verify().expect("verify");
    assert!(verification.ok);
    assert_eq!(verification.event_count, 3);

    // The refreshed head lets the first instance carry on from the new head.
    assert_eq!(
        second
            .append_json("chat.turn", r#"{"text":"four"}"#)
            .expect("append")
            .sequence,
        4
    );
    assert_eq!(first.verify().expect("verify").event_count, 4);
}

#[test]
fn a_broken_chain_is_not_extended_after_a_reopen() {
    // A journal opened on a broken chain refuses every write.
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    journal
        .append_json("chat.turn", r#"{"text":"hello"}"#)
        .expect("append");
    journal
        .append_json("chat.turn", r#"{"text":"again"}"#)
        .expect("append");
    journal.close().expect("close");

    tamper(
        &directory.path().join(JOURNAL_FILE_NAME),
        "UPDATE studio_events SET payload_json = '{\"text\":\"hellp\"}' WHERE sequence = 1",
    );

    let reopened = open(directory.path());
    let error = reopened
        .append_json("chat.turn", r#"{"text":"third"}"#)
        .expect_err("a broken chain must not be extended");
    assert_eq!(error.kind(), kala_studio_journal::ErrorKind::Integrity);
    assert_eq!(
        reopened.verify().expect("verify").first_bad_sequence,
        Some(1)
    );
    assert_eq!(reopened.page(0, 10).expect("page")[0].sequence, 1);
}

#[test]
fn a_repaired_chain_is_writable_again_after_a_verification() {
    // The refusal is lifted when a later verification finds the chain intact.
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    journal
        .append_json("chat.turn", r#"{"text":"hello"}"#)
        .expect("append");
    journal.close().expect("close");

    tamper(
        &directory.path().join(JOURNAL_FILE_NAME),
        "UPDATE studio_events SET payload_json = '{\"text\":\"hellp\"}' WHERE sequence = 1",
    );

    let reopened = open(directory.path());
    let error = reopened
        .append_json("chat.turn", r#"{"text":"again"}"#)
        .expect_err("a broken chain must not be extended");
    assert_eq!(error.kind(), kala_studio_journal::ErrorKind::Integrity);

    tamper(
        &directory.path().join(JOURNAL_FILE_NAME),
        "UPDATE studio_events SET payload_json = '{\"text\":\"hello\"}' WHERE sequence = 1",
    );

    assert!(reopened.verify().expect("verify").ok);
    let event = reopened
        .append_json("chat.turn", r#"{"text":"again"}"#)
        .expect("append");
    assert_eq!(event.sequence, 2);
}

#[test]
fn a_closed_journal_refuses_every_operation() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    journal.close().expect("close");
    journal.close().expect("closing twice is harmless");

    let error = journal
        .append_json("chat.turn", "{}")
        .expect_err("a closed journal must refuse");
    assert_eq!(error.kind(), kala_studio_journal::ErrorKind::Closed);
    assert_eq!(error.to_string(), "the studio session journal is closed");
    assert_eq!(
        journal.verify().expect_err("verify").to_string(),
        "the studio session journal is closed"
    );
}

#[test]
fn the_validation_order_is_the_python_order() {
    // The Python module checks the payload type, then the event type, then
    // the aggregate id, then the canonical form of the payload, and finally
    // the size of the canonical payload. The order decides which error a
    // caller sees, so it is pinned here.
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    // The payload type is checked before the identifiers.
    let error = journal
        .append_json("not a safe event type", "\"hello\"")
        .expect_err("a non-object payload must be refused");
    assert_eq!(error.kind(), kala_studio_journal::ErrorKind::Type);
    assert_eq!(error.to_string(), "event payload must be a mapping");

    // The identifiers are checked before the canonical form.
    let mut entries = kala_json::CanonicalObject::new();
    entries.insert(
        kala_json::JsonText::from("value"),
        kala_json::CanonicalValue::Float(f64::NAN),
    );
    let payload = kala_json::CanonicalValue::Object(entries);
    let error = journal
        .append_with("not a safe event type", &payload, &AppendOptions::default())
        .expect_err("an unsafe event type must be refused");
    assert_eq!(error.to_string(), "event_type must be a safe identifier");

    // With a valid identifier, the canonical form is the failure.
    let error = journal
        .append_with("chat.turn", &payload, &AppendOptions::default())
        .expect_err("a non-finite number must be refused");
    assert_eq!(error.kind(), kala_studio_journal::ErrorKind::Value);
    assert_eq!(error.to_string(), "value must be a canonical JSON object");

    // And the size is checked last.
    let journal = StudioSessionJournal::open_with_config(
        directory.path(),
        JournalConfig {
            max_payload_bytes: 4,
            ..JournalConfig::default()
        },
    )
    .expect("open the journal");
    let error = journal
        .append_json("not a safe event type", "{\"a\":1}")
        .expect_err("an unsafe event type must be refused");
    assert_eq!(error.to_string(), "event_type must be a safe identifier");
}

#[test]
fn the_journal_is_send_and_sync() {
    // The Studio server may hand one journal to a thread pool.
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<StudioSessionJournal>();
}

#[test]
fn the_append_options_carry_the_aggregate_and_the_timestamp() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    let timestamp = chrono::DateTime::parse_from_rfc3339("2026-10-20T12:00:00.123456Z")
        .expect("a timestamp")
        .with_timezone(&chrono::Utc);

    let event = journal
        .append_json_with(
            "chat.turn",
            "{}",
            &AppendOptions::default()
                .with_aggregate_id("design-42")
                .with_occurred_at(timestamp),
        )
        .expect("append");

    assert_eq!(event.aggregate_id, "design-42");
    assert_eq!(event.occurred_at, "2026-10-20T12:00:00.123456Z");
    assert_eq!(
        event.timestamp().expect("a timestamp").to_rfc3339(),
        "2026-10-20T12:00:00.123456+00:00"
    );
}
