//! Schema integrity, finality, foreign keys, triggers and tampered rows.

mod support;

use kala_persistence::EventJournal;
use kala_repositories::{
    ApprovalRequestRepository, ContractModel, DesignRevisionRepository, DesignSpecRepository,
    MemoryGetOptions, MemoryRepository, RepositoryError, ToolCallRepository, ToolResultRepository,
};
use support::{
    approval, copy_fixture_database, design_revision, design_spec, memory_at, model, timestamp,
    tool_call, tool_result,
};

fn raw_model_json(database: &kala_repositories::Database, sql: &str) -> String {
    database
        .with_connection(|connection| connection.query_row(sql, [], |row| row.get::<_, String>(0)))
        .expect("query")
        .expect("raw row")
}

#[test]
fn schema_v2_tables_triggers_and_foreign_keys_are_the_accepted_ones() {
    let database = kala_repositories::Database::in_memory().expect("database");
    assert_eq!(database.schema_version().expect("version"), 2);
    let (tables, triggers, foreign_keys) = database
        .with_connection(|connection| {
            let tables: Vec<String> = connection
                .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
                .expect("prepare")
                .query_map([], |row| row.get(0))
                .expect("query")
                .collect::<Result<_, _>>()
                .expect("tables");
            let triggers: Vec<String> = connection
                .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger' ORDER BY name")
                .expect("prepare")
                .query_map([], |row| row.get(0))
                .expect("query")
                .collect::<Result<_, _>>()
                .expect("triggers");
            let foreign_keys: i64 = connection
                .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                .expect("fk");
            (tables, triggers, foreign_keys)
        })
        .expect("schema");
    for table in [
        "approval_requests",
        "design_revisions",
        "design_specs",
        "events",
        "memory_entries",
        "tool_calls",
        "tool_results",
        "workflow_snapshots",
    ] {
        assert!(tables.iter().any(|name| name == table), "missing {table}");
    }
    assert_eq!(
        triggers,
        vec![
            "events_prevent_delete",
            "events_prevent_update",
            "events_require_event_chain"
        ]
    );
    assert_eq!(foreign_keys, 1);
}

#[test]
fn append_only_event_triggers_and_foreign_keys_reject_direct_sql() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let event = EventJournal::new(&database)
        .expect("journal")
        .append("step.started", "step-1", "{}")
        .expect("event");

    for sql in [
        "UPDATE events SET event_type = 'changed'",
        "DELETE FROM events",
    ] {
        let error = database
            .transaction(|connection| {
                connection.execute(sql, [])?;
                Ok(())
            })
            .expect_err("trigger");
        assert!(
            error.to_string().contains("append-only"),
            "unexpected trigger error: {error}"
        );
    }
    assert_eq!(
        EventJournal::new(&database)
            .expect("journal")
            .read()
            .expect("read")
            .len(),
        1
    );

    let calls = ToolCallRepository::new(&database);
    calls.save(&tool_call("call-1", "tool-1")).expect("call");
    ToolResultRepository::new(&database)
        .save(&tool_result("call-1", "tool-1"))
        .expect("result");

    // A direct result insert for a missing call is rejected by the schema FK.
    let error = database
        .transaction(|connection| {
            connection.execute(
                "INSERT INTO tool_results(call_id, tool_id, status, model_json) VALUES (?, ?, ?, ?)",
                rusqlite::params!["missing", "tool-1", "succeeded", "{}"],
            )?;
            Ok(())
        })
        .expect_err("foreign key");
    assert!(
        error.to_string().contains("FOREIGN KEY"),
        "unexpected FK error: {error}"
    );

    // ON UPDATE RESTRICT keeps a result attached to its original call.
    let error = database
        .transaction(|connection| {
            connection.execute(
                "UPDATE tool_calls SET call_id = 'call-2' WHERE call_id = 'call-1'",
                [],
            )?;
            Ok(())
        })
        .expect_err("restrict update");
    assert!(
        error.to_string().contains("FOREIGN KEY"),
        "unexpected restrict error: {error}"
    );

    let revision = design_revision("revision-0", "design-1", 0, &design_spec("spec-1"));
    DesignRevisionRepository::new(&database)
        .commit_revision(&revision, None)
        .expect("revision");
    let duplicate = design_revision("revision-1", "design-1", 0, &design_spec("spec-2"));
    let duplicate_json = duplicate.to_wire_json().expect("wire");
    let error = database
        .transaction(|connection| {
            connection.execute(
                "INSERT INTO design_revisions(revision_id, design_id, sequence, model_json, created_at) VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![
                    duplicate.revision_id(),
                    duplicate.design_id(),
                    0_i64,
                    duplicate_json,
                    "2026-01-02T03:04:05.000000Z"
                ],
            )?;
            Ok(())
        })
        .expect_err("unique sequence");
    assert!(error.to_string().contains("UNIQUE"), "{error}");
    assert_eq!(
        DesignRevisionRepository::new(&database)
            .latest_for_design("design-1")
            .expect("latest")
            .expect("row")
            .revision_id(),
        "revision-0"
    );
    assert_eq!(event.sequence, 1);
}

