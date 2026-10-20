//! Security and durability semantics ported from the Python
//! `tests/security/test_revision_commit.py` and `tests/integration/test_mechanical_loop.py`.
//!
//! Every test asserts the state left behind, not only the error. The cases are:
//! a prepared commit is not a revision; stale approvals, rejections, forks, a
//! lost head race and a revoked capability write nothing; a workflow revision
//! needs its own approved checkpoint; the journal is append-only and verified;
//! and policy/budget state is restored or the launch stops.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Barrier};

use kala_api::config::{SettingValue, Settings, settings_from_values};
use kala_application::{
    BUDGET_STATE_AGGREGATE, Components, ErrorKind, KERNEL_STATE_NAMESPACE, KalaApplication,
    MAX_EVENT_LIMIT, POLICY_STATE_AGGREGATE, kernel_state_aggregate, reserved_workflow_ids,
};
use kala_artifacts::ContentAddressedArtifactStore;
use kala_budget::{BudgetLedger, RunBudget};
use kala_contracts::{ApprovalStatus, Text};
use kala_json::CanonicalValue;
use kala_kernel::policy::PolicyEngine;
use kala_persistence::Database;
use num_bigint::BigInt;

const DESIGN: &str = "security-design";
const WORKFLOW: &str = "workflow-security-design";

fn settings_for(data_dir: &Path) -> Settings {
    let mut values = BTreeMap::new();
    values.insert(
        "data_dir".to_owned(),
        SettingValue::Text(data_dir.to_string_lossy().into_owned()),
    );
    settings_from_values(&values).expect("valid settings")
}

fn open(data_dir: &Path) -> KalaApplication {
    KalaApplication::open(settings_for(data_dir)).expect("open")
}

fn database_path(data_dir: &Path) -> std::path::PathBuf {
    settings_for(data_dir).database_path()
}

fn sql<T>(data_dir: &Path, operation: impl FnOnce(&rusqlite::Connection) -> T) -> T {
    let connection = rusqlite::Connection::open(database_path(data_dir)).expect("open sqlite");
    operation(&connection)
}

fn revision_rows(application: &KalaApplication) -> Vec<(String, i64)> {
    let connection =
        rusqlite::Connection::open(application.settings().database_path()).expect("open sqlite");
    let mut statement = connection
        .prepare("SELECT revision_id, sequence FROM design_revisions ORDER BY sequence")
        .expect("prepare");
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query")
        .collect::<Result<Vec<(String, i64)>, _>>()
        .expect("rows")
}

fn snapshot_aggregates(data_dir: &Path) -> Vec<String> {
    sql(data_dir, |connection| {
        let mut statement = connection
            .prepare("SELECT DISTINCT workflow_id FROM workflow_snapshots ORDER BY workflow_id")
            .expect("prepare");
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query")
            .collect::<Result<Vec<String>, _>>()
            .expect("rows")
    })
}

fn prepare(application: &KalaApplication, design_id: &str) -> kala_application::RevisionCommitView {
    application
        .prepare_design_revision(design_id, "mechanical", None, None, None, &[], &[], None)
        .expect("prepare")
}

fn create(application: &KalaApplication, design_id: &str) -> kala_application::RevisionRecord {
    application
        .create_design_revision(
            design_id,
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
            "local-user",
            None,
            None,
        )
        .expect("create")
}

#[test]
fn fresh_directory_restores_default_kernel_state_without_a_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    {
        let application = open(&data);
        let grants = application.policy().grants();
        assert_eq!(grants.len(), 5);
        assert_eq!(
            application.budget().usage().artifact_bytes().to_string(),
            "0"
        );
        sql(&data, |connection| {
            assert_eq!(
                connection
                    .query_row("SELECT COUNT(*) FROM events", [], |row| row
                        .get::<_, i64>(0))
                    .expect("count"),
                0
            );
            assert_eq!(
                connection
                    .query_row("SELECT COUNT(*) FROM workflow_snapshots", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .expect("count"),
                0
            );
        });
        application.close().expect("close");
    }
    {
        let application = open(&data);
        assert_eq!(application.policy().grants().len(), 5);
        assert_eq!(
            application.budget().usage().artifact_bytes().to_string(),
            "0"
        );
        application.close().expect("close");
    }
}

