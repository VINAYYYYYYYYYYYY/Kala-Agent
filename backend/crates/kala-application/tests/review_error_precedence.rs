//! Review probe 6: error precedence and messages against Python.
//!
//! One shared table of more than fifty operations is driven through the live
//! Python root and through the Rust root. For every case the error class
//! family (the code), the message and the success/failure verdict must be
//! identical, and the table deliberately mixes the order of the checks so the
//! *precedence* is compared and not only the outcome.
//!
//! The cases that need a real prepared commit substitute the placeholder
//! `<prepared>` with the request identity each implementation derived for
//! itself; that identity is the one value the two implementations are allowed
//! to differ in, and it is the declared native package identity.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kala_api::config::{SettingValue, Settings, settings_from_values};
use kala_application::{Components, KalaApplication};
use kala_json::CanonicalValue;
use serde_json::Value as Json;

/// The shared case table. The Python probe reads the very same file.
const CASES: &str = r#"
[
 {"name": "empty_design_id", "op": "prepare", "design_id": "", "domain": "mechanical"},
 {"name": "space_design_id", "op": "prepare", "design_id": "a b", "domain": "mechanical"},
 {"name": "leading_dash_design_id", "op": "prepare", "design_id": "-abc", "domain": "mechanical"},
 {"name": "slash_design_id", "op": "prepare", "design_id": "a/b", "domain": "mechanical"},
 {"name": "unicode_design_id", "op": "prepare", "design_id": "désign", "domain": "mechanical"},
 {"name": "control_character_design_id", "op": "prepare", "design_id": "a\u0001b", "domain": "mechanical"},
 {"name": "long_design_id", "op": "prepare", "design_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "domain": "mechanical"},
 {"name": "very_long_design_id", "op": "prepare", "design_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "domain": "mechanical"},
 {"name": "max_design_id", "op": "prepare", "design_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "domain": "mechanical"},
 {"name": "trimmed_design_id", "op": "prepare", "design_id": "  spaced  ", "domain": "mechanical"},
 {"name": "empty_domain", "op": "describe_domain", "domain": ""},
 {"name": "unknown_domain", "op": "describe_domain", "domain": "nope"},
 {"name": "bad_domain_char", "op": "describe_domain", "domain": "mech anical"},
 {"name": "uppercase_domain", "op": "describe_domain", "domain": "Mechanical"},
 {"name": "dot_domain", "op": "describe_domain", "domain": "mechanical."},
 {"name": "reserved_workflow_id", "op": "workflow_state", "workflow_id": "kernel-policy-state"},
 {"name": "reserved_budget_workflow_id", "op": "workflow_state", "workflow_id": "kernel-budget-state"},
 {"name": "reserved_workflow_namespace", "op": "workflow_state", "workflow_id": "kernel-state:anything"},
 {"name": "reserved_workflow_snapshot_name", "op": "workflow_state", "workflow_id": "kernel-state:kernel-policy-state-0123456789ab"},
 {"name": "bad_workflow_id", "op": "workflow_state", "workflow_id": "a b"},
 {"name": "empty_workflow_id", "op": "workflow_state", "workflow_id": ""},
 {"name": "parameters_not_an_object", "op": "evaluate", "domain": "mechanical", "parameters": [1, 2]},
 {"name": "parameters_string", "op": "evaluate", "domain": "mechanical", "parameters": "x"},
 {"name": "parameters_unknown_field", "op": "evaluate", "domain": "mechanical", "parameters": {"nope": 1}},
 {"name": "parameters_wrong_type", "op": "evaluate", "domain": "mechanical", "parameters": {"load_kn": "heavy"}},
 {"name": "parameters_missing_field", "op": "evaluate", "domain": "mechanical", "parameters": {"load_kn": 9.0}},
 {"name": "parameters_out_of_bounds", "op": "evaluate", "domain": "mechanical", "parameters": {"thickness_mm": 100000.0}},
 {"name": "parameters_too_many", "op": "evaluate", "domain": "mechanical", "parameters": {"load_kn": 9.0, "thickness_mm": 4.0, "span_mm": 120.0, "hole_diameter_mm": 6.0}},
 {"name": "parameters_building_on_mechanical", "op": "evaluate", "domain": "mechanical", "parameters": {"storey_height_m": 3.0}},
 {"name": "parameters_over_the_entry_bound", "op": "evaluate_overflow", "domain": "mechanical"},
 {"name": "limit_zero", "op": "list_revisions", "design_id": "precedence", "limit": 0},
 {"name": "limit_negative", "op": "list_revisions", "design_id": "precedence", "limit": -1},
 {"name": "limit_too_big", "op": "list_revisions", "design_id": "precedence", "limit": 1001},
 {"name": "limit_max", "op": "list_revisions", "design_id": "precedence", "limit": 1000},
 {"name": "event_limit_zero", "op": "list_events", "limit": 0},
 {"name": "event_limit_too_big", "op": "list_events", "limit": 1001},
 {"name": "event_after_sequence_negative", "op": "list_events", "after_sequence": -1},
 {"name": "event_bad_event_type", "op": "list_events", "event_type": "a b"},
 {"name": "event_bad_aggregate", "op": "list_events", "aggregate_id": "a b"},
 {"name": "event_reserved_workflow", "op": "list_events", "workflow_id": "kernel-policy-state"},
 {"name": "empty_summary", "op": "prepare", "design_id": "precedence", "domain": "mechanical", "summary": ""},
 {"name": "whitespace_summary", "op": "prepare", "design_id": "precedence", "domain": "mechanical", "summary": "   "},
 {"name": "long_summary", "op": "prepare", "design_id": "precedence", "domain": "mechanical", "summary": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
 {"name": "empty_reason", "op": "decide_revision", "request_id": "<prepared>", "decision": "approve", "reason": ""},
 {"name": "unknown_revision", "op": "get_revision", "revision_id": "rev-missing"},
 {"name": "unknown_parent", "op": "prepare", "design_id": "precedence", "domain": "mechanical", "parent_revision_id": "rev-missing"},
 {"name": "unknown_prepared_commit", "op": "decide_revision", "request_id": "commit-does-not-exist", "decision": "approve"},
 {"name": "bad_decision", "op": "decide_revision", "request_id": "<prepared>", "decision": "maybe"},
 {"name": "uppercase_decision", "op": "decide_revision", "request_id": "<prepared>", "decision": "APPROVE"},
 {"name": "bad_actor_id", "op": "decide_revision", "request_id": "<prepared>", "decision": "approve", "actor_id": "a b"},
 {"name": "long_actor_id", "op": "decide_revision", "request_id": "<prepared>", "decision": "approve", "actor_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
 {"name": "stale_state_digest", "op": "decide_revision", "request_id": "<prepared>", "decision": "approve", "state_digest": "0000000000000000000000000000000000000000000000000000000000000000"},
 {"name": "stale_action_digest", "op": "decide_revision", "request_id": "<prepared>", "decision": "approve", "action_digest": "0000000000000000000000000000000000000000000000000000000000000000"},
 {"name": "read_prepared_commit", "op": "get_revision_commit", "request_id": "<prepared>"},
 {"name": "checkpoint_bad_decision", "op": "decide_checkpoint", "workflow_id": "workflow-precedence", "decision": "maybe"},
 {"name": "checkpoint_bad_workflow", "op": "approve_checkpoint", "workflow_id": "workflow-precedence"},
 {"name": "checkpoint_reserved_workflow", "op": "approve_checkpoint", "workflow_id": "kernel-policy-state"},
 {"name": "checkpoint_stale_state_digest", "op": "approve_checkpoint", "workflow_id": "workflow-precedence", "state_digest": "0000000000000000000000000000000000000000000000000000000000000000"},
 {"name": "run_workflow_bad_design", "op": "run_workflow", "design_id": ""},
 {"name": "run_workflow_reserved", "op": "run_workflow", "workflow_id": "kernel-state:x", "design_id": "precedence"},
 {"name": "run_workflow_bad_actor", "op": "run_workflow", "design_id": "precedence", "actor_id": "a b"},
 {"name": "precedence_bad_design_before_unknown_domain", "op": "prepare", "design_id": "a b", "domain": "nope"},
 {"name": "precedence_unknown_domain_before_bad_design", "op": "evaluate", "domain": "nope", "design_id": "a b"},
 {"name": "precedence_request_id_before_decision", "op": "decide_revision", "request_id": "a b", "decision": "maybe"},
 {"name": "precedence_after_sequence_before_limit", "op": "list_events", "after_sequence": -1, "limit": 0},
 {"name": "precedence_design_id_before_limit", "op": "list_revisions", "design_id": "a b", "limit": 0},
 {"name": "precedence_decision_before_actor", "op": "decide_checkpoint", "workflow_id": "workflow-precedence", "decision": "maybe", "actor_id": "a b"},
 {"name": "precedence_workflow_id_before_decision", "op": "decide_checkpoint", "workflow_id": "a b", "decision": "maybe"},
 {"name": "trimmed_decision", "op": "decide_revision", "request_id": "<prepared>", "decision": " approve ", "note": "runs last: a trimmed decision is a valid decision and approves the prepared commit"}
]
"#;

const PYTHON_SCRIPT: &str = r#"
import json
import sys
from datetime import UTC, datetime, timedelta
from pathlib import Path

repo = Path(sys.argv[1])
sys.path.insert(0, str(repo / "src"))

from kala_agent.application import KalaApplication
from kala_agent.config import Settings

data_dir = Path(sys.argv[2])
cases_path = Path(sys.argv[3])
data_dir.mkdir(parents=True, exist_ok=True)

BASE = datetime(2031, 1, 1, tzinfo=UTC)
counter = {"n": 0}


def clock():
    value = BASE + timedelta(seconds=counter["n"])
    counter["n"] += 1
    return value


app = KalaApplication(Settings(data_dir=data_dir), clock=clock)
prepared = app.prepare_design_revision(design_id="precedence", domain="mechanical")
app.run_mechanical_reference_workflow(
    workflow_id="workflow-precedence", design_id="precedence"
)


def substitute(value):
    if isinstance(value, str):
        return value.replace("<prepared>", prepared.request_id)
    if isinstance(value, list):
        return [substitute(item) for item in value]
    if isinstance(value, dict):
        return {key: substitute(item) for key, item in value.items()}
    return value


def run_case(case):
    op = case["op"]
    a = {key: value for key, value in case.items() if key not in ("name", "op")}
    if op == "prepare":
        return app.prepare_design_revision(
            design_id=a["design_id"],
            domain=a["domain"],
            summary=a.get("summary"),
            parameters=a.get("parameters"),
            parent_revision_id=a.get("parent_revision_id"),
            parent_provenance_ids=a.get("parent_provenance_ids", ()),
            transformation_ids=a.get("transformation_ids", ()),
            workflow_id=a.get("workflow_id"),
        )
    if op == "create":
        return app.create_design_revision(
            design_id=a["design_id"],
            domain=a["domain"],
            summary=a.get("summary"),
            parameters=a.get("parameters"),
            parent_revision_id=a.get("parent_revision_id"),
            parent_provenance_ids=a.get("parent_provenance_ids", ()),
            transformation_ids=a.get("transformation_ids", ()),
            workflow_id=a.get("workflow_id"),
            actor_id=a.get("actor_id", "local-user"),
            state_digest=a.get("state_digest"),
            action_digest=a.get("action_digest"),
        )
    if op == "evaluate":
        return app.evaluate_domain_design(
            a["domain"],
            parameters=a.get("parameters"),
            design_id=a.get("design_id"),
        )
    if op == "evaluate_overflow":
        return app.evaluate_domain_design(
            a["domain"],
            parameters={f"field_{index}": 1.0 for index in range(4097)},
        )
    if op == "describe_domain":
        return app.describe_domain(a["domain"])
    if op == "catalog_steps":
        return app.list_catalog_steps(a["domain"])
    if op == "workflow_state":
        return app.get_workflow_state(a["workflow_id"])
    if op == "checkpoints":
        return app.list_workflow_checkpoints(a["workflow_id"])
    if op == "approve_checkpoint":
        return app.approve_workflow_checkpoint(
            a["workflow_id"],
            actor_id=a.get("actor_id", "local-user"),
            state_digest=a.get("state_digest"),
            action_digest=a.get("action_digest"),
            reason=a.get("reason"),
        )
    if op == "reject_checkpoint":
        return app.reject_workflow_checkpoint(
            a["workflow_id"],
            actor_id=a.get("actor_id", "local-user"),
            state_digest=a.get("state_digest"),
            action_digest=a.get("action_digest"),
            reason=a.get("reason"),
        )
    if op == "decide_checkpoint":
        return app.decide_workflow_checkpoint(
            a["workflow_id"],
            a["decision"],
            actor_id=a.get("actor_id", "local-user"),
            state_digest=a.get("state_digest"),
            action_digest=a.get("action_digest"),
            reason=a.get("reason"),
        )
    if op == "decide_revision":
        return app.decide_revision_commit(
            a["request_id"],
            a["decision"],
            actor_id=a.get("actor_id", "local-user"),
            state_digest=a.get("state_digest"),
            action_digest=a.get("action_digest"),
            reason=a.get("reason"),
            workflow_id=a.get("workflow_id"),
        )
    if op == "commit_revision":
        return app.commit_design_revision(
            a["request_id"],
            actor_id=a.get("actor_id", "local-user"),
            state_digest=a.get("state_digest"),
            action_digest=a.get("action_digest"),
            reason=a.get("reason"),
            workflow_id=a.get("workflow_id"),
        )
    if op == "get_revision_commit":
        return app.get_revision_commit(a["request_id"])
    if op == "list_revisions":
        return app.list_design_revisions(a["design_id"], limit=a.get("limit", 1000))
    if op == "get_revision":
        return app.get_design_revision(a["revision_id"])
    if op == "list_events":
        return app.list_events(
            after_sequence=a.get("after_sequence", 0),
            limit=a.get("limit", 50),
            workflow_id=a.get("workflow_id"),
            event_type=a.get("event_type"),
            aggregate_id=a.get("aggregate_id"),
        )
    if op == "run_workflow":
        return app.run_mechanical_reference_workflow(
            workflow_id=a.get("workflow_id"),
            design_id=a.get("design_id", "mechanical-reference"),
            actor_id=a.get("actor_id", "local-user"),
        )
    raise SystemExit(f"unknown op {op}")


cases = json.loads(cases_path.read_text())
steps = []
for case in cases:
    resolved = substitute(case)
    try:
        run_case(resolved)
    except Exception as error:  # noqa: BLE001 - the error shape is the observation
        steps.append(
            {
                "name": case["name"],
                "ok": False,
                "class": type(error).__name__,
                "code": getattr(error, "code", None),
                "message": str(error),
            }
        )
    else:
        steps.append({"name": case["name"], "ok": True})

app.close()
sys.stdout.write(json.dumps({"steps": steps}))
"#;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
        "the live Python oracle requires {}",
        venv.display()
    );
    venv
}

fn settings_for(data_dir: &Path) -> Settings {
    let mut values = BTreeMap::new();
    values.insert(
        "data_dir".to_owned(),
        SettingValue::Text(data_dir.to_string_lossy().into_owned()),
    );
    settings_from_values(&values).expect("valid settings")
}

fn open(data_dir: &Path) -> KalaApplication {
    KalaApplication::with_components(settings_for(data_dir), Components::default())
        .expect("open the data directory")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    name: String,
    ok: bool,
    code: Option<String>,
    message: Option<String>,
}

impl Step {
    fn from_json(value: &Json) -> Self {
        let name = value
            .get("name")
            .and_then(Json::as_str)
            .expect("step name")
            .to_owned();
        if value.get("ok").and_then(Json::as_bool).unwrap_or(false) {
            Self {
                name,
                ok: true,
                code: None,
                message: None,
            }
        } else {
            Self {
                name,
                ok: false,
                code: value.get("code").and_then(Json::as_str).map(str::to_owned),
                message: value
                    .get("message")
                    .and_then(Json::as_str)
                    .map(str::to_owned),
            }
        }
    }
}

/// Read one optional string argument.
fn text_argument(arguments: &Json, name: &str) -> Option<String> {
    arguments
        .get(name)
        .and_then(Json::as_str)
        .map(str::to_owned)
}

/// Read one optional parameter object.
fn parameter_argument(arguments: &Json) -> Option<CanonicalValue> {
    let value = arguments.get("parameters")?;
    if value.is_null() {
        return None;
    }
    let text = serde_json::to_string(value).expect("json text");
    Some(
        kala_json::parse_strict(text.as_bytes(), &kala_json::Limits::default())
            .expect("canonical parameters"),
    )
}

/// Read one optional list of identifiers.
fn identifier_list(arguments: &Json, name: &str) -> Vec<String> {
    arguments
        .get(name)
        .and_then(Json::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().expect("identifier").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn integer_argument(arguments: &Json, name: &str, default: i64) -> i64 {
    arguments
        .get(name)
        .and_then(Json::as_i64)
        .unwrap_or(default)
}

/// The request identity embeds the two digests, and the digests embed the
/// declared native package identity, so the request id itself differs between
/// the two implementations. The identity is normalised out of the compared
/// messages; nothing else is.
fn normalize_request_identity(message: &str) -> String {
    const PREFIX: &str = "commit-";
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    loop {
        let Some(position) = rest.find(PREFIX) else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..position]);
        rest = &rest[position..];
        let body = &rest[PREFIX.len()..];
        let bytes = body.as_bytes();
        let mut candidate = None;
        let mut index = 0usize;
        while index < bytes.len() {
            if bytes[index] == b'-'
                && index + 34 < bytes.len()
                && bytes[index + 1..index + 17]
                    .iter()
                    .all(u8::is_ascii_hexdigit)
                && bytes[index + 17] == b'-'
                && bytes[index + 18..index + 34]
                    .iter()
                    .all(u8::is_ascii_hexdigit)
            {
                candidate = Some(index);
                break;
            }
            index += 1;
        }
        match candidate {
            Some(index) => {
                out.push_str(&rest[..PREFIX.len() + index]);
                out.push_str("-<digest>-<digest>");
                rest = &rest[PREFIX.len() + index + 34..];
            }
            None => {
                out.push_str(PREFIX);
                rest = body;
            }
        }
    }
    out
}

#[test]
fn the_error_precedence_and_messages_match_python_on_fifty_invalid_inputs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python_dir = temp.path().join("python");
    let rust_dir = temp.path().join("rust");
    let cases_path = temp.path().join("review_error_cases.json");
    std::fs::write(&cases_path, CASES).expect("write the case table");

    // The Python side.
    let script = temp.path().join("review_error_probe.py");
    std::fs::write(&script, PYTHON_SCRIPT).expect("write the probe script");
    let output = Command::new(python_executable())
        .arg("-I")
        .arg(&script)
        .arg(repo_root())
        .arg(&python_dir)
        .arg(&cases_path)
        .env("PYTHONHASHSEED", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn the Python probe");
    assert!(
        output.status.success(),
        "the Python probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let python: Json = serde_json::from_slice(&output.stdout).expect("probe JSON");
    let python_steps: Vec<Step> = python["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .map(Step::from_json)
        .collect();

    // The Rust side.
    let rust_steps = rust_steps(&rust_dir, &cases_path);
    assert_eq!(python_steps.len(), rust_steps.len(), "the same cases");
    assert!(
        python_steps.len() >= 30,
        "the table must carry at least thirty cases, found {}",
        python_steps.len()
    );

    let mut failures = Vec::new();
    for (python_step, rust_step) in python_steps.iter().zip(rust_steps.iter()) {
        assert_eq!(python_step.name, rust_step.name, "the same case names");
        if python_step.ok != rust_step.ok
            || python_step.code != rust_step.code
            || python_step
                .message
                .as_deref()
                .map(normalize_request_identity)
                != rust_step.message.as_deref().map(normalize_request_identity)
        {
            failures.push(format!(
                "{name}: python=(ok={ok} code={code} message={message}) rust=(ok={rok} code={rcode} message={rmsg})",
                name = python_step.name,
                ok = python_step.ok,
                code = python_step.code.as_deref().unwrap_or("<none>"),
                message = python_step
                    .message
                    .as_deref()
                    .map(normalize_request_identity)
                    .unwrap_or_else(|| "<none>".to_owned()),
                rok = rust_step.ok,
                rcode = rust_step.code.as_deref().unwrap_or("<none>"),
                rmsg = rust_step
                    .message
                    .as_deref()
                    .map(normalize_request_identity)
                    .unwrap_or_else(|| "<none>".to_owned()),
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "the error shape differs in {} cases:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn rust_steps(data_dir: &Path, cases_path: &Path) -> Vec<Step> {
    let application = open(data_dir);
    let prepared = application
        .prepare_design_revision("precedence", "mechanical", None, None, None, &[], &[], None)
        .expect("prepare");
    application
        .run_mechanical_reference_workflow(Some("workflow-precedence"), "precedence", "local-user")
        .expect("run the workflow");

    let cases: Vec<Json> =
        serde_json::from_slice(&std::fs::read(cases_path).expect("cases")).expect("case table");
    let mut steps = Vec::new();
    for case in &cases {
        let name = case
            .get("name")
            .and_then(Json::as_str)
            .expect("case name")
            .to_owned();
        let op = case.get("op").and_then(Json::as_str).expect("op");
        let arguments = case.clone();
        let outcome = run_case(&application, op, &arguments, &prepared.request_id);
        steps.push(match outcome {
            Ok(()) => Step {
                name,
                ok: true,
                code: None,
                message: None,
            },
            Err(error) => Step {
                name,
                ok: false,
                code: Some(error.code().to_owned()),
                message: Some(error.message().to_owned()),
            },
        });
    }
    application.close().expect("close");
    steps
}

fn run_case(
    application: &KalaApplication,
    op: &str,
    arguments: &Json,
    prepared_request_id: &str,
) -> Result<(), kala_application::ApplicationError> {
    let request_id = |name: &str| -> String {
        text_argument(arguments, name)
            .unwrap_or_default()
            .replace("<prepared>", prepared_request_id)
    };
    match op {
        "prepare" => application
            .prepare_design_revision(
                &text_argument(arguments, "design_id").unwrap_or_default(),
                &text_argument(arguments, "domain").unwrap_or_default(),
                text_argument(arguments, "summary").as_deref(),
                parameter_argument(arguments).as_ref(),
                text_argument(arguments, "parent_revision_id").as_deref(),
                &identifier_list(arguments, "parent_provenance_ids"),
                &identifier_list(arguments, "transformation_ids"),
                text_argument(arguments, "workflow_id").as_deref(),
            )
            .map(|_| ()),
        "create" => application
            .create_design_revision(
                &text_argument(arguments, "design_id").unwrap_or_default(),
                &text_argument(arguments, "domain").unwrap_or_default(),
                text_argument(arguments, "summary").as_deref(),
                parameter_argument(arguments).as_ref(),
                text_argument(arguments, "parent_revision_id").as_deref(),
                &identifier_list(arguments, "parent_provenance_ids"),
                &identifier_list(arguments, "transformation_ids"),
                text_argument(arguments, "workflow_id").as_deref(),
                &text_argument(arguments, "actor_id").unwrap_or_else(|| "local-user".to_owned()),
                text_argument(arguments, "state_digest").as_deref(),
                text_argument(arguments, "action_digest").as_deref(),
            )
            .map(|_| ()),
        "evaluate" => application
            .evaluate_domain_design(
                &text_argument(arguments, "domain").unwrap_or_default(),
                parameter_argument(arguments).as_ref(),
                text_argument(arguments, "design_id").as_deref(),
            )
            .map(|_| ()),
        "evaluate_overflow" => {
            let mut entries: Vec<(String, CanonicalValue)> = Vec::with_capacity(4_097);
            for index in 0..4_097u32 {
                entries.push((format!("field_{index}"), CanonicalValue::Float(1.0)));
            }
            let parameters = CanonicalValue::object(entries);
            application
                .evaluate_domain_design(
                    &text_argument(arguments, "domain").unwrap_or_default(),
                    Some(&parameters),
                    None,
                )
                .map(|_| ())
        }
        "describe_domain" => application
            .describe_domain(&text_argument(arguments, "domain").unwrap_or_default())
            .map(|_| ()),
        "catalog_steps" => application
            .list_catalog_steps(&text_argument(arguments, "domain").unwrap_or_default())
            .map(|_| ()),
        "workflow_state" => application
            .get_workflow_state(&text_argument(arguments, "workflow_id").unwrap_or_default())
            .map(|_| ()),
        "checkpoints" => application
            .list_workflow_checkpoints(&text_argument(arguments, "workflow_id").unwrap_or_default())
            .map(|_| ()),
        "approve_checkpoint" => application
            .approve_workflow_checkpoint(
                &text_argument(arguments, "workflow_id").unwrap_or_default(),
                &text_argument(arguments, "actor_id").unwrap_or_else(|| "local-user".to_owned()),
                text_argument(arguments, "state_digest").as_deref(),
                text_argument(arguments, "action_digest").as_deref(),
                text_argument(arguments, "reason").as_deref(),
            )
            .map(|_| ()),
        "reject_checkpoint" => application
            .reject_workflow_checkpoint(
                &text_argument(arguments, "workflow_id").unwrap_or_default(),
                &text_argument(arguments, "actor_id").unwrap_or_else(|| "local-user".to_owned()),
                text_argument(arguments, "state_digest").as_deref(),
                text_argument(arguments, "action_digest").as_deref(),
                text_argument(arguments, "reason").as_deref(),
            )
            .map(|_| ()),
        "decide_checkpoint" => application
            .decide_workflow_checkpoint(
                &text_argument(arguments, "workflow_id").unwrap_or_default(),
                &text_argument(arguments, "decision").unwrap_or_default(),
                &text_argument(arguments, "actor_id").unwrap_or_else(|| "local-user".to_owned()),
                text_argument(arguments, "state_digest").as_deref(),
                text_argument(arguments, "action_digest").as_deref(),
                text_argument(arguments, "reason").as_deref(),
            )
            .map(|_| ()),
        "decide_revision" => application
            .decide_revision_commit(
                &request_id("request_id"),
                &text_argument(arguments, "decision").unwrap_or_default(),
                &text_argument(arguments, "actor_id").unwrap_or_else(|| "local-user".to_owned()),
                text_argument(arguments, "state_digest").as_deref(),
                text_argument(arguments, "action_digest").as_deref(),
                text_argument(arguments, "reason").as_deref(),
                text_argument(arguments, "workflow_id").as_deref(),
            )
            .map(|_| ()),
        "commit_revision" => application
            .commit_design_revision(
                &request_id("request_id"),
                &text_argument(arguments, "actor_id").unwrap_or_else(|| "local-user".to_owned()),
                text_argument(arguments, "state_digest").as_deref(),
                text_argument(arguments, "action_digest").as_deref(),
                text_argument(arguments, "reason").as_deref(),
                text_argument(arguments, "workflow_id").as_deref(),
            )
            .map(|_| ()),
        "get_revision_commit" => application
            .get_revision_commit(&request_id("request_id"))
            .map(|_| ()),
        "list_revisions" => application
            .list_design_revisions(
                &text_argument(arguments, "design_id").unwrap_or_default(),
                integer_argument(arguments, "limit", 1_000),
            )
            .map(|_| ()),
        "get_revision" => application
            .get_design_revision(&text_argument(arguments, "revision_id").unwrap_or_default())
            .map(|_| ()),
        "list_events" => application
            .list_events(
                integer_argument(arguments, "after_sequence", 0),
                integer_argument(arguments, "limit", 50),
                text_argument(arguments, "workflow_id").as_deref(),
                text_argument(arguments, "event_type").as_deref(),
                text_argument(arguments, "aggregate_id").as_deref(),
            )
            .map(|_| ()),
        "run_workflow" => application
            .run_mechanical_reference_workflow(
                text_argument(arguments, "workflow_id").as_deref(),
                &text_argument(arguments, "design_id")
                    .unwrap_or_else(|| "mechanical-reference".to_owned()),
                &text_argument(arguments, "actor_id").unwrap_or_else(|| "local-user".to_owned()),
            )
            .map(|_| ()),
        other => panic!("unknown op {other}"),
    }
}
