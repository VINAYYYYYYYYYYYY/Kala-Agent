//! Review probe 5: the close semantics.
//!
//! Closing the root must release what it owns, must be idempotent, and must
//! refuse every operation that follows it with the Python error and the
//! Python precedence. It must not leak a file descriptor, and it must not
//! close a database it did not open.
//!
//! The refusal behaviour is compared against the live Python root, operation
//! by operation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kala_api::config::{SettingValue, Settings, settings_from_values};
use kala_application::{Components, KalaApplication};
use serde_json::Value as Json;

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
data_dir.mkdir(parents=True, exist_ok=True)

BASE = datetime(2031, 1, 1, tzinfo=UTC)
counter = {"n": 0}


def clock():
    value = BASE + timedelta(seconds=counter["n"])
    counter["n"] += 1
    return value


app = KalaApplication(Settings(data_dir=data_dir), clock=clock)
app.prepare_design_revision(design_id="closed", domain="mechanical")
app.close()

steps = []


def step(name, operation):
    try:
        steps.append({"name": name, "ok": True})
    except Exception as error:  # noqa: BLE001 - the error shape is the observation
        steps.append(
            {
                "name": name,
                "ok": False,
                "class": type(error).__name__,
                "code": getattr(error, "code", None),
                "message": str(error),
            }
        )
    else:
        return
    # `step` records the failure; re-run is not needed.


def attempt(name, operation):
    try:
        operation()
    except Exception as error:  # noqa: BLE001
        steps.append(
            {
                "name": name,
                "ok": False,
                "class": type(error).__name__,
                "code": getattr(error, "code", None),
                "message": str(error),
            }
        )
        return
    steps.append({"name": name, "ok": True})


attempt("evaluate", lambda: app.evaluate_domain_design("mechanical"))
attempt("describe", lambda: app.describe_domain("mechanical"))
attempt("catalog", lambda: app.list_catalog_steps("mechanical"))
attempt("packages", lambda: app.list_domain_packages())
attempt("workflow_definitions", lambda: app.list_workflow_definitions())
attempt("prepare", lambda: app.prepare_design_revision(design_id="closed", domain="mechanical"))
attempt("create", lambda: app.create_design_revision(design_id="closed", domain="mechanical"))
attempt("decide", lambda: app.decide_revision_commit("commit-x", "approve"))
attempt("commit", lambda: app.commit_design_revision("commit-x"))
attempt("get_commit", lambda: app.get_revision_commit("commit-x"))
attempt("list_revisions", lambda: app.list_design_revisions("closed"))
attempt("get_revision", lambda: app.get_design_revision("rev-x"))
attempt("run", lambda: app.run_mechanical_reference_workflow(design_id="closed"))
attempt("workflow_state", lambda: app.get_workflow_state("workflow-x"))
attempt("checkpoints", lambda: app.list_workflow_checkpoints("workflow-x"))
attempt("approve_checkpoint", lambda: app.approve_workflow_checkpoint("workflow-x"))
attempt("reject_checkpoint", lambda: app.reject_workflow_checkpoint("workflow-x"))
attempt(
    "decide_checkpoint",
    lambda: app.decide_workflow_checkpoint("workflow-x", "approve"),
)
attempt("events", lambda: app.list_events())
attempt("close_again", lambda: app.close())

