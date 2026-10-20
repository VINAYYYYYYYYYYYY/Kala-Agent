//! End-to-end repository behavior on real SQLite files.

mod support;

use kala_repositories::{
    ApprovalRequestRepository, ContractModel, DEFAULT_LIST_LIMIT, DesignRevisionRepository,
    DesignSpecRepository, MemoryGetOptions, MemoryListOptions, MemoryRepository, RepositoryError,
    ToolCallRepository, ToolResultRepository,
};
use support::{
    approval, design_revision, design_spec, memory_at, memory_record, model, timestamp, tool_call,
    tool_result,
};
use tempfile::TempDir;

#[test]
fn all_six_repositories_round_trip_on_a_real_database_file() {
    let directory = TempDir::new().expect("temp dir");
    let path = directory.path().join("repositories.sqlite3");
    let database = kala_repositories::Database::open(&path).expect("open");

    let specs = DesignSpecRepository::new(&database);
    let revisions = DesignRevisionRepository::new(&database);
    let approvals = ApprovalRequestRepository::new(&database);
    let calls = ToolCallRepository::new(&database);
    let results = ToolResultRepository::new(&database);
    let memory = MemoryRepository::new(&database);

    let spec = design_spec("spec-1");
    let first = design_revision("revision-0", "design-1", 0, &spec);
    let second = design_revision("revision-1", "design-1", 1, &spec);
    let pending = approval("approval-1", "pending");
    let decided = approval("approval-1", "approved");
    let call = tool_call("call-1", "tool-1");
    let result = tool_result("call-1", "tool-1");
    let active = memory_record("memory-1", "conversation-1");

    specs.save(&spec).expect("save spec");
    assert_eq!(
        revisions
            .commit_revision(&first, None)
            .expect("commit first"),
        first
    );
    assert_eq!(
        revisions
            .commit_revision(&second, Some("revision-0"))
            .expect("commit second"),
        second
    );
    approvals.save(&pending).expect("save pending");
    approvals.save(&decided).expect("decide");
    calls.save(&call).expect("save call");
    results.save(&result).expect("save result");
    memory.put(&active).expect("save memory");

    assert_eq!(specs.get("spec-1").expect("get spec"), Some(spec));
    assert_eq!(
        specs.list().expect("list specs"),
        vec![design_spec("spec-1")]
    );
    assert_eq!(
        revisions.get("revision-0").expect("get revision"),
        Some(first)
    );
    assert_eq!(
        revisions
            .list_for_design("design-1")
            .expect("list revisions"),
        vec![
            design_revision("revision-0", "design-1", 0, &design_spec("spec-1")),
            second.clone()
        ]
    );
    assert_eq!(
        revisions.latest_for_design("design-1").expect("latest"),
        Some(second)
    );
    assert_eq!(
        approvals.get("approval-1").expect("get approval"),
        Some(decided)
    );
    assert_eq!(
        approvals
            .list_for_workflow("workflow-1")
            .expect("list approvals"),
        vec![approval("approval-1", "approved")]
    );
    assert_eq!(calls.get("call-1").expect("get call"), Some(call));
    assert_eq!(
        calls.list_for_tool("tool-1").expect("list calls"),
        vec![tool_call("call-1", "tool-1")]
    );
    assert_eq!(results.get("call-1").expect("get result"), Some(result));
    let fixed_now = timestamp("2026-01-02T03:30:00Z");
    assert_eq!(
        memory
            .get_with_options(
                "memory-1",
                &MemoryGetOptions::default().with_now(fixed_now.clone())
            )
            .expect("get memory"),
        Some(memory_record("memory-1", "conversation-1"))
    );
    assert_eq!(
        memory
            .list_with_options(&MemoryListOptions::default().with_now(fixed_now))
            .expect("list memory"),
        vec![memory_record("memory-1", "conversation-1")]
    );

    drop(database);
    // Reopening proves the bytes live in the database file, not in memory.
    let reopened = kala_repositories::Database::open(&path).expect("reopen");
    assert_eq!(
        DesignSpecRepository::new(&reopened)
            .get("spec-1")
            .expect("reopen spec"),
        Some(design_spec("spec-1"))
    );
    assert_eq!(
        DesignRevisionRepository::new(&reopened)
            .latest_for_design("design-1")
            .expect("reopen latest")
            .expect("latest")
            .sequence()
            .to_u64(),
        Some(1)
    );
    assert_eq!(
        MemoryRepository::new(&reopened)
            .get_with_options(
                "memory-1",
                &MemoryGetOptions::default().with_now(timestamp("2026-01-02T03:30:00Z"))
            )
            .expect("reopen memory")
            .expect("memory")
            .content()
            .to_utf8()
            .expect("utf8"),
        "Remember the local database location."
    );
}