#[test]
fn decided_approvals_are_immutable_except_for_a_byte_identical_no_op() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let approvals = ApprovalRequestRepository::new(&database);
    approvals
        .save(&approval("approval-1", "pending"))
        .expect("pending");
    let decided = approval("approval-1", "approved");
    approvals.save(&decided).expect("decide");

    // A byte-identical write is an actual SQL update: tampered side columns
    // are repaired from the model rather than skipped by an early return.
    database
        .transaction(|connection| {
            connection.execute(
                "UPDATE approval_requests SET created_at = '1999-01-01T00:00:00.000000Z', status = 'pending' WHERE request_id = 'approval-1'",
                [],
            )?;
            Ok(())
        })
        .expect("tamper");
    approvals.save(&decided).expect("no-op repair");
    let (status, created_at): (String, String) = database
        .with_connection(|connection| {
            connection.query_row(
                "SELECT status, created_at FROM approval_requests WHERE request_id = 'approval-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .expect("row")
        .expect("row");
    assert_eq!(status, "approved");
    assert_eq!(created_at, "2026-01-02T03:04:05.000000Z");

    let changed_actor: kala_repositories::ApprovalRequest = model(
        r#"{"request_id":"approval-1","checkpoint_type":"egress","status":"approved","workflow_id":"workflow-1","step_id":null,"action_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","options":["approve","deny"],"selected_option_id":"approve","requested_capabilities":[],"artifact_ids":[],"created_at":"2026-01-02T03:04:05Z","decided_at":"2026-01-02T03:05:05Z","actor_id":"different","modification_digest":null}"#,
    );
    let error = approvals
        .save(&changed_actor)
        .expect_err("changed decision");
    assert_eq!(
        error
            .as_approval_already_decided()
            .expect("decided")
            .request_id(),
        "approval-1"
    );
    assert_eq!(
        approvals.get("approval-1").expect("get"),
        Some(decided.clone())
    );

    let reopen = approval("approval-1", "pending");
    assert!(matches!(
        approvals.save(&reopen),
        Err(RepositoryError::ApprovalAlreadyDecided(_))
    ));
    assert_eq!(approvals.get("approval-1").expect("get"), Some(decided));

    // Pending records may still change.
    approvals
        .save(&approval("approval-2", "pending"))
        .expect("pending");
    let updated_pending: kala_repositories::ApprovalRequest = model(
        r#"{"request_id":"approval-2","checkpoint_type":"egress","status":"pending","workflow_id":"workflow-2","step_id":null,"action_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state_digest":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","options":["approve","deny"],"selected_option_id":null,"requested_capabilities":[],"artifact_ids":[],"created_at":"2026-01-02T03:04:05Z","decided_at":null,"actor_id":null,"modification_digest":null}"#,
    );
    approvals.save(&updated_pending).expect("pending update");
    assert_eq!(
        approvals.get("approval-2").expect("get"),
        Some(updated_pending)
    );
}

#[test]
fn tool_result_identity_checks_are_atomic_and_leave_existing_rows() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let calls = ToolCallRepository::new(&database);
    let results = ToolResultRepository::new(&database);
    calls.save(&tool_call("call-1", "tool-1")).expect("call");

    let missing = results
        .save(&tool_result("call-missing", "tool-1"))
        .expect_err("missing call");
    assert!(matches!(missing, RepositoryError::ToolResultMissingCall));
    assert_eq!(results.get("call-missing").expect("get"), None);

    let mismatch = results
        .save(&tool_result("call-1", "tool-2"))
        .expect_err("mismatch");
    assert!(matches!(mismatch, RepositoryError::ToolResultToolMismatch));
    assert_eq!(results.get("call-1").expect("get"), None);

    results
        .save(&tool_result("call-1", "tool-1"))
        .expect("correct");
    let stored = results.get("call-1").expect("get").expect("row");
    assert_eq!(stored.tool_id(), "tool-1");

    // A later mismatched write leaves the committed result untouched.
    let mismatch = results
        .save(&tool_result("call-1", "tool-2"))
        .expect_err("mismatch");
    assert!(matches!(mismatch, RepositoryError::ToolResultToolMismatch));
    assert_eq!(results.get("call-1").expect("get").expect("row"), stored);
    assert!(
        database
            .with_connection(|connection| connection.is_autocommit())
            .expect("connection")
    );
}

#[test]
fn tampered_stored_models_fail_validation_without_being_rewritten() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let spec = design_spec("spec-1");
    DesignSpecRepository::new(&database)
        .save(&spec)
        .expect("spec");
    let revision = design_revision("revision-0", "design-1", 0, &design_spec("spec-1"));
    DesignRevisionRepository::new(&database)
        .commit_revision(&revision, None)
        .expect("revision");
    ApprovalRequestRepository::new(&database)
        .save(&approval("approval-1", "pending"))
        .expect("approval");
    ToolCallRepository::new(&database)
        .save(&tool_call("call-1", "tool-1"))
        .expect("call");
    ToolResultRepository::new(&database)
        .save(&tool_result("call-1", "tool-1"))
        .expect("result");
    MemoryRepository::new(&database)
        .put(&memory_at(
            "memory-1",
            "source-1",
            &timestamp("2026-01-02T03:04:05Z"),
            None,
        ))
        .expect("memory");

    for (table, key) in [
        ("design_specs", "id = 'spec-1'"),
        ("design_revisions", "revision_id = 'revision-0'"),
        ("approval_requests", "request_id = 'approval-1'"),
        ("tool_calls", "call_id = 'call-1'"),
        ("tool_results", "call_id = 'call-1'"),
        ("memory_entries", "memory_id = 'memory-1'"),
    ] {
        database
            .transaction(|connection| {
                connection.execute(
                    &format!("UPDATE {table} SET model_json = 'not-json' WHERE {key}"),
                    [],
                )?;
                Ok(())
            })
            .expect("tamper");
    }

    let errors = [
        DesignSpecRepository::new(&database)
            .get("spec-1")
            .expect_err("spec"),
        DesignRevisionRepository::new(&database)
            .get("revision-0")
            .expect_err("revision"),
        ApprovalRequestRepository::new(&database)
            .get("approval-1")
            .expect_err("approval"),
        ToolCallRepository::new(&database)
            .get("call-1")
            .expect_err("call"),
        ToolResultRepository::new(&database)
            .get("call-1")
            .expect_err("result"),
        MemoryRepository::new(&database)
            .get_with_options(
                "memory-1",
                &MemoryGetOptions::default().with_now(timestamp("2026-01-02T03:30:00Z")),
            )
            .expect_err("memory"),
    ];
    for (error, expected) in errors.iter().zip([
        "DesignSpec",
        "DesignRevision",
        "ApprovalRequest",
        "ToolCall",
        "ToolResult",
        "MemoryRecord",
    ]) {
        match error {
            RepositoryError::StoredModel { model, .. } => assert_eq!(*model, expected),
            other => panic!("expected StoredModel for {expected}, got {other:?}"),
        }
    }

    // Reads never rewrite the tampered bytes.
    assert_eq!(
        raw_model_json(
            &database,
            "SELECT model_json FROM design_specs WHERE id = 'spec-1'"
        ),
        "not-json"
    );

    // Valid JSON that fails a contract constraint is also a stored-model
    // failure, not a generic SQLite error.
    database
        .transaction(|connection| {
            connection.execute(
                "UPDATE design_specs SET model_json = '{\"id\":\"spec-1\"}' WHERE id = 'spec-1'",
                [],
            )?;
            Ok(())
        })
        .expect("tamper");
    assert!(matches!(
        DesignSpecRepository::new(&database).get("spec-1"),
        Err(RepositoryError::StoredModel {
            model: "DesignSpec",
            ..
        })
    ));
}

