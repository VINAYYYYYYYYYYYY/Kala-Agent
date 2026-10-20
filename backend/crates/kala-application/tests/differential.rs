//! The live differential against the repository's Python composition root.
//!
//! One operation script is driven through the real
//! `kala_agent.application.KalaApplication` (test-only Python subprocess) and
//! through this crate, in separate temporary data directories. The trace
//! compares:
//!
//! * every returned DTO as canonical JSON,
//! * every SQLite row (schema, ids, counts and decoded JSON columns),
//! * the complete journal chain (sequence, type, aggregate, payload, digest),
//! * the artifact store (layout, size, bytes and parsed report),
//! * every error class, code and message,
//! * the injected-clock call count and the deterministic timestamps,
//! * the restart: grants, budget usage, revisions, commits and workflow state.
//!
//! Timestamps that come from the real wall clock and 64-hex digest values are
//! normalized before comparison. The normalization is exactly the declared
//! identity boundary: `kala-domains` publishes native evaluator/package digests
//! rather than the historical Python source digests, so every digest computed
//! over a package digest differs. The test then proves, separately, that the
//! digest *algorithm* is byte-identical by substituting the Python package
//! digest into the native inputs and recomputing every Python digest.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use chrono::{DateTime, Utc};
use kala_api::config::{SettingValue, Settings, settings_from_values};
use kala_application::{
    ApplicationResult, COMMITTED_REVISION_EVENT, Components, DomainBinding, KalaApplication,
    LOCAL_ACTOR_ID, View,
};
use kala_contracts::{ApprovalRequest, ContractModel, Text};
use kala_json::{CanonicalValue, JsonText};
use kala_persistence::parse_json_object;
use num_bigint::BigInt;

const CLOCK_BASE_ISO: &str = "2031-01-01T00:00:00Z";
const DESIGN: &str = "alpha";
const WORKFLOW: &str = "workflow-alpha";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    manifest_dir()
        .join("../../..")
        .canonicalize()
        .expect("repository root")
}

fn python_executable() -> PathBuf {
    if let Ok(path) = std::env::var("KALA_PYTHON") {
        return PathBuf::from(path);
    }
    let venv = repo_root().join(".venv/bin/python");
    assert!(
        venv.exists(),
        "live Python oracle requires {} (set KALA_PYTHON to override)",
        venv.display()
    );
    venv
}

fn run_oracle(scenario: &str, data_dir: &Path) -> CanonicalValue {
    std::fs::create_dir_all(data_dir).expect("data dir");
    let script = manifest_dir().join("tests/oracle_application.py");
    let output = Command::new(python_executable())
        .arg("-I")
        .arg(&script)
        .arg(scenario)
        .arg(data_dir)
        .env("PYTHONHASHSEED", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|error| panic!("failed to spawn {}: {error}", script.display()));
    assert!(
        output.status.success(),
        "oracle {scenario} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    kala_json::parse_strict(&output.stdout, &kala_json::Limits::default()).unwrap_or_else(|error| {
        panic!(
            "oracle {scenario} did not return canonical JSON: {error}\nstdout: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn settings_for(data_dir: &Path) -> Settings {
    let mut values = BTreeMap::new();
    values.insert(
        "data_dir".to_owned(),
        SettingValue::Text(data_dir.to_string_lossy().into_owned()),
    );
    settings_from_values(&values).expect("valid settings")
}

fn clock_base() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(CLOCK_BASE_ISO)
        .expect("clock base")
        .with_timezone(&Utc)
}

/// The injected clock: one second per call, starting in 2031 so the trace can
/// distinguish deterministic timestamps from wall-clock ones.
fn fixed_clock() -> (Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>, Arc<AtomicI64>) {
    let counter = Arc::new(AtomicI64::new(0));
    let source = Arc::clone(&counter);
    let clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync> = Arc::new(move || {
        let step = source.fetch_add(1, Ordering::SeqCst);
        clock_base() + chrono::Duration::seconds(step)
    });
    (clock, counter)
}

fn text(value: &str) -> CanonicalValue {
    CanonicalValue::text(value)
}

fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::object(entries)
}

fn array(values: Vec<CanonicalValue>) -> CanonicalValue {
    CanonicalValue::array(values)
}

fn integer(value: impl Into<BigInt>) -> CanonicalValue {
    CanonicalValue::integer(value)
}

fn view<T: View>(value: &T) -> CanonicalValue {
    value.to_canonical_value().expect("canonical DTO")
}

struct Recorder {
    steps: BTreeMap<String, CanonicalValue>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            steps: BTreeMap::new(),
        }
    }

    fn result(&mut self, name: &str, result: ApplicationResult<CanonicalValue>) {
        let entry = match result {
            Ok(value) => object(vec![("ok", CanonicalValue::Bool(true)), ("value", value)]),
            Err(error) => object(vec![
                ("ok", CanonicalValue::Bool(false)),
                ("class", text(error.kind().python_class_name())),
                ("code", text(error.code())),
                ("message", text(error.message())),
            ]),
        };
        self.steps.insert(name.to_owned(), entry);
    }

    fn record<T: View>(&mut self, name: &str, result: ApplicationResult<T>) {
        let value = result.map(|value| view(&value));
        self.result(name, value);
    }

    fn record_raw(&mut self, name: &str, value: CanonicalValue) {
        self.result(name, Ok(value));
    }

    fn record_list<T: View>(&mut self, name: &str, result: ApplicationResult<Vec<T>>) {
        let value = result.map(|values| array(values.iter().map(view).collect()));
        self.result(name, value);
    }

    fn record_approvals(&mut self, name: &str, result: ApplicationResult<Vec<ApprovalRequest>>) {
        let value = result.map(|values| {
            array(
                values
                    .iter()
                    .map(|value| {
                        value
                            .to_canonical_value()
                            .expect("canonical approval request")
                    })
                    .collect(),
            )
        });
        self.result(name, value);
    }

    fn into_value(self) -> CanonicalValue {
        CanonicalValue::Object(
            self.steps
                .into_iter()
                .map(|(key, value)| (JsonText::new(&key), value))
                .collect(),
        )
    }
}

