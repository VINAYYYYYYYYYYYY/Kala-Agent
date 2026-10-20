//! Revision compare-and-swap, approval finality and transaction atomicity.

mod support;

use std::sync::{Arc, Barrier};
use std::thread;

use kala_repositories::{
    ApprovalRequestRepository, ContractModel, DesignRevisionRepository, DesignSpecRepository,
    MemoryRepository, PersistenceError, RepositoryError,
};
use support::{approval, design_revision, design_spec, model};
use tempfile::TempDir;

fn revision_with_spec(
    revision_id: &str,
    sequence: u64,
    spec_id: &str,
) -> kala_repositories::DesignRevision {
    design_revision(revision_id, "design-1", sequence, &design_spec(spec_id))
}

#[test]
fn head_moved_errors_carry_every_python_field_and_write_nothing() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let revisions = DesignRevisionRepository::new(&database);
    let specs = DesignSpecRepository::new(&database);
    let first = revision_with_spec("revision-0", 0, "spec-0");
    revisions.commit_revision(&first, None).expect("first");

    let stale = revision_with_spec("revision-stale", 1, "spec-stale");
    let error = revisions
        .commit_revision(&stale, None)
        .expect_err("stale head");
    let head = error.as_head_moved().expect("head moved");
    assert_eq!(
        head.message,
        "the head of design design-1 moved before the revision was written"
    );
    assert_eq!(head.design_id, "design-1");
    assert_eq!(head.expected_head_revision_id, None);
    assert_eq!(head.actual_head_revision_id.as_deref(), Some("revision-0"));
    assert_eq!(head.expected_sequence.to_string(), "1");
    assert_eq!(head.actual_sequence.to_string(), "0");
    assert_eq!(revisions.get("revision-stale").expect("get"), None);
    assert_eq!(specs.get("spec-stale").expect("get"), None);
    assert_eq!(
        revisions.latest_for_design("design-1").expect("latest"),
        Some(first)
    );

    let wrong_sequence = revision_with_spec("revision-wrong", 2, "spec-wrong");
    let error = revisions
        .commit_revision(&wrong_sequence, Some("revision-0"))
        .expect_err("wrong sequence");
    let head = error.as_head_moved().expect("head moved");
    assert_eq!(
        head.message,
        "revision revision-wrong does not extend the head of design design-1"
    );
    assert_eq!(
        head.expected_head_revision_id.as_deref(),
        Some("revision-0")
    );
    assert_eq!(head.actual_head_revision_id.as_deref(), Some("revision-0"));
    assert_eq!(head.expected_sequence.to_string(), "2");
    assert_eq!(head.actual_sequence.to_string(), "0");
    assert_eq!(revisions.get("revision-wrong").expect("get"), None);
    assert_eq!(specs.get("spec-wrong").expect("get"), None);

    let unknown_head = revision_with_spec("revision-fork", 1, "spec-fork");
    let error = revisions
        .commit_revision(&unknown_head, Some("revision-9"))
        .expect_err("unknown head");
    let head = error.as_head_moved().expect("head moved");
    assert_eq!(
        head.expected_head_revision_id.as_deref(),
        Some("revision-9")
    );
    assert_eq!(head.actual_head_revision_id.as_deref(), Some("revision-0"));
    assert_eq!(head.expected_sequence.to_string(), "1");
    assert_eq!(head.actual_sequence.to_string(), "0");
    assert_eq!(revisions.get("revision-fork").expect("get"), None);
    assert_eq!(specs.get("spec-fork").expect("get"), None);
}

#[test]
fn a_constraint_failure_rolls_back_the_spec_inserted_with_the_revision() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let revisions = DesignRevisionRepository::new(&database);
    let specs = DesignSpecRepository::new(&database);
    revisions
        .commit_revision(&revision_with_spec("revision-0", 0, "spec-0"), None)
        .expect("first");

    // The head and sequence checks pass; the spec is inserted and then the
    // revision insert violates its primary key. The whole transaction must
    // roll back, including the spec row.
    let duplicate_id = revision_with_spec("revision-0", 1, "spec-rollback");
    let error = revisions
        .commit_revision(&duplicate_id, Some("revision-0"))
        .expect_err("duplicate revision id");
    assert!(
        error.as_sqlite().is_some(),
        "expected SQLite failure: {error}"
    );
    assert_eq!(specs.get("spec-rollback").expect("get"), None);
    assert_eq!(
        revisions.get("revision-0").expect("get").expect("row"),
        revision_with_spec("revision-0", 0, "spec-0")
    );
    assert_eq!(
        revisions
            .latest_for_design("design-1")
            .expect("latest")
            .expect("row")
            .sequence()
            .to_u64(),
        Some(0)
    );
    assert!(
        database
            .with_connection(|connection| connection.is_autocommit())
            .expect("connection")
    );

    // The schema's UNIQUE(design_id, sequence) remains the final backstop.
    let duplicate_sequence = revision_with_spec("revision-direct", 0, "spec-direct");
    let model_json = duplicate_sequence.to_wire_json().expect("wire");
    let error = database
        .transaction(|connection| {
            connection.execute(
                "INSERT INTO design_revisions(revision_id, design_id, sequence, model_json, created_at) VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![
                    duplicate_sequence.revision_id(),
                    duplicate_sequence.design_id(),
                    0_i64,
                    model_json,
                    "2026-01-02T03:04:05.000000Z"
                ],
            )?;
            Ok(())
        })
        .expect_err("unique sequence");
    assert!(matches!(
        error,
        PersistenceError::Sqlite(ref sqlite)
            if sqlite.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation)
    ));
    assert_eq!(
        revisions
            .latest_for_design("design-1")
            .expect("latest")
            .expect("row")
            .revision_id(),
        "revision-0"
    );
}