#[test]
fn kernel_state_is_stored_under_the_reserved_namespace() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    {
        let application = open(&data);
        create(&application, DESIGN);
        application.close().expect("close");
    }
    let mut aggregates = snapshot_aggregates(&data);
    let mut expected = vec![
        kernel_state_aggregate(POLICY_STATE_AGGREGATE).expect("name"),
        kernel_state_aggregate(BUDGET_STATE_AGGREGATE).expect("name"),
    ];
    aggregates.sort();
    expected.sort();
    assert_eq!(aggregates, expected);
    for bare in reserved_workflow_ids() {
        assert!(!aggregates.iter().any(|value| value == bare));
    }
}

#[test]
fn reserved_workflow_identifiers_are_refused_before_any_work() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    let reserved = [
        POLICY_STATE_AGGREGATE.to_owned(),
        BUDGET_STATE_AGGREGATE.to_owned(),
        kernel_state_aggregate(POLICY_STATE_AGGREGATE).expect("name"),
        format!(
            "{}-0123456789ab",
            kernel_state_aggregate(BUDGET_STATE_AGGREGATE).expect("name")
        ),
        "kernel-state:anything".to_owned(),
    ];
    for workflow_id in reserved {
        let error = application
            .run_mechanical_reference_workflow(Some(&workflow_id), DESIGN, "local-user")
            .expect_err("reserved");
        assert_eq!(error.code(), "reserved_identifier");
        assert_eq!(
            application
                .get_workflow_state(&workflow_id)
                .unwrap_err()
                .code(),
            "reserved_identifier"
        );
        assert_eq!(
            application
                .list_workflow_checkpoints(&workflow_id)
                .unwrap_err()
                .code(),
            "reserved_identifier"
        );
        assert_eq!(
            application
                .approve_workflow_checkpoint(&workflow_id, "local-user", None, None, None)
                .unwrap_err()
                .code(),
            "reserved_identifier"
        );
        assert_eq!(
            application
                .reject_workflow_checkpoint(&workflow_id, "local-user", None, None, None)
                .unwrap_err()
                .code(),
            "reserved_identifier"
        );
        assert_eq!(
            application
                .list_events(0, 50, Some(&workflow_id), None, None)
                .unwrap_err()
                .code(),
            "reserved_identifier"
        );
        assert_eq!(
            application
                .prepare_design_revision(
                    DESIGN,
                    "mechanical",
                    None,
                    None,
                    None,
                    &[],
                    &[],
                    Some(&workflow_id)
                )
                .unwrap_err()
                .code(),
            "reserved_identifier"
        );
    }
    assert_eq!(
        application
            .list_workflow_checkpoints(&format!("workflow-{DESIGN}"))
            .expect("empty"),
        Vec::new()
    );
    let aggregates = snapshot_aggregates(&data);
    assert!(
        aggregates
            .iter()
            .all(|value| value.starts_with(KERNEL_STATE_NAMESPACE)),
        "only kernel state may be stored before any workflow starts: {aggregates:?}"
    );
    application.close().expect("close");
}

#[test]
fn a_prepared_commit_is_not_a_revision_and_needs_an_approval() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    let prepared = prepare(&application, DESIGN);
    assert_eq!(prepared.status, ApprovalStatus::Pending);
    assert!(!prepared.committed);
    assert_eq!(prepared.revision_id, format!("rev-{DESIGN}-r0"));
    assert_eq!(prepared.sequence, BigInt::from(0));
    assert!(prepared.parent_revision_id.is_none());
    assert_eq!(prepared.state_digest.len(), 64);
    assert_eq!(prepared.action_digest.len(), 64);
    assert_ne!(prepared.spec_digest, prepared.state_digest);
    assert!(
        application
            .list_design_revisions(DESIGN, 1_000)
            .unwrap()
            .is_empty()
    );
    assert!(revision_rows(&application).is_empty());

    let record = application
        .commit_design_revision(
            &prepared.request_id,
            "security-operator",
            Some(&prepared.state_digest),
            Some(&prepared.action_digest),
            None,
            None,
        )
        .expect("commit");
    assert_eq!(record.revision.revision_id(), format!("rev-{DESIGN}-r0"));
    assert_eq!(
        revision_rows(&application),
        vec![(format!("rev-{DESIGN}-r0"), 0)]
    );
    let events = application
        .list_events(0, 100, None, None, None)
        .expect("events");
    let types: Vec<&str> = events
        .events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect();
    assert_eq!(
        types
            .iter()
            .filter(|value| **value == "revision.prepared")
            .count(),
        1
    );
    assert_eq!(
        types
            .iter()
            .filter(|value| **value == "revision.created")
            .count(),
        1
    );
    application.close().expect("close");
}