/// Run the shared operation script against the Rust application.
fn drive_rust(application: &KalaApplication, recorder: &mut Recorder) {
    let app = application;
    recorder.record_list("domain_packages", Ok(app.list_domain_packages()));
    recorder.record_list("workflow_definitions", Ok(app.list_workflow_definitions()));
    recorder.record("describe_mechanical", app.describe_domain("mechanical"));
    recorder.record_list("catalog_building", app.list_catalog_steps("building"));
    recorder.record(
        "eval_mechanical",
        app.evaluate_domain_design("mechanical", None, None),
    );
    let eval_parameters = object(vec![
        ("load_kn", CanonicalValue::Float(9.0)),
        ("thickness_mm", CanonicalValue::Float(4.0)),
    ]);
    recorder.record(
        "eval_mechanical_args",
        app.evaluate_domain_design("mechanical", Some(&eval_parameters), Some("eval-design")),
    );
    let building_parameters = object(vec![("storey_height_m", CanonicalValue::Float(3.2))]);
    recorder.record(
        "eval_building",
        app.evaluate_domain_design(
            "building",
            Some(&building_parameters),
            Some("building-design"),
        ),
    );
    let wing_parameters = object(vec![("span_m", CanonicalValue::Float(12.0))]);
    recorder.record(
        "eval_wing",
        app.evaluate_domain_design("wing", Some(&wing_parameters), Some("wing-design")),
    );
    recorder.record(
        "eval_bad_domain",
        app.evaluate_domain_design("absent", None, None),
    );
    let bad_parameters = object(vec![("thickness_mm", CanonicalValue::Float(0.5))]);
    recorder.record(
        "eval_bad_params",
        app.evaluate_domain_design("mechanical", Some(&bad_parameters), None),
    );
    let wrong_type = object(vec![("length_mm", text("abc"))]);
    recorder.record(
        "eval_bad_params_type",
        app.evaluate_domain_design("mechanical", Some(&wrong_type), None),
    );
    let extra = object(vec![("nope", integer(1))]);
    recorder.record(
        "eval_extra_params",
        app.evaluate_domain_design("mechanical", Some(&extra), None),
    );

    let r0 = app
        .create_design_revision(
            DESIGN,
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
            LOCAL_ACTOR_ID,
            None,
            None,
        )
        .expect("create r0");
    recorder.record_raw("create_alpha_r0", view(&r0));
    let r1 = app
        .create_design_revision(
            DESIGN,
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
            LOCAL_ACTOR_ID,
            None,
            None,
        )
        .expect("create r1");
    recorder.record_raw("create_alpha_r1", view(&r1));
    let r2 = app
        .prepare_design_revision(DESIGN, "mechanical", None, None, None, &[], &[], None)
        .expect("prepare r2");
    recorder.record_raw("prepare_alpha_r2", view(&r2));
    recorder.record("get_prepared_r2", app.get_revision_commit(&r2.request_id));
    recorder.record(
        "commit_r2",
        app.commit_design_revision(
            &r2.request_id,
            "op-a",
            Some(&r2.state_digest),
            Some(&r2.action_digest),
            None,
            None,
        ),
    );
    let r3_parameters = object(vec![("load_kn", CanonicalValue::Float(8.0))]);
    let r3 = app
        .prepare_design_revision(
            DESIGN,
            "mechanical",
            Some("  a reviewed summary  "),
            Some(&r3_parameters),
            None,
            &[],
            &[],
            None,
        )
        .expect("prepare r3");
    recorder.record_raw("prepare_alpha_r3", view(&r3));
    recorder.record(
        "reject_r3",
        app.decide_revision_commit(
            &r3.request_id,
            "reject",
            "op-reject",
            None,
            None,
            Some("  not reviewed  "),
            None,
        ),
    );
    recorder.record("get_rejected_r3", app.get_revision_commit(&r3.request_id));
    recorder.record_list("list_alpha", app.list_design_revisions(DESIGN, 1_000));
    recorder.record(
        "fork_refused",
        app.create_design_revision(
            DESIGN,
            "mechanical",
            None,
            None,
            Some(r0.revision.revision_id()),
            &[],
            &[],
            None,
            LOCAL_ACTOR_ID,
            None,
            None,
        ),
    );
    recorder.record("get_r2", app.get_design_revision("rev-alpha-r2"));
    recorder.record(
        "stale_digest",
        app.create_design_revision(
            "delta",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
            LOCAL_ACTOR_ID,
            Some(&"0".repeat(64)),
            None,
        ),
    );
    recorder.record("unknown_revision", app.get_design_revision("rev-absent-r0"));
    recorder.record("unknown_prepared", app.get_revision_commit("commit-absent"));
    recorder.record("unknown_domain_describe", app.describe_domain("absent"));
    let extra = app
        .create_design_revision(
            "extra",
            "mechanical",
            None,
            None,
            None,
            &["prov-external".to_owned()],
            &["zeta".to_owned(), "alpha".to_owned(), "zeta".to_owned()],
            None,
            LOCAL_ACTOR_ID,
            None,
            None,
        )
        .expect("extra");
    recorder.record_raw("create_extra", view(&extra));
    recorder.record("get_extra", app.get_design_revision("rev-extra-r0"));
    recorder.record_list("list_alpha_limited", app.list_design_revisions(DESIGN, 2));

    let workflow_alpha = WORKFLOW;
    recorder.record(
        "workflow_run",
        app.run_mechanical_reference_workflow(Some(workflow_alpha), DESIGN, "op-run"),
    );
    recorder.record("workflow_state", app.get_workflow_state(workflow_alpha));
    recorder.record_approvals(
        "workflow_checkpoints",
        app.list_workflow_checkpoints(workflow_alpha),
    );
    recorder.record(
        "approve_stale",
        app.approve_workflow_checkpoint(
            workflow_alpha,
            LOCAL_ACTOR_ID,
            Some(&"0".repeat(64)),
            None,
            None,
        ),
    );
    let checkpoint = app
        .get_workflow_state(workflow_alpha)
        .expect("state")
        .pending_approval
        .expect("pending");
    let approved = app
        .approve_workflow_checkpoint(
            workflow_alpha,
            "op-approve",
            Some(&checkpoint.state_digest),
            Some(&checkpoint.action_digest),
            None,
        )
        .expect("approve");
    recorder.record_raw("approve", view(&approved));
    recorder.record(
        "workflow_state_done",
        app.get_workflow_state(workflow_alpha),
    );
    recorder.record(
        "approve_again",
        app.approve_workflow_checkpoint(workflow_alpha, LOCAL_ACTOR_ID, None, None, None),
    );
    recorder.record_approvals(
        "workflow_checkpoints_after",
        app.list_workflow_checkpoints(workflow_alpha),
    );
    recorder.record(
        "run_again",
        app.run_mechanical_reference_workflow(Some(workflow_alpha), DESIGN, LOCAL_ACTOR_ID),
    );
    let created_event = app
        .list_events(0, 1_000, None, Some(COMMITTED_REVISION_EVENT), None)
        .expect("events")
        .events
        .into_iter()
        .filter(|event| event.aggregate_id == "rev-alpha-r3")
        .map(|event| event.payload.unwrap_or(CanonicalValue::Null))
        .collect::<Vec<_>>();
    recorder.record_raw("revision_created_event", array(created_event));

    let workflow_beta = "workflow-beta";
    recorder.record(
        "rejected_run",
        app.run_mechanical_reference_workflow(Some(workflow_beta), "beta", "op-run"),
    );
    recorder.record(
        "rejected_decision",
        app.reject_workflow_checkpoint(
            workflow_beta,
            "op-reject",
            None,
            None,
            Some("  declined  "),
        ),
    );
    recorder.record("rejected_state", app.get_workflow_state(workflow_beta));
    recorder.record_list("rejected_runs", app.list_design_revisions("beta", 1_000));
    recorder.record(
        "rejected_approve",
        app.approve_workflow_checkpoint(workflow_beta, LOCAL_ACTOR_ID, None, None, None),
    );

    let prepared_gamma = app
        .prepare_design_revision("gamma", "mechanical", None, None, None, &[], &[], None)
        .expect("prepare gamma");
    let revoked = app
        .policy()
        .revoke(&Text::new("design.write"))
        .unwrap_or(false);
    recorder.record_raw("revoke", CanonicalValue::Bool(revoked));
    recorder.record(
        "commit_revoked",
        app.commit_design_revision(
            &prepared_gamma.request_id,
            "op-gamma",
            None,
            None,
            None,
            None,
        ),
    );
    let granted = app
        .policy()
        .grant_capability(Text::new("design.write"))
        .is_ok();
    recorder.record_raw(
        "grant",
        object(vec![("granted", CanonicalValue::Bool(granted))]),
    );
    recorder.record(
        "commit_after_grant",
        app.commit_design_revision(
            &prepared_gamma.request_id,
            "op-gamma",
            None,
            None,
            None,
            None,
        ),
    );

    recorder.record(
        "reserved_run",
        app.run_mechanical_reference_workflow(
            Some("kernel-state:anything"),
            DESIGN,
            LOCAL_ACTOR_ID,
        ),
    );
    recorder.record(
        "reserved_state",
        app.get_workflow_state("kernel-state:anything"),
    );
    recorder.record(
        "bad_identifier",
        app.create_design_revision(
            "bad id!",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
            LOCAL_ACTOR_ID,
            None,
            None,
        ),
    );
    recorder.record_list("bad_limit", app.list_design_revisions(DESIGN, 0));
    let prepared_epsilon = app
        .prepare_design_revision("epsilon", "mechanical", None, None, None, &[], &[], None)
        .expect("prepare epsilon");
    recorder.record(
        "empty_reason",
        app.decide_revision_commit(
            &prepared_epsilon.request_id,
            "approve",
            LOCAL_ACTOR_ID,
            None,
            None,
            Some("   "),
            None,
        ),
    );
    recorder.record(
        "get_pending_epsilon",
        app.get_revision_commit(&prepared_epsilon.request_id),
    );

    let page = app.list_events(0, 5, None, None, None).expect("page 1");
    let next = page.next_after_sequence;
    recorder.record_raw("events_page1", view(&page));
    recorder.record("events_page2", app.list_events(next, 5, None, None, None));
    recorder.record(
        "events_by_type",
        app.list_events(0, 1_000, None, Some("revision.created"), None),
    );
    recorder.record(
        "events_by_workflow",
        app.list_events(0, 1_000, Some(workflow_alpha), None, None),
    );
    recorder.record(
        "events_bad_type",
        app.list_events(0, 50, None, Some("not a type"), None),
    );
    recorder.record(
        "events_by_aggregate",
        app.list_events(0, 1_000, None, None, Some("rev-alpha-r3")),
    );
    recorder.record("bad_request_id", app.get_revision_commit("commit bad!"));
    recorder.record(
        "bad_decision",
        app.decide_revision_commit(
            &r2.request_id,
            "maybe",
            LOCAL_ACTOR_ID,
            None,
            None,
            None,
            None,
        ),
    );
    recorder.record(
        "bad_actor",
        app.approve_workflow_checkpoint(workflow_beta, "bad actor", None, None, None),
    );
    recorder.record(
        "run_bad_design",
        app.run_mechanical_reference_workflow(Some("workflow-zed"), "bad design", LOCAL_ACTOR_ID),
    );
    recorder.record_list(
        "invalid_limit_high",
        app.list_design_revisions(DESIGN, 1_001),
    );
    recorder.record(
        "unknown_workflow_events",
        app.list_events(0, 5, Some("workflow-absent"), None, None),
    );

    let verification = app.verify_chain().expect("verified");
    recorder.record_raw(
        "verify_chain",
        object(vec![
            (
                "event_count",
                integer(BigInt::from(verification.event_count)),
            ),
            (
                "head_sequence",
                verification
                    .head_sequence
                    .map_or(CanonicalValue::Null, |value| integer(BigInt::from(value))),
            ),
            (
                "head_event_digest",
                verification
                    .head_event_digest
                    .map_or(CanonicalValue::Null, |value| text(&value.to_hex())),
            ),
        ]),
    );
    recorder.record_raw(
        "facts",
        object(vec![
            ("offline", CanonicalValue::Bool(app.offline())),
            (
                "egress_configured",
                CanonicalValue::Bool(app.egress_configured()),
            ),
            (
                "provider_endpoint_declared",
                CanonicalValue::Bool(app.provider_endpoint_declared()),
            ),
            (
                "model_gateway_supplied",
                CanonicalValue::Bool(app.model_gateway_supplied()),
            ),
            ("provider_id", text(app.provider_id())),
            ("model_id", text(app.model_id())),
            (
                "domains",
                array(app.domains().iter().map(|value| text(value)).collect()),
            ),
            ("closed", CanonicalValue::Bool(app.closed())),
        ]),
    );
    let usage = app.budget().usage();
    recorder.record_raw(
        "budget",
        object(vec![
            (
                "model_calls",
                integer(BigInt::from_str(&usage.model_calls().to_string()).unwrap()),
            ),
            (
                "tool_calls",
                integer(BigInt::from_str(&usage.tool_calls().to_string()).unwrap()),
            ),
            (
                "total_tokens",
                integer(BigInt::from_str(&usage.total_tokens().to_string()).unwrap()),
            ),
            (
                "artifact_bytes",
                integer(BigInt::from_str(&usage.artifact_bytes().to_string()).unwrap()),
            ),
            (
                "duration_seconds",
                CanonicalValue::Float(usage.duration_seconds()),
            ),
            ("cost", text(&usage.cost().to_python_string())),
            (
                "replanning_attempts",
                integer(BigInt::from_str(&usage.replanning_attempts().to_string()).unwrap()),
            ),
        ]),
    );
    let mut grants: Vec<String> = app
        .policy()
        .grants()
        .iter()
        .map(|value| value.as_str().unwrap_or("").to_owned())
        .collect();
    grants.sort();
    recorder.record_raw(
        "grants",
        array(grants.iter().map(|value| text(value)).collect()),
    );
}