#[test]
fn parent_and_nested_transactions_roll_back_and_leave_commit_usable() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let specs = DesignSpecRepository::new(&database);
    let revisions = DesignRevisionRepository::new(&database);

    let outer = database.transaction(|_| {
        specs.save(&design_spec("spec-a")).expect("a");
        let nested = database.transaction(|_| {
            specs.save(&design_spec("spec-b")).expect("b");
            Err::<(), _>(PersistenceError::InvalidLimit)
        });
        assert!(nested.is_err());

        revisions
            .commit_revision(&revision_with_spec("revision-head", 0, "spec-head"), None)
            .expect("head");
        let loser = revision_with_spec("revision-loser", 1, "spec-loser");
        let conflict = revisions
            .commit_revision(&loser, None)
            .expect_err("cas conflict");
        assert!(matches!(conflict, RepositoryError::HeadMoved(_)));

        specs.save(&design_spec("spec-c")).expect("c");
        Ok(())
    });
    outer.expect("outer commit");

    assert!(specs.get("spec-a").expect("a").is_some());
    assert!(specs.get("spec-b").expect("b").is_none());
    assert!(specs.get("spec-c").expect("c").is_some());
    assert!(specs.get("spec-loser").expect("loser").is_none());

    assert_eq!(
        revisions.list_for_design("design-1").expect("list").len(),
        1
    );
    assert!(
        database
            .with_connection(|connection| connection.is_autocommit())
            .expect("connection")
    );

    // A failed outer transaction rolls every repository write back too,
    // including a successfully nested revision commit and its spec.
    let result = database.transaction(|_| {
        specs.save(&design_spec("spec-outer")).expect("outer");
        revisions
            .commit_revision(
                &revision_with_spec("revision-outer", 1, "spec-outer-revision"),
                Some("revision-head"),
            )
            .expect("nested revision");
        Err::<(), _>(PersistenceError::InvalidLimit)
    });
    assert!(result.is_err());
    assert!(specs.get("spec-outer").expect("outer").is_none());
    assert!(
        specs
            .get("spec-outer-revision")
            .expect("outer revision spec")
            .is_none()
    );
    assert!(revisions.get("revision-outer").expect("get").is_none());
    assert_eq!(
        revisions
            .latest_for_design("design-1")
            .expect("latest")
            .expect("row")
            .revision_id(),
        "revision-head"
    );
    assert!(
        database
            .with_connection(|connection| connection.is_autocommit())
            .expect("connection")
    );
}

#[test]
fn failed_commit_rolls_back_and_the_connection_stays_usable() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let specs = DesignSpecRepository::new(&database);

    let failure = database.transaction(|connection| {
        connection.execute_batch("PRAGMA defer_foreign_keys = ON")?;
        connection.execute(
            "INSERT INTO tool_results(call_id, tool_id, status, model_json) VALUES (?, ?, ?, ?)",
            rusqlite::params!["missing-call", "tool-1", "succeeded", "{}"],
        )?;
        Ok(())
    });
    assert!(failure.is_err(), "deferred foreign key must fail at COMMIT");
    assert!(
        database
            .with_connection(|connection| connection.is_autocommit())
            .expect("connection")
    );
    let rows: i64 = database
        .with_connection(|connection| {
            connection.query_row("SELECT COUNT(*) FROM tool_results", [], |row| row.get(0))
        })
        .expect("query")
        .expect("query");
    assert_eq!(rows, 0);

    specs.save(&design_spec("spec-after")).expect("write after");
    assert!(specs.get("spec-after").expect("read after").is_some());
}