#[test]
fn a_stale_approval_writes_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    let reviewed = application
        .prepare_design_revision(
            DESIGN,
            "mechanical",
            Some("the configuration under review"),
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("reviewed");
    let other = application
        .prepare_design_revision(
            DESIGN,
            "mechanical",
            Some("a different configuration"),
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("other");
    application
        .commit_design_revision(&other.request_id, "local-user", None, None, None, None)
        .expect("commit other");
    assert_eq!(
        revision_rows(&application),
        vec![(format!("rev-{DESIGN}-r0"), 0)]
    );
    let error = application
        .commit_design_revision(&reviewed.request_id, "local-user", None, None, None, None)
        .expect_err("stale");
    assert_eq!(error.code(), "checkpoint_digest_mismatch");
    assert_eq!(
        application
            .get_revision_commit(&reviewed.request_id)
            .expect("read")
            .status,
        ApprovalStatus::Pending
    );

    let supplied = prepare(&application, "supplied");
    assert_eq!(
        application
            .commit_design_revision(
                &supplied.request_id,
                "local-user",
                Some(&"0".repeat(64)),
                None,
                None,
                None
            )
            .expect_err("stale state")
            .code(),
        "checkpoint_digest_mismatch"
    );
    assert_eq!(
        application
            .commit_design_revision(
                &supplied.request_id,
                "local-user",
                None,
                Some(&"0".repeat(64)),
                None,
                None
            )
            .expect_err("stale action")
            .code(),
        "checkpoint_digest_mismatch"
    );
    assert!(
        application
            .list_design_revisions("supplied", 1_000)
            .unwrap()
            .is_empty()
    );
    let record = application
        .commit_design_revision(
            &supplied.request_id,
            "local-user",
            Some(&supplied.state_digest),
            Some(&supplied.action_digest),
            None,
            None,
        )
        .expect("commit supplied");
    assert_eq!(record.revision.revision_id(), "rev-supplied-r0");
    application.close().expect("close");
}

#[test]
fn a_rejected_approval_writes_nothing_and_cannot_be_reused() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    let prepared = prepare(&application, DESIGN);
    let rejected = application
        .decide_revision_commit(
            &prepared.request_id,
            "reject",
            "security-operator",
            None,
            None,
            Some("the surrogate result is not what the operator reviewed"),
            None,
        )
        .expect("reject");
    assert_eq!(rejected.status, ApprovalStatus::Rejected);
    assert!(!rejected.committed);
    assert!(
        application
            .list_design_revisions(DESIGN, 1_000)
            .unwrap()
            .is_empty()
    );
    assert!(revision_rows(&application).is_empty());
    let types: Vec<String> = application
        .list_events(0, 100, None, None, None)
        .expect("events")
        .events
        .into_iter()
        .map(|event| event.event_type)
        .collect();
    assert!(types.contains(&"revision.commit_rejected".to_owned()));
    assert!(!types.contains(&"revision.created".to_owned()));
    assert_eq!(
        application
            .commit_design_revision(&prepared.request_id, "local-user", None, None, None, None)
            .expect_err("decided")
            .code(),
        "checkpoint_not_pending"
    );
    let read_back = application
        .get_revision_commit(&prepared.request_id)
        .expect("read");
    assert_eq!(read_back.status, ApprovalStatus::Rejected);
    assert_eq!(read_back.actor_id.as_deref(), Some("security-operator"));
    assert_eq!(
        read_back.reason.as_deref(),
        Some("the surrogate result is not what the operator reviewed")
    );
    let stored = application
        .approval_requests()
        .get(&prepared.request_id)
        .expect("stored")
        .expect("present");
    assert_eq!(stored.status(), ApprovalStatus::Rejected);
    assert_eq!(stored.selected_option_id(), Some("reject"));
    assert_eq!(stored.actor_id(), Some("security-operator"));
    application.close().expect("close");
}

#[test]
fn a_commit_cannot_fork_the_design_history() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    let first = create(&application, DESIGN);
    let second = create(&application, DESIGN);
    let error = application
        .create_design_revision(
            DESIGN,
            "mechanical",
            None,
            None,
            Some(first.revision.revision_id()),
            &[],
            &[],
            None,
            "local-user",
            None,
            None,
        )
        .expect_err("fork");
    assert_eq!(error.code(), "revision_fork_refused");
    assert_eq!(
        application
            .prepare_design_revision(
                DESIGN,
                "mechanical",
                None,
                None,
                Some(first.revision.revision_id()),
                &[],
                &[],
                None
            )
            .expect_err("fork")
            .code(),
        "revision_fork_refused"
    );
    assert_eq!(
        application
            .create_design_revision(
                "unborn",
                "mechanical",
                None,
                None,
                Some(second.revision.revision_id()),
                &[],
                &[],
                None,
                "local-user",
                None,
                None,
            )
            .expect_err("orphan")
            .code(),
        "revision_fork_refused"
    );
    assert_eq!(
        revision_rows(&application),
        vec![
            (first.revision.revision_id().to_owned(), 0),
            (second.revision.revision_id().to_owned(), 1),
        ]
    );
    application.close().expect("close");
}

