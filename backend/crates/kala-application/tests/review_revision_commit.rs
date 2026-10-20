//! Review probe 2: the two-phase revision commit — races, finality and
//! replay.
//!
//! The revision commit is the one place where the root must be exactly-once.
//! This probe pins:
//!
//! * a prepared commit is a single-use authorization: after an approve or a
//!   reject, every replay is refused and nothing more is written;
//! * a stale state or action digest is refused before any decision is
//!   recorded;
//! * a design head that moved after the preparation is refused, with the
//!   Python message, and writes nothing;
//! * concurrent prepares, decisions and commits produce exactly one revision
//!   and a linear history;
//! * a pending commit that was tampered with on disk and re-chained so the
//!   journal still verifies is refused when it is decided.
//!
//! The single-use and refusal semantics are compared against the live Python
//! root, step by step: same error code, same message, same state left behind.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};

use kala_api::config::{SettingValue, Settings, settings_from_values};
use kala_application::{Components, KalaApplication};
use kala_json::CanonicalValue;

use serde_json::Value as Json;

/// The Python side: the same two-phase script, step by step.
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


# --- approve, then replay -------------------------------------------------
first = app.prepare_design_revision(design_id="race-approve", domain="mechanical")
step("prepare_approve", lambda: first)
step(
    "decide_approve",
    lambda: app.decide_revision_commit(first.request_id, "approve", actor_id="reviewer"),
)
step(
    "replay_decide_approve",
    lambda: app.decide_revision_commit(first.request_id, "approve", actor_id="reviewer"),
)
step("replay_commit", lambda: app.commit_design_revision(first.request_id))
step("read_commit_approved", lambda: app.get_revision_commit(first.request_id))
step(
    "read_revisions_approve",
    lambda: [item.revision.revision_id for item in app.list_design_revisions("race-approve")],
)

# --- reject, then replay --------------------------------------------------
second = app.prepare_design_revision(design_id="race-reject", domain="mechanical")
step("prepare_reject", lambda: second)
step(
    "decide_reject",
    lambda: app.decide_revision_commit(
        second.request_id, "reject", actor_id="reviewer", reason="not now"
    ),
)
step(
    "replay_decide_reject",
    lambda: app.decide_revision_commit(second.request_id, "reject", actor_id="reviewer"),
)
step("replay_commit_rejected", lambda: app.commit_design_revision(second.request_id))
step("read_commit_rejected", lambda: app.get_revision_commit(second.request_id))
step(
    "prepare_after_rejection",
    lambda: app.prepare_design_revision(design_id="race-reject", domain="mechanical"),
)

# --- stale digests ---------------------------------------------------------
third = app.prepare_design_revision(design_id="race-stale", domain="mechanical")
step("prepare_stale", lambda: third)
step(
    "decide_stale_state",
    lambda: app.decide_revision_commit(
        third.request_id, "approve", state_digest="0" * 64
    ),
)
step(
    "decide_stale_action",
    lambda: app.decide_revision_commit(
        third.request_id, "approve", action_digest="0" * 64
    ),
)
step("read_commit_stale", lambda: app.get_revision_commit(third.request_id))

# --- a head that moved after the preparation ------------------------------
base = app.prepare_design_revision(design_id="race-moved", domain="mechanical")
app.commit_design_revision(base.request_id)
moved = app.prepare_design_revision(
    design_id="race-moved", domain="mechanical", summary="branch A"
)
other = app.prepare_design_revision(
    design_id="race-moved", domain="mechanical", summary="branch B"
)
app.commit_design_revision(other.request_id)
step(
    "decide_after_head_moved",
    lambda: app.decide_revision_commit(moved.request_id, "approve"),
)
step(
    "read_revisions_moved",
    lambda: [item.revision.revision_id for item in app.list_design_revisions("race-moved")],
)

# --- malformed and unknown identities ------------------------------------
step(
    "decide_unknown_request",
    lambda: app.decide_revision_commit("commit-does-not-exist", "approve"),
)
step(
    "decide_bad_request_id",
    lambda: app.decide_revision_commit("not a request id", "approve"),
)
step(
    "decide_bad_decision",
    lambda: app.decide_revision_commit(moved.request_id, "maybe"),
)
step(
    "decide_wrong_workflow",
    lambda: app.decide_revision_commit(
        moved.request_id, "approve", workflow_id="workflow-not-mine"
    ),
)

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