#[test]
fn concurrent_cas_writers_on_independent_connections_produce_one_winner() {
    let directory = TempDir::new().expect("temp dir");
    let path = directory.path().join("cas.sqlite3");
    let initialize = kala_repositories::Database::open(&path).expect("initialize");
    initialize.close().expect("close");
    let first_db = kala_repositories::Database::open(&path).expect("first db");
    let second_db = kala_repositories::Database::open(&path).expect("second db");
    let barrier = Arc::new(Barrier::new(2));

    let run = |database: kala_repositories::Database,
               revision: kala_repositories::DesignRevision,
               barrier: Arc<Barrier>| {
        thread::spawn(move || {
            let repository = DesignRevisionRepository::new(&database);
            barrier.wait();
            let result = repository.commit_revision(&revision, None);
            (result, database)
        })
    };
    let first = run(
        first_db,
        revision_with_spec("revision-first", 0, "spec-first"),
        Arc::clone(&barrier),
    );
    let second = run(
        second_db,
        revision_with_spec("revision-second", 0, "spec-second"),
        Arc::clone(&barrier),
    );
    let (first, first_db) = first.join().expect("first thread");
    let (second, second_db) = second.join().expect("second thread");
    assert_eq!(
        usize::from(first.is_ok()) + usize::from(second.is_ok()),
        1,
        "{first:?} / {second:?}"
    );
    let (loser, loser_db, winner_db) = match (&first, &second) {
        (Ok(_), Err(error)) => (
            error.as_head_moved().expect("head moved"),
            &second_db,
            &first_db,
        ),
        (Err(error), Ok(_)) => (
            error.as_head_moved().expect("head moved"),
            &first_db,
            &second_db,
        ),
        _ => panic!("expected one winner"),
    };
    assert_eq!(loser.expected_head_revision_id, None);
    assert_eq!(loser.actual_sequence.to_string(), "0");
    assert_eq!(loser.expected_sequence.to_string(), "0");
    assert!(loser.actual_head_revision_id.is_some());
    assert!(
        loser_db
            .with_connection(|connection| connection.is_autocommit())
            .expect("loser connection")
    );
    assert!(
        winner_db
            .with_connection(|connection| connection.is_autocommit())
            .expect("winner connection")
    );

    let verify = kala_repositories::Database::open(&path).expect("verify db");
    let revisions = DesignRevisionRepository::new(&verify);
    let specs = DesignSpecRepository::new(&verify);
    let rows = revisions.list_for_design("design-1").expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].sequence().to_u64(), Some(0));
    let winner_id = rows[0].revision_id().to_owned();
    let (winner_spec, loser_spec) = if winner_id == "revision-first" {
        ("spec-first", "spec-second")
    } else {
        ("spec-second", "spec-first")
    };
    assert!(specs.get(winner_spec).expect("winner spec").is_some());
    assert!(specs.get(loser_spec).expect("loser spec").is_none());
    assert!(
        verify
            .with_connection(|connection| connection.is_autocommit())
            .expect("connection")
    );

    // The surviving connection is usable for a subsequent append.
    revisions
        .commit_revision(
            &revision_with_spec("revision-next", 1, "spec-next"),
            Some(rows[0].revision_id()),
        )
        .expect("next revision");
}