#[test]
fn the_workflow_revision_needs_its_own_approved_checkpoint() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    let run = application
        .run_mechanical_reference_workflow(Some(WORKFLOW), DESIGN, "local-user")
        .expect("run");
    assert_eq!(run.status.as_str(), "waiting_for_approval");
    let prepared = application
        .prepare_design_revision(
            DESIGN,
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            Some(WORKFLOW),
        )
        .expect("prepared");
    let error = application
        .commit_design_revision(
            &prepared.request_id,
            "local-user",
            None,
            None,
            None,
            Some(WORKFLOW),
        )
        .expect_err("not completed");
    assert!(error.message().contains("has not completed"), "{error}");
    assert!(
        application
            .list_design_revisions(DESIGN, 1_000)
            .unwrap()
            .is_empty()
    );

    let decided = application
        .approve_workflow_checkpoint(WORKFLOW, "operator-a", None, None, None)
        .expect("approve");
    assert_eq!(decided.status.as_str(), "completed");
    let revision = decided.revision.expect("revision");
    assert_eq!(revision.revision.revision_id(), format!("rev-{DESIGN}-r0"));
    let approvals = application
        .list_workflow_checkpoints(WORKFLOW)
        .expect("approvals");
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].status(), ApprovalStatus::Approved);
    assert_eq!(approvals[0].actor_id(), Some("operator-a"));
    assert_eq!(approvals[0].selected_option_id(), Some("approve"));
    assert!(approvals[0].decided_at().is_some());

    let created = application
        .list_events(0, 200, None, Some("revision.created"), None)
        .expect("events")
        .events
        .into_iter()
        .next()
        .expect("created event");
    let payload = created.payload.expect("payload");
    let request_id = match &payload {
        CanonicalValue::Object(map) => map
            .get(&kala_json::JsonText::new("request_id"))
            .and_then(|value| match value {
                CanonicalValue::Text(text) => text.to_utf8().ok(),
                _ => None,
            })
            .expect("request id"),
        _ => panic!("payload object"),
    };
    let commit = application
        .get_revision_commit(&request_id)
        .expect("commit");
    assert_eq!(commit.status, ApprovalStatus::Approved);
    assert!(commit.committed);
    assert_eq!(commit.actor_id.as_deref(), Some("operator-a"));
    assert_eq!(commit.workflow_id.as_deref(), Some(WORKFLOW));
    assert_eq!(commit.sequence, BigInt::from(0));
    let stored = application
        .approval_requests()
        .get(&commit.request_id)
        .expect("stored")
        .expect("present");
    assert_eq!(stored.workflow_id(), commit.request_id);
    assert_eq!(stored.selected_option_id(), Some("approve"));
    application.close().expect("close");
}

#[test]
fn a_rejected_workflow_leaves_no_revision_and_no_authorization() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    let run = application
        .run_mechanical_reference_workflow(Some(WORKFLOW), DESIGN, "local-user")
        .expect("run");
    let decided = application
        .reject_workflow_checkpoint(WORKFLOW, "local-user", None, None, Some("declined"))
        .expect("reject");
    assert_eq!(decided.status.as_str(), "failed");
    assert!(decided.revision.is_none());
    assert!(
        application
            .list_design_revisions(DESIGN, 1_000)
            .unwrap()
            .is_empty()
    );
    let state = application.get_workflow_state(WORKFLOW).expect("state");
    assert_eq!(state.last_error.as_deref(), Some("declined"));
    let prepared = application
        .prepare_design_revision(
            DESIGN,
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            Some(WORKFLOW),
        )
        .expect("prepared");
    let error = application
        .commit_design_revision(
            &prepared.request_id,
            "local-user",
            None,
            None,
            None,
            Some(WORKFLOW),
        )
        .expect_err("not authorized");
    assert!(
        error.message().contains("authorizes no revision commit"),
        "{error}"
    );
    let _ = run;
    application.close().expect("close");
}