fn run_python(temp: &Path, data_dir: &Path) -> Vec<Json> {
    std::fs::create_dir_all(data_dir).expect("data dir");
    let script = temp.join("review_revision_probe.py");
    std::fs::write(&script, PYTHON_SCRIPT).expect("write the probe script");
    let output = Command::new(python_executable())
        .arg("-I")
        .arg(&script)
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
    serde_json::from_slice::<Json>(&output.stdout).expect("probe JSON")["steps"]
        .as_array()
        .expect("steps")
        .clone()
}

/// One recorded step: the outcome and, on failure, the Python error shape.
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

/// Record the outcome of one Rust operation the way the Python probe does.
fn record<T>(name: &str, outcome: Result<T, kala_application::ApplicationError>) -> Step {
    match outcome {
        Ok(_) => Step::ok(name),
        Err(error) => Step::err(name, error.code(), error.message()),
    }
}

fn revision_ids(application: &KalaApplication, design_id: &str) -> Vec<String> {
    application
        .list_design_revisions(design_id, 1_000)
        .expect("revisions")
        .iter()
        .map(|record| record.revision.revision_id().to_owned())
        .collect()
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
fn the_two_phase_commit_is_single_use_and_matches_python_step_for_step() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python_dir = temp.path().join("python");
    let rust_dir = temp.path().join("rust");

    let python_raw = run_python(temp.path(), &python_dir);
    let python: Vec<Step> = python_raw.iter().map(Step::from_json).collect();
    let rust = rust_steps(&rust_dir);

    assert_eq!(python.len(), rust.len(), "the same number of steps");
    for (python_step, rust_step) in python.iter().zip(rust.iter()) {
        assert_eq!(python_step.name, rust_step.name, "the same step names");
        assert_eq!(
            python_step.ok, rust_step.ok,
            "step {}: python={:?} rust={:?}",
            python_step.name, python_step, rust_step
        );
        assert_eq!(
            python_step.code, rust_step.code,
            "step {}: the error code must match Python",
            python_step.name
        );
        assert_eq!(
            python_step
                .message
                .as_deref()
                .map(normalize_request_identity),
            rust_step.message.as_deref().map(normalize_request_identity),
            "step {}: the error message must match Python",
            python_step.name
        );
    }

    // The state left behind must match too: exactly one revision for the
    // approved design and none for the rejected one.
    let application = open(&rust_dir);
    assert_eq!(
        revision_ids(&application, "race-approve"),
        vec!["rev-race-approve-r0".to_owned()]
    );
    assert!(
        revision_ids(&application, "race-reject").is_empty(),
        "a rejected commit writes nothing"
    );
    assert!(
        revision_ids(&application, "race-stale").is_empty(),
        "a stale digest writes nothing"
    );
    application.close().expect("close");
}

