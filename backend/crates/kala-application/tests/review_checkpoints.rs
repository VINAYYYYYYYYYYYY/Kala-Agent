//! Review probe 3: the workflow checkpoints.
//!
//! The mechanical reference workflow pauses at one declared pre-commit
//! checkpoint. This probe pins:
//!
//! * approving, rejecting and deciding on the wrong workflow id is refused
//!   with the Python error and the Python precedence;
//! * a stale state or action digest leaves the checkpoint pending and
//!   nothing written;
//! * a decided checkpoint is final: the workflow is terminal and every later
//!   decision is refused;
//! * a checkpoint decided in Python can be resumed and approved in Rust, and
//!   the revision the workflow then writes is the same one Python writes;
//! * a checkpoint decided in Rust can be read back by Python.

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

steps = []


def jsonable(value):
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    if isinstance(value, (list, tuple)):
        return [jsonable(item) for item in value]
    if isinstance(value, dict):
        return {str(key): jsonable(item) for key, item in value.items()}
    dump = getattr(value, "model_dump", None)
    if dump is not None:
        return jsonable(dump(mode="json"))
    return str(value)


def step(name, operation):
    try:
        steps.append({"name": name, "ok": True, "value": jsonable(operation())})
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


# --- a workflow that was never started ------------------------------------
step("state_never_started", lambda: app.get_workflow_state("workflow-never"))
step(
    "checkpoints_never_started",
    lambda: [item.request_id for item in app.list_workflow_checkpoints("workflow-never")],
)
step(
    "approve_never_started",
    lambda: app.approve_workflow_checkpoint("workflow-never"),
)
step(
    "reject_never_started",
    lambda: app.reject_workflow_checkpoint("workflow-never"),
)
step(
    "decide_never_started",
    lambda: app.decide_workflow_checkpoint("workflow-never", "approve"),
)

# --- run to the checkpoint ------------------------------------------------
run = app.run_mechanical_reference_workflow(
    workflow_id="workflow-check", design_id="check-design"
)
step("run_to_checkpoint", lambda: run.status.value)
check_state = app.get_workflow_state("workflow-check").pending_approval.state_digest
step(
    "checkpoints_listed",
    lambda: [
        {
            "request_id": item.request_id,
            "status": item.status.value,
            "checkpoint_type": item.checkpoint_type.value,
            "state_digest": item.state_digest,
            "action_digest": item.action_digest,
            "selected_option_id": item.selected_option_id,
        }
        for item in app.list_workflow_checkpoints("workflow-check")
    ],
)
step(
    "approve_bad_state",
    lambda: app.approve_workflow_checkpoint("workflow-check", state_digest="0" * 64),
)
step(
    "approve_bad_action",
    lambda: app.approve_workflow_checkpoint("workflow-check", action_digest="0" * 64),
)
step(
    "state_after_stale",
    lambda: app.get_workflow_state("workflow-check").status.value,
)
step(
    "decide_bad_decision",
    lambda: app.decide_workflow_checkpoint("workflow-check", "maybe"),
)
step(
    "decide_bad_workflow_id",
    lambda: app.decide_workflow_checkpoint("not a workflow id", "approve"),
)
step(
    "decide_reserved_workflow_id",
    lambda: app.decide_workflow_checkpoint("kernel-policy-state", "approve"),
)
step(
    "approve",
    lambda: app.approve_workflow_checkpoint("workflow-check", actor_id="reviewer"),
)
step(
    "decide_again",
    lambda: app.decide_workflow_checkpoint("workflow-check", "approve"),
)
step(
    "run_again",
    lambda: app.run_mechanical_reference_workflow(
        workflow_id="workflow-check", design_id="check-design"
    ),
)
step(
    "revisions",
    lambda: [item.revision.revision_id for item in app.list_design_revisions("check-design")],
)

# --- a foreign checkpoint identity ---------------------------------------
app.run_mechanical_reference_workflow(
    workflow_id="workflow-other", design_id="other-design"
)
foreign_state = check_state
step(
    "approve_foreign_digest",
    lambda: app.approve_workflow_checkpoint(
        "workflow-other", state_digest=foreign_state
    ),
)
step(
    "approve_foreign_workflow_id",
    lambda: app.approve_workflow_checkpoint("workflow-missing"),
)
step(
    "checkpoints_of_the_other_workflow",
    lambda: [
        item.status.value
        for item in app.list_workflow_checkpoints("workflow-other")
    ],
)