#[test]
fn a_workflow_cannot_authorize_another_designs_commit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    application
        .run_mechanical_reference_workflow(Some(WORKFLOW), DESIGN, "local-user")
        .expect("run");
    application
        .approve_workflow_checkpoint(WORKFLOW, "local-user", None, None, None)
        .expect("approve");
    let prepared = prepare(&application, "unrelated");
    let error = application
        .commit_design_revision(
            &prepared.request_id,
            "local-user",
            None,
            None,
            None,
            Some(WORKFLOW),
        )
        .expect_err("another design");
    assert!(
        error.message().contains("was not prepared for workflow"),
        "{error}"
    );
    assert!(
        application
            .list_design_revisions("unrelated", 1_000)
            .unwrap()
            .is_empty()
    );
    application.close().expect("close");
}

#[test]
fn a_completed_workflow_without_a_stored_approval_authorizes_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    application
        .run_mechanical_reference_workflow(Some(WORKFLOW), DESIGN, "local-user")
        .expect("run");
    application
        .approve_workflow_checkpoint(WORKFLOW, "local-user", None, None, None)
        .expect("approve");
    let checkpoint = application
        .list_workflow_checkpoints(WORKFLOW)
        .expect("approvals")
        .into_iter()
        .next()
        .expect("checkpoint");
    sql(&data, |connection| {
        // The repository decodes `model_json`, so the tamper has to rewrite
        // the stored model, exactly as the Python security test does.
        let model: String = connection
            .query_row(
                "SELECT model_json FROM approval_requests WHERE request_id = ?",
                rusqlite::params![checkpoint.request_id()],
                |row| row.get(0),
            )
            .expect("stored model");
        let model = model
            .replace("\"status\":\"approved\"", "\"status\":\"rejected\"")
            .replace(
                "\"selected_option_id\":\"approve\"",
                "\"selected_option_id\":\"reject\"",
            );
        connection
            .execute(
                "UPDATE approval_requests SET status = ?, model_json = ? WHERE request_id = ?",
                rusqlite::params!["rejected", model, checkpoint.request_id()],
            )
            .expect("tamper");
    });
    let prepared = application
        .prepare_design_revision(
            DESIGN,
            "mechanical",
            Some("a second configuration of the same design"),
            None,
            None,
            &[],
            &[],
            Some(WORKFLOW),
        )
        .expect("prepared");
    let error = match application.commit_design_revision(
        &prepared.request_id,
        "local-user",
        None,
        None,
        None,
        Some(WORKFLOW),
    ) {
        Ok(_) => panic!("no single approved checkpoint authorizes this commit"),
        Err(error) => error,
    };
    assert!(
        error
            .message()
            .contains("no single approved pre-commit checkpoint"),
        "{error}"
    );
    assert_eq!(
        application
            .list_design_revisions(DESIGN, 1_000)
            .expect("revisions")
            .iter()
            .map(|record| record.revision.revision_id().to_owned())
            .collect::<Vec<_>>(),
        vec![format!("rev-{DESIGN}-r0")]
    );
    application.close().expect("close");
}

#[test]
fn a_revoked_capability_prevents_the_write_and_stays_revoked_across_a_restart() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let request_id;
    {
        let application = open(&data);
        let prepared = prepare(&application, DESIGN);
        request_id = prepared.request_id.clone();
        assert!(
            application
                .policy()
                .revoke(&Text::new("design.write"))
                .unwrap()
        );
        let error = application
            .commit_design_revision(&request_id, "local-user", None, None, None, None)
            .expect_err("revoked");
        assert!(
            error.message().contains("capability_not_granted"),
            "{error}"
        );
        assert!(
            application
                .list_design_revisions(DESIGN, 1_000)
                .unwrap()
                .is_empty()
        );
        application.close().expect("close");
    }
    {
        let application = open(&data);
        assert!(
            !application
                .policy()
                .grants()
                .contains(&Text::new("design.write"))
        );
        application
            .commit_design_revision(&request_id, "local-user", None, None, None, None)
            .expect_err("still revoked");
        assert!(
            application
                .list_design_revisions(DESIGN, 1_000)
                .unwrap()
                .is_empty()
        );
        assert!(
            application
                .policy()
                .grant_capability(Text::new("design.write"))
                .is_ok()
        );
        application
            .commit_design_revision(&request_id, "local-user", None, None, None, None)
            .expect("granted");
        assert_eq!(
            revision_rows(&application),
            vec![(format!("rev-{DESIGN}-r0"), 0)]
        );
        application.close().expect("close");
    }
}