#[test]
fn updates_follow_the_python_upsert_paths() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let specs = DesignSpecRepository::new(&database);
    let approvals = ApprovalRequestRepository::new(&database);
    let calls = ToolCallRepository::new(&database);
    let results = ToolResultRepository::new(&database);
    let memory = MemoryRepository::new(&database);

    specs.save(&design_spec("spec-1")).expect("insert spec");
    let replacement: kala_repositories::DesignSpec = model(
        r#"{"id":"spec-1","name":"Replacement","summary":"Updated.","parameter_schema_digest":null,"attributes_json":"{}","requirement_ids":[],"constraints":[]}"#,
    );
    specs.save(&replacement).expect("replace spec");
    assert_eq!(specs.get("spec-1").expect("get"), Some(replacement));

    approvals
        .save(&approval("approval-1", "pending"))
        .expect("pending");
    let updated_pending: kala_repositories::ApprovalRequest = model(
        r#"{"request_id":"approval-1","checkpoint_type":"egress","status":"pending","workflow_id":"workflow-1","step_id":null,"action_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state_digest":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","options":["approve","deny"],"selected_option_id":null,"requested_capabilities":[],"artifact_ids":[],"created_at":"2026-01-02T03:04:05Z","decided_at":null,"actor_id":null,"modification_digest":null}"#,
    );
    approvals.save(&updated_pending).expect("update pending");
    assert_eq!(
        approvals.get("approval-1").expect("get"),
        Some(updated_pending)
    );

    let first_call = tool_call("call-1", "tool-1");
    calls.save(&first_call).expect("call");
    let replacement_call: kala_repositories::ToolCall = model(
        r#"{"call_id":"call-1","tool_id":"tool-1","tool_version":"2.0.0","arguments_json":"{\"value\":9}","requested_capabilities":[],"side_effect_level":"none","timeout_seconds":1.0,"issued_at":"2026-01-02T03:05:05Z"}"#,
    );
    calls.save(&replacement_call).expect("replace call");
    assert_eq!(
        calls.get("call-1").expect("get call"),
        Some(replacement_call)
    );

    results
        .save(&tool_result("call-1", "tool-1"))
        .expect("result");
    let failed: kala_repositories::ToolResult = model(
        r#"{"call_id":"call-1","tool_id":"tool-1","status":"failed","result_json":null,"error_code":"tool.failed","error_message":"boom","artifacts":[],"started_at":"2026-01-02T03:04:05Z","finished_at":"2026-01-02T03:04:06Z","duration_ms":1000.0}"#,
    );
    results.save(&failed).expect("replace result");
    assert_eq!(results.get("call-1").expect("get result"), Some(failed));

    memory
        .put(&memory_record("memory-1", "source-1"))
        .expect("memory");
    let replacement_memory = memory_at(
        "memory-1",
        "source-2",
        &timestamp("2026-01-03T00:00:00Z"),
        None,
    );
    memory.put(&replacement_memory).expect("replace memory");
    assert_eq!(
        memory
            .get_with_options(
                "memory-1",
                &MemoryGetOptions::default().with_now(timestamp("2026-01-03T00:00:00Z"))
            )
            .expect("get memory"),
        Some(replacement_memory)
    );
}

