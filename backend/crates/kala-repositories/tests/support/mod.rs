//! Shared integration-test helpers.
//!
//! Python is spawned only by cross-reader tests. The library itself never
//! invokes Python.

#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kala_contracts::{ContractModel, Timestamp};
use kala_repositories::{
    ApprovalRequest, DesignRevision, DesignSpec, MemoryRecord, ToolCall, ToolResult,
};
use serde_json::Value;
use tempfile::TempDir;

pub fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

pub fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/v1")
}

pub fn python_executable() -> PathBuf {
    if let Ok(path) = std::env::var("KALA_REPOSITORIES_PYTHON") {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var("KALA_PYTHON") {
        return PathBuf::from(path);
    }
    let venv = repository_root().join(".venv/bin/python");
    if venv.is_file() {
        return venv;
    }
    let python3 = Path::new("python3");
    if Command::new(python3)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        return python3.to_path_buf();
    }
    panic!(
        "no Python oracle available; set KALA_REPOSITORIES_PYTHON to the repository virtual environment"
    );
}

pub fn oracle_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/repositories_oracle.py")
}

pub fn run_oracle(args: &[&str]) -> Value {
    let output = Command::new(python_executable())
        .arg(oracle_script())
        .args(args)
        .env("PYTHONPATH", repository_root().join("src"))
        .output()
        .expect("spawn Python oracle");
    assert!(
        output.status.success(),
        "Python oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("oracle JSON: {error}; output={:?}", output.stdout))
}

pub fn run_oracle_with_input(args: &[&str], input: &[u8]) -> Value {
    let mut child = Command::new(python_executable())
        .arg(oracle_script())
        .args(args)
        .env("PYTHONPATH", repository_root().join("src"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn Python oracle");
    child
        .stdin
        .as_mut()
        .expect("oracle stdin")
        .write_all(input)
        .expect("write oracle input");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for oracle");
    assert!(
        output.status.success(),
        "Python oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("oracle JSON: {error}; output={:?}", output.stdout))
}

pub fn model<T: ContractModel>(source: &str) -> T {
    T::from_json_str(source).unwrap_or_else(|error| panic!("model JSON: {error}\n{source}"))
}

pub fn timestamp(source: &str) -> Timestamp {
    Timestamp::parse(source).unwrap_or_else(|error| panic!("timestamp {source}: {error}"))
}

pub fn design_spec(id: &str) -> DesignSpec {
    model(&format!(
        r#"{{"id":"{id}","name":"Fixture design","summary":"A persisted design.","parameter_schema_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","attributes_json":"{{\"z\":2,\"a\":1}}","requirement_ids":["requirement-1"],"constraints":["Remain local-first."]}}"#
    ))
}

pub fn design_revision(
    revision_id: &str,
    design_id: &str,
    sequence: u64,
    spec: &DesignSpec,
) -> DesignRevision {
    model(&format!(
        r#"{{"revision_id":"{revision_id}","design_id":"{design_id}","sequence":{sequence},"spec":{spec_json},"parent_revision_id":null,"artifacts":[],"domain_packages":[],"provenance_id":"provenance-1","created_at":"2026-01-02T03:04:05Z"}}"#,
        spec_json = spec.to_wire_json().expect("spec wire"),
    ))
}

pub fn approval(request_id: &str, status: &str) -> ApprovalRequest {
    if status == "pending" {
        model(&format!(
            r#"{{"request_id":"{request_id}","checkpoint_type":"egress","status":"pending","workflow_id":"workflow-1","step_id":null,"action_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","options":["approve","deny"],"selected_option_id":null,"requested_capabilities":[],"artifact_ids":[],"created_at":"2026-01-02T03:04:05Z","decided_at":null,"actor_id":null,"modification_digest":null}}"#
        ))
    } else {
        model(&format!(
            r#"{{"request_id":"{request_id}","checkpoint_type":"egress","status":"{status}","workflow_id":"workflow-1","step_id":null,"action_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","options":["approve","deny"],"selected_option_id":"approve","requested_capabilities":[],"artifact_ids":[],"created_at":"2026-01-02T03:04:05Z","decided_at":"2026-01-02T03:05:05Z","actor_id":"reviewer-1","modification_digest":null}}"#
        ))
    }
}

pub fn tool_call(call_id: &str, tool_id: &str) -> ToolCall {
    model(&format!(
        r#"{{"call_id":"{call_id}","tool_id":"{tool_id}","tool_version":"1.0.0","arguments_json":"{{\"value\":2,\"enabled\":true}}","requested_capabilities":["compute.local"],"side_effect_level":"read","timeout_seconds":30.0,"issued_at":"2026-01-02T03:04:05Z"}}"#
    ))
}

pub fn tool_result(call_id: &str, tool_id: &str) -> ToolResult {
    model(&format!(
        r#"{{"call_id":"{call_id}","tool_id":"{tool_id}","status":"succeeded","result_json":"{{\"value\":4}}","error_code":null,"error_message":null,"artifacts":[],"started_at":"2026-01-02T03:04:05Z","finished_at":"2026-01-02T03:04:05.025000Z","duration_ms":25.0}}"#
    ))
}

pub fn memory_record(memory_id: &str, source: &str) -> MemoryRecord {
    model(&format!(
        r#"{{"memory_id":"{memory_id}","content":"Remember the local database location.","source":"{source}","provenance_id":"provenance-1","metadata_json":"{{\"priority\":\"high\",\"labels\":[\"local\",\"db\"]}}","created_at":"2026-01-02T03:04:05Z","expires_at":"2026-01-02T04:04:05Z"}}"#
    ))
}

pub fn memory_at(
    memory_id: &str,
    source: &str,
    created_at: &Timestamp,
    expires_at: Option<&Timestamp>,
) -> MemoryRecord {
    let expires = expires_at.map_or_else(
        || "null".to_owned(),
        |value| format!("\"{}\"", value.to_wire_string()),
    );
    model(&format!(
        r#"{{"memory_id":"{memory_id}","content":"Remember the local database location.","source":"{source}","provenance_id":"provenance-1","metadata_json":"{{\"priority\":\"high\"}}","created_at":"{created}","expires_at":{expires}}}"#,
        created = created_at.to_wire_string(),
    ))
}

pub struct FixtureCopy {
    pub directory: TempDir,
    pub path: PathBuf,
}

pub fn copy_fixture_database() -> FixtureCopy {
    let directory = TempDir::new().expect("temp dir");
    let path = directory.path().join("schema_v2.sqlite3");
    let source = fixture_dir().join("schema_v2.sqlite3");
    fs::copy(&source, &path).expect("copy fixture database");
    // Preserve any uncheckpointed WAL sidecars if the fixture currently has
    // them; the fixture itself is never opened or modified.
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", source.display()));
        if sidecar.is_file() {
            let destination = PathBuf::from(format!("{}{suffix}", path.display()));
            fs::copy(&sidecar, &destination).expect("copy fixture sidecar");
        }
    }
    FixtureCopy { directory, path }
}
