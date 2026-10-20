//! The security properties of the native Studio session journal.
//!
//! The cases are the ones the Python module's own behaviour implies: the
//! storage stays inside the caller's directory, the files stay owner-only,
//! an unsafe identifier never reaches the database, a path never escapes the
//! artifact root, and a broken chain is never extended.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use kala_studio_journal::{JournalConfig, StudioSessionJournal};
use tempfile::TempDir;

fn open(directory: &Path) -> StudioSessionJournal {
    StudioSessionJournal::open(directory).expect("open the journal")
}

#[test]
fn the_data_directory_and_the_journal_file_stay_owner_only() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    let directory_mode = fs::metadata(journal.data_dir())
        .expect("stat the data directory")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        directory_mode, 0o700,
        "the data directory must be owner-only"
    );

    let file_mode = fs::metadata(journal.database_path())
        .expect("stat the journal file")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(file_mode, 0o600, "the journal file must be owner-only");
}

#[test]
fn the_journal_writes_nothing_outside_its_data_directory() {
    let directory = TempDir::new().expect("temp dir");
    let data_dir = directory.path().join("studio-data");
    let journal = open(&data_dir);
    journal
        .append_json("chat.turn", r#"{"text":"hello"}"#)
        .expect("append");
    journal.put_artifact(b"bytes").expect("store");

    let mut entries: Vec<String> = fs::read_dir(&data_dir)
        .expect("read the data directory")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    // The write-ahead log and the shared-memory index belong to the journal
    // file, so they live in the same directory.
    assert_eq!(
        entries,
        vec![
            "artifacts",
            "studio-journal.sqlite3",
            "studio-journal.sqlite3-shm",
            "studio-journal.sqlite3-wal",
        ]
    );
}

#[test]
fn an_unsafe_identifier_never_reaches_the_database() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    for identifier in [
        "",
        " leading space",
        "trailing space ",
        "../escape",
        "with/slash",
        "with\\backslash",
        "with space",
        "with\0null",
        &"a".repeat(129),
    ] {
        let error = journal
            .append_json(identifier, "{}")
            .expect_err("an unsafe identifier must be refused");
        assert_eq!(
            error.kind(),
            kala_studio_journal::ErrorKind::Value,
            "the identifier {identifier:?} must be refused"
        );
    }

    let error = journal
        .append_json_with(
            "chat.turn",
            "{}",
            &kala_studio_journal::AppendOptions::default().with_aggregate_id("../escape"),
        )
        .expect_err("an unsafe aggregate must be refused");
    assert_eq!(error.to_string(), "aggregate_id must be a safe identifier");

    assert_eq!(journal.verify().expect("verify").event_count, 0);
}

#[test]
fn a_valid_identifier_is_normalized_the_python_way() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    let event = journal
        .append_json("  chat.turn  ", "{}")
        .expect("a padded identifier is stripped, not refused");

    assert_eq!(event.event_type, "chat.turn");
}

#[test]
fn the_bounds_of_the_journal_must_be_positive() {
    let directory = TempDir::new().expect("temp dir");
    for config in [
        JournalConfig {
            max_payload_bytes: 0,
            ..JournalConfig::default()
        },
        JournalConfig {
            max_artifact_bytes: 0,
            ..JournalConfig::default()
        },
        JournalConfig {
            max_page_size: 0,
            ..JournalConfig::default()
        },
        JournalConfig {
            max_page_size: -1,
            ..JournalConfig::default()
        },
    ] {
        let error = StudioSessionJournal::open_with_config(directory.path(), config)
            .expect_err("a bound that is not positive must be refused");
        assert_eq!(error.kind(), kala_studio_journal::ErrorKind::Value);
    }
}

#[test]
fn an_artifact_digest_cannot_escape_the_artifact_root() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    for digest in [
        "../../etc/passwd",
        "/etc/passwd",
        "..",
        "",
        &"a".repeat(63),
        &"z".repeat(64),
    ] {
        let error = journal
            .get_artifact(digest)
            .expect_err("an invalid digest must be refused");
        assert_eq!(
            error.kind(),
            kala_studio_journal::ErrorKind::Value,
            "the digest {digest:?} must be refused"
        );
    }
}