#[test]
fn concurrent_approval_decisions_are_final_on_independent_connections() {
    let directory = TempDir::new().expect("temp dir");
    let path = directory.path().join("approvals.sqlite3");
    let initialize = kala_repositories::Database::open(&path).expect("initialize");
    ApprovalRequestRepository::new(&initialize)
        .save(&approval("approval-1", "pending"))
        .expect("pending");
    initialize.close().expect("close");

    let first_db = kala_repositories::Database::open(&path).expect("first db");
    let second_db = kala_repositories::Database::open(&path).expect("second db");
    let barrier = Arc::new(Barrier::new(2));
    let approved = approval("approval-1", "approved");
    let rejected: kala_repositories::ApprovalRequest = model(
        r#"{"request_id":"approval-1","checkpoint_type":"egress","status":"rejected","workflow_id":"workflow-1","step_id":null,"action_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","options":["approve","deny"],"selected_option_id":"deny","requested_capabilities":[],"artifact_ids":[],"created_at":"2026-01-02T03:04:05Z","decided_at":"2026-01-02T03:05:05Z","actor_id":"reviewer-1","modification_digest":null}"#,
    );

    let run = |database: kala_repositories::Database,
               request: kala_repositories::ApprovalRequest,
               barrier: Arc<Barrier>| {
        thread::spawn(move || {
            let repository = ApprovalRequestRepository::new(&database);
            barrier.wait();
            repository.save(&request)
        })
    };
    let first = run(first_db, approved, Arc::clone(&barrier));
    let second = run(second_db, rejected, Arc::clone(&barrier));
    let first = first.join().expect("first thread");
    let second = second.join().expect("second thread");
    assert_eq!(
        usize::from(first.is_ok()) + usize::from(second.is_ok()),
        1,
        "{first:?} / {second:?}"
    );
    let error = match (&first, &second) {
        (Ok(_), Err(error)) | (Err(error), Ok(_)) => error,
        _ => panic!("expected one winner"),
    };
    assert_eq!(
        error
            .as_approval_already_decided()
            .expect("decided")
            .request_id(),
        "approval-1"
    );

    let verify = kala_repositories::Database::open(&path).expect("verify db");
    let stored = ApprovalRequestRepository::new(&verify)
        .get("approval-1")
        .expect("get")
        .expect("row");
    assert!(matches!(
        stored.status(),
        kala_repositories::ApprovalStatus::Approved | kala_repositories::ApprovalStatus::Rejected
    ));
    let rows: i64 = verify
        .with_connection(|connection| {
            connection.query_row("SELECT COUNT(*) FROM approval_requests", [], |row| {
                row.get(0)
            })
        })
        .expect("count")
        .expect("count");
    assert_eq!(rows, 1);

    // The final row cannot be reopened as pending.
    let pending = approval("approval-1", "pending");
    assert!(matches!(
        ApprovalRequestRepository::new(&verify).save(&pending),
        Err(RepositoryError::ApprovalAlreadyDecided(_))
    ));

    // The connection lock is released and the same database still works.
    MemoryRepository::new(&verify)
        .put(&support::memory_record("memory-after", "source-after"))
        .expect("memory after races");
}

#[test]
fn arbitrary_sequence_values_stay_in_the_typed_error_until_sqlite_range() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let revisions = DesignRevisionRepository::new(&database);
    let spec = design_spec("spec-huge");
    let huge: kala_repositories::DesignRevision = model(&format!(
        r#"{{"revision_id":"revision-huge","design_id":"design-huge","sequence":9223372036854775808,"spec":{spec_json},"parent_revision_id":null,"artifacts":[],"domain_packages":[],"provenance_id":null,"created_at":"2026-01-02T03:04:05Z"}}"#,
        spec_json = spec.to_wire_json().expect("spec"),
    ));
    let error = revisions
        .commit_revision(&huge, None)
        .expect_err("head mismatch before range conversion");
    let head = error.as_head_moved().expect("head moved");
    assert_eq!(head.expected_sequence.to_string(), "9223372036854775808");
    assert_eq!(head.actual_sequence.to_string(), "0");
    assert_eq!(revisions.get("revision-huge").expect("get"), None);

    // A matching head at SQLite's ceiling makes the next sequence 2^63; the
    // repository reports the documented typed range error instead of binding
    // an unrepresentable INTEGER.
    let max_revision: kala_repositories::DesignRevision = model(&format!(
        r#"{{"revision_id":"revision-max","design_id":"design-max","sequence":9223372036854775807,"spec":{spec_json},"parent_revision_id":null,"artifacts":[],"domain_packages":[],"provenance_id":null,"created_at":"2026-01-02T03:04:05Z"}}"#,
        spec_json = spec.to_wire_json().expect("spec"),
    ));
    let next: kala_repositories::DesignRevision = model(&format!(
        r#"{{"revision_id":"revision-overflow","design_id":"design-max","sequence":9223372036854775808,"spec":{spec_json},"parent_revision_id":null,"artifacts":[],"domain_packages":[],"provenance_id":null,"created_at":"2026-01-02T03:04:05Z"}}"#,
        spec_json = spec.to_wire_json().expect("spec"),
    ));
    let max_json = max_revision.to_wire_json().expect("wire");
    database
        .transaction(|connection| {
            connection.execute(
                "INSERT INTO design_revisions(revision_id, design_id, sequence, model_json, created_at) VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![
                    max_revision.revision_id(),
                    max_revision.design_id(),
                    i64::MAX,
                    max_json,
                    "2026-01-02T03:04:05.000000Z"
                ],
            )?;
            Ok(())
        })
        .expect("max row");
    assert!(matches!(
        revisions.commit_revision(&next, Some("revision-max")),
        Err(RepositoryError::SequenceOutOfRange)
    ));
    assert_eq!(revisions.get("revision-overflow").expect("get"), None);
    assert_eq!(
        revisions
            .latest_for_design("design-max")
            .expect("latest")
            .expect("row")
            .sequence()
            .to_u64(),
        Some(i64::MAX as u64)
    );
}