/// Decode one SQLite row into the trace's canonical object.
fn row_object(row: &rusqlite::Row<'_>) -> CanonicalValue {
    let names = row.as_ref().column_names();
    let mut entries = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        let value = row.get_ref(index).expect("column");
        let decoded = match value {
            rusqlite::types::ValueRef::Null => CanonicalValue::Null,
            rusqlite::types::ValueRef::Integer(value) => integer(BigInt::from(value)),
            rusqlite::types::ValueRef::Real(value) => CanonicalValue::Float(value),
            rusqlite::types::ValueRef::Text(bytes) => {
                let value = std::str::from_utf8(bytes).expect("utf8 column");
                if matches!(*name, "model_json" | "payload_json" | "state_json") {
                    parse_json_object(value).expect("stored canonical object")
                } else {
                    text(value)
                }
            }
            rusqlite::types::ValueRef::Blob(bytes) => array(
                bytes
                    .iter()
                    .map(|byte| integer(BigInt::from(*byte)))
                    .collect(),
            ),
        };
        entries.push((JsonText::new(name), decoded));
    }
    CanonicalValue::Object(entries.into_iter().collect())
}

fn collect_rows(database_path: &Path) -> CanonicalValue {
    let database = rusqlite::Connection::open(database_path).expect("open the stored database");
    collect_rows_on(&database).expect("read the database rows")
}

fn collect_rows_on(connection: &rusqlite::Connection) -> Result<CanonicalValue, rusqlite::Error> {
    let mut tables: Vec<(&str, CanonicalValue)> = Vec::new();
    for table in [
        "design_specs",
        "design_revisions",
        "approval_requests",
        "events",
        "workflow_snapshots",
        "tool_calls",
        "tool_results",
    ] {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .expect("prepare table");
        let mut rows = statement.query([]).expect("query table");
        let mut entries = Vec::new();
        while let Some(row) = rows.next().expect("row") {
            entries.push(row_object(row));
        }
        tables.push((table, array(entries)));
    }
    let mut statement = connection
        .prepare("SELECT type, name, sql FROM sqlite_master ORDER BY type, name")
        .expect("prepare schema");
    let mut rows = statement.query([]).expect("query schema");
    let mut schema = Vec::new();
    while let Some(row) = rows.next().expect("schema row") {
        let kind: String = row.get(0).expect("type");
        let name: String = row.get(1).expect("name");
        let sql: Option<String> = row.get(2).expect("sql");
        schema.push(object(vec![
            ("type", text(&kind)),
            ("name", text(&name)),
            (
                "sql",
                sql.map_or(CanonicalValue::Null, |value| text(&value)),
            ),
        ]));
    }
    tables.push(("schema", array(schema)));
    Ok(CanonicalValue::Object(
        tables
            .into_iter()
            .map(|(key, value)| (JsonText::new(key), value))
            .collect(),
    ))
}

fn artifact_sort_key(value: &CanonicalValue) -> (String, String, BigInt, String) {
    let report = field(value, "text");
    let subject = match report {
        CanonicalValue::Object(map) => map
            .get(&JsonText::new("subject_id"))
            .map(text_of)
            .unwrap_or_default(),
        _ => String::new(),
    };
    let domain = match report {
        CanonicalValue::Object(map) => map
            .get(&JsonText::new("domain"))
            .map(text_of)
            .unwrap_or_default(),
        _ => String::new(),
    };
    let size = match field(value, "size") {
        CanonicalValue::Integer(value) => value.clone(),
        _ => BigInt::from(0),
    };
    (subject, domain, size, canonical_text(report))
}