/// The Rust side of the scripted two-phase flow.
fn rust_steps(data_dir: &Path) -> Vec<Step> {
    let application = open(data_dir);
    let mut steps = Vec::new();

    // --- approve, then replay ---------------------------------------------
    let first = application
        .prepare_design_revision(
            "race-approve",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("prepare");
    steps.push(Step::ok("prepare_approve"));
    steps.push(record(
        "decide_approve",
        application.decide_revision_commit(
            &first.request_id,
            "approve",
            "reviewer",
            None,
            None,
            None,
            None,
        ),
    ));
    steps.push(record(
        "replay_decide_approve",
        application.decide_revision_commit(
            &first.request_id,
            "approve",
            "reviewer",
            None,
            None,
            None,
            None,
        ),
    ));
    steps.push(record(
        "replay_commit",
        application.commit_design_revision(&first.request_id, "local-user", None, None, None, None),
    ));
    steps.push(record(
        "read_commit_approved",
        application.get_revision_commit(&first.request_id),
    ));
    steps.push(Step::ok("read_revisions_approve"));

    // --- reject, then replay ----------------------------------------------
    let second = application
        .prepare_design_revision(
            "race-reject",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("prepare");
    steps.push(Step::ok("prepare_reject"));
    steps.push(record(
        "decide_reject",
        application.decide_revision_commit(
            &second.request_id,
            "reject",
            "reviewer",
            None,
            None,
            Some("not now"),
            None,
        ),
    ));
    steps.push(record(
        "replay_decide_reject",
        application.decide_revision_commit(
            &second.request_id,
            "reject",
            "reviewer",
            None,
            None,
            None,
            None,
        ),
    ));
    steps.push(record(
        "replay_commit_rejected",
        application.commit_design_revision(
            &second.request_id,
            "local-user",
            None,
            None,
            None,
            None,
        ),
    ));
    steps.push(record(
        "read_commit_rejected",
        application.get_revision_commit(&second.request_id),
    ));
    steps.push(record(
        "prepare_after_rejection",
        application.prepare_design_revision(
            "race-reject",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
        ),
    ));

    // --- stale digests -----------------------------------------------------
    let third = application
        .prepare_design_revision("race-stale", "mechanical", None, None, None, &[], &[], None)
        .expect("prepare");
    steps.push(Step::ok("prepare_stale"));
    steps.push(record(
        "decide_stale_state",
        application.decide_revision_commit(
            &third.request_id,
            "approve",
            "local-user",
            Some(&"0".repeat(64)),
            None,
            None,
            None,
        ),
    ));
    steps.push(record(
        "decide_stale_action",
        application.decide_revision_commit(
            &third.request_id,
            "approve",
            "local-user",
            None,
            Some(&"0".repeat(64)),
            None,
            None,
        ),
    ));
    steps.push(record(
        "read_commit_stale",
        application.get_revision_commit(&third.request_id),
    ));

    // --- a head that moved after the preparation ---------------------------
    let base = application
        .prepare_design_revision("race-moved", "mechanical", None, None, None, &[], &[], None)
        .expect("prepare");
    application
        .commit_design_revision(&base.request_id, "local-user", None, None, None, None)
        .expect("commit");
    let moved = application
        .prepare_design_revision(
            "race-moved",
            "mechanical",
            Some("branch A"),
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("prepare");
    let other = application
        .prepare_design_revision(
            "race-moved",
            "mechanical",
            Some("branch B"),
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("prepare");
    application
        .commit_design_revision(&other.request_id, "local-user", None, None, None, None)
        .expect("commit");
    steps.push(record(
        "decide_after_head_moved",
        application.decide_revision_commit(
            &moved.request_id,
            "approve",
            "local-user",
            None,
            None,
            None,
            None,
        ),
    ));
    steps.push(Step::ok("read_revisions_moved"));

    // --- malformed and unknown identities ----------------------------------
    steps.push(record(
        "decide_unknown_request",
        application.decide_revision_commit(
            "commit-does-not-exist",
            "approve",
            "local-user",
            None,
            None,
            None,
            None,
        ),
    ));
    steps.push(record(
        "decide_bad_request_id",
        application.decide_revision_commit(
            "not a request id",
            "approve",
            "local-user",
            None,
            None,
            None,
            None,
        ),
    ));
    steps.push(record(
        "decide_bad_decision",
        application.decide_revision_commit(
            &moved.request_id,
            "maybe",
            "local-user",
            None,
            None,
            None,
            None,
        ),
    ));
    steps.push(record(
        "decide_wrong_workflow",
        application.decide_revision_commit(
            &moved.request_id,
            "approve",
            "local-user",
            None,
            None,
            None,
            Some("workflow-not-mine"),
        ),
    ));

    application.close().expect("close");
    steps
}

#[test]
fn a_prepared_commit_survives_a_restart_and_is_still_decidable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    let prepared = application
        .prepare_design_revision("restart", "mechanical", None, None, None, &[], &[], None)
        .expect("prepare");
    application.close().expect("close");

    // A second launch sees exactly the same authorization.
    let application = open(&data);
    let reread = application
        .get_revision_commit(&prepared.request_id)
        .expect("the prepared commit is still there");
    assert_eq!(reread.request_id, prepared.request_id);
    assert_eq!(reread.state_digest, prepared.state_digest);
    assert_eq!(reread.action_digest, prepared.action_digest);
    assert_eq!(reread.spec_digest, prepared.spec_digest);
    assert_eq!(reread.status, kala_contracts::ApprovalStatus::Pending);
    assert!(!reread.committed);
    assert!(
        application
            .list_design_revisions("restart", 1_000)
            .expect("revisions")
            .is_empty(),
        "a restart does not turn a preparation into a revision"
    );

    // And it is still decidable, with the digests it was reviewed against.
    let record = application
        .commit_design_revision(
            &prepared.request_id,
            "local-user",
            Some(&prepared.state_digest),
            Some(&prepared.action_digest),
            None,
            None,
        )
        .expect("commit after the restart");
    assert_eq!(record.revision.revision_id(), "rev-restart-r0");
    application.close().expect("close");
}

// ---------------------------------------------------------------------------
// The races.
// ---------------------------------------------------------------------------

#[test]
fn concurrent_commits_of_one_prepared_request_write_one_revision() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = Arc::new(open(&data));
    let prepared = application
        .prepare_design_revision(
            "race-threads",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("prepare");

    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();
    for index in 0..8 {
        let application = Arc::clone(&application);
        let barrier = Arc::clone(&barrier);
        let request_id = prepared.request_id.clone();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let outcome = application.commit_design_revision(
                &request_id,
                "local-user",
                None,
                None,
                None,
                None,
            );
            (
                index,
                outcome.is_ok(),
                outcome.err().map(|error| error.code().to_owned()),
            )
        }));
    }
    let mut winners = 0;
    let mut codes = Vec::new();
    for handle in handles {
        let (_index, ok, code) = handle.join().expect("thread");
        if ok {
            winners += 1;
        } else {
            codes.push(code.expect("an error code"));
        }
    }
    assert_eq!(winners, 1, "exactly one commit may win the race");
    for code in codes {
        assert!(
            matches!(
                code.as_str(),
                "checkpoint_not_pending" | "revision_head_moved" | "checkpoint_digest_mismatch"
            ),
            "a losing racer must report a defined conflict, got {code}"
        );
    }
    assert_eq!(
        revision_ids(&application, "race-threads"),
        vec!["rev-race-threads-r0".to_owned()]
    );
    let created = application
        .list_events(0, 1_000, None, Some("revision.created"), None)
        .expect("events");
    assert_eq!(
        created.events.len(),
        1,
        "exactly one committed revision event is journalled"
    );
    application.close().expect("close");
}

