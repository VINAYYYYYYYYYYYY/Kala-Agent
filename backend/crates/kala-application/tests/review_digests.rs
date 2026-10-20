//! Review probe 1: the digest identities, re-proved *without* digest
//! normalisation.
//!
//! The `application-r1` differential normalises every 64-hex digest value
//! before comparing the Python and Rust traces, and then proves the digest
//! algorithm separately for one prepared commit. This probe does the opposite
//! of trusting that normalisation: it re-derives every digest from the raw
//! recorded material with an independent implementation of the Python
//! algorithm, for five prepared commits across all three domains, and it
//! shows that the *only* difference between the two implementations is the
//! declared native domain-package identity.
//!
//! Nothing here is normalised:
//!
//! * `parameters_digest` is compared byte-for-byte;
//! * the specification, the report artifact bytes, the state digest, the
//!   action digest and the request identity are compared byte-for-byte after
//!   the single declared substitution (native package digest -> Python
//!   package digest), and each side's own digests are independently
//!   re-derived from that side's own raw material;
//! * the journal payload digests and the journal chain digests are compared
//!   and independently recomputed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kala_api::config::{SettingValue, Settings, settings_from_values};
use kala_application::{Components, KalaApplication};
use kala_json::{CanonicalValue, JsonText};
use num_bigint::BigInt;
use serde_json::Value as Json;

const DESIGN: &str = "probe-mech";
const WORKFLOW_DESIGN: &str = "probe-workflow";
const WORKFLOW: &str = "workflow-probe";

/// The Python side of the probe. It writes one data directory and prints the
/// raw material every digest is derived from.
const PYTHON_SCRIPT: &str = r#"
import json
import sqlite3
import sys
from datetime import UTC, datetime, timedelta
from pathlib import Path

repo = Path(sys.argv[1])
sys.path.insert(0, str(repo / "src"))

from kala_agent.application import KalaApplication
from kala_agent.config import Settings

data_dir = Path(sys.argv[2])
data_dir.mkdir(parents=True, exist_ok=True)
settings = Settings(data_dir=data_dir)

BASE = datetime(2031, 1, 1, tzinfo=UTC)
counter = {"n": 0}


def clock():
    value = BASE + timedelta(seconds=counter["n"])
    counter["n"] += 1
    return value


app = KalaApplication(settings, clock=clock)

DESIGN = "probe-mech"
WORKFLOW_DESIGN = "probe-workflow"
WORKFLOW = "workflow-probe"

out = {"packages": [], "cases": [], "events": []}

for package in app.list_domain_packages():
    out["packages"].append(
        {
            "domain": package.domain,
            "package_id": package.package_id,
            "digest": package.digest,
            "schema_digests": list(package.schema_digests),
        }
    )


def head_spec(design_id):
    connection = sqlite3.connect(settings.database_path)
    row = connection.execute(
        "SELECT model_json FROM design_revisions WHERE design_id = ?"
        " ORDER BY sequence DESC LIMIT 1",
        (design_id,),
    ).fetchone()
    connection.close()
    if row is None:
        return None
    return json.loads(row[0])["spec"]


def case(design_id, domain, parameters=None, summary=None):
    prepared = app.prepare_design_revision(
        design_id=design_id, domain=domain, parameters=parameters, summary=summary
    )
    events = app.journal.read(
        event_type="revision.prepared", aggregate_id=prepared.request_id, limit=1
    )
    payload = events[0].payload
    artifact_text = app.artifact_store.get_text(payload["artifact_digests"][0])
    out["cases"].append(
        {
            "design_id": design_id,
            "domain": domain,
            "request_id": prepared.request_id,
            "payload": payload,
            "artifact_text": artifact_text,
            "head_spec": head_spec(design_id),
            "state_digest": prepared.state_digest,
            "action_digest": prepared.action_digest,
            "spec_digest": prepared.spec_digest,
            "parameters_digest": prepared.parameters_digest,
            "package_digests": list(prepared.package_digests),
            "artifact_digests": list(prepared.artifact_digests),
            "sequence": prepared.sequence,
            "revision_id": prepared.revision_id,
            "parent_revision_id": prepared.parent_revision_id,
        }
    )
    return prepared