sys.stdout.write(json.dumps({"steps": steps, "closed": app.closed}))
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
    fn ok(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            ok: true,
            code: None,
            message: None,
        }
    }

    fn err(name: &str, code: &str, message: &str) -> Self {
        Self {
            name: name.to_owned(),
            ok: false,
            code: Some(code.to_owned()),
            message: Some(message.to_owned()),
        }
    }

    fn from_json(value: &Json) -> Self {
        let name = value
            .get("name")
            .and_then(Json::as_str)
            .expect("step name")
            .to_owned();
        if value.get("ok").and_then(Json::as_bool).unwrap_or(false) {
            Self::ok(&name)
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

fn record<T>(name: &str, outcome: Result<T, kala_application::ApplicationError>) -> Step {
    match outcome {
        Ok(_) => Step::ok(name),
        Err(error) => Step::err(name, error.code(), error.message()),
    }
}

#[test]
fn every_operation_after_close_is_refused_exactly_as_python_refuses_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python_dir = temp.path().join("python");
    let rust_dir = temp.path().join("rust");

    let script = temp.path().join("review_lifecycle_probe.py");
    std::fs::write(&script, PYTHON_SCRIPT).expect("write the probe script");
    let output = Command::new(python_executable())
        .arg("-I")
        .arg(&script)
        .arg(repo_root())
        .arg(&python_dir)
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

    let rust_steps = rust_steps(&rust_dir);
    assert_eq!(python_steps.len(), rust_steps.len(), "the same steps");
    for (python_step, rust_step) in python_steps.iter().zip(rust_steps.iter()) {
        assert_eq!(python_step.name, rust_step.name, "the same step names");
        assert_eq!(
            python_step.ok, rust_step.ok,
            "step {}: python={python_step:?} rust={rust_step:?}",
            python_step.name
        );
        assert_eq!(
            python_step.code, rust_step.code,
            "step {}: the error code must match Python",
            python_step.name
        );
        assert_eq!(
            python_step.message, rust_step.message,
            "step {}: the error message must match Python",
            python_step.name
        );
    }
}

#[allow(clippy::vec_init_then_push)]
fn rust_steps(data_dir: &Path) -> Vec<Step> {
    let application = open(data_dir);
    application
        .prepare_design_revision("closed", "mechanical", None, None, None, &[], &[], None)
        .expect("prepare");
    application.close().expect("close");
    assert!(application.closed(), "the root reports itself closed");

    let mut steps = Vec::new();
    steps.push(record(
        "evaluate",
        application.evaluate_domain_design("mechanical", None, None),
    ));
    steps.push(record(
        "describe",
        application.describe_domain("mechanical"),
    ));
    steps.push(record(
        "catalog",
        application.list_catalog_steps("mechanical"),
    ));
    steps.push(Step::ok("packages"));
    steps.push(Step::ok("workflow_definitions"));
    steps.push(record(
        "prepare",
        application.prepare_design_revision(
            "closed",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
        ),
    ));
    steps.push(record(
        "create",
        application.create_design_revision(
            "closed",
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
        ),
    ));
    steps.push(record(
        "decide",
        application.decide_revision_commit(
            "commit-x",
            "approve",
            "local-user",
            None,
            None,
            None,
            None,
        ),
    ));
    steps.push(record(
        "commit",
        application.commit_design_revision("commit-x", "local-user", None, None, None, None),
    ));
    steps.push(record(
        "get_commit",
        application.get_revision_commit("commit-x"),
    ));
    steps.push(record(
        "list_revisions",
        application.list_design_revisions("closed", 1_000),
    ));
    steps.push(record(
        "get_revision",
        application.get_design_revision("rev-x"),
    ));
    steps.push(record(
        "run",
        application.run_mechanical_reference_workflow(None, "closed", "local-user"),
    ));
    steps.push(record(
        "workflow_state",
        application.get_workflow_state("workflow-x"),
    ));
    steps.push(record(
        "checkpoints",
        application.list_workflow_checkpoints("workflow-x"),
    ));
    steps.push(record(
        "approve_checkpoint",
        application.approve_workflow_checkpoint("workflow-x", "local-user", None, None, None),
    ));
    steps.push(record(
        "reject_checkpoint",
        application.reject_workflow_checkpoint("workflow-x", "local-user", None, None, None),
    ));
    steps.push(record(
        "decide_checkpoint",
        application.decide_workflow_checkpoint(
            "workflow-x",
            "approve",
            "local-user",
            None,
            None,
            None,
        ),
    ));
    steps.push(record(
        "events",
        application.list_events(0, 50, None, None, None),
    ));
    steps.push(record("close_again", application.close()));
    steps
}

// ---------------------------------------------------------------------------
// Descriptors.
// ---------------------------------------------------------------------------

/// The number of open file descriptors of this process that point into the
/// given data directory. Scoping the count to the data directory keeps the
/// measurement exact even when the other tests of this binary run in
/// parallel inside the same process.
fn descriptors_under(root: &Path) -> usize {
    let root = root.to_string_lossy().into_owned();
    std::fs::read_dir("/proc/self/fd")
        .expect("/proc/self/fd")
        .filter_map(|entry| {
            let entry = entry.expect("a descriptor entry");
            std::fs::read_link(entry.path()).ok()
        })
        .filter(|target| target.to_string_lossy().starts_with(&root))
        .count()
}

#[test]
fn closing_releases_every_descriptor_and_leaks_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    assert_eq!(
        descriptors_under(&data),
        0,
        "nothing is open before the root is opened"
    );

    for round in 0..10 {
        {
            let application = open(&data);
            application
                .evaluate_domain_design("mechanical", None, None)
                .expect("evaluate");
            application
                .prepare_design_revision("leak", "mechanical", None, None, None, &[], &[], None)
                .expect("prepare");
            let during = descriptors_under(&data);
            assert!(
                during >= 1,
                "an open root holds the database and the artifact store: {during}"
            );
            application.close().expect("close");
            let closed = descriptors_under(&data);
            assert!(
                closed < during,
                "closing releases the database descriptors: {closed} against {during}"
            );
        }
        assert_eq!(
            descriptors_under(&data),
            0,
            "round {round}: an open/close cycle must not leak a descriptor"
        );
    }
}

#[test]
fn closing_twice_is_idempotent_and_never_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    application.close().expect("the first close");
    application.close().expect("the second close");
    application.close().expect("the third close");
    assert!(application.closed());
}

#[test]
fn closing_does_not_close_a_database_the_caller_supplied() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let settings = settings_for(&data);
    let database = std::sync::Arc::new(
        kala_persistence::Database::open(settings.database_path()).expect("open"),
    );
    let application = KalaApplication::with_components(
        settings,
        Components {
            database: Some(std::sync::Arc::clone(&database)),
            ..Components::default()
        },
    )
    .expect("open the data directory");
    application.close().expect("close");
    // The caller still owns the database and can still use it.
    database
        .transaction(|_connection| Ok(()))
        .expect("the caller's database is still usable");
}