#[test]
fn a_symlinked_artifact_root_is_refused() {
    let directory = TempDir::new().expect("temp dir");
    let data_dir = directory.path().join("studio-data");
    fs::create_dir_all(&data_dir).expect("create the data directory");
    let elsewhere = directory.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).expect("create the target directory");
    std::os::unix::fs::symlink(&elsewhere, data_dir.join("artifacts")).expect("create the symlink");

    let error = StudioSessionJournal::open(&data_dir)
        .expect_err("a symlinked artifact root must be refused");
    assert!(
        error.to_string().contains("symlink"),
        "the error must name the symlink: {error}"
    );
}

#[test]
fn a_broken_chain_is_never_extended() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());
    for index in 0..5 {
        journal
            .append_json("chat.turn", &format!(r#"{{"index":{index}}}"#))
            .expect("append");
    }

    // Break the middle of the chain.
    let connection = rusqlite::Connection::open(journal.database_path()).expect("open");
    connection
        .execute(
            "UPDATE studio_events SET payload_json = '{\"index\":999}' WHERE sequence = 3",
            [],
        )
        .expect("tamper");
    connection.close().expect("close");

    // The documented trade-off: an edit in the middle of the chain is found
    // by the next full walk, so an append made before that walk extends the
    // chain as it stands. The break is reported as soon as the chain is
    // walked in full.
    let event = journal
        .append_json("chat.turn", r#"{"index":5}"#)
        .expect("the append extends the chain as it stands");
    assert_eq!(event.sequence, 6);

    let verification = journal.verify().expect("verify");
    assert!(!verification.ok);
    assert_eq!(verification.first_bad_sequence, Some(3));
    assert_eq!(verification.event_count, 6);

    // Once the break is known, the chain is never extended again.
    let error = journal
        .append_json("chat.turn", r#"{"index":6}"#)
        .expect_err("a broken chain must not be extended");
    assert_eq!(error.kind(), kala_studio_journal::ErrorKind::Integrity);
    assert_eq!(
        error.to_string(),
        "the studio session journal is broken at sequence 3; refusing to extend it"
    );

    // The history is still readable.
    assert_eq!(journal.page(0, 10).expect("page").len(), 6);
}

#[test]
fn an_oversized_payload_writes_nothing() {
    let directory = TempDir::new().expect("temp dir");
    let journal = StudioSessionJournal::open_with_config(
        directory.path(),
        JournalConfig {
            max_payload_bytes: 64,
            ..JournalConfig::default()
        },
    )
    .expect("open the journal");

    let error = journal
        .append_json(
            "chat.turn",
            &format!(r#"{{"text":"{}"}}"#, "x".repeat(1_000)),
        )
        .expect_err("an oversized payload must be refused");
    assert_eq!(
        error.to_string(),
        "event payload exceeds the configured size limit"
    );
    assert_eq!(journal.verify().expect("verify").event_count, 0);
    assert_eq!(journal.page(0, 10).expect("page").len(), 0);
}

#[test]
fn an_oversized_artifact_writes_nothing() {
    let directory = TempDir::new().expect("temp dir");
    let journal = StudioSessionJournal::open_with_config(
        directory.path(),
        JournalConfig {
            max_artifact_bytes: 16,
            ..JournalConfig::default()
        },
    )
    .expect("open the journal");

    let error = journal
        .put_artifact(&[b'x'; 17])
        .expect_err("an oversized artifact must be refused");
    assert_eq!(
        error.to_string(),
        "artifact exceeds the configured size limit"
    );
    let objects = journal.artifact_root().join("objects");
    let stored = fs::read_dir(&objects)
        .expect("read the objects directory")
        .flatten()
        .count();
    assert_eq!(stored, 0, "no object may be published");
}