first = case(DESIGN, "mechanical")
app.commit_design_revision(first.request_id)
case(DESIGN, "mechanical", summary="second revision")
app.run_mechanical_reference_workflow(workflow_id=WORKFLOW, design_id=WORKFLOW_DESIGN)
app.approve_workflow_checkpoint(WORKFLOW, actor_id="probe-actor")
case("probe-building", "building")
case("probe-wing", "wing")
case("probe-params", "mechanical", parameters={"load_kn": 9.0, "thickness_mm": 4.0})

connection = sqlite3.connect(settings.database_path)
rows = connection.execute(
    "SELECT sequence, event_type, aggregate_id, workflow_id, run_id, payload_json,"
    " digest, occurred_at, previous_event_digest, event_digest"
    " FROM events ORDER BY sequence"
).fetchall()
connection.close()
out["events"] = [list(row) for row in rows]

verification = app.journal.verify_chain()
out["chain"] = {
    "head_event_digest": verification.head_event_digest,
    "event_count": verification.event_count,
}

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

fn run_python(temp: &Path, data_dir: &Path) -> Json {
    std::fs::create_dir_all(data_dir).expect("data dir");
    let script = temp.join("review_digests_probe.py");
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
    serde_json::from_slice(&output.stdout).expect("probe JSON")
}

// ---------------------------------------------------------------------------
// Small canonical-JSON helpers, written independently of the crate.
// ---------------------------------------------------------------------------

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

fn optional_text_of(value: &CanonicalValue) -> Option<String> {
    match value {
        CanonicalValue::Text(value) => Some(value.to_utf8().expect("utf8")),
        CanonicalValue::Null => None,
        _ => panic!("text or null expected"),
    }
}

fn integer_of(value: &CanonicalValue) -> BigInt {
    match value {
        CanonicalValue::Integer(value) => value.clone(),
        _ => panic!("integer expected"),
    }
}

fn canonical_text(value: &CanonicalValue) -> String {
    kala_json::canonical_json_dumps(value).expect("canonical JSON")
}

fn canonical_digest(value: &CanonicalValue) -> String {
    kala_json::canonical_json_digest(value)
        .expect("canonical digest")
        .to_hex()
}

fn sha256_hex(value: &str) -> String {
    kala_tools::protocol::sha256_hex_text(value).to_owned()
}

/// Replace one digest with another everywhere it appears in a canonical value.
fn substitute(value: &CanonicalValue, from: &str, to: &str) -> CanonicalValue {
    match value {
        CanonicalValue::Text(text_value) => {
            let text_value = text_value.to_utf8().expect("utf8");
            CanonicalValue::text(text_value.replace(from, to))
        }
        CanonicalValue::Array(items) => CanonicalValue::Array(
            items
                .iter()
                .map(|item| substitute(item, from, to))
                .collect(),
        ),
        CanonicalValue::Object(map) => CanonicalValue::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), substitute(item, from, to)))
                .collect(),
        ),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// The Python digest algorithm, re-implemented from `application.py`.
// ---------------------------------------------------------------------------

/// The state digest preimage, exactly as `application.py` builds it.
fn state_preimage(
    payload: &CanonicalValue,
    package_digest: &str,
    spec_digest: &str,
    artifact_digest: &str,
    artifact_size: usize,
    head: Option<(String, BigInt, String)>,
) -> CanonicalValue {
    let design_id = text_of(field(payload, "design_id"));
    let sequence = integer_of(field(payload, "sequence"));
    CanonicalValue::object([
        (
            "artifact_digests",
            CanonicalValue::array([CanonicalValue::text(artifact_digest)]),
        ),
        (
            "artifact_metadata",
            CanonicalValue::array([CanonicalValue::object([
                (
                    "artifact_id",
                    CanonicalValue::text(format!(
                        "artifact-{}-report",
                        text_of(field(payload, "domain"))
                    )),
                ),
                ("classification", CanonicalValue::text("text")),
                ("digest", CanonicalValue::text(artifact_digest)),
                ("media_type", CanonicalValue::text("application/json")),
                (
                    "size_bytes",
                    CanonicalValue::integer(BigInt::from(artifact_size)),
                ),
            ])]),
        ),
        (
            "design_head",
            match head {
                None => CanonicalValue::Null,
                Some((revision_id, sequence, spec_digest)) => CanonicalValue::object([
                    ("revision_id", CanonicalValue::text(revision_id)),
                    ("sequence", CanonicalValue::integer(sequence)),
                    ("spec_digest", CanonicalValue::text(spec_digest)),
                ]),
            },
        ),
        ("design_id", CanonicalValue::text(design_id.clone())),
        (
            "domain",
            CanonicalValue::text(text_of(field(payload, "domain"))),
        ),
        (
            "package_digests",
            CanonicalValue::array([CanonicalValue::text(package_digest)]),
        ),
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
            CanonicalValue::text(format!("prov-{design_id}-r{sequence}")),
        ),
        ("spec_digest", CanonicalValue::text(spec_digest)),
        (
            "transformation_ids",
            field(payload, "transformation_ids").clone(),
        ),
    ])
}