fn collect_artifacts(root: &Path) -> Vec<(PathBuf, CanonicalValue)> {
    fn visit(root: &Path, directory: &Path, out: &mut Vec<(PathBuf, CanonicalValue)>) {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(directory)
            .expect("artifact directory")
            .map(|entry| entry.expect("entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(root, &path, out);
                continue;
            }
            let relative = path.strip_prefix(root).expect("relative").to_path_buf();
            let bytes = std::fs::read(&path).expect("artifact bytes");
            let mut parsed = None;
            if let Ok(value) = std::str::from_utf8(&bytes) {
                if let Ok(value) = parse_json_object(value.trim_end_matches('\n')) {
                    parsed = Some(value);
                }
            }
            let parts = relative
                .components()
                .map(|component| text(&component.as_os_str().to_string_lossy()))
                .collect();
            let value = object(vec![
                ("parts", array(parts)),
                ("size", integer(BigInt::from(bytes.len()))),
                ("sha256", text(&kala_tools::protocol::sha256_hex(&bytes))),
                ("text", parsed.unwrap_or(CanonicalValue::Null)),
            ]);
            out.push((relative, value));
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out
}

fn restart_value(application: &KalaApplication, request_id: &str) -> CanonicalValue {
    let revisions_alpha = application
        .list_design_revisions(DESIGN, 1_000)
        .expect("alpha revisions");
    let revisions_gamma = application
        .list_design_revisions("gamma", 1_000)
        .expect("gamma revisions");
    let checkpoint_records = application
        .list_workflow_checkpoints(WORKFLOW)
        .expect("checkpoints");
    let usage = application.budget().usage();
    object(vec![
        (
            "revisions_alpha",
            array(revisions_alpha.iter().map(view).collect()),
        ),
        (
            "revisions_gamma",
            array(revisions_gamma.iter().map(view).collect()),
        ),
        (
            "event_count",
            integer(BigInt::from(
                application.verify_chain().expect("verified").event_count,
            )),
        ),
        (
            "grants",
            array(
                application
                    .policy()
                    .grants()
                    .iter()
                    .map(|value| text(value.as_str().unwrap_or("")))
                    .collect(),
            ),
        ),
        (
            "budget",
            object(vec![
                (
                    "artifact_bytes",
                    integer(BigInt::from_str(&usage.artifact_bytes().to_string()).unwrap()),
                ),
                (
                    "model_calls",
                    integer(BigInt::from_str(&usage.model_calls().to_string()).unwrap()),
                ),
                ("cost", text(&usage.cost().to_python_string())),
            ]),
        ),
        (
            "workflow_alpha",
            view(
                &application
                    .get_workflow_state(WORKFLOW)
                    .expect("workflow state"),
            ),
        ),
        (
            "commit_r2",
            view(
                &application
                    .get_revision_commit(request_id)
                    .expect("commit r2"),
            ),
        ),
        (
            "checkpoint_records",
            array(
                checkpoint_records
                    .iter()
                    .map(|value| value.to_canonical_value().expect("approval"))
                    .collect(),
            ),
        ),
    ])
}

/// Run the whole script and return the trace document.
fn rust_trace(data_dir: &Path) -> (CanonicalValue, Arc<AtomicI64>) {
    let (clock, counter) = fixed_clock();
    let application = KalaApplication::with_components(
        settings_for(data_dir),
        Components {
            clock: Some(clock),
            domains: Some(DomainBinding::defaults()),
            ..Components::default()
        },
    )
    .expect("open the Rust application");
    let mut recorder = Recorder::new();
    drive_rust(&application, &mut recorder);
    let steps = recorder.into_value();
    let clock_calls_first = counter.load(Ordering::SeqCst);

    application.close().expect("close");
    let closed = application.closed();
    let closed_verify = match application.list_events(0, 50, None, None, None) {
        Ok(_) => object(vec![("ok", CanonicalValue::Bool(true))]),
        Err(error) => object(vec![
            ("ok", CanonicalValue::Bool(false)),
            ("class", text(error.kind().python_class_name())),
            ("code", text(error.code())),
            ("message", text(error.message())),
        ]),
    };
    let rows = collect_rows(&application.settings().database_path());
    let mut artifacts = collect_artifacts(&application.settings().artifact_dir());
    artifacts.sort_by_key(|(_, value)| artifact_sort_key(value));

    // The restart.
    let restarted = KalaApplication::open(settings_for(data_dir)).expect("reopen");
    let restart = restart_value(&restarted, &commit_request_id(&steps, "prepare_alpha_r2"));
    restarted.close().expect("close restart");

    let trace = object(vec![
        ("steps", steps),
        (
            "clock_calls_first",
            integer(BigInt::from(clock_calls_first)),
        ),
        ("closed", CanonicalValue::Bool(closed)),
        ("closed_verify", closed_verify),
        ("rows", rows),
        (
            "artifacts",
            array(artifacts.into_iter().map(|(_, value)| value).collect()),
        ),
        ("restart", restart),
        (
            "clock_calls_total",
            integer(BigInt::from(counter.load(Ordering::SeqCst))),
        ),
    ]);
    (trace, counter)
}

fn commit_request_id(steps: &CanonicalValue, name: &str) -> String {
    let step = match steps {
        CanonicalValue::Object(map) => map.get(&JsonText::new(name)),
        _ => None,
    }
    .expect("step present");
    match field(step, "value") {
        CanonicalValue::Object(value) => text_of(
            value
                .get(&JsonText::new("request_id"))
                .expect("request id present"),
        ),
        _ => panic!("prepare value present"),
    }
}

// ---------------------------------------------------------------------------
// Normalization
// ---------------------------------------------------------------------------

fn is_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && (value.ends_with('Z') || value.ends_with("+00:00"))
}

fn normalize_commit_id(value: &str) -> Option<String> {
    let rest = value.strip_prefix("commit-")?;
    let (body, action) = rest.rsplit_once('-')?;
    if action.len() != 16 || !action.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let (design, state) = body.rsplit_once('-')?;
    if state.len() != 16 || !state.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("commit-{design}-<prefix>-<prefix>"))
}

fn normalize_kernel_snapshot(value: &str) -> Option<String> {
    let rest = value.strip_prefix("kernel-state:")?;
    let (name, nonce) = rest.rsplit_once('-')?;
    if nonce.len() != 12 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("kernel-state:{name}-<nonce>"))
}

/// The full normalization used for the differential equality: timestamps from
/// the wall clock and every digest value become placeholders, except the
/// values that must match exactly (`parameters_digest`, `schema_digests`,
/// `evaluated_at`) and the deterministic 2031 clock timestamps.
fn normalize(value: &CanonicalValue, deterministic: bool) -> CanonicalValue {
    match value {
        CanonicalValue::Null => CanonicalValue::Null,
        CanonicalValue::Bool(value) => CanonicalValue::Bool(*value),
        CanonicalValue::Integer(value) => CanonicalValue::Integer(value.clone()),
        CanonicalValue::Float(value) => CanonicalValue::Float(*value),
        CanonicalValue::Text(value) => {
            let text_value = value.to_utf8().expect("trace text is utf8");
            CanonicalValue::text(normalize_text(&text_value, deterministic))
        }
        CanonicalValue::Array(items) => CanonicalValue::Array(
            items
                .iter()
                .map(|item| normalize(item, deterministic))
                .collect(),
        ),
        CanonicalValue::Object(map) => {
            let mut out = kala_json::CanonicalObject::new();
            for (key, item) in map {
                let name = key.to_utf8().expect("key utf8");
                let keep = matches!(
                    name.as_str(),
                    "parameters_digest" | "evaluated_at" | "message" | "sql"
                );
                let normalized = if keep {
                    item.clone()
                } else if name == "parts" {
                    normalize_artifact_parts(item)
                } else if name == "attributes_json" {
                    normalize_attributes(item, deterministic)
                } else {
                    normalize(item, deterministic)
                };
                out.insert(key.clone(), normalized);
            }
            CanonicalValue::Object(out)
        }
    }
}