#[test]
fn stored_model_json_uses_pydantic_last_wins_duplicate_parsing() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let valid = design_spec("spec-1").to_wire_json().expect("wire");
    let duplicated = valid.replacen(r#"{"id":"spec-1""#, r#"{"id":"other","id":"spec-1""#, 1);
    assert_ne!(duplicated, valid);
    database
        .transaction(|connection| {
            connection.execute(
                "INSERT INTO design_specs(id, model_json) VALUES (?, ?)",
                rusqlite::params!["spec-1", duplicated],
            )?;
            Ok(())
        })
        .expect("insert");
    assert_eq!(
        DesignSpecRepository::new(&database)
            .get("spec-1")
            .expect("get")
            .expect("row")
            .id(),
        "spec-1"
    );
}

#[test]
fn timestamp_columns_are_utc_microseconds_while_model_json_keeps_offsets() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let spec = design_spec("spec-1");
    let revision: kala_repositories::DesignRevision = model(&format!(
        r#"{{"revision_id":"revision-0","design_id":"design-1","sequence":0,"spec":{spec_json},"parent_revision_id":null,"artifacts":[],"domain_packages":[],"provenance_id":null,"created_at":"2026-01-02T08:34:05.123456+05:30"}}"#,
        spec_json = spec.to_wire_json().expect("spec"),
    ));
    DesignRevisionRepository::new(&database)
        .commit_revision(&revision, None)
        .expect("revision");
    assert_eq!(
        raw_model_json(
            &database,
            "SELECT created_at FROM design_revisions WHERE revision_id = 'revision-0'"
        ),
        "2026-01-02T03:04:05.123456Z"
    );
    let stored = DesignRevisionRepository::new(&database)
        .get("revision-0")
        .expect("get")
        .expect("row");
    assert_eq!(
        stored.created_at().to_wire_string(),
        "2026-01-02T08:34:05.123456+05:30"
    );

    let call: kala_repositories::ToolCall = model(
        r#"{"call_id":"call-1","tool_id":"tool-1","tool_version":"1.0.0","arguments_json":"{}","requested_capabilities":[],"side_effect_level":"read","timeout_seconds":1.0,"issued_at":"2026-01-02T09:34:05+06:30"}"#,
    );
    ToolCallRepository::new(&database)
        .save(&call)
        .expect("call");
    assert_eq!(
        raw_model_json(
            &database,
            "SELECT issued_at FROM tool_calls WHERE call_id = 'call-1'"
        ),
        "2026-01-02T03:04:05.000000Z"
    );

    let memory = memory_at(
        "memory-1",
        "source-1",
        &timestamp("2026-01-02T08:34:05+05:30"),
        Some(&timestamp("2026-01-02T09:34:05+05:30")),
    );
    MemoryRepository::new(&database)
        .put(&memory)
        .expect("memory");
    let (created, expires): (String, String) = database
        .with_connection(|connection| {
            connection.query_row(
                "SELECT created_at, expires_at FROM memory_entries WHERE memory_id = 'memory-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .expect("row")
        .expect("row");
    assert_eq!(created, "2026-01-02T03:04:05.000000Z");
    assert_eq!(expires, "2026-01-02T04:04:05.000000Z");
    assert!(
        memory
            .to_wire_json()
            .expect("wire")
            .contains("2026-01-02T08:34:05+05:30")
    );
}

#[test]
fn the_original_frozen_schema_database_remains_readable() {
    let fixture = copy_fixture_database();
    let database = kala_repositories::Database::open(&fixture.path).expect("open fixture");
    assert_eq!(database.schema_version().expect("version"), 2);
    let repository = DesignSpecRepository::new(&database);
    let spec = repository
        .get("design-corpus")
        .expect("get")
        .expect("fixture spec");
    assert_eq!(spec.id(), "design-corpus");
    assert_eq!(spec.name().to_utf8().expect("name"), "Corpus design");
    assert_eq!(
        spec.attributes_json().as_str(),
        r#"{"domain":"mechanical","nested":{"a":1,"b":2}}"#
    );
    let listed = repository.list().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], spec);
}