#[test]
fn a_clean_restart_restores_usage_grants_and_approvals() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let (grants, artifact_bytes, request_id);
    {
        let application = open(&data);
        let prepared = prepare(&application, DESIGN);
        request_id = prepared.request_id.clone();
        grants = application.policy().grants();
        artifact_bytes = application.budget().usage().artifact_bytes().to_string();
        assert_ne!(artifact_bytes, "0");
        assert!(
            application
                .policy()
                .get_approval(&Text::new(&request_id), None, None)
                .expect("approval")
                .is_some()
        );
        application.close().expect("close");
    }
    {
        let application = open(&data);
        assert_eq!(
            application.budget().usage().artifact_bytes().to_string(),
            artifact_bytes
        );
        assert_eq!(application.policy().grants(), grants);
        let restored = application
            .policy()
            .get_approval(&Text::new(&request_id), None, None)
            .expect("approval")
            .expect("present");
        assert_eq!(restored.status(), ApprovalStatus::Pending);
        assert_eq!(
            application
                .get_revision_commit(&request_id)
                .expect("read")
                .status,
            ApprovalStatus::Pending
        );
        let decided = application
            .decide_revision_commit(
                &request_id,
                "approve",
                "restarted-operator",
                None,
                None,
                None,
                None,
            )
            .expect("approve");
        assert!(decided.committed);
        application.close().expect("close");
    }
    {
        let application = open(&data);
        assert_eq!(
            application
                .list_design_revisions(DESIGN, 1_000)
                .expect("revisions")
                .len(),
            1
        );
        let approved = application
            .policy()
            .get_approval(&Text::new(&request_id), None, None)
            .expect("approval")
            .expect("present");
        assert_eq!(approved.status(), ApprovalStatus::Approved);
        assert_eq!(approved.actor_id(), Some(&Text::new("restarted-operator")));
        application.close().expect("close");
    }
}

#[test]
fn a_tampered_kernel_state_record_stops_the_launch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    {
        let application = open(&data);
        create(&application, DESIGN);
        application.close().expect("close");
    }
    sql(&data, |connection| {
        connection
            .execute(
                "UPDATE workflow_snapshots SET digest = 'tampered' WHERE workflow_id = ?",
                rusqlite::params![kernel_state_aggregate(POLICY_STATE_AGGREGATE).expect("name")],
            )
            .expect("tamper");
    });
    let error = KalaApplication::open(settings_for(&data)).expect_err("tampered");
    assert_eq!(error.kind(), ErrorKind::CorruptKernelState);
    assert_eq!(error.code(), "corrupt_kernel_state");
    assert!(error.message().contains("verification"), "{error}");
    // A failed launch must not heal the record by writing a fresh snapshot.
    assert!(KalaApplication::open(settings_for(&data)).is_err());
}

#[test]
fn a_kernel_state_record_that_does_not_validate_stops_the_launch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    {
        let application = open(&data);
        create(&application, DESIGN);
        application.close().expect("close");
    }
    let corrupted = "{\"grants\":\"not-a-collection\",\"version\":1}";
    let digest = kala_tools::protocol::sha256_hex(corrupted.as_bytes());
    sql(&data, |connection| {
        connection
            .execute(
                "UPDATE workflow_snapshots SET state_json = ?, digest = ? WHERE workflow_id = ?",
                rusqlite::params![
                    corrupted,
                    digest,
                    kernel_state_aggregate(POLICY_STATE_AGGREGATE).expect("name")
                ],
            )
            .expect("tamper");
    });
    let error = KalaApplication::open(settings_for(&data)).expect_err("invalid");
    assert_eq!(error.code(), "corrupt_kernel_state");
    assert!(error.message().contains("policy state"), "{error}");
    assert!(KalaApplication::open(settings_for(&data)).is_err());
}

#[test]
fn missing_kernel_state_in_a_used_directory_stops_the_launch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    {
        let application = open(&data);
        create(&application, DESIGN);
        application.close().expect("close");
    }
    sql(&data, |connection| {
        connection
            .execute(
                "DELETE FROM workflow_snapshots WHERE workflow_id = ?",
                rusqlite::params![kernel_state_aggregate(BUDGET_STATE_AGGREGATE).expect("name")],
            )
            .expect("delete");
    });
    let error = KalaApplication::open(settings_for(&data)).expect_err("missing");
    assert_eq!(error.code(), "corrupt_kernel_state");
    assert!(error.message().contains("budget state"), "{error}");
    assert!(error.message().contains("missing"), "{error}");
    assert!(KalaApplication::open(settings_for(&data)).is_err());
}