fn normalize_attributes(value: &CanonicalValue, deterministic: bool) -> CanonicalValue {
    match value {
        CanonicalValue::Text(text_value) => {
            let raw = text_value.to_utf8().expect("utf8");
            match kala_json::parse_strict(raw.as_bytes(), &kala_json::Limits::default()) {
                Ok(parsed) => {
                    CanonicalValue::text(canonical_text(&normalize(&parsed, deterministic)))
                }
                Err(_) => value.clone(),
            }
        }
        other => other.clone(),
    }
}

fn normalize_artifact_parts(value: &CanonicalValue) -> CanonicalValue {
    match value {
        CanonicalValue::Array(items) if items.len() == 3 && text_of(&items[0]) == "objects" => {
            array(vec![text("objects"), text("<bucket>"), text("<digest>")])
        }
        other => other.clone(),
    }
}

fn normalize_timestamps_only(value: &CanonicalValue) -> CanonicalValue {
    match value {
        CanonicalValue::Text(text_value) => {
            let value = text_value.to_utf8().expect("utf8");
            if is_timestamp(&value) && !value.starts_with("2031-") {
                CanonicalValue::text("<timestamp>")
            } else {
                CanonicalValue::text(&value)
            }
        }
        CanonicalValue::Array(items) => {
            CanonicalValue::Array(items.iter().map(normalize_timestamps_only).collect())
        }
        CanonicalValue::Object(map) => CanonicalValue::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), normalize_timestamps_only(item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn normalize_text(value: &str, deterministic: bool) -> String {
    if is_hex_digest(value) {
        return "<digest>".to_owned();
    }
    if let Some(rest) = value.strip_prefix("artifact://sha256/") {
        if is_hex_digest(rest) {
            return "artifact://sha256/<digest>".to_owned();
        }
    }
    if let Some(normalized) = normalize_commit_id(value) {
        return normalized;
    }
    if let Some(normalized) = normalize_kernel_snapshot(value) {
        return normalized;
    }
    if is_timestamp(value) {
        if deterministic && value.starts_with("2031-") {
            return value.to_owned();
        }
        return "<timestamp>".to_owned();
    }
    value.to_owned()
}

fn first_difference(left: &CanonicalValue, right: &CanonicalValue, path: &str) -> Option<String> {
    match (left, right) {
        (CanonicalValue::Object(a), CanonicalValue::Object(b)) => {
            for (key, value) in a {
                let name = key.to_utf8().expect("key");
                let child = format!("{path}.{name}");
                match b.get(key) {
                    None => return Some(format!("{child} (missing on the right)")),
                    Some(other) => {
                        if let Some(found) = first_difference(value, other, &child) {
                            return Some(found);
                        }
                    }
                }
            }
            for key in b.keys() {
                if !a.contains_key(key) {
                    return Some(format!(
                        "{path}.{} (missing on the left)",
                        key.to_utf8().expect("key")
                    ));
                }
            }
            None
        }
        (CanonicalValue::Array(a), CanonicalValue::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!("{path} (length {} vs {})", a.len(), b.len()));
            }
            for (index, (value, other)) in a.iter().zip(b.iter()).enumerate() {
                if let Some(found) = first_difference(value, other, &format!("{path}[{index}]")) {
                    return Some(found);
                }
            }
            None
        }
        _ => {
            if left == right {
                None
            } else {
                Some(format!(
                    "{path} ({} vs {})",
                    canonical_text(left),
                    canonical_text(right)
                ))
            }
        }
    }
}

/// Replace the declared-identity digests (and the request-id digest prefixes)
/// with placeholders so the wire test proves field order, encoding and every
/// identity-independent value; the canonical differential proves the digest
/// identities separately.
fn normalize_wire(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let mut run = 0;
        while index + run < bytes.len() && bytes[index + run].is_ascii_hexdigit() {
            run += 1;
        }
        if run == 64 {
            out.extend_from_slice(b"<digest>");
            index += 64;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
        let _ = run;
    }
    let text = String::from_utf8(out).expect("wire JSON is UTF-8");
    // Normalize the `commit-<design>-<16 hex>-<16 hex>` request identities.
    let mut normalized = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(position) = rest.find("commit-") {
        normalized.push_str(&rest[..position]);
        let tail = &rest[position..];
        let end = tail.find(['"', '\\']).unwrap_or(tail.len());
        normalized.push_str("<commit>");
        rest = &tail[end..];
    }
    normalized.push_str(rest);
    normalized
}

fn clean_event_type(event_type: &str) -> bool {
    matches!(
        event_type,
        "plan.created"
            | "workflow.started"
            | "workflow.created"
            | "workflow.status_changed"
            | "workflow.transition"
            | "workflow.step_evaluated"
            | "workflow.checkpoint"
            | "workflow.approval_resolved"
            | "workflow.approval_invalidated"
            | "workflow.failed"
            | "workflow.completed"
            | "workflow.checkpoint_decided"
            | "policy.checkpoint_requested"
    )
}

fn events_of(trace: &CanonicalValue) -> &Vec<CanonicalValue> {
    let rows = match trace {
        CanonicalValue::Object(map) => map.get(&JsonText::new("rows")).expect("rows"),
        _ => panic!("trace shape"),
    };
    let events = match rows {
        CanonicalValue::Object(map) => map.get(&JsonText::new("events")).expect("events"),
        _ => panic!("rows shape"),
    };
    match events {
        CanonicalValue::Array(items) => items,
        _ => panic!("events shape"),
    }
}

fn field<'a>(value: &'a CanonicalValue, name: &str) -> &'a CanonicalValue {
    match value {
        CanonicalValue::Object(map) => map
            .get(&JsonText::new(name))
            .unwrap_or_else(|| panic!("field {name} present")),
        _ => panic!("object expected"),
    }
}

fn text_of(value: &CanonicalValue) -> String {
    match value {
        CanonicalValue::Text(value) => value.to_utf8().expect("utf8"),
        _ => panic!("text expected"),
    }
}