#[test]
fn list_defaults_limits_and_tie_breakers_match_python() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let specs = DesignSpecRepository::new(&database);
    let revisions = DesignRevisionRepository::new(&database);
    let approvals = ApprovalRequestRepository::new(&database);
    let calls = ToolCallRepository::new(&database);
    let memory = MemoryRepository::new(&database);

    let spec_json = design_spec("spec-0000").to_wire_json().expect("spec wire");
    let revision_template =
        design_revision("revision-0000", "design-1", 0, &design_spec("spec-0000"));
    let approval_template = approval("approval-0000", "pending");
    let call_template = tool_call("call-0000", "tool-1");
    let memory_template = memory_record("memory-0000", "source-1");

    database
        .transaction(|connection| {
            for index in 0..=DEFAULT_LIST_LIMIT as usize {
                let spec_id = format!("spec-{index:04}");
                let spec = if index == 0 {
                    spec_json.clone()
                } else {
                    format!(
                        r#"{{"id":"{spec_id}","name":"Fixture","summary":"Fixture.","parameter_schema_digest":null,"attributes_json":"{{}}","requirement_ids":[],"constraints":[]}}"#
                    )
                };
                connection.execute(
                    "INSERT INTO design_specs(id, model_json) VALUES (?, ?)",
                    rusqlite::params![spec_id, spec],
                )?;

                let revision_id = format!("revision-{index:04}");
                let sequence = index as i64;
                let revision_json = if index == 0 {
                    revision_template.to_wire_json().expect("revision")
                } else {
                    revision_template
                        .to_wire_json()
                        .expect("revision")
                        .replace("revision-0000", &revision_id)
                        .replace("\"sequence\":0", &format!("\"sequence\":{sequence}"))
                };
                connection.execute(
                    "INSERT INTO design_revisions(revision_id, design_id, sequence, model_json, created_at) VALUES (?, ?, ?, ?, ?)",
                    rusqlite::params![
                        revision_id,
                        "design-1",
                        sequence,
                        revision_json,
                        "2026-01-02T03:04:05.000000Z"
                    ],
                )?;

                let request_id = format!("approval-{index:04}");
                let approval_json = if index == 0 {
                    approval_template.to_wire_json().expect("approval")
                } else {
                    approval_template
                        .to_wire_json()
                        .expect("approval")
                        .replace("approval-0000", &request_id)
                };
                connection.execute(
                    "INSERT INTO approval_requests(request_id, workflow_id, status, created_at, model_json) VALUES (?, ?, ?, ?, ?)",
                    rusqlite::params![
                        request_id,
                        "workflow-1",
                        "pending",
                        "2026-01-02T03:04:05.000000Z",
                        approval_json
                    ],
                )?;

                let call_id = format!("call-{index:04}");
                let call_json = if index == 0 {
                    call_template.to_wire_json().expect("call")
                } else {
                    call_template
                        .to_wire_json()
                        .expect("call")
                        .replace("call-0000", &call_id)
                };
                connection.execute(
                    "INSERT INTO tool_calls(call_id, tool_id, issued_at, model_json) VALUES (?, ?, ?, ?)",
                    rusqlite::params![
                        call_id,
                        "tool-1",
                        "2026-01-02T03:04:05.000000Z",
                        call_json
                    ],
                )?;

                let memory_id = format!("memory-{index:04}");
                let memory_json = if index == 0 {
                    memory_template.to_wire_json().expect("memory")
                } else {
                    memory_template
                        .to_wire_json()
                        .expect("memory")
                        .replace("memory-0000", &memory_id)
                };
                connection.execute(
                    "INSERT INTO memory_entries(memory_id, source, provenance_id, created_at, expires_at, model_json) VALUES (?, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        memory_id,
                        "source-1",
                        "provenance-1",
                        "2026-01-02T03:04:05.000000Z",
                        "2026-01-02T04:04:05.000000Z",
                        memory_json
                    ],
                )?;
            }
            Ok(())
        })
        .expect("bulk rows");

    assert_eq!(
        specs.list().expect("default specs").len(),
        DEFAULT_LIST_LIMIT as usize
    );
    assert_eq!(
        specs
            .list_with_limit(DEFAULT_LIST_LIMIT + 1)
            .expect("all specs")
            .len(),
        DEFAULT_LIST_LIMIT as usize + 1
    );
    assert_eq!(
        specs.list_with_limit(1).expect("one spec")[0].id(),
        "spec-0000"
    );

    assert_eq!(
        revisions
            .list_for_design("design-1")
            .expect("default revisions")
            .len(),
        DEFAULT_LIST_LIMIT as usize
    );
    assert_eq!(
        revisions
            .list_for_design_with_limit("design-1", DEFAULT_LIST_LIMIT + 1)
            .expect("all revisions")
            .len(),
        DEFAULT_LIST_LIMIT as usize + 1
    );
    assert_eq!(
        revisions
            .latest_for_design("design-1")
            .expect("latest")
            .expect("row")
            .sequence()
            .to_u64(),
        Some(DEFAULT_LIST_LIMIT as u64)
    );

    assert_eq!(
        approvals
            .list_for_workflow("workflow-1")
            .expect("default approvals")
            .len(),
        DEFAULT_LIST_LIMIT as usize
    );
    assert_eq!(
        approvals
            .list_for_workflow_with_limit("workflow-1", DEFAULT_LIST_LIMIT + 1)
            .expect("all approvals")
            .len(),
        DEFAULT_LIST_LIMIT as usize + 1
    );
    assert_eq!(
        approvals
            .list_for_workflow_with_limit("workflow-1", 2)
            .expect("two approvals")
            .iter()
            .map(|row| row.request_id())
            .collect::<Vec<_>>(),
        vec!["approval-0000", "approval-0001"]
    );

    assert_eq!(
        calls.list_for_tool("tool-1").expect("default calls").len(),
        DEFAULT_LIST_LIMIT as usize
    );
    assert_eq!(
        calls
            .list_for_tool_with_limit("tool-1", DEFAULT_LIST_LIMIT + 1)
            .expect("all calls")
            .len(),
        DEFAULT_LIST_LIMIT as usize + 1
    );

    assert_eq!(
        memory
            .list_with_options(
                &MemoryListOptions::default().with_now(timestamp("2026-01-02T03:30:00Z"))
            )
            .expect("default memory")
            .len(),
        DEFAULT_LIST_LIMIT as usize
    );
    assert_eq!(
        memory
            .list_with_options(
                &MemoryListOptions::default()
                    .with_now(timestamp("2026-01-02T03:30:00Z"))
                    .with_limit(DEFAULT_LIST_LIMIT + 1)
            )
            .expect("all memory")
            .len(),
        DEFAULT_LIST_LIMIT as usize + 1
    );

    for limit in [0, -1, i64::MIN] {
        assert!(matches!(
            specs.list_with_limit(limit),
            Err(RepositoryError::InvalidLimit)
        ));
        assert!(matches!(
            revisions.list_for_design_with_limit("design-1", limit),
            Err(RepositoryError::InvalidLimit)
        ));
        assert!(matches!(
            approvals.list_for_workflow_with_limit("workflow-1", limit),
            Err(RepositoryError::InvalidLimit)
        ));
        assert!(matches!(
            calls.list_for_tool_with_limit("tool-1", limit),
            Err(RepositoryError::InvalidLimit)
        ));
        assert!(matches!(
            memory.list_with_options(&MemoryListOptions::default().with_limit(limit)),
            Err(RepositoryError::InvalidLimit)
        ));
    }
}