#[test]
fn a_legacy_kernel_state_record_is_still_restored() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let (grants, artifact_bytes);
    {
        let application = open(&data);
        create(&application, DESIGN);
        grants = application.policy().grants();
        artifact_bytes = application.budget().usage().artifact_bytes().to_string();
        application.close().expect("close");
    }
    sql(&data, |connection| {
        for aggregate in [POLICY_STATE_AGGREGATE, BUDGET_STATE_AGGREGATE] {
            let namespaced = kernel_state_aggregate(aggregate).expect("name");
            connection
                .execute(
                    "UPDATE workflow_snapshots SET snapshot_id = snapshot_id || '-legacy', workflow_id = ? WHERE workflow_id = ?",
                    rusqlite::params![aggregate, namespaced],
                )
                .expect("move");
        }
    });
    let application = open(&data);
    assert_eq!(application.policy().grants(), grants);
    assert_eq!(
        application.budget().usage().artifact_bytes().to_string(),
        artifact_bytes
    );
    application.close().expect("close");
}

#[test]
fn the_journal_is_append_only_and_verified() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    create(&application, DESIGN);
    let database = rusqlite::Connection::open(database_path(&data)).expect("open");
    let delete = database.execute("DELETE FROM events", []);
    assert!(
        delete.is_err(),
        "the append-only trigger must refuse a delete"
    );
    drop(database);
    assert!(application.verify_chain().is_ok());
    application.close().expect("close");

    // An attacker with database access can drop the guard; verification still
    // notices the edited payload.
    sql(&data, |connection| {
        connection
            .execute("DROP TRIGGER events_prevent_update", [])
            .expect("drop");
        connection
            .execute(
                "UPDATE events SET payload_json = '{}' WHERE sequence = 1",
                [],
            )
            .expect("edit");
    });
    let error = KalaApplication::open(settings_for(&data)).expect_err("tampered journal");
    assert_eq!(error.code(), "application_error");
    assert!(error.message().contains("journal"), "{error}");
}

#[test]
fn closed_application_refuses_further_work() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    create(&application, DESIGN);
    application.close().expect("close");
    let error = application
        .list_events(0, 50, None, None, None)
        .expect_err("closed");
    assert_eq!(error.code(), "application_closed");
    assert_eq!(
        application
            .evaluate_domain_design("mechanical", None, None)
            .expect_err("closed")
            .code(),
        "application_closed"
    );
    assert!(application.closed());
    // A second close is a no-op.
    application.close().expect("second close");
}

#[test]
fn injected_components_belong_to_the_caller() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let settings = settings_for(&data);
    let database = Arc::new(Database::open(settings.database_path()).expect("database"));
    let store = ContentAddressedArtifactStore::new(
        settings.artifact_dir(),
        kala_artifacts::DEFAULT_MAX_OBJECT_SIZE,
    )
    .expect("artifacts");
    let ledger = Arc::new(BudgetLedger::new(RunBudget::new()));
    let policy = Arc::new(
        PolicyEngine::from_capabilities(
            [
                "artifact.write",
                "design.write",
                "artifact.read",
                "design.read",
                "tool.execute",
            ]
            .into_iter()
            .map(Text::new)
            .collect::<Vec<_>>(),
        )
        .expect("policy"),
    );
    let application = KalaApplication::with_components(
        settings.clone(),
        Components {
            database: Some(Arc::clone(&database)),
            artifact_store: Some(store),
            budget: Some(Arc::clone(&ledger)),
            policy: Some(Arc::clone(&policy)),
            ..Components::default()
        },
    )
    .expect("open with injected components");
    let record = create(&application, DESIGN);
    assert!(!record.provenance.provenance_id().is_empty());
    application.close().expect("close");
    assert!(application.closed());
    // An injected database is not closed by the application.
    assert!(!database.closed());
    assert!(
        database
            .with_connection(|connection| connection
                .query_row("SELECT COUNT(*) FROM design_revisions", [], |row| row
                    .get::<_, i64>(0))
                .expect("count"))
            .is_ok()
    );
    database.close().expect("caller closes");
}

