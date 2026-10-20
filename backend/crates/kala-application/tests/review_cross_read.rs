//! Review probe 4: cross-read safety.
//!
//! A Rust root must refuse a Python data directory it cannot trust, exactly
//! as the Python root refuses it, and it must never continue silently. The
//! probe covers three ways a foreign data directory can be untrustworthy:
//!
//! * a journal row whose payload was edited (the chain no longer verifies);
//! * a stored artifact whose bytes were truncated (the digest no longer
//!   verifies);
//! * a stored artifact that was removed (the report the preparation was
//!   derived against is gone).
//!
//! In every case the two implementations are compared step for step, and the
//! failed launch is checked not to have written anything over the evidence.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kala_api::config::{SettingValue, Settings, settings_from_values};
use kala_application::{Components, KalaApplication};
use serde_json::Value as Json;

/// Writes a data directory with one pending prepared commit.
const PYTHON_SEED_SCRIPT: &str = r#"
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
prepared = app.prepare_design_revision(
    design_id="cross-artifact", domain="mechanical"
)
if len(sys.argv) > 3 and sys.argv[3] == "commit":
    app.commit_design_revision(prepared.request_id)
app.close()
sys.stdout.write(
    json.dumps(
        {
            "request_id": prepared.request_id,
            "artifact_digest": prepared.artifact_digests[0],
            "artifact_path": str(
                Settings(data_dir=data_dir).artifact_dir
                / "objects"
                / prepared.artifact_digests[0][:2]
                / prepared.artifact_digests[0]
            ),
        }
    )
)
"#;

/// Decides the pending prepared commit in a (possibly tampered) directory.
const PYTHON_DECIDE_SCRIPT: &str = r#"
import json
import sys
from pathlib import Path

repo = Path(sys.argv[1])
sys.path.insert(0, str(repo / "src"))

from kala_agent.application import KalaApplication
from kala_agent.config import Settings

data_dir = Path(sys.argv[2])
request_id = sys.argv[3]

out = {}
try:
    app = KalaApplication(Settings(data_dir=data_dir))
    out["opened"] = True
except Exception as error:  # noqa: BLE001
    out["opened"] = False
    out["open_class"] = type(error).__name__
    out["open_code"] = getattr(error, "code", None)
    out["open_message"] = str(error)
    sys.stdout.write(json.dumps(out))
    raise SystemExit(0)

try:
    decided = app.decide_revision_commit(request_id, "approve")
    out["decided"] = True
    out["committed"] = decided.committed
except Exception as error:  # noqa: BLE001
    out["decided"] = False
    out["class"] = type(error).__name__
    out["code"] = getattr(error, "code", None)
    out["message"] = str(error)
out["revisions"] = [
    item.revision.revision_id for item in app.list_design_revisions("cross-artifact")
]
app.close()
sys.stdout.write(json.dumps(out))
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