#[test]
fn identifiers_are_normalized_for_every_lookup() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let specs = DesignSpecRepository::new(&database);
    let revisions = DesignRevisionRepository::new(&database);
    let approvals = ApprovalRequestRepository::new(&database);
    let calls = ToolCallRepository::new(&database);
    let results = ToolResultRepository::new(&database);
    let memory = MemoryRepository::new(&database);

    let spec = design_spec("spec-1");
    specs.save(&spec).expect("spec");
    assert_eq!(
        specs.get("\u{1c}\u{3000}spec-1\t\n").expect("normalized"),
        Some(spec)
    );
    assert!(matches!(
        specs.get("../spec-1"),
        Err(RepositoryError::InvalidIdentifier { field: "spec_id" })
    ));

    let max_id = format!("a{}", "b".repeat(127));
    assert_eq!(specs.get(&max_id).expect("128 characters"), None);
    let too_long = format!("a{}", "b".repeat(128));
    assert!(matches!(
        specs.get(&too_long),
        Err(RepositoryError::InvalidIdentifier { field: "spec_id" })
    ));
    for invalid in [".spec-1", "spec 1", "spëc-1", ""] {
        assert!(
            matches!(
                specs.get(invalid),
                Err(RepositoryError::InvalidIdentifier { field: "spec_id" })
            ),
            "accepted {invalid:?}"
        );
    }
    assert!(specs.list_with_limit(1).is_ok());

    revisions
        .commit_revision(
            &design_revision("revision-0", "design-1", 0, &design_spec("spec-1")),
            None,
        )
        .expect("revision");
    assert_eq!(
        revisions
            .get(" revision-0\u{1f}")
            .expect("normalized")
            .expect("row")
            .revision_id(),
        "revision-0"
    );
    assert!(matches!(
        revisions.commit_revision(
            &design_revision("revision-1", "design-1", 1, &design_spec("spec-1")),
            Some("../revision-0")
        ),
        Err(RepositoryError::InvalidIdentifier {
            field: "expected_head_revision_id"
        })
    ));

    let second = design_revision("revision-1", "design-1", 1, &design_spec("spec-1"));
    revisions
        .commit_revision(&second, Some(" \u{1c}revision-0\u{3000} "))
        .expect("normalized expected head");
    assert_eq!(
        revisions
            .latest_for_design("design-1")
            .expect("latest")
            .expect("row")
            .revision_id(),
        "revision-1"
    );

    approvals
        .save(&approval("approval-1", "pending"))
        .expect("approval");
    assert_eq!(
        approvals
            .get("approval-1\u{0085}")
            .expect("normalized")
            .expect("row")
            .request_id(),
        "approval-1"
    );
    assert!(matches!(
        approvals.list_for_workflow("../workflow-1"),
        Err(RepositoryError::InvalidIdentifier {
            field: "workflow_id"
        })
    ));

    calls.save(&tool_call("call-1", "tool-1")).expect("call");
    assert_eq!(
        calls
            .get("\u{00a0}call-1")
            .expect("normalized")
            .expect("row")
            .call_id(),
        "call-1"
    );
    assert!(matches!(
        calls.list_for_tool("bad/tool"),
        Err(RepositoryError::InvalidIdentifier { field: "tool_id" })
    ));

    results
        .save(&tool_result("call-1", "tool-1"))
        .expect("result");
    assert_eq!(
        results
            .get("call-1 ")
            .expect("normalized")
            .expect("row")
            .call_id(),
        "call-1"
    );
    assert!(matches!(
        results.get("bad/call"),
        Err(RepositoryError::InvalidIdentifier { field: "call_id" })
    ));

    memory
        .put(&memory_record("memory-1", "source-1"))
        .expect("memory");
    assert_eq!(
        memory
            .get_with_options(
                " memory-1",
                &MemoryGetOptions::default().with_now(timestamp("2026-01-02T03:30:00Z"))
            )
            .expect("normalized")
            .expect("row")
            .memory_id(),
        "memory-1"
    );
    assert!(matches!(
        memory.get("bad/memory"),
        Err(RepositoryError::InvalidIdentifier { field: "memory_id" })
    ));
    assert!(matches!(
        memory.list_with_options(&MemoryListOptions::default().with_source("bad/source")),
        Err(RepositoryError::InvalidIdentifier { field: "source" })
    ));
}