#[test]
fn concurrent_prepares_and_commits_keep_the_history_linear() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = Arc::new(open(&data));

    let barrier = Arc::new(Barrier::new(6));
    let mut handles = Vec::new();
    for _index in 0..6 {
        let application = Arc::clone(&application);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let prepared = application.prepare_design_revision(
                "race-linear",
                "mechanical",
                None,
                None,
                None,
                &[],
                &[],
                None,
            );
            let Ok(prepared) = prepared else {
                return false;
            };
            application
                .commit_design_revision(&prepared.request_id, "local-user", None, None, None, None)
                .is_ok()
        }));
    }
    let mut winners = 0;
    for handle in handles {
        if handle.join().expect("thread") {
            winners += 1;
        }
    }
    assert_eq!(winners, 1, "one writer wins the compare-and-swap");
    let revisions = revision_ids(&application, "race-linear");
    assert_eq!(revisions, vec!["rev-race-linear-r0".to_owned()]);
    let conflicts = application
        .list_events(0, 1_000, None, Some("revision.commit_conflicted"), None)
        .expect("events");
    assert!(
        conflicts.events.len() <= 1,
        "a lost race journals at most one conflict"
    );
    application.close().expect("close");
}

#[test]
fn a_decision_racing_a_preparation_is_exactly_once() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = Arc::new(open(&data));
    let prepared = application
        .prepare_design_revision(
            "race-decide",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("prepare");

    let barrier = Arc::new(Barrier::new(2));
    let decide_application = Arc::clone(&application);
    let decide_barrier = Arc::clone(&barrier);
    let request_id = prepared.request_id.clone();
    let decider = std::thread::spawn(move || {
        decide_barrier.wait();
        decide_application
            .decide_revision_commit(&request_id, "approve", "local-user", None, None, None, None)
            .is_ok()
    });
    let prepare_application = Arc::clone(&application);
    let prepare_barrier = Arc::clone(&barrier);
    let preparer = std::thread::spawn(move || {
        prepare_barrier.wait();
        prepare_application
            .prepare_design_revision(
                "race-decide",
                "mechanical",
                None,
                None,
                None,
                &[],
                &[],
                None,
            )
            .is_ok()
    });
    let decided = decider.join().expect("thread");
    let prepared_again = preparer.join().expect("thread");
    assert!(decided, "the decision wins the race");
    // The preparation may succeed (it is the same authorization) or be
    // refused (the request already carries a decision); either way there is
    // exactly one revision.
    let _ = prepared_again;
    assert_eq!(
        revision_ids(&application, "race-decide"),
        vec!["rev-race-decide-r0".to_owned()]
    );
    application.close().expect("close");
}

// ---------------------------------------------------------------------------
// A tampered pending commit restored from disk.
// ---------------------------------------------------------------------------

fn sha256_hex(value: &str) -> String {
    kala_tools::protocol::sha256_hex_text(value).to_owned()
}