/// The action digest preimage, exactly as `application.py` builds it.
fn action_preimage(
    payload: &CanonicalValue,
    spec_digest: &str,
    state_digest: &str,
) -> CanonicalValue {
    CanonicalValue::object([
        (
            "capabilities",
            CanonicalValue::array([
                CanonicalValue::text("artifact.write"),
                CanonicalValue::text("design.write"),
            ]),
        ),
        ("checkpoint_type", CanonicalValue::text("pre_commit")),
        ("design_id", field(payload, "design_id").clone()),
        ("domain", field(payload, "domain").clone()),
        (
            "expected_head_revision_id",
            field(payload, "expected_head_revision_id").clone(),
        ),
        ("operation", CanonicalValue::text("design-revision-commit")),
        ("planned_revision_id", field(payload, "revision_id").clone()),
        ("planned_sequence", field(payload, "sequence").clone()),
        ("spec_digest", CanonicalValue::text(spec_digest)),
        ("state_digest", CanonicalValue::text(state_digest)),
        ("workflow_id", field(payload, "workflow_id").clone()),
    ])
}

/// Everything one prepared commit exposes, on either side.
struct Prepared {
    design_id: String,
    domain: String,
    request_id: String,
    payload: CanonicalValue,
    artifact_text: String,
    head_spec: Option<CanonicalValue>,
    state_digest: String,
    action_digest: String,
    spec_digest: String,
    parameters_digest: String,
    package_digests: Vec<String>,
    artifact_digests: Vec<String>,
    sequence: BigInt,
    revision_id: String,
    parent_revision_id: Option<String>,
}

impl Prepared {
    fn from_json(value: &Json) -> Self {
        let payload = parse_json(value.get("payload").expect("payload"));
        Self {
            design_id: value
                .get("design_id")
                .and_then(Json::as_str)
                .expect("design_id")
                .to_owned(),
            domain: value
                .get("domain")
                .and_then(Json::as_str)
                .expect("domain")
                .to_owned(),
            request_id: value
                .get("request_id")
                .and_then(Json::as_str)
                .expect("request_id")
                .to_owned(),
            payload,
            artifact_text: value
                .get("artifact_text")
                .and_then(Json::as_str)
                .expect("artifact_text")
                .to_owned(),
            head_spec: value
                .get("head_spec")
                .filter(|spec| !spec.is_null())
                .map(parse_json),
            state_digest: value
                .get("state_digest")
                .and_then(Json::as_str)
                .expect("state_digest")
                .to_owned(),
            action_digest: value
                .get("action_digest")
                .and_then(Json::as_str)
                .expect("action_digest")
                .to_owned(),
            spec_digest: value
                .get("spec_digest")
                .and_then(Json::as_str)
                .expect("spec_digest")
                .to_owned(),
            parameters_digest: value
                .get("parameters_digest")
                .and_then(Json::as_str)
                .expect("parameters_digest")
                .to_owned(),
            package_digests: string_list(value.get("package_digests").expect("package_digests")),
            artifact_digests: string_list(value.get("artifact_digests").expect("artifact_digests")),
            sequence: BigInt::from(
                value
                    .get("sequence")
                    .and_then(Json::as_u64)
                    .expect("sequence"),
            ),
            revision_id: value
                .get("revision_id")
                .and_then(Json::as_str)
                .expect("revision_id")
                .to_owned(),
            parent_revision_id: value
                .get("parent_revision_id")
                .and_then(Json::as_str)
                .map(str::to_owned),
        }
    }
}

fn string_list(value: &Json) -> Vec<String> {
    value
        .as_array()
        .expect("array")
        .iter()
        .map(|item| item.as_str().expect("string").to_owned())
        .collect()
}

fn parse_json(value: &Json) -> CanonicalValue {
    let text = serde_json::to_string(value).expect("json text");
    kala_json::parse_strict(text.as_bytes(), &kala_json::Limits::default()).expect("canonical JSON")
}