#[test]
fn memory_expiry_is_strict_and_inclusive_flags_work() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let memory = MemoryRepository::new(&database);
    let created = timestamp("2026-01-02T03:04:05Z");
    let expires = timestamp("2026-01-02T04:04:05Z");
    let record = memory_at("memory-1", "source-1", &created, Some(&expires));
    memory.put(&record).expect("put");

    assert_eq!(
        memory
            .get_with_options(
                "memory-1",
                &MemoryGetOptions::default().with_now(created.clone())
            )
            .expect("before expiry"),
        Some(record.clone())
    );
    assert_eq!(
        memory
            .get_with_options(
                "memory-1",
                &MemoryGetOptions::default().with_now(timestamp("2026-01-02T04:04:04.999999Z"))
            )
            .expect("just before"),
        Some(record.clone())
    );
    assert_eq!(
        memory
            .get_with_options(
                "memory-1",
                &MemoryGetOptions::default().with_now(expires.clone())
            )
            .expect("at expiry"),
        None
    );
    assert_eq!(
        memory
            .get_with_options(
                "memory-1",
                &MemoryGetOptions::default().with_now(timestamp("2026-01-02T09:34:05+05:30"))
            )
            .expect("same instant in another offset"),
        None
    );
    assert_eq!(
        memory
            .get_with_options(
                "memory-1",
                &MemoryGetOptions::default()
                    .with_now(expires.clone())
                    .with_include_expired(true)
            )
            .expect("include expired"),
        Some(record.clone())
    );
    assert_eq!(
        memory
            .list_with_options(
                &MemoryListOptions::default()
                    .with_now(expires.clone())
                    .with_include_expired(true)
            )
            .expect("list include"),
        vec![record.clone()]
    );
    assert_eq!(
        memory
            .list_with_options(&MemoryListOptions::default().with_now(expires))
            .expect("list active"),
        Vec::new()
    );

    let no_expiry = memory_at("memory-2", "source-2", &created, None);
    memory.put(&no_expiry).expect("put second");
    assert_eq!(
        memory
            .get_with_options(
                "memory-2",
                &MemoryGetOptions::default().with_now(timestamp("2030-01-01T00:00:00Z"))
            )
            .expect("never expires"),
        Some(no_expiry)
    );

    assert!(memory.delete("memory-1").expect("delete"));
    assert!(!memory.delete("memory-1").expect("delete again"));
    assert_eq!(
        memory
            .get_with_options(
                "memory-1",
                &MemoryGetOptions::default().with_include_expired(true)
            )
            .expect("deleted"),
        None
    );
}