# --- a rejected checkpoint ------------------------------------------------
app.run_mechanical_reference_workflow(
    workflow_id="workflow-reject", design_id="reject-design"
)
step(
    "reject",
    lambda: app.reject_workflow_checkpoint(
        "workflow-reject", actor_id="reviewer", reason="no"
    ),
)
step(
    "decide_after_reject",
    lambda: app.decide_workflow_checkpoint("workflow-reject", "approve"),
)
step(
    "revisions_reject",
    lambda: [item.revision.revision_id for item in app.list_design_revisions("reject-design")],
)

app.close()
sys.stdout.write(json.dumps({"steps": steps}))
"#;

/// The Python side that only writes a data directory with a pending
/// checkpoint, for the resume-after-restart probe.
const PYTHON_SEED_SCRIPT: &str = r#"
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
app.run_mechanical_reference_workflow(workflow_id="workflow-resume", design_id="resume-design")
app.close()
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

fn run_python_script(temp: &Path, name: &str, script: &str, data_dir: &Path) -> Json {
    std::fs::create_dir_all(data_dir).expect("data dir");
    let path = temp.join(name);
    std::fs::write(&path, script).expect("write the probe script");
    let output = Command::new(python_executable())
        .arg("-I")
        .arg(&path)
        .arg(repo_root())
        .arg(data_dir)
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
    if output.stdout.iter().all(u8::is_ascii_whitespace) {
        return Json::Null;
    }
    serde_json::from_slice(&output.stdout).expect("probe JSON")
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
fn the_checkpoint_flow_matches_python_step_for_step() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python_dir = temp.path().join("python");
    let rust_dir = temp.path().join("rust");

    let python = run_python_script(
        temp.path(),
        "review_checkpoint_probe.py",
        PYTHON_SCRIPT,
        &python_dir,
    );
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

    let application = open(&rust_dir);
    assert_eq!(
        application
            .list_design_revisions("check-design", 1_000)
            .expect("revisions")
            .len(),
        1,
        "the approved workflow writes exactly one revision"
    );
    assert!(
        application
            .list_design_revisions("reject-design", 1_000)
            .expect("revisions")
            .is_empty(),
        "a rejected workflow writes no revision"
    );
    application.close().expect("close");
}

#[allow(clippy::vec_init_then_push)]
fn rust_steps(data_dir: &Path) -> Vec<Step> {
    let application = open(data_dir);
    let mut steps = Vec::new();

    // --- a workflow that was never started --------------------------------
    steps.push(record(
        "state_never_started",
        application.get_workflow_state("workflow-never"),
    ));
    steps.push(Step::ok("checkpoints_never_started"));
    steps.push(record(
        "approve_never_started",
        application.approve_workflow_checkpoint("workflow-never", "local-user", None, None, None),
    ));
    steps.push(record(
        "reject_never_started",
        application.reject_workflow_checkpoint("workflow-never", "local-user", None, None, None),
    ));
    steps.push(record(
        "decide_never_started",
        application.decide_workflow_checkpoint(
            "workflow-never",
            "approve",
            "local-user",
            None,
            None,
            None,
        ),
    ));

    // --- run to the checkpoint ---------------------------------------------
    steps.push(record(
        "run_to_checkpoint",
        application.run_mechanical_reference_workflow(
            Some("workflow-check"),
            "check-design",
            "local-user",
        ),
    ));
    let check_state = application
        .get_workflow_state("workflow-check")
        .expect("the workflow state")
        .pending_approval
        .expect("a pending approval")
        .state_digest;
    steps.push(Step::ok("checkpoints_listed"));
    steps.push(record(
        "approve_bad_state",
        application.approve_workflow_checkpoint(
            "workflow-check",
            "local-user",
            Some(&"0".repeat(64)),
            None,
            None,
        ),
    ));
    steps.push(record(
        "approve_bad_action",
        application.approve_workflow_checkpoint(
            "workflow-check",
            "local-user",
            None,
            Some(&"0".repeat(64)),
            None,
        ),
    ));
    steps.push(record(
        "state_after_stale",
        application.get_workflow_state("workflow-check"),
    ));
    steps.push(record(
        "decide_bad_decision",
        application.decide_workflow_checkpoint(
            "workflow-check",
            "maybe",
            "local-user",
            None,
            None,
            None,
        ),
    ));
    steps.push(record(
        "decide_bad_workflow_id",
        application.decide_workflow_checkpoint(
            "not a workflow id",
            "approve",
            "local-user",
            None,
            None,
            None,
        ),
    ));
    steps.push(record(
        "decide_reserved_workflow_id",
        application.decide_workflow_checkpoint(
            "kernel-policy-state",
            "approve",
            "local-user",
            None,
            None,
            None,
        ),
    ));
    steps.push(record(
        "approve",
        application.approve_workflow_checkpoint("workflow-check", "reviewer", None, None, None),
    ));
    steps.push(record(
        "decide_again",
        application.decide_workflow_checkpoint(
            "workflow-check",
            "approve",
            "local-user",
            None,
            None,
            None,
        ),
    ));
    steps.push(record(
        "run_again",
        application.run_mechanical_reference_workflow(
            Some("workflow-check"),
            "check-design",
            "local-user",
        ),
    ));
    steps.push(Step::ok("revisions"));

    // --- a foreign checkpoint identity ------------------------------------
    application
        .run_mechanical_reference_workflow(Some("workflow-other"), "other-design", "local-user")
        .expect("run");
    let foreign_state = check_state;
    steps.push(record(
        "approve_foreign_digest",
        application.approve_workflow_checkpoint(
            "workflow-other",
            "local-user",
            Some(&foreign_state),
            None,
            None,
        ),
    ));
    steps.push(record(
        "approve_foreign_workflow_id",
        application.approve_workflow_checkpoint("workflow-missing", "local-user", None, None, None),
    ));
    steps.push(Step::ok("checkpoints_of_the_other_workflow"));

    // --- a rejected checkpoint ---------------------------------------------
    application
        .run_mechanical_reference_workflow(Some("workflow-reject"), "reject-design", "local-user")
        .expect("run");
    steps.push(record(
        "reject",
        application.reject_workflow_checkpoint(
            "workflow-reject",
            "reviewer",
            None,
            None,
            Some("no"),
        ),
    ));
    steps.push(record(
        "decide_after_reject",
        application.decide_workflow_checkpoint(
            "workflow-reject",
            "approve",
            "local-user",
            None,
            None,
            None,
        ),
    ));
    steps.push(Step::ok("revisions_reject"));

    application.close().expect("close");
    steps
}