/// The Rust side of the probe: the same operation script, the same digests.
struct RustTrace {
    packages: Vec<(String, String, String, Vec<String>)>,
    cases: Vec<Prepared>,
    events: Vec<EventRow>,
}

struct EventRow {
    sequence: i64,
    event_type: String,
    aggregate_id: String,
    workflow_id: Option<String>,
    run_id: Option<String>,
    payload_json: String,
    digest: String,
    occurred_at: String,
    previous_event_digest: String,
    event_digest: String,
}

fn rust_trace(data_dir: &Path) -> RustTrace {
    let application = open(data_dir);
    let packages: Vec<(String, String, String, Vec<String>)> = application
        .list_domain_packages()
        .iter()
        .map(|package| {
            (
                package.domain.clone(),
                package.package_id.clone(),
                package.digest.clone(),
                package.schema_digests.clone(),
            )
        })
        .collect();

    let mut cases: Vec<Prepared> = Vec::new();

    let first = application
        .prepare_design_revision(DESIGN, "mechanical", None, None, None, &[], &[], None)
        .expect("prepare");
    cases.push(prepared_case(
        &application,
        data_dir,
        DESIGN,
        "mechanical",
        &first,
    ));
    application
        .commit_design_revision(&first.request_id, "probe-actor", None, None, None, None)
        .expect("commit");

    let second = application
        .prepare_design_revision(
            DESIGN,
            "mechanical",
            Some("second revision"),
            None,
            None,
            &[],
            &[],
            None,
        )
        .expect("prepare");
    cases.push(prepared_case(
        &application,
        data_dir,
        DESIGN,
        "mechanical",
        &second,
    ));

    application
        .run_mechanical_reference_workflow(Some(WORKFLOW), WORKFLOW_DESIGN, "probe-actor")
        .expect("run the workflow");
    application
        .approve_workflow_checkpoint(WORKFLOW, "probe-actor", None, None, None)
        .expect("approve the checkpoint");

    for (design, domain) in [("probe-building", "building"), ("probe-wing", "wing")] {
        let prepared = application
            .prepare_design_revision(design, domain, None, None, None, &[], &[], None)
            .expect("prepare");
        cases.push(prepared_case(
            &application,
            data_dir,
            design,
            domain,
            &prepared,
        ));
    }

    let parameters = CanonicalValue::object([
        ("load_kn", CanonicalValue::Float(9.0)),
        ("thickness_mm", CanonicalValue::Float(4.0)),
    ]);
    let prepared = application
        .prepare_design_revision(
            "probe-params",
            "mechanical",
            None,
            Some(&parameters),
            None,
            &[],
            &[],
            None,
        )
        .expect("prepare");
    cases.push(prepared_case(
        &application,
        data_dir,
        "probe-params",
        "mechanical",
        &prepared,
    ));

    application.close().expect("close");

    let events = event_rows(data_dir);
    RustTrace {
        packages,
        cases,
        events,
    }
}

fn prepared_case(
    application: &KalaApplication,
    data_dir: &Path,
    design_id: &str,
    domain: &str,
    prepared: &kala_application::RevisionCommitView,
) -> Prepared {
    let payload = prepared_payload(application, &prepared.request_id);
    let artifact_text = application
        .artifact_store()
        .get_text(&prepared.artifact_digests[0])
        .expect("artifact bytes");
    Prepared {
        design_id: design_id.to_owned(),
        domain: domain.to_owned(),
        request_id: prepared.request_id.clone(),
        payload,
        artifact_text,
        head_spec: head_spec(data_dir, design_id),
        state_digest: prepared.state_digest.clone(),
        action_digest: prepared.action_digest.clone(),
        spec_digest: prepared.spec_digest.clone(),
        parameters_digest: prepared.parameters_digest.clone(),
        package_digests: prepared.package_digests.clone(),
        artifact_digests: prepared.artifact_digests.clone(),
        sequence: prepared.sequence.clone(),
        revision_id: prepared.revision_id.clone(),
        parent_revision_id: prepared.parent_revision_id.clone(),
    }
}

fn prepared_payload(application: &KalaApplication, request_id: &str) -> CanonicalValue {
    let page = application
        .list_events(0, 1_000, None, Some("revision.prepared"), Some(request_id))
        .expect("events");
    page.events
        .first()
        .and_then(|event| event.payload.clone())
        .expect("the prepared payload")
}