#[test]
fn memory_source_filtering_and_absent_lookups_are_exact() {
    use kala_repositories::ApprovalStatus;

    let database = kala_repositories::Database::in_memory().expect("database");
    let specs = DesignSpecRepository::new(&database);
    let revisions = DesignRevisionRepository::new(&database);
    let approvals = ApprovalRequestRepository::new(&database);
    let calls = ToolCallRepository::new(&database);
    let results = ToolResultRepository::new(&database);
    let memory = MemoryRepository::new(&database);

    assert_eq!(specs.get("missing").expect("get"), None);
    assert_eq!(specs.list().expect("list"), Vec::new());
    assert_eq!(
        revisions.latest_for_design("missing").expect("latest"),
        None
    );
    assert_eq!(
        revisions.list_for_design("missing").expect("list"),
        Vec::new()
    );
    assert_eq!(approvals.get("missing").expect("get"), None);
    assert_eq!(
        approvals.list_for_workflow("missing").expect("list"),
        Vec::new()
    );
    assert_eq!(calls.get("missing").expect("get"), None);
    assert_eq!(calls.list_for_tool("missing").expect("list"), Vec::new());
    assert_eq!(results.get("missing").expect("get"), None);
    assert_eq!(
        memory
            .get_with_options(
                "missing",
                &MemoryGetOptions::default()
                    .with_now(timestamp("2026-01-02T03:04:05Z"))
                    .with_include_expired(true)
            )
            .expect("get"),
        None
    );

    let created = timestamp("2026-01-02T03:04:05Z");
    for (id, source) in [
        ("memory-a", "source-a"),
        ("memory-b", "source-b"),
        ("memory-c", "source-a"),
    ] {
        memory
            .put(&memory_at(id, source, &created, None))
            .expect("put");
    }
    let now = MemoryGetOptions::default().with_now(created.clone());
    assert_eq!(
        memory
            .list_with_options(
                &MemoryListOptions::default()
                    .with_source(" \u{1c}source-a\u{3000} ")
                    .with_now(created.clone())
            )
            .expect("filtered")
            .iter()
            .map(|record| record.memory_id())
            .collect::<Vec<_>>(),
        vec!["memory-a", "memory-c"]
    );
    assert_eq!(
        memory
            .list_with_options(
                &MemoryListOptions::default()
                    .with_source("source-missing")
                    .with_now(created)
            )
            .expect("empty"),
        Vec::new()
    );
    assert_eq!(
        memory
            .get_with_options("memory-a", &now)
            .expect("active")
            .expect("row")
            .source(),
        "source-a"
    );

    // A committed approved approval still reads back with the final status.
    approvals
        .save(&approval("approval-1", "approved"))
        .expect("approved");
    assert_eq!(
        approvals
            .get("approval-1")
            .expect("get")
            .expect("row")
            .status(),
        ApprovalStatus::Approved
    );
}