#[test]
fn the_head_compare_and_swap_is_transactional() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let writer = open(&data);
    let prepared = prepare(&writer, DESIGN);
    {
        let winner = open(&data);
        winner
            .create_design_revision(
                DESIGN,
                "mechanical",
                Some("committed while the review was open"),
                None,
                None,
                &[],
                &[],
                None,
                "local-user",
                None,
                None,
            )
            .expect("winner");
        winner.close().expect("close");
    }
    let error = writer
        .commit_design_revision(&prepared.request_id, "local-user", None, None, None, None)
        .expect_err("head moved");
    assert_eq!(error.code(), "checkpoint_digest_mismatch");
    assert_eq!(
        revision_rows(&writer),
        vec![(format!("rev-{DESIGN}-r0"), 0)]
    );
    writer.close().expect("close");
    let sequences = sql(&data, |connection| {
        let mut statement = connection
            .prepare("SELECT sequence FROM design_revisions ORDER BY sequence")
            .expect("prepare");
        statement
            .query_map([], |row| row.get::<_, i64>(0))
            .expect("query")
            .collect::<Result<Vec<i64>, _>>()
            .expect("rows")
    });
    assert_eq!(sequences, vec![0]);
}

#[test]
fn two_concurrent_commits_produce_one_revision() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let first = Arc::new(open(&data));
    let second = Arc::new(open(&data));
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for (application, load) in [(Arc::clone(&first), 8.0), (Arc::clone(&second), 9.0)] {
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let parameters = CanonicalValue::object([(
                kala_json::JsonText::new("load_kn"),
                CanonicalValue::Float(load),
            )]);
            application
                .create_design_revision(
                    DESIGN,
                    "mechanical",
                    None,
                    Some(&parameters),
                    None,
                    &[],
                    &[],
                    None,
                    "local-user",
                    None,
                    None,
                )
                .map(|record| record.revision.revision_id().to_owned())
                .map_err(|error| error.kind())
        }));
    }
    let mut results = Vec::new();
    let mut errors = Vec::new();
    for handle in handles {
        match handle.join().expect("thread") {
            Ok(value) => results.push(value),
            Err(kind) => errors.push(kind),
        }
    }
    assert_eq!(results.len(), 1, "exactly one commit wins");
    assert!(
        errors.iter().all(|kind| matches!(
            kind,
            ErrorKind::CheckpointDigestMismatch
                | ErrorKind::RevisionHeadMoved
                | ErrorKind::CheckpointDecisionDenied
        )),
        "unexpected losing errors: {errors:?}"
    );
    assert_eq!(revision_rows(&first), vec![(format!("rev-{DESIGN}-r0"), 0)]);
    first.close().expect("close");
    second.close().expect("close");
}

#[test]
fn identifier_and_limit_validation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    assert_eq!(
        application
            .evaluate_domain_design("absent", None, None)
            .expect_err("unknown domain")
            .code(),
        "unknown_domain"
    );
    assert_eq!(
        application
            .get_design_revision("rev-absent-r0")
            .expect_err("unknown revision")
            .code(),
        "unknown_revision"
    );
    assert_eq!(
        application
            .get_revision_commit("commit-absent")
            .expect_err("unknown prepared")
            .code(),
        "unknown_prepared_revision"
    );
    assert_eq!(
        application
            .get_workflow_state("workflow-absent")
            .expect_err("unknown workflow")
            .code(),
        "unknown_workflow"
    );
    assert_eq!(
        application
            .list_events(0, 0, None, None, None)
            .expect_err("bad limit")
            .code(),
        "invalid_parameters"
    );
    assert_eq!(
        application
            .list_events(0, MAX_EVENT_LIMIT + 1, None, None, None)
            .expect_err("bad limit")
            .code(),
        "invalid_parameters"
    );
    assert_eq!(
        application
            .list_events(-1, 10, None, None, None)
            .expect_err("negative")
            .message(),
        "after_sequence must be nonnegative"
    );
    assert_eq!(
        application
            .list_events(0, 10, None, Some("not a type"), None)
            .expect_err("bad type")
            .code(),
        "invalid_identifier"
    );
    let error = application
        .evaluate_domain_design(
            "mechanical",
            Some(&CanonicalValue::object([(
                kala_json::JsonText::new("nope"),
                CanonicalValue::integer(1),
            )])),
            None,
        )
        .expect_err("extra parameter");
    assert_eq!(error.code(), "invalid_parameters");
    let error = application
        .evaluate_domain_design(
            "mechanical",
            Some(&CanonicalValue::object([(
                kala_json::JsonText::new("length_mm"),
                CanonicalValue::text("abc"),
            )])),
            None,
        )
        .expect_err("wrong type");
    assert_eq!(error.code(), "invalid_parameters");
    let parts = "bad id!";
    {
        let prepared = application
            .prepare_design_revision(parts, "mechanical", None, None, None, &[], &[], None)
            .expect_err("bad identifier");
        assert_eq!(prepared.code(), "invalid_identifier");
        assert_eq!(
            prepared.message(),
            "design_id must start with a letter or digit and contain only safe characters"
        );
    }
    application.close().expect("close");
}