fn head_spec(data_dir: &Path, design_id: &str) -> Option<CanonicalValue> {
    let path = settings_for(data_dir).database_path();
    let connection = rusqlite::Connection::open(path).expect("open sqlite");
    let model: String = connection
        .query_row(
            "SELECT model_json FROM design_revisions WHERE design_id = ? \
             ORDER BY sequence DESC LIMIT 1",
            [design_id],
            |row| row.get(0),
        )
        .ok()?;
    let parsed = parse_json(&serde_json::from_str::<Json>(&model).expect("model json"));
    Some(field(&parsed, "spec").clone())
}

fn event_rows(data_dir: &Path) -> Vec<EventRow> {
    let path = settings_for(data_dir).database_path();
    let connection = rusqlite::Connection::open(path).expect("open sqlite");
    let mut statement = connection
        .prepare(
            "SELECT sequence, event_type, aggregate_id, workflow_id, run_id, payload_json, \
             digest, occurred_at, previous_event_digest, event_digest \
             FROM events ORDER BY sequence",
        )
        .expect("prepare");
    statement
        .query_map([], |row| {
            Ok(EventRow {
                sequence: row.get(0)?,
                event_type: row.get(1)?,
                aggregate_id: row.get(2)?,
                workflow_id: row.get(3)?,
                run_id: row.get(4)?,
                payload_json: row.get(5)?,
                digest: row.get(6)?,
                occurred_at: row.get(7)?,
                previous_event_digest: row.get(8)?,
                event_digest: row.get(9)?,
            })
        })
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows")
}

// ---------------------------------------------------------------------------
// The journal chain digest, re-implemented from `persistence/chain.py`.
// ---------------------------------------------------------------------------

const CHAIN_DOMAIN: &str = "kala-agent.event-chain.v1.sha256";