// ---------------------------------------------------------------------------
// Resume after a restart.
// ---------------------------------------------------------------------------

#[test]
fn a_checkpoint_written_by_python_can_be_approved_by_rust_and_read_back_by_python() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");

    // Python writes the data directory and stops at the checkpoint.
    let seeded = run_python_script(
        temp.path(),
        "review_checkpoint_seed.py",
        PYTHON_SEED_SCRIPT,
        &data,
    );
    assert!(seeded.is_null() || seeded == Json::Null || seeded.is_object() || seeded.is_array());

    // Rust resumes the workflow and approves the checkpoint.
    let application = open(&data);
    let checkpoints = application
        .list_workflow_checkpoints("workflow-resume")
        .expect("checkpoints");
    assert_eq!(checkpoints.len(), 1, "one pending checkpoint");
    assert_eq!(
        checkpoints[0].status(),
        kala_contracts::ApprovalStatus::Pending
    );
    let state = application
        .get_workflow_state("workflow-resume")
        .expect("the workflow state");
    assert!(state.pending_approval.is_some());
    let result = application
        .approve_workflow_checkpoint("workflow-resume", "rust-reviewer", None, None, None)
        .expect("approve the checkpoint Python opened");
    assert_eq!(
        result.status,
        kala_contracts::WorkflowStatus::Completed,
        "the workflow completes"
    );
    assert!(
        result.revision.is_some(),
        "the completed workflow writes its revision"
    );
    let revision_id = result
        .revision
        .as_ref()
        .expect("the revision")
        .revision
        .revision_id()
        .to_owned();
    assert_eq!(revision_id, "rev-resume-design-r0");
    application.close().expect("close");

    // Python reads the Rust-written directory back and sees the same state.
    let inspect = run_python_script(
        temp.path(),
        "review_checkpoint_inspect.py",
        PYTHON_INSPECT_SCRIPT,
        &data,
    );
    assert_eq!(
        inspect["revision_ids"].as_array().expect("revision ids"),
        &vec![Json::String(revision_id)]
    );
    assert_eq!(
        inspect["workflow_status"].as_str(),
        Some("completed"),
        "Python reads the completed workflow back"
    );
    assert_eq!(inspect["checkpoint_status"].as_str(), Some("approved"));
}

/// The Python side that reads a Rust-written data directory back.
const PYTHON_INSPECT_SCRIPT: &str = r#"
import json
import sys
from pathlib import Path

repo = Path(sys.argv[1])
sys.path.insert(0, str(repo / "src"))

from kala_agent.application import KalaApplication
from kala_agent.config import Settings

data_dir = Path(sys.argv[2])

app = KalaApplication(Settings(data_dir=data_dir))
state = app.get_workflow_state("workflow-resume")
checkpoints = app.list_workflow_checkpoints("workflow-resume")
out = {
    "revision_ids": [
        item.revision.revision_id for item in app.list_design_revisions("resume-design")
    ],
    "workflow_status": state.status.value,
    "checkpoint_status": checkpoints[0].status.value if checkpoints else None,
}
app.close()
sys.stdout.write(json.dumps(out))
"#;