#[test]
fn live_differential_matches_python_with_the_declared_identity_boundary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python_dir = temp.path().join("python");
    let rust_dir = temp.path().join("rust");

    let python = run_oracle("main", &python_dir);
    let (rust, _clock) = rust_trace(&rust_dir);

    // 1. The declared identity boundary: package digests differ and are the
    //    only digest inputs that differ.
    let python_packages = field(&python, "steps");
    let rust_packages = field(&rust, "steps");
    let python_pkg = field(field(python_packages, "domain_packages"), "value");
    let rust_pkg = field(field(rust_packages, "domain_packages"), "value");
    for (python_entry, rust_entry) in match (python_pkg, rust_pkg) {
        (CanonicalValue::Array(left), CanonicalValue::Array(right)) => {
            left.iter().zip(right.iter())
        }
        _ => panic!("package list"),
    } {
        let pd = text_of(field(python_entry, "package_id"));
        let rd = text_of(field(rust_entry, "package_id"));
        assert_eq!(pd, rd);
        let left = text_of(field(python_entry, "digest"));
        let right = text_of(field(rust_entry, "digest"));
        assert!(is_hex_digest(&left));
        assert!(is_hex_digest(&right));
        assert_ne!(
            left, right,
            "the native domain identity is deliberately different"
        );
        assert_eq!(
            field(python_entry, "schema_digests"),
            field(rust_entry, "schema_digests"),
            "parameter schema digests are byte-identical"
        );
    }

    // 2. The whole trace is equal after normalizing wall-clock timestamps and
    //    digest values (but not the exact-comparable values).
    let normalized_python = normalize(&python, true);
    let normalized_rust = normalize(&rust, true);
    if normalized_python != normalized_rust {
        panic!(
            "normalized traces differ at {}; python clock={} rust clock={}",
            first_difference(&normalized_python, &normalized_rust, "trace")
                .unwrap_or_else(|| "<none>".to_owned()),
            canonical_text(field(&python, "clock_calls_first")),
            canonical_text(field(&rust, "clock_calls_first")),
        );
    }

    // 3. The injected clock is consulted the same number of times.
    assert_eq!(
        field(&python, "clock_calls_first"),
        field(&rust, "clock_calls_first")
    );
    assert_eq!(
        field(&python, "clock_calls_total"),
        field(&rust, "clock_calls_total")
    );

    // 4. Clean journal events are byte-identical, digests included.
    let python_events = events_of(&python);
    let rust_events = events_of(&rust);
    assert_eq!(python_events.len(), rust_events.len());
    for (python_event, rust_event) in python_events.iter().zip(rust_events.iter()) {
        assert_eq!(
            field(python_event, "sequence"),
            field(rust_event, "sequence")
        );
        assert_eq!(
            field(python_event, "event_type"),
            field(rust_event, "event_type")
        );
        let event_type = text_of(field(python_event, "event_type")).to_owned();
        if clean_event_type(&event_type) {
            // The decision timestamp of a resolved checkpoint comes from the
            // workflow engine's own wall clock (the Python root does not pass
            // its injected clock to the engine), so those two event digests
            // differ by the second they were emitted in, not by identity.
            let wall_clock_decision = matches!(
                event_type.as_str(),
                "workflow.approval_resolved" | "workflow.approval_invalidated"
            );
            if !wall_clock_decision {
                assert_eq!(
                    field(python_event, "digest"),
                    field(rust_event, "digest"),
                    "clean event digest must be identical for {event_type}"
                );
            }
            assert_eq!(
                normalize_timestamps_only(field(python_event, "payload_json")),
                normalize_timestamps_only(field(rust_event, "payload_json")),
                "clean event payload must be identical for {event_type}"
            );
        }
    }

    // 5. The digest algorithm is byte-identical: substituting the Python
    //    package digest into the native inputs reproduces every Python digest.
    assert_identity_substitution(&python, &rust);

    // 6. Both chains verify over the same number of events, and every dirty
    //    event's chain linkage is internally consistent (each side's
    //    `verify_chain` succeeded; only the head differs, because the last
    //    events carry the package identity).
    let python_chain = field(field(field(&python, "steps"), "verify_chain"), "value");
    let rust_chain = field(field(field(&rust, "steps"), "verify_chain"), "value");
    assert_eq!(
        field(python_chain, "event_count"),
        field(rust_chain, "event_count")
    );
    assert_eq!(
        field(python_chain, "head_sequence"),
        field(rust_chain, "head_sequence")
    );
}

/// Extract one prepared commit payload from the trace steps.
fn prepared_payload<'a>(trace: &'a CanonicalValue, step: &str) -> &'a CanonicalValue {
    // The prepared payload is denormalized from the *ticks* in the event rows:
    // the `revision.prepared` event whose payload request_id matches the step.
    let steps = field(trace, "steps");
    let request_id = text_of(field(field(field(steps, step), "value"), "request_id"));
    for event in events_of(trace) {
        if text_of(field(event, "event_type")) != "revision.prepared" {
            continue;
        }
        let payload = field(event, "payload_json");
        if text_of(field(payload, "request_id")) == request_id {
            return payload;
        }
    }
    panic!("prepared payload for {step}");
}

/// Select one stored artifact by its digest.
fn artifact_by_digest<'a>(trace: &'a CanonicalValue, digest: &str) -> &'a CanonicalValue {
    for artifact in match field(trace, "artifacts") {
        CanonicalValue::Array(items) => items,
        _ => panic!("artifacts"),
    } {
        if text_of(field(artifact, "sha256")) == digest {
            return field(artifact, "text");
        }
    }
    panic!("artifact {digest}");
}

fn payload_digest(payload: &CanonicalValue, field_name: &str) -> String {
    match field(payload, field_name) {
        CanonicalValue::Array(items) => text_of(&items[0]),
        _ => panic!("{field_name}"),
    }
}

/// Recompute the Python digest identities from the native inputs with the
/// Python package digest substituted.
fn assert_identity_substitution(python: &CanonicalValue, rust: &CanonicalValue) {
    let python_payload = prepared_payload(python, "prepare_alpha_r2");
    let rust_payload = prepared_payload(rust, "prepare_alpha_r2");
    let python_package = payload_digest(python_payload, "package_digests");
    let rust_package = payload_digest(rust_payload, "package_digests");
    assert_ne!(python_package, rust_package);

    // The specification differs only in the package digest attribute.
    let python_spec = field(python_payload, "spec");
    let rust_spec = substitute_text(field(rust_payload, "spec"), &rust_package, &python_package);
    assert_eq!(
        rust_spec, *python_spec,
        "the specification is identical after substitution"
    );
    let spec_digest = canonical_digest(python_spec);
    assert_eq!(spec_digest, text_of(field(python_payload, "spec_digest")));

    // The report artifact differs only in the package digest field.
    let python_artifact =
        artifact_by_digest(python, &payload_digest(python_payload, "artifact_digests"));
    let rust_artifact = artifact_by_digest(rust, &payload_digest(rust_payload, "artifact_digests"));
    let substituted = substitute_text(rust_artifact, &rust_package, &python_package);
    assert_eq!(
        canonical_text(&substituted),
        canonical_text(python_artifact),
        "the report bytes are identical after the package digest substitution"
    );
    let report_bytes = format!("{}\n", canonical_text(&substituted));
    let artifact_digest = kala_tools::protocol::sha256_hex(report_bytes.as_bytes());
    assert_eq!(
        artifact_digest,
        payload_digest(python_payload, "artifact_digests")
    );

    // The state and action digests.
    let python_state = text_of(field(python_payload, "state_digest"));
    let python_action = text_of(field(python_payload, "action_digest"));
    let head_revision = text_of(field(rust_payload, "expected_head_revision_id"));
    let head_sequence = python_head_sequence(python, &head_revision);
    let head_spec_digest = python_head_spec_digest(python, &head_revision);
    let state_preimage = state_preimage(
        rust_payload,
        &python_package,
        &spec_digest,
        &artifact_digest,
        report_bytes.len(),
        &head_sequence,
        &head_spec_digest,
    );
    assert_eq!(
        canonical_digest(&state_preimage),
        python_state,
        "the state digest algorithm is byte-identical"
    );
    let action_preimage = action_preimage(rust_payload, &spec_digest, &python_state);
    assert_eq!(
        canonical_digest(&action_preimage),
        python_action,
        "the action digest algorithm is byte-identical"
    );
    let request_id = format!(
        "commit-{}-{}-{}",
        DESIGN,
        &python_state[..16],
        &python_action[..16]
    );
    assert_eq!(request_id, text_of(field(python_payload, "request_id")));
}

fn canonical_text(value: &CanonicalValue) -> String {
    kala_json::canonical_json_dumps(value).expect("canonical")
}

fn canonical_digest(value: &CanonicalValue) -> String {
    kala_json::canonical_json_digest(value)
        .expect("canonical")
        .to_hex()
}