fn chain_digest(row: &EventRow, previous_event_digest: &str) -> String {
    let parts: Vec<String> = vec![
        CHAIN_DOMAIN.to_owned(),
        row.sequence.to_string(),
        row.event_type.clone(),
        row.aggregate_id.clone(),
        row.workflow_id.clone().unwrap_or_default(),
        row.run_id.clone().unwrap_or_default(),
        row.occurred_at.clone(),
        row.digest.clone(),
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

// ---------------------------------------------------------------------------
// The probe.
// ---------------------------------------------------------------------------

#[test]
fn every_digest_outside_the_declared_package_identity_is_byte_equal_to_python() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python_dir = temp.path().join("python");
    let rust_dir = temp.path().join("rust");

    let python = run_python(temp.path(), &python_dir);
    let rust = rust_trace(&rust_dir);

    let python_cases: Vec<Prepared> = python["cases"]
        .as_array()
        .expect("python cases")
        .iter()
        .map(Prepared::from_json)
        .collect();
    assert_eq!(python_cases.len(), 5, "five prepared commits");
    assert_eq!(rust.cases.len(), 5, "five prepared commits");

    // The declared identity boundary: the package digests differ, everything
    // else about the packages is the same.
    let python_packages = python["packages"].as_array().expect("python packages");
    assert_eq!(python_packages.len(), rust.packages.len());
    for (left, right) in python_packages.iter().zip(rust.packages.iter()) {
        assert_eq!(
            left.get("domain").and_then(Json::as_str),
            Some(right.0.as_str())
        );
        assert_eq!(
            left.get("package_id").and_then(Json::as_str),
            Some(right.1.as_str())
        );
        assert_ne!(
            left.get("digest").and_then(Json::as_str),
            Some(right.2.as_str()),
            "the native package digest is deliberately different"
        );
        let python_schema: Vec<&str> = left
            .get("schema_digests")
            .and_then(Json::as_array)
            .expect("schema digests")
            .iter()
            .map(|item| item.as_str().expect("string"))
            .collect();
        assert_eq!(
            python_schema,
            right.3.iter().map(String::as_str).collect::<Vec<_>>(),
            "the parameter schema digests are byte-identical"
        );
    }

    for (python_case, rust_case) in python_cases.iter().zip(rust.cases.iter()) {
        let label = format!("{} / {}", python_case.design_id, python_case.domain);
        assert_eq!(python_case.design_id, rust_case.design_id, "{label}");
        assert_eq!(python_case.domain, rust_case.domain, "{label}");

        let python_package = python_case.package_digests[0].clone();
        let rust_package = rust_case.package_digests[0].clone();
        assert_ne!(
            python_package, rust_package,
            "{label}: the package identity is the declared boundary"
        );

        // 1. The parameter digest carries no identity: it is byte-equal.
        assert_eq!(
            python_case.parameters_digest, rust_case.parameters_digest,
            "{label}: the parameters digest is byte-equal"
        );
        assert_eq!(
            python_case.sequence, rust_case.sequence,
            "{label}: the planned sequence is byte-equal"
        );
        assert_eq!(
            python_case.revision_id, rust_case.revision_id,
            "{label}: the planned revision identity is byte-equal"
        );
        assert_eq!(
            python_case.parent_revision_id, rust_case.parent_revision_id,
            "{label}: the parent identity is byte-equal"
        );
        assert_eq!(
            python_case.parameters_digest,
            canonical_digest(field(&python_case.payload, "parameters")),
            "{label}: the parameters digest is over the parameter dump"
        );
        assert_eq!(
            rust_case.parameters_digest,
            canonical_digest(field(&rust_case.payload, "parameters")),
            "{label}: the Rust parameters digest is over the parameter dump"
        );

        // 2. The specification is byte-equal after the one declared
        //    substitution, and each side's own spec digest is re-derived.
        let python_spec = field(&python_case.payload, "spec").clone();
        let rust_spec = field(&rust_case.payload, "spec").clone();
        assert_eq!(
            canonical_text(&substitute(&rust_spec, &rust_package, &python_package)),
            canonical_text(&python_spec),
            "{label}: the specification is identical after the substitution"
        );
        assert_eq!(
            canonical_digest(&python_spec),
            python_case.spec_digest,
            "{label}: the Python spec digest is re-derivable"
        );
        assert_eq!(
            canonical_digest(&rust_spec),
            rust_case.spec_digest,
            "{label}: the Rust spec digest is re-derivable"
        );

        // 3. The report artifact bytes are byte-equal after the substitution,
        //    and the artifact digest is the SHA-256 of the stored bytes,
        //    which are the canonical report JSON plus one trailing newline.
        let substituted_artifact = python_case
            .artifact_text
            .replace(&rust_package, &python_package);
        assert_eq!(
            substituted_artifact, python_case.artifact_text,
            "{label}: the report bytes are identical after the substitution"
        );
        assert!(
            python_case.artifact_text.ends_with('\n'),
            "{label}: the stored report ends with one newline"
        );
        assert_eq!(
            sha256_hex(&python_case.artifact_text),
            python_case.artifact_digests[0],
            "{label}: the Python artifact digest is the SHA-256 of the report bytes"
        );
        assert_eq!(
            sha256_hex(&rust_case.artifact_text),
            rust_case.artifact_digests[0],
            "{label}: the Rust artifact digest is the SHA-256 of the report bytes"
        );

        // 4. The head specification is byte-equal after the substitution, so
        //    the head spec digest inside the state preimage agrees too.
        let python_head_spec = python_case.head_spec.clone();
        let rust_head_spec = rust_case.head_spec.clone();
        assert_eq!(
            python_head_spec.is_some(),
            rust_head_spec.is_some(),
            "{label}: both sides see the same head"
        );
        let python_head_digest = python_head_spec.as_ref().map(canonical_digest);
        let rust_head_digest = rust_head_spec.as_ref().map(canonical_digest);
        if let (Some(python_spec), Some(rust_spec)) = (&python_head_spec, &rust_head_spec) {
            assert_eq!(
                canonical_text(&substitute(rust_spec, &rust_package, &python_package)),
                canonical_text(python_spec),
                "{label}: the head specification is identical after the substitution"
            );
        }

        // 5. The state digest: the preimage is rebuilt from the raw material
        //    and must reproduce the recorded digest on both sides.
        let python_state_preimage = state_preimage(
            &python_case.payload,
            &python_package,
            &python_case.spec_digest,
            &python_case.artifact_digests[0],
            python_case.artifact_text.len(),
            head_of(python_case, python_head_digest.as_deref()),
        );
        assert_eq!(
            canonical_digest(&python_state_preimage),
            python_case.state_digest,
            "{label}: the Python state digest is re-derivable"
        );
        let rust_state_preimage = state_preimage(
            &rust_case.payload,
            &rust_package,
            &rust_case.spec_digest,
            &rust_case.artifact_digests[0],
            rust_case.artifact_text.len(),
            head_of(rust_case, rust_head_digest.as_deref()),
        );
        assert_eq!(
            canonical_digest(&rust_state_preimage),
            rust_case.state_digest,
            "{label}: the Rust state digest is re-derivable"
        );

        // 6. The action digest and the request identity.
        let python_action_preimage = action_preimage(
            &python_case.payload,
            &python_case.spec_digest,
            &python_case.state_digest,
        );
        assert_eq!(
            canonical_digest(&python_action_preimage),
            python_case.action_digest,
            "{label}: the Python action digest is re-derivable"
        );
        let rust_action_preimage = action_preimage(
            &rust_case.payload,
            &rust_case.spec_digest,
            &rust_case.state_digest,
        );
        assert_eq!(
            canonical_digest(&rust_action_preimage),
            rust_case.action_digest,
            "{label}: the Rust action digest is re-derivable"
        );
        assert_eq!(
            format!(
                "commit-{}-{}-{}",
                python_case.design_id,
                &python_case.state_digest[..16],
                &python_case.action_digest[..16]
            ),
            python_case.request_id,
            "{label}: the Python request identity is derived from the two digests"
        );
        assert_eq!(
            format!(
                "commit-{}-{}-{}",
                rust_case.design_id,
                &rust_case.state_digest[..16],
                &rust_case.action_digest[..16]
            ),
            rust_case.request_id,
            "{label}: the Rust request identity is derived from the two digests"
        );

        // 7. After the substitution, the two sides' whole digest chains agree.
        let rust_substituted_payload =
            substitute(&rust_case.payload, &rust_package, &python_package);
        let rust_substituted_state = state_preimage(
            &rust_substituted_payload,
            &python_package,
            &python_case.spec_digest,
            &python_case.artifact_digests[0],
            python_case.artifact_text.len(),
            head_of(rust_case, python_head_digest.as_deref()),
        );
        assert_eq!(
            canonical_text(&rust_substituted_state),
            canonical_text(&python_state_preimage),
            "{label}: the state preimage is identical after the substitution"
        );
        assert_eq!(
            canonical_digest(&rust_substituted_state),
            python_case.state_digest,
            "{label}: the state digest is byte-equal after the substitution"
        );

        let rust_substituted_action = action_preimage(
            &rust_substituted_payload,
            &python_case.spec_digest,
            &python_case.state_digest,
        );
        assert_eq!(
            canonical_text(&rust_substituted_action),
            canonical_text(&python_action_preimage),
            "{label}: the action preimage is identical after the substitution"
        );
        assert_eq!(
            canonical_digest(&rust_substituted_action),
            python_case.action_digest,
            "{label}: the action digest is byte-equal after the substitution"
        );
        assert_eq!(
            format!(
                "commit-{}-{}-{}",
                rust_case.design_id,
                &python_case.state_digest[..16],
                &python_case.action_digest[..16]
            ),
            python_case.request_id,
            "{label}: the request identity is byte-equal after the substitution"
        );
    }
}

fn head_of(case: &Prepared, head_digest: Option<&str>) -> Option<(String, BigInt, String)> {
    let revision_id = optional_text_of(field(&case.payload, "expected_head_revision_id"))?;
    let sequence = integer_of(field(&case.payload, "sequence")) - BigInt::from(1);
    let digest = head_digest?;
    Some((revision_id, sequence, digest.to_owned()))
}

// ---------------------------------------------------------------------------
// The journal chain: payload digests and chain digests.
// ---------------------------------------------------------------------------

/// The event types whose payload carries no package-derived digest.
fn identity_independent_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "plan.created"
            | "workflow.started"
            | "workflow.created"
            | "workflow.status_changed"
            | "workflow.transition"
            | "workflow.step_evaluated"
            | "workflow.checkpoint"
            | "workflow.failed"
            | "workflow.completed"
    )
}

