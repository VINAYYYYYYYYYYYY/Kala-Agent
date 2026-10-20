//! Cross-reader compatibility with the Python composition root.
//!
//! Python and Rust must be able to open each other's data directory: the same
//! SQLite schema, the same rows, the same journal chain, the same artifact
//! layout and the same kernel policy/budget state. Each direction is exercised
//! by writing with one implementation, reading and *extending* with the other,
//! and reopening the result.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kala_api::config::{SettingValue, Settings, settings_from_values};
use kala_application::{Components, KalaApplication, View};
use kala_json::{CanonicalValue, JsonText};
use num_bigint::BigInt;

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
        "live Python oracle requires {}",
        venv.display()
    );
    venv
}

fn run_oracle(scenario: &str, data_dir: &Path) -> CanonicalValue {
    std::fs::create_dir_all(data_dir).expect("data dir");
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle_application.py");
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
        .expect("spawn the Python oracle");
    assert!(
        output.status.success(),
        "oracle {scenario} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    kala_json::parse_strict(&output.stdout, &kala_json::Limits::default())
        .expect("canonical oracle JSON")
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

fn view<T: View>(value: &T) -> CanonicalValue {
    value.to_canonical_value().expect("canonical DTO")
}

fn cross_step<'a>(trace: &'a CanonicalValue, name: &str) -> &'a CanonicalValue {
    field(field(trace, "steps"), name)
}

fn string_array(value: &CanonicalValue) -> Vec<String> {
    match value {
        CanonicalValue::Array(items) => items.iter().map(text_of).collect(),
        _ => panic!("array expected"),
    }
}

/// Replace wall-clock timestamps with a placeholder. Stored rows written by
/// either implementation are compared exactly otherwise.
fn normalize_timestamps(value: &CanonicalValue) -> CanonicalValue {
    match value {
        CanonicalValue::Text(text_value) => {
            let text_value = text_value.to_utf8().expect("utf8");
            if looks_like_timestamp(&text_value) {
                CanonicalValue::text("<timestamp>")
            } else {
                CanonicalValue::text(&text_value)
            }
        }
        CanonicalValue::Array(items) => {
            CanonicalValue::Array(items.iter().map(normalize_timestamps).collect())
        }
        CanonicalValue::Object(map) => CanonicalValue::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), normalize_timestamps(item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn looks_like_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && (value.ends_with('Z') || value.ends_with("+00:00"))
}

fn integer(value: i64) -> CanonicalValue {
    CanonicalValue::integer(BigInt::from(value))
}

fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::object(entries)
}

fn text(value: &str) -> CanonicalValue {
    CanonicalValue::text(value)
}