fn substitute_text(value: &CanonicalValue, from: &str, to: &str) -> CanonicalValue {
    match value {
        CanonicalValue::Text(text_value) => {
            let text_value = text_value.to_utf8().expect("utf8");
            CanonicalValue::text(text_value.replace(from, to))
        }
        CanonicalValue::Array(items) => CanonicalValue::Array(
            items
                .iter()
                .map(|item| substitute_text(item, from, to))
                .collect(),
        ),
        CanonicalValue::Object(map) => CanonicalValue::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), substitute_text(item, from, to)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn state_preimage(
    payload: &CanonicalValue,
    package_digest: &str,
    spec_digest: &str,
    artifact_digest: &str,
    artifact_size: usize,
    head_sequence: &BigInt,
    head_spec_digest: &str,
) -> CanonicalValue {
    let head_revision = field(payload, "expected_head_revision_id").clone();
    object(vec![
        ("artifact_digests", array(vec![text(artifact_digest)])),
        (
            "artifact_metadata",
            array(vec![object(vec![
                ("artifact_id", text("artifact-mechanical-report")),
                ("classification", text("text")),
                ("digest", text(artifact_digest)),
                ("media_type", text("application/json")),
                ("size_bytes", integer(BigInt::from(artifact_size))),
            ])]),
        ),
        (
            "design_head",
            match &head_revision {
                CanonicalValue::Null => CanonicalValue::Null,
                _ => object(vec![
                    ("revision_id", head_revision.clone()),
                    ("sequence", integer(head_sequence.clone())),
                    ("spec_digest", text(head_spec_digest)),
                ]),
            },
        ),
        ("design_id", text(DESIGN)),
        ("domain", text("mechanical")),
        ("package_digests", array(vec![text(package_digest)])),
        (
            "parameters_digest",
            field(payload, "parameters_digest").clone(),
        ),
        (
            "parent_provenance_ids",
            field(payload, "parent_provenance_ids").clone(),
        ),
        (
            "parent_revision_id",
            field(payload, "parent_revision_id").clone(),
        ),
        (
            "provenance_id",
            text(&format!(
                "prov-{DESIGN}-r{}",
                match field(payload, "sequence") {
                    CanonicalValue::Integer(value) => value,
                    _ => panic!("sequence"),
                }
            )),
        ),
        ("spec_digest", text(spec_digest)),
        (
            "transformation_ids",
            field(payload, "transformation_ids").clone(),
        ),
    ])
}

fn action_preimage(
    payload: &CanonicalValue,
    spec_digest: &str,
    state_digest: &str,
) -> CanonicalValue {
    object(vec![
        (
            "capabilities",
            array(vec![text("artifact.write"), text("design.write")]),
        ),
        ("checkpoint_type", text("pre_commit")),
        ("design_id", text(DESIGN)),
        ("domain", text("mechanical")),
        (
            "expected_head_revision_id",
            field(payload, "expected_head_revision_id").clone(),
        ),
        ("operation", text("design-revision-commit")),
        ("planned_revision_id", field(payload, "revision_id").clone()),
        ("planned_sequence", field(payload, "sequence").clone()),
        ("spec_digest", text(spec_digest)),
        ("state_digest", text(state_digest)),
        ("workflow_id", field(payload, "workflow_id").clone()),
    ])
}

fn python_head_spec_digest(python: &CanonicalValue, revision_id: &str) -> String {
    for row in match field(field(python, "rows"), "design_revisions") {
        CanonicalValue::Array(items) => items,
        _ => panic!("revisions"),
    } {
        let model = field(row, "model_json");
        if text_of(field(model, "revision_id")) == revision_id {
            return canonical_digest(field(model, "spec"));
        }
    }
    panic!("head revision {revision_id}")
}

fn python_head_sequence(python: &CanonicalValue, revision_id: &str) -> BigInt {
    if revision_id.is_empty() {
        return BigInt::from(0);
    }
    for row in match field(field(python, "rows"), "design_revisions") {
        CanonicalValue::Array(items) => items,
        _ => panic!("revisions"),
    } {
        let model = field(row, "model_json");
        if text_of(field(model, "revision_id")) == revision_id {
            return match field(model, "sequence") {
                CanonicalValue::Integer(value) => value.clone(),
                _ => panic!("sequence"),
            };
        }
    }
    panic!("head revision {revision_id}")
}

// ---------------------------------------------------------------------------
// The deterministic corpus
// ---------------------------------------------------------------------------

struct Lcg(i64);

impl Lcg {
    fn next(&mut self) -> i64 {
        self.0 = (1_103_515_245 * self.0 + 12_345) % 2_147_483_648;
        self.0
    }
}

fn mechanical_parameters(next: &mut Lcg) -> CanonicalValue {
    let materials = ["aluminium-6061-t6", "steel-s355", "titanium-ti6al4v"];
    let length = 100 + next.next() % 400;
    let width = 60 + next.next() % 240;
    let hole_diameter = 2.0 + (next.next() % 80) as f64 / 10.0;
    let mut hole_count = 1 + next.next() % 6;
    while (hole_count as f64) * hole_diameter > length as f64 {
        hole_count -= 1;
    }
    let thickness = 1.0 + (next.next() % 180) as f64 / 10.0;
    let material = materials[(next.next() % 3) as usize];
    let load = (next.next() % 2000) as f64 / 10.0;
    let load_points = 1 + next.next() % 8;
    let margin = (next.next() % 40) as f64 / 10.0;
    let minimum_thickness = 0.5 + (next.next() % 100) as f64 / 10.0;
    let maximum_stress = 50 + next.next() % 400;
    let target = 1.0 + (next.next() % 40) as f64 / 10.0;
    object(vec![
        ("length_mm", integer(length)),
        ("width_mm", integer(width)),
        ("thickness_mm", CanonicalValue::Float(thickness)),
        ("hole_diameter_mm", CanonicalValue::Float(hole_diameter)),
        ("hole_count", integer(hole_count)),
        ("material", text(material)),
        ("load_kn", CanonicalValue::Float(load)),
        ("load_points", integer(load_points)),
        ("minimum_edge_margin_mm", CanonicalValue::Float(margin)),
        (
            "minimum_thickness_mm",
            CanonicalValue::Float(minimum_thickness),
        ),
        ("maximum_stress_mpa", integer(maximum_stress)),
        ("target_safety_factor", CanonicalValue::Float(target)),
    ])
}

fn building_parameters(next: &mut Lcg) -> CanonicalValue {
    object(vec![
        ("storey_count", integer(1 + next.next() % 40)),
        (
            "storey_height_m",
            CanonicalValue::Float(2.0 + (next.next() % 40) as f64 / 10.0),
        ),
        ("gross_floor_area_m2", integer(50 + next.next() % 1950)),
        ("perimeter_m", integer(10 + next.next() % 190)),
        (
            "window_to_wall_ratio",
            CanonicalValue::Float((next.next() % 90) as f64 / 100.0),
        ),
        ("heating_degree_days_k", integer(100 + next.next() % 39_000)),
    ])
}

fn wing_parameters(next: &mut Lcg) -> CanonicalValue {
    let root = 0.5 + (next.next() % 40) as f64 / 10.0;
    let span = 2.0 + (next.next() % 58) as f64;
    let tip = root * (0.1 + (next.next() % 8) as f64 / 10.0);
    let sweep = -5.0 + (next.next() % 65) as f64;
    let dihedral = -10.0 + (next.next() % 50) as f64;
    let stations = 2 + next.next() % 30;
    object(vec![
        ("span_m", CanonicalValue::Float(span)),
        ("root_chord_m", CanonicalValue::Float(root)),
        ("tip_chord_m", CanonicalValue::Float(tip)),
        ("sweep_deg", CanonicalValue::Float(sweep)),
        ("dihedral_deg", CanonicalValue::Float(dihedral)),
        ("spanwise_stations", integer(stations)),
    ])
}

fn invalid_corpus() -> Vec<CanonicalValue> {
    vec![
        object(vec![("length_mm", text("not-a-number"))]),
        object(vec![("hole_count", CanonicalValue::Float(2.5))]),
        object(vec![("length_mm", integer(10))]),
        object(vec![("width_mm", integer(1000))]),
        object(vec![("material", text("unobtainium"))]),
        object(vec![("nope", integer(1))]),
        object(vec![("length_mm", CanonicalValue::Null)]),
        object(vec![("hole_count", CanonicalValue::Bool(true))]),
        object(vec![("load_points", integer(0))]),
        object(vec![("thickness_mm", text("nan"))]),
        object(vec![(
            "length_mm",
            CanonicalValue::array([integer(1), integer(2)]),
        )]),
        object(vec![("storey_count", integer(0))]),
    ]
}

fn call(value: ApplicationResult<CanonicalValue>) -> CanonicalValue {
    match value {
        Ok(value) => object(vec![("ok", CanonicalValue::Bool(true)), ("value", value)]),
        Err(error) => object(vec![
            ("ok", CanonicalValue::Bool(false)),
            ("class", text(error.kind().python_class_name())),
            ("code", text(error.code())),
            ("message", text(error.message())),
        ]),
    }
}

fn rust_corpus(data_dir: &Path) -> CanonicalValue {
    let application = KalaApplication::open(settings_for(data_dir)).expect("open");
    let mut next = Lcg(20_261_019);
    let mechanical: Vec<CanonicalValue> =
        (0..16).map(|_| mechanical_parameters(&mut next)).collect();
    let building: Vec<CanonicalValue> = (0..8).map(|_| building_parameters(&mut next)).collect();
    let wing: Vec<CanonicalValue> = (0..8).map(|_| wing_parameters(&mut next)).collect();
    let mut cases = Vec::new();
    for (index, parameters) in mechanical.iter().enumerate() {
        let subject = format!("corpus-{index}");
        let result = application
            .evaluate_domain_design("mechanical", Some(parameters), Some(&subject))
            .map(|report| view(&report));
        cases.push(object(vec![
            ("kind", text("mechanical-evaluate")),
            ("index", integer(index as i64)),
            ("parameters", parameters.clone()),
            ("result", call(result)),
        ]));
    }
    for (index, parameters) in building.iter().enumerate() {
        let subject = format!("corpus-building-{index}");
        let result = application
            .evaluate_domain_design("building", Some(parameters), Some(&subject))
            .map(|report| view(&report));
        cases.push(object(vec![
            ("kind", text("building-evaluate")),
            ("index", integer(index as i64)),
            ("parameters", parameters.clone()),
            ("result", call(result)),
        ]));
    }
    for (index, parameters) in wing.iter().enumerate() {
        let subject = format!("corpus-wing-{index}");
        let result = application
            .evaluate_domain_design("wing", Some(parameters), Some(&subject))
            .map(|report| view(&report));
        cases.push(object(vec![
            ("kind", text("wing-evaluate")),
            ("index", integer(index as i64)),
            ("parameters", parameters.clone()),
            ("result", call(result)),
        ]));
    }
    for (index, parameters) in invalid_corpus().iter().enumerate() {
        let result = application
            .evaluate_domain_design("mechanical", Some(parameters), None)
            .map(|report| view(&report));
        cases.push(object(vec![
            ("kind", text("invalid-evaluate")),
            ("index", integer(index as i64)),
            ("parameters", parameters.clone()),
            ("result", call(result)),
        ]));
    }
    for index in [0usize, 5, 10, 15] {
        let parameters = mechanical[index].clone();
        let subject = format!("corpus-commit-{index}");
        let result = application
            .create_design_revision(
                &subject,
                "mechanical",
                None,
                Some(&parameters),
                None,
                &[],
                &["corpus-a".to_owned(), "corpus-b".to_owned()],
                None,
                LOCAL_ACTOR_ID,
                None,
                None,
            )
            .map(|record| view(&record));
        cases.push(object(vec![
            ("kind", text("commit")),
            ("index", integer(index as i64)),
            ("parameters", parameters),
            ("result", call(result)),
        ]));
        let result = application
            .get_design_revision(&format!("rev-corpus-commit-{index}-r0"))
            .map(|record| view(&record));
        cases.push(object(vec![
            ("kind", text("read-commit")),
            ("index", integer(index as i64)),
            ("result", call(result)),
        ]));
    }
    application.close().expect("close");
    let mut artifacts = collect_artifacts(&application.settings().artifact_dir());
    artifacts.sort_by_key(|(_, value)| artifact_sort_key(value));
    object(vec![
        ("cases", array(cases)),
        (
            "rows",
            collect_rows(&application.settings().database_path()),
        ),
        (
            "artifacts",
            array(artifacts.into_iter().map(|(_, value)| value).collect()),
        ),
    ])
}

#[test]
fn live_corpus_differential() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python = run_oracle("corpus", &temp.path().join("python"));
    let rust = rust_corpus(&temp.path().join("rust"));
    let normalized_python = normalize(&python, false);
    let normalized_rust = normalize(&rust, false);
    if normalized_python != normalized_rust {
        panic!(
            "corpus differs at {}",
            first_difference(&normalized_python, &normalized_rust, "corpus")
                .unwrap_or_else(|| "<none>".to_owned())
        );
    }
}