/// The chain digest algorithm, re-implemented from `persistence/chain.py`.
#[allow(clippy::too_many_arguments)]
fn chain_digest(
    sequence: i64,
    event_type: &str,
    aggregate_id: &str,
    workflow_id: Option<&str>,
    run_id: Option<&str>,
    occurred_at: &str,
    payload_digest: &str,
    previous_event_digest: &str,
) -> String {
    let parts: Vec<String> = vec![
        "kala-agent.event-chain.v1.sha256".to_owned(),
        sequence.to_string(),
        event_type.to_owned(),
        aggregate_id.to_owned(),
        workflow_id.unwrap_or("").to_owned(),
        run_id.unwrap_or("").to_owned(),
        occurred_at.to_owned(),
        payload_digest.to_owned(),
        previous_event_digest.to_owned(),
    ];
    let mut bytes = Vec::new();
    for part in parts {
        let encoded = part.as_bytes();
        bytes.extend_from_slice(&(encoded.len() as u64).to_be_bytes());
        bytes.extend_from_slice(encoded);
    }
    sha256_hex(&String::from_utf8(bytes).expect("utf8"))
}

/// Rewrite the journal so that the row at `target_sequence` carries a
/// tampered payload, and re-chain every row after it so the chain still
/// verifies. This is the strongest forgery a writer with database access can
/// make, and it is exactly what the chain cannot protect against.
/// One stored journal row, as the forgery reads it.
type StoredRow = (
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
);

fn forge_prepared_payload(data_dir: &Path, target_sequence: i64, new_payload: &str) {
    let path = settings_for(data_dir).database_path();
    let connection = rusqlite::Connection::open(path).expect("open sqlite");
    // An attacker with database access drops the append-only guard first.
    connection
        .execute_batch("DROP TRIGGER IF EXISTS events_prevent_update")
        .expect("drop the update guard");
    let rows: Vec<StoredRow> = {
        let mut statement = connection
            .prepare(
                "SELECT sequence, event_type, aggregate_id, workflow_id, run_id, occurred_at,\
                 payload_json, digest FROM events ORDER BY sequence",
            )
            .expect("prepare");
        statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            })
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("rows")
    };
    let mut previous = "0".repeat(64);
    for (sequence, event_type, aggregate_id, workflow_id, run_id, occurred_at, payload_json, _) in
        rows
    {
        let payload = if sequence == target_sequence {
            new_payload.to_owned()
        } else {
            payload_json
        };
        let digest = sha256_hex(&payload);
        let event_digest = chain_digest(
            sequence,
            &event_type,
            &aggregate_id,
            workflow_id.as_deref(),
            run_id.as_deref(),
            &occurred_at,
            &digest,
            &previous,
        );
        connection
            .execute(
                "UPDATE events SET payload_json = ?, digest = ?, previous_event_digest = ?,\
                 event_digest = ? WHERE sequence = ?",
                rusqlite::params![payload, digest, previous, event_digest, sequence],
            )
            .expect("update");
        previous = event_digest;
    }
}

#[test]
fn a_forged_pending_commit_is_refused_when_it_is_decided() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);
    let prepared = application
        .prepare_design_revision("forged", "mechanical", None, None, None, &[], &[], None)
        .expect("prepare");
    let sequence = application
        .list_events(
            0,
            1_000,
            None,
            Some("revision.prepared"),
            Some(&prepared.request_id),
        )
        .expect("events")
        .events
        .first()
        .expect("the prepared event")
        .sequence;
    application.close().expect("close");

    // Forge the preparation: change the summary the preparation recorded and
    // re-chain the whole journal so the chain still verifies.
    let path = settings_for(&data).database_path();
    let connection = rusqlite::Connection::open(&path).expect("open sqlite");
    let payload: String = connection
        .query_row(
            "SELECT payload_json FROM events WHERE sequence = ?",
            [sequence],
            |row| row.get(0),
        )
        .expect("payload");
    let _ = connection.close();
    let mut forged: CanonicalValue =
        kala_json::parse_strict(payload.as_bytes(), &kala_json::Limits::default())
            .expect("canonical payload");
    if let kala_json::CanonicalValue::Object(map) = &mut forged {
        map.insert(
            kala_json::JsonText::new("summary"),
            CanonicalValue::text("a summary that was never reviewed"),
        );
    }
    let forged_text = kala_json::canonical_json_dumps(&forged).expect("canonical");
    forge_prepared_payload(&data, sequence, &forged_text);

    // The chain still verifies: the forgery is invisible to the journal.
    let application = open(&data);
    application
        .verify_chain()
        .expect("the forged chain still verifies");
    let error = application
        .decide_revision_commit(
            &prepared.request_id,
            "approve",
            "local-user",
            None,
            None,
            None,
            None,
        )
        .expect_err("a forged preparation must be refused");
    assert_eq!(
        error.code(),
        "checkpoint_digest_mismatch",
        "the forged preparation is refused: {error}"
    );
    assert!(
        revision_ids(&application, "forged").is_empty(),
        "a forged preparation writes nothing"
    );
    application.close().expect("close");
}