fn run_python_script(temp: &Path, name: &str, script: &str, args: &[&str]) -> Json {
    let path = temp.join(name);
    std::fs::write(&path, script).expect("write the probe script");
    let mut command = Command::new(python_executable());
    command
        .arg("-I")
        .arg(&path)
        .arg(repo_root())
        .args(args)
        .env("PYTHONHASHSEED", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = command.output().expect("spawn the Python probe");
    assert!(
        output.status.success(),
        "the Python probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("probe JSON")
}

/// The rows of the workflow snapshot table, for the "nothing was written"
/// check.
fn snapshot_count(data_dir: &Path) -> i64 {
    let path = settings_for(data_dir).database_path();
    let connection = rusqlite::Connection::open(path).expect("open sqlite");
    connection
        .query_row("SELECT COUNT(*) FROM workflow_snapshots", [], |row| {
            row.get(0)
        })
        .expect("count")
}

fn revision_count(data_dir: &Path) -> i64 {
    let path = settings_for(data_dir).database_path();
    let connection = rusqlite::Connection::open(path).expect("open sqlite");
    connection
        .query_row("SELECT COUNT(*) FROM design_revisions", [], |row| {
            row.get(0)
        })
        .expect("count")
}

// ---------------------------------------------------------------------------
// A tampered journal row.
// ---------------------------------------------------------------------------

#[test]
fn a_tampered_journal_row_stops_the_launch_on_both_sides() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python_dir = temp.path().join("python");
    let rust_dir = temp.path().join("rust");

    // Both sides write a data directory with a recorded history.
    let seeded = run_python_script(
        temp.path(),
        "review_cross_seed.py",
        PYTHON_SEED_SCRIPT,
        &[python_dir.to_str().expect("path")],
    );
    let request_id = seeded["request_id"]
        .as_str()
        .expect("request id")
        .to_owned();
    let artifact_digest = seeded["artifact_digest"]
        .as_str()
        .expect("artifact digest")
        .to_owned();

    let rust_application = open(&rust_dir);
    rust_application
        .prepare_design_revision(
            "cross-artifact",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("prepare");
    rust_application.close().expect("close");

    // Edit the payload of the first journal row without repairing the chain.
    for data_dir in [&python_dir, &rust_dir] {
        let path = settings_for(data_dir).database_path();
        let connection = rusqlite::Connection::open(path).expect("open sqlite");
        connection
            .execute_batch("DROP TRIGGER IF EXISTS events_prevent_update")
            .expect("drop the update guard");
        connection
            .execute(
                "UPDATE events SET payload_json = ? WHERE sequence = 1",
                [r#"{"tampered":true}"#],
            )
            .expect("tamper");
        connection.close().expect("close");
    }

    // Rust refuses the launch.
    let rust_error =
        KalaApplication::with_components(settings_for(&rust_dir), Components::default())
            .expect_err("Rust must refuse a tampered journal");
    assert!(
        rust_error.message().contains("journal"),
        "the refusal names the journal: {rust_error}"
    );
    assert_eq!(rust_error.code(), "application_error");

    // Python refuses the same launch.
    let python = run_python_script(
        temp.path(),
        "review_cross_decide.py",
        PYTHON_DECIDE_SCRIPT,
        &[python_dir.to_str().expect("path"), request_id.as_str()],
    );
    assert_eq!(
        python["opened"].as_bool(),
        Some(false),
        "Python must refuse a tampered journal too: {python}"
    );
    assert_eq!(
        python["open_class"].as_str(),
        Some("JournalIntegrityError"),
        "Python reports the journal integrity failure: {python}"
    );

    // The failed launch wrote nothing over the evidence.
    let rust_snapshots = snapshot_count(&rust_dir);
    let python_snapshots = snapshot_count(&python_dir);
    assert!(
        rust_snapshots > 0 && python_snapshots > 0,
        "the evidence is still there"
    );
    let failed = KalaApplication::with_components(settings_for(&rust_dir), Components::default())
        .expect_err("Rust must refuse again");
    assert_eq!(
        snapshot_count(&rust_dir),
        rust_snapshots,
        "a refused launch must not write a fresh kernel state over the evidence"
    );
    assert_eq!(failed.code(), "application_error");
    assert_eq!(revision_count(&rust_dir), 0);
    let _ = artifact_digest;
}

// ---------------------------------------------------------------------------
// A truncated artifact.
// ---------------------------------------------------------------------------

#[test]
fn a_truncated_artifact_is_refused_when_the_prepared_commit_is_decided() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python_dir = temp.path().join("python");
    let rust_dir = temp.path().join("rust");

    let seeded = run_python_script(
        temp.path(),
        "review_cross_seed.py",
        PYTHON_SEED_SCRIPT,
        &[python_dir.to_str().expect("path")],
    );
    let request_id = seeded["request_id"]
        .as_str()
        .expect("request id")
        .to_owned();
    let python_artifact = PathBuf::from(seeded["artifact_path"].as_str().expect("path"));

    let rust_application = open(&rust_dir);
    let rust_prepared = rust_application
        .prepare_design_revision(
            "cross-artifact",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("prepare");
    rust_application.close().expect("close");
    let rust_artifact = settings_for(&rust_dir)
        .artifact_dir()
        .join("objects")
        .join(&rust_prepared.artifact_digests[0][..2])
        .join(&rust_prepared.artifact_digests[0]);

    // Truncate both stored reports.
    for artifact in [&python_artifact, &rust_artifact] {
        let bytes = std::fs::read(artifact).expect("read the artifact");
        std::fs::write(artifact, &bytes[..bytes.len() / 2]).expect("truncate");
    }

    // Rust refuses the decision.
    let rust_application = open(&rust_dir);
    let rust_error = rust_application
        .decide_revision_commit(
            &rust_prepared.request_id,
            "approve",
            "local-user",
            None,
            None,
            None,
            None,
        )
        .expect_err("Rust must refuse a truncated artifact");
    assert_eq!(
        rust_error.message(),
        "artifact digest verification failed",
        "the artifact digest is verified on the way in"
    );
    assert_eq!(rust_error.code(), "application_error");
    assert_eq!(
        revision_count(&rust_dir),
        0,
        "a truncated artifact writes no revision"
    );
    rust_application.close().expect("close");

    // Python refuses the same decision with the same message.
    let python = run_python_script(
        temp.path(),
        "review_cross_decide.py",
        PYTHON_DECIDE_SCRIPT,
        &[python_dir.to_str().expect("path"), request_id.as_str()],
    );
    assert_eq!(python["opened"].as_bool(), Some(true), "{python}");
    assert_eq!(
        python["decided"].as_bool(),
        Some(false),
        "Python must refuse a truncated artifact too: {python}"
    );
    assert_eq!(
        python["message"].as_str(),
        Some("artifact digest verification failed"),
        "the message matches the Rust refusal: {python}"
    );
    assert_eq!(
        python["revisions"].as_array().expect("revisions"),
        &Vec::<Json>::new(),
        "a truncated artifact writes no revision"
    );
}

/// Insert a revision row that no journalled commit backs, so the provenance
/// it names is missing.
fn insert_ghost_revision(data_dir: &Path) {
    let path = settings_for(data_dir).database_path();
    let connection = rusqlite::Connection::open(path).expect("open sqlite");
    let model: String = connection
        .query_row(
            "SELECT model_json FROM design_revisions ORDER BY sequence DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("a stored revision");
    let mut value: Json = serde_json::from_str(&model).expect("model json");
    if let Json::Object(map) = &mut value {
        map.insert(
            "revision_id".to_owned(),
            Json::String("rev-ghost-r0".to_owned()),
        );
        map.insert("design_id".to_owned(), Json::String("ghost".to_owned()));
        map.insert("sequence".to_owned(), Json::from(0));
        map.insert(
            "parent_revision_id".to_owned(),
            Json::String("rev-ghost-parent".to_owned()),
        );
    }
    let model = serde_json::to_string(&value).expect("model json");
    connection
        .execute(
            "INSERT INTO design_revisions(revision_id, design_id, sequence, model_json, created_at)\
             VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![
                "rev-ghost-r0",
                "ghost",
                0_i64,
                model,
                "2031-01-01T00:00:00+00:00"
            ],
        )
        .expect("insert the ghost revision");
}

/// The Python side that reads one revision out of a data directory.
const PYTHON_REVISION_SCRIPT: &str = r#"
import json
import sys
from pathlib import Path

repo = Path(sys.argv[1])
sys.path.insert(0, str(repo / "src"))

from kala_agent.application import KalaApplication
from kala_agent.config import Settings

data_dir = Path(sys.argv[2])
revision_id = sys.argv[3]
out = {}
try:
    app = KalaApplication(Settings(data_dir=data_dir))
    record = app.get_design_revision(revision_id)
    out["ok"] = True
    out["revision_id"] = record.revision.revision_id
except Exception as error:  # noqa: BLE001
    out["ok"] = False
    out["class"] = type(error).__name__
    out["code"] = getattr(error, "code", None)
    out["message"] = str(error)
sys.stdout.write(json.dumps(out))
"#;

#[test]
fn a_revision_without_a_recorded_provenance_is_refused_on_both_sides() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python_dir = temp.path().join("python");
    let rust_dir = temp.path().join("rust");

    let _ = run_python_script(
        temp.path(),
        "review_cross_seed.py",
        PYTHON_SEED_SCRIPT,
        &[python_dir.to_str().expect("path"), "commit"],
    );
    let rust_application = open(&rust_dir);
    let rust_prepared = rust_application
        .prepare_design_revision(
            "cross-artifact",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("prepare");
    rust_application
        .commit_design_revision(
            &rust_prepared.request_id,
            "local-user",
            None,
            None,
            None,
            None,
        )
        .expect("commit");
    rust_application.close().expect("close");

    insert_ghost_revision(&python_dir);
    insert_ghost_revision(&rust_dir);

    let rust_application = open(&rust_dir);
    let rust_error = rust_application
        .get_design_revision("rev-ghost-r0")
        .expect_err("a revision without a recorded provenance must be refused");
    rust_application.close().expect("close");

    let python = run_python_script(
        temp.path(),
        "review_cross_revision.py",
        PYTHON_REVISION_SCRIPT,
        &[python_dir.to_str().expect("path"), "rev-ghost-r0"],
    );
    assert_eq!(
        python["ok"].as_bool(),
        Some(false),
        "Python must refuse the unbacked revision too: {python}"
    );
    assert_eq!(
        rust_error.code(),
        python["code"].as_str().expect("the Python code"),
        "the refusal must carry the Python code"
    );
    assert_eq!(
        rust_error.message(),
        python["message"].as_str().expect("the Python message"),
        "the refusal must carry the Python message"
    );
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
    kala_tools::protocol::sha256_hex_text(&String::from_utf8(bytes).expect("utf8")).to_owned()
}

/// Append one synthetic event whose payload is larger than the bounded event
/// payload size, chained onto the current head so the journal still verifies.
fn append_oversized_event(data_dir: &Path) {
    let path = settings_for(data_dir).database_path();
    let connection = rusqlite::Connection::open(path).expect("open sqlite");
    let (sequence, previous_event_digest, occurred_at): (i64, String, String) = connection
        .query_row(
            "SELECT sequence, event_digest, occurred_at FROM events ORDER BY sequence DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("the journal head");
    let payload = format!("{{\"filler\":\"{}\"}}", "x".repeat(70_000_usize));
    let digest = kala_tools::protocol::sha256_hex_text(&payload).to_owned();
    let event_digest = chain_digest(
        sequence + 1,
        "review.oversized",
        "review-oversized",
        None,
        None,
        &occurred_at,
        &digest,
        &previous_event_digest,
    );
    connection
        .execute(
            "INSERT INTO events(sequence, event_type, aggregate_id, workflow_id, run_id,\
             payload_json, digest, occurred_at, previous_event_digest, event_digest)\
             VALUES (?, ?, ?, NULL, NULL, ?, ?, ?, ?, ?)",
            rusqlite::params![
                sequence + 1,
                "review.oversized",
                "review-oversized",
                payload,
                digest,
                occurred_at,
                previous_event_digest,
                event_digest
            ],
        )
        .expect("append the oversized event");
}

/// The Python side that reads a data directory with an oversized event.
const PYTHON_EVENTS_SCRIPT: &str = r#"
import json
import sys
from pathlib import Path

repo = Path(sys.argv[1])
sys.path.insert(0, str(repo / "src"))

from kala_agent.application import KalaApplication
from kala_agent.config import Settings

data_dir = Path(sys.argv[2])
out = {}
try:
    app = KalaApplication(Settings(data_dir=data_dir))
    page = app.list_events(limit=1000)
    out["ok"] = True
    out["count"] = page.count
except Exception as error:  # noqa: BLE001
    out["ok"] = False
    out["class"] = type(error).__name__
    out["code"] = getattr(error, "code", None)
    out["message"] = str(error)
sys.stdout.write(json.dumps(out))
"#;

#[test]
fn an_oversized_event_payload_is_reported_the_same_way_on_both_sides() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python_dir = temp.path().join("python");
    let rust_dir = temp.path().join("rust");

    let _ = run_python_script(
        temp.path(),
        "review_cross_seed.py",
        PYTHON_SEED_SCRIPT,
        &[python_dir.to_str().expect("path")],
    );
    let rust_application = open(&rust_dir);
    rust_application
        .prepare_design_revision(
            "cross-artifact",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("prepare");
    rust_application.close().expect("close");

    append_oversized_event(&python_dir);
    append_oversized_event(&rust_dir);

    // The chain still verifies on both sides: the oversized payload is a
    // well-formed event, it is only too large to publish.
    let application = open(&rust_dir);
    application
        .verify_chain()
        .expect("the chain still verifies");

    let rust_error = application
        .list_events(0, 1_000, None, None, None)
        .expect_err("an oversized event payload must not be published silently");
    application.close().expect("close");

    let python = run_python_script(
        temp.path(),
        "review_cross_events.py",
        PYTHON_EVENTS_SCRIPT,
        &[python_dir.to_str().expect("path")],
    );

    assert_eq!(
        python["ok"].as_bool(),
        Some(false),
        "Python must refuse the oversized payload too: {python}"
    );

    // KNOWN DIVERGENCE (see REVIEW.md, finding D1): both sides refuse, so the
    // fail-closed property holds, but the Rust refusal wraps the Python
    // message in a prefix and reports the base `application_error` code while
    // Python raises a `JsonEnvelopeError` that carries no code at all. The
    // Python text is preserved inside the Rust text.
    let python_message = python["message"].as_str().expect("the Python message");
    assert!(
        rust_error.message().ends_with(python_message),
        "the Rust refusal must still carry the Python message: {rust_error} against {python_message}"
    );
    assert_ne!(
        rust_error.message(),
        python_message,
        "the divergence this probe records is the prefix, not the text"
    );
    assert_eq!(rust_error.code(), "application_error");
    assert_eq!(
        python["code"].as_str(),
        None,
        "Python raises a non-ApplicationError, so it carries no code"
    );
}

#[test]
fn a_removed_artifact_is_refused_when_the_prepared_commit_is_decided() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python_dir = temp.path().join("python");
    let rust_dir = temp.path().join("rust");

    let seeded = run_python_script(
        temp.path(),
        "review_cross_seed.py",
        PYTHON_SEED_SCRIPT,
        &[python_dir.to_str().expect("path")],
    );
    let request_id = seeded["request_id"]
        .as_str()
        .expect("request id")
        .to_owned();
    let python_artifact = PathBuf::from(seeded["artifact_path"].as_str().expect("path"));

    let rust_application = open(&rust_dir);
    let rust_prepared = rust_application
        .prepare_design_revision(
            "cross-artifact",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("prepare");
    rust_application.close().expect("close");
    let rust_artifact = settings_for(&rust_dir)
        .artifact_dir()
        .join("objects")
        .join(&rust_prepared.artifact_digests[0][..2])
        .join(&rust_prepared.artifact_digests[0]);

    std::fs::remove_file(&python_artifact).expect("remove the Python artifact");
    std::fs::remove_file(&rust_artifact).expect("remove the Rust artifact");

    let rust_application = open(&rust_dir);
    let rust_error = rust_application
        .decide_revision_commit(
            &rust_prepared.request_id,
            "approve",
            "local-user",
            None,
            None,
            None,
            None,
        )
        .expect_err("Rust must refuse a missing artifact");
    assert!(
        rust_error.message().starts_with(
            "the report artifact a revision commit was prepared against is not in the store: "
        ),
        "the refusal names the missing report: {rust_error}"
    );
    assert_eq!(rust_error.code(), "application_error");
    assert_eq!(revision_count(&rust_dir), 0);
    rust_application.close().expect("close");

    let python = run_python_script(
        temp.path(),
        "review_cross_decide.py",
        PYTHON_DECIDE_SCRIPT,
        &[python_dir.to_str().expect("path"), request_id.as_str()],
    );
    assert_eq!(python["opened"].as_bool(), Some(true), "{python}");
    assert_eq!(
        python["decided"].as_bool(),
        Some(false),
        "Python must refuse a missing artifact too: {python}"
    );
    assert!(
        python["message"].as_str().expect("message").starts_with(
            "the report artifact a revision commit was prepared against is not in the store: "
        ),
        "the message matches the Rust refusal: {python}"
    );
}