/// Write a data directory from the Rust side.
fn rust_seed(data_dir: &Path) -> CanonicalValue {
    let application = open(data_dir);
    let first = application
        .create_design_revision(
            "cross",
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
        .expect("r0");
    let second = application
        .create_design_revision(
            "cross",
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
        .expect("r1");
    let run = application
        .run_mechanical_reference_workflow(Some("workflow-cross"), "cross", "rs-operator")
        .expect("run");
    let decided = application
        .approve_workflow_checkpoint(
            "workflow-cross",
            "rs-operator",
            run.checkpoint
                .as_ref()
                .map(|value| value.state_digest.as_str()),
            run.checkpoint
                .as_ref()
                .map(|value| value.action_digest.as_str()),
            None,
        )
        .expect("approve");
    assert!(
        application
            .policy()
            .revoke(&kala_contracts::Text::new("design.write"))
            .expect("revoke")
    );
    let pending = application
        .prepare_design_revision(
            "cross-pending",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("pending");
    let grants = application
        .policy()
        .grants()
        .iter()
        .map(|value| value.as_str().unwrap_or("").to_owned())
        .collect::<Vec<_>>();
    let value = object(vec![
        (
            "revision_ids",
            CanonicalValue::array(vec![
                text(first.revision.revision_id()),
                text(second.revision.revision_id()),
            ]),
        ),
        (
            "run_checkpoint",
            run.checkpoint.as_ref().map_or(CanonicalValue::Null, view),
        ),
        ("decision_status", text(decided.status.as_str())),
        (
            "decision_revision",
            decided.revision.as_ref().map_or(CanonicalValue::Null, view),
        ),
        (
            "events",
            integer(application.verify_chain().expect("verified").event_count as i64),
        ),
        (
            "grants",
            CanonicalValue::array(grants.iter().map(|value| text(value)).collect::<Vec<_>>()),
        ),
        (
            "artifact_bytes",
            integer(
                application
                    .budget()
                    .usage()
                    .artifact_bytes()
                    .to_string()
                    .parse::<i64>()
                    .expect("fits"),
            ),
        ),
        ("pending_request", text(&pending.request_id)),
        ("pending_status", text(pending.status.as_str())),
    ]);
    application.close().expect("close");
    object(vec![("steps", value)])
}

/// Compare one stored revision exactly (timestamps aside) across readers.
fn assert_same_revision(left: &CanonicalValue, right: &CanonicalValue) {
    assert_eq!(
        normalize_timestamps(left),
        normalize_timestamps(right),
        "the same stored revision must read back identically"
    );
}

#[test]
fn python_writes_rust_reads_and_extends_and_python_reads_again() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("shared");

    let python_seed = run_oracle("seed", &data);
    let seeded = cross_step(&python_seed, "decision_revision");

    // Rust opens Python's directory, reads its rows and kernel state, and
    // extends the design.
    {
        let application = open(&data);
        assert_eq!(
            application.verify_chain().expect("verified").event_count as i64,
            match cross_step(&python_seed, "events") {
                CanonicalValue::Integer(value) => value.to_string().parse::<i64>().unwrap(),
                _ => panic!("event count"),
            }
        );
        let ids = application
            .list_design_revisions("cross", 1_000)
            .expect("revisions")
            .iter()
            .map(|record| record.revision.revision_id().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["rev-cross-r0", "rev-cross-r1", "rev-cross-r2"]);
        let mut grants = application
            .policy()
            .grants()
            .iter()
            .map(|value| value.as_str().unwrap_or("").to_owned())
            .collect::<Vec<_>>();
        grants.sort();
        assert_eq!(grants, string_array(cross_step(&python_seed, "grants")));
        let pending_request = text_of(cross_step(&python_seed, "pending_request"));
        assert_eq!(
            application
                .get_revision_commit(&pending_request)
                .expect("pending read")
                .status
                .as_str(),
            text_of(cross_step(&python_seed, "pending_status"))
        );
        let record = application
            .get_design_revision("rev-cross-r2")
            .expect("python revision");
        let artifact = record.revision.artifacts()[0].digest().to_hex();
        let bytes = application
            .artifact_store()
            .get_bytes(&artifact)
            .expect("python artifact");
        assert_eq!(
            num_bigint::BigInt::from(bytes.len()),
            *record.revision.artifacts()[0].size_bytes().as_bigint()
        );
        assert!(
            application
                .policy()
                .grant_capability(kala_contracts::Text::new("design.write"))
                .is_ok()
        );
        assert_eq!(
            application.budget().usage().artifact_bytes().to_string(),
            match cross_step(&python_seed, "artifact_bytes") {
                CanonicalValue::Integer(value) => value.to_string(),
                _ => panic!("artifact bytes"),
            }
        );
        let read = application
            .get_design_revision("rev-cross-r2")
            .expect("python revision");
        assert_same_revision(&view(&read), seeded);
        application
            .create_design_revision(
                "cross",
                "mechanical",
                Some("rust cross write"),
                None,
                None,
                &[],
                &[],
                None,
                "rs-cross",
                None,
                None,
            )
            .expect("rust write on a Python directory");
        application.close().expect("close");
    }

    // Python opens the extended directory, reads the Rust row, and extends it.
    let python_inspect = run_oracle("inspect", &data);
    let listed = string_array(cross_step(&python_inspect, "revision_ids"));
    assert_eq!(
        listed,
        vec![
            "rev-cross-r0",
            "rev-cross-r1",
            "rev-cross-r2",
            "rev-cross-r3"
        ]
    );
    assert_eq!(
        cross_step(&python_inspect, "approved_status"),
        &text("completed")
    );
    // Rust re-granted `design.write` before extending the design, so Python
    // sees the seed grants plus that grant.
    let mut expected_grants = string_array(cross_step(&python_seed, "grants"));
    expected_grants.push("design.write".to_owned());
    expected_grants.sort();
    assert_eq!(
        string_array(cross_step(&python_inspect, "grants_before")),
        expected_grants
    );
    assert_eq!(
        text_of(cross_step(&python_inspect, "pending_status")),
        text_of(cross_step(&python_seed, "pending_status"))
    );
    let artifact = cross_step(&python_inspect, "artifact");
    assert_eq!(
        text_of(field(artifact, "digest")),
        text_of(field(artifact, "sha256"))
    );
    let final_python = string_array(cross_step(&python_inspect, "final_ids"));
    assert_eq!(
        final_python,
        vec![
            "rev-cross-r0",
            "rev-cross-r1",
            "rev-cross-r2",
            "rev-cross-r3",
            "rev-cross-r4",
            "rev-cross-r5"
        ]
    );

    // Rust reopens the directory Python extended and reads Python's writes.
    {
        let application = open(&data);
        let ids = application
            .list_design_revisions("cross", 1_000)
            .expect("revisions")
            .iter()
            .map(|record| record.revision.revision_id().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(ids, final_python);
        assert_eq!(
            application.verify_chain().expect("verified").event_count as i64,
            match cross_step(&python_inspect, "events") {
                CanonicalValue::Integer(value) => value.to_string().parse::<i64>().unwrap(),
                _ => panic!("event count"),
            }
        );
        let python_created = cross_step(&python_inspect, "created");
        let read = application
            .get_design_revision("rev-cross-r4")
            .expect("python r4");
        assert_same_revision(&view(&read), python_created);
        let python_approved = cross_step(&python_inspect, "approved_revision");
        let read = application
            .get_design_revision("rev-cross-r5")
            .expect("python r5");
        assert_same_revision(&view(&read), python_approved);
        application.close().expect("close");
    }
}

#[test]
fn rust_writes_python_reads_and_extends_and_rust_reads_again() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("shared");

    let rust_seed = rust_seed(&data);
    let seeded = cross_step(&rust_seed, "decision_revision");

    // Python opens the Rust directory, restores the native policy/budget
    // state, reads the Rust revision and extends the design.
    let python_inspect = run_oracle("inspect", &data);
    let listed = string_array(cross_step(&python_inspect, "revision_ids"));
    assert_eq!(listed, vec!["rev-cross-r0", "rev-cross-r1", "rev-cross-r2"]);
    assert_same_revision(cross_step(&python_inspect, "last_revision"), seeded);
    assert_eq!(
        string_array(cross_step(&python_inspect, "grants_before")),
        string_array(cross_step(&rust_seed, "grants"))
    );
    assert_eq!(
        text_of(cross_step(&python_inspect, "pending_status")),
        text_of(cross_step(&rust_seed, "pending_status"))
    );
    let artifact = cross_step(&python_inspect, "artifact");
    assert_eq!(
        text_of(field(artifact, "digest")),
        text_of(field(artifact, "sha256"))
    );
    let final_python = string_array(cross_step(&python_inspect, "final_ids"));
    assert_eq!(
        final_python,
        vec![
            "rev-cross-r0",
            "rev-cross-r1",
            "rev-cross-r2",
            "rev-cross-r3",
            "rev-cross-r4"
        ]
    );

    // Rust reopens and reads Python's writes.
    {
        let application = open(&data);
        let ids = application
            .list_design_revisions("cross", 1_000)
            .expect("revisions")
            .iter()
            .map(|record| record.revision.revision_id().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(ids, final_python);
        let read = application
            .get_design_revision("rev-cross-r3")
            .expect("python r3");
        assert_same_revision(&view(&read), cross_step(&python_inspect, "created"));
        application.close().expect("close");
    }
}