#[test]
fn default_now_paths_return_unexpired_records() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let memory = MemoryRepository::new(&database);
    let record = memory_at(
        "memory-future",
        "source-1",
        &timestamp("2000-01-01T00:00:00Z"),
        Some(&timestamp("9999-12-31T23:59:59Z")),
    );
    memory.put(&record).expect("put");
    assert_eq!(
        memory.get("memory-future").expect("get"),
        Some(record.clone())
    );
    assert_eq!(memory.list().expect("list"), vec![record]);
}

#[test]
fn persisted_canonical_object_fields_use_the_unbounded_domain() {
    let database = kala_repositories::Database::in_memory().expect("database");
    let specs = DesignSpecRepository::new(&database);
    let large = "a".repeat(1_100_000);
    let source = format!(
        r#"{{"id":"spec-large","name":"Large","summary":"Large attributes.","parameter_schema_digest":null,"attributes_json":{{"large":"{large}"}},"requirement_ids":[],"constraints":[]}}"#
    );
    let spec: kala_repositories::DesignSpec = model(&source);
    assert!(spec.attributes_json().as_str().len() > 1_048_576);
    specs.save(&spec).expect("save large spec");
    let stored = specs.get("spec-large").expect("get").expect("row");
    assert_eq!(stored, spec);
    assert!(stored.attributes_json().as_str().len() > 1_048_576);
    let raw: String = database
        .with_connection(|connection| {
            connection.query_row(
                "SELECT model_json FROM design_specs WHERE id = 'spec-large'",
                [],
                |row| row.get(0),
            )
        })
        .expect("query")
        .expect("row");
    assert!(raw.len() > 1_048_576);
    assert_eq!(raw, spec.to_wire_json().expect("wire"));
}