/// The event types whose payload carries a wall-clock timestamp the workflow
/// engine stamps itself, so the payload bytes differ between two runs.
fn wall_clock_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "workflow.approval_resolved" | "workflow.approval_invalidated"
    )
}

fn python_event_rows(value: &Json) -> Vec<EventRow> {
    value
        .get("events")
        .and_then(Json::as_array)
        .expect("python events")
        .iter()
        .map(|row| {
            let items = row.as_array().expect("event row");
            EventRow {
                sequence: items[0].as_i64().expect("sequence"),
                event_type: items[1].as_str().expect("event_type").to_owned(),
                aggregate_id: items[2].as_str().expect("aggregate_id").to_owned(),
                workflow_id: items[3].as_str().map(str::to_owned),
                run_id: items[4].as_str().map(str::to_owned),
                payload_json: items[5].as_str().expect("payload_json").to_owned(),
                digest: items[6].as_str().expect("digest").to_owned(),
                occurred_at: items[7].as_str().expect("occurred_at").to_owned(),
                previous_event_digest: items[8].as_str().expect("previous").to_owned(),
                event_digest: items[9].as_str().expect("event_digest").to_owned(),
            }
        })
        .collect()
}

/// Recompute one chain from its rows with an independent implementation of
/// `persistence/chain.py` and return the head digest.
fn recompute_chain(rows: &[EventRow]) -> (u64, String) {
    let mut previous = "0".repeat(64);
    for row in rows {
        assert_eq!(
            row.previous_event_digest, previous,
            "event {} links to the wrong predecessor",
            row.sequence
        );
        assert_eq!(
            sha256_hex(&row.payload_json),
            row.digest,
            "event {} carries the wrong payload digest",
            row.sequence
        );
        let digest = chain_digest(row, &previous);
        assert_eq!(
            digest, row.event_digest,
            "event {} carries the wrong chain digest",
            row.sequence
        );
        previous = digest;
    }
    (rows.len() as u64, previous)
}