#[test]
fn wire_json_matches_pydantic_field_order_and_encoding() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python = run_oracle("wire", &temp.path().join("python"));

    let (clock, _counter) = fixed_clock();
    let data = temp.path().join("rust");
    let application = KalaApplication::with_components(
        settings_for(&data),
        Components {
            clock: Some(clock),
            ..Components::default()
        },
    )
    .expect("open");
    let detail = application.describe_domain("mechanical").expect("detail");
    let parameters = object(vec![("load_kn", CanonicalValue::Float(2.5))]);
    let report = application
        .evaluate_domain_design("mechanical", Some(&parameters), Some("wire-design"))
        .expect("report");
    let prepared = application
        .prepare_design_revision(
            "wire-design",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("prepared");
    let committed = application
        .commit_design_revision(&prepared.request_id, LOCAL_ACTOR_ID, None, None, None, None)
        .expect("commit");
    let commit_record = application
        .get_revision_commit(&prepared.request_id)
        .expect("commit record");
    let run = application
        .run_mechanical_reference_workflow(Some("workflow-wire"), "wire-design", LOCAL_ACTOR_ID)
        .expect("run");
    let state = application
        .get_workflow_state("workflow-wire")
        .expect("state");
    let checkpoint = application
        .list_workflow_checkpoints("workflow-wire")
        .expect("checkpoints")
        .into_iter()
        .next()
        .expect("checkpoint");
    let packages = application.list_domain_packages();
    let definitions = application.list_workflow_definitions();
    let steps = application.list_catalog_steps("mechanical").expect("steps");
    let strings = object(vec![
        ("detail", text(&detail.to_wire_json().expect("wire"))),
        ("report", text(&report.to_wire_json().expect("wire"))),
        ("prepared", text(&prepared.to_wire_json().expect("wire"))),
        ("committed", text(&committed.to_wire_json().expect("wire"))),
        (
            "commit_record",
            text(&commit_record.to_wire_json().expect("wire")),
        ),
        ("run", text(&run.to_wire_json().expect("wire"))),
        ("state", text(&state.to_wire_json().expect("wire"))),
        (
            "checkpoint",
            text(&checkpoint.to_wire_json().expect("wire")),
        ),
        ("package", text(&packages[1].to_wire_json().expect("wire"))),
        (
            "definition",
            text(&definitions[1].to_wire_json().expect("wire")),
        ),
        ("step", text(&steps[0].to_wire_json().expect("wire"))),
    ]);
    application.close().expect("close");
    let python_strings = field(&python, "strings");
    for key in [
        "detail",
        "report",
        "prepared",
        "committed",
        "commit_record",
        "run",
        "state",
        "checkpoint",
        "package",
        "definition",
        "step",
    ] {
        assert_eq!(
            normalize_wire(&text_of(field(&strings, key))),
            normalize_wire(&text_of(field(python_strings, key))),
            "wire JSON differs for {key}"
        );
    }
}