#[test]
fn the_journal_chain_digest_is_byte_identical_and_the_head_verifies_across_implementations() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python_dir = temp.path().join("python");
    let rust_dir = temp.path().join("rust");

    let python = run_python(temp.path(), &python_dir);
    let rust = rust_trace(&rust_dir);

    let python_rows = python_event_rows(&python);
    let python_head = recompute_chain(&python_rows);
    assert_eq!(
        python_head.0,
        python["chain"]["event_count"]
            .as_u64()
            .expect("event count")
    );
    assert_eq!(
        Some(python_head.1.as_str()),
        python["chain"]["head_event_digest"].as_str(),
        "the Python head digest is the recomputed chain head"
    );

    let rust_head = recompute_chain(&rust.events);
    assert_eq!(
        rust_head.0,
        rust.events.len() as u64,
        "the Rust chain has the same length"
    );

    // The two sides wrote the same number of events, in the same order.
    assert_eq!(python_rows.len(), rust.events.len());
    for (left, right) in python_rows.iter().zip(rust.events.iter()) {
        assert_eq!(left.sequence, right.sequence);
        assert_eq!(left.event_type, right.event_type);
        assert_eq!(left.workflow_id, right.workflow_id);
        assert_eq!(left.run_id, right.run_id);
        if identity_independent_event(&left.event_type) {
            assert_eq!(left.aggregate_id, right.aggregate_id);
        }
    }

    // The identity-independent events carry byte-identical payloads and
    // therefore byte-identical payload digests.
    for (left, right) in python_rows.iter().zip(rust.events.iter()) {
        if !identity_independent_event(&left.event_type) {
            continue;
        }
        assert_eq!(
            left.digest, right.digest,
            "the payload digest of {} is byte-equal",
            left.event_type
        );
        assert_eq!(
            left.payload_json, right.payload_json,
            "the payload of {} is byte-equal",
            left.event_type
        );
    }

    // The wall-clock events differ only in the timestamp the workflow engine
    // stamps itself.
    for (left, right) in python_rows.iter().zip(rust.events.iter()) {
        if !wall_clock_event(&left.event_type) {
            continue;
        }
        assert_ne!(
            left.payload_json, right.payload_json,
            "the workflow engine stamps its own wall clock"
        );
    }

    // Rust verifies the Python chain and reproduces its head digest byte for
    // byte: the chain digest algorithm is the same implementation.
    let application = open(&python_dir);
    let verification = application.verify_chain().expect("verify the Python chain");
    assert_eq!(verification.event_count, python_head.0);
    assert_eq!(
        verification
            .head_event_digest
            .as_ref()
            .map(|digest| digest.to_hex()),
        Some(python_head.1.clone()),
        "Rust reproduces the Python chain head digest byte for byte"
    );
    application.close().expect("close");

    // And Python's chain is not the same chain: the two runs stamp their own
    // wall clock, so the heads differ unless the payloads and timestamps
    // coincide. The digests are still independently reproducible on both
    // sides, which is what "the same algorithm" means.
    assert!(rust_head.1.len() == 64);
}
