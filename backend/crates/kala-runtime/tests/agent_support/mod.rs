//! Shared helpers for the agent integration tests.

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_tools::executor::{ExecutorConfig, IsolationMode, ToolExecutor};
use kala_tools::registry::ToolRegistry;
use kala_tools::simulator::{
    EVALUATOR_TOOL_ID, EVALUATOR_TOOL_VERSION, SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION,
    register_reference_evaluator, register_simulator,
};

/// The capabilities the simulator needs.
pub const SIMULATOR_CAPABILITIES: [&str; 4] = [
    "artifact.read",
    "artifact.write",
    "design.read",
    "tool.execute",
];

/// Build an executor with the shipped simulator tools.
///
/// The pure reference evaluator runs inside the kernel process, exactly like
/// the Python test double, so the differential never spawns a worker.
pub fn executor(workdir: &Path) -> Arc<ToolExecutor> {
    let registry = Arc::new(ToolRegistry::new());
    register_reference_evaluator(&registry, EVALUATOR_TOOL_ID, EVALUATOR_TOOL_VERSION)
        .expect("the reference evaluator registers");
    register_simulator(&registry, SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION)
        .expect("the simulator registers");
    let mut config = ExecutorConfig::new(registry, workdir.join("run"));
    config.granted_capabilities = SIMULATOR_CAPABILITIES
        .iter()
        .map(|value| (*value).to_owned())
        .collect();
    config.isolation = IsolationMode::TrustedInProcessEvaluator;
    Arc::new(ToolExecutor::new(config).expect("the executor is built"))
}

/// The repository root.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repository root")
}

/// The live Python oracle interpreter.
pub fn python_executable() -> PathBuf {
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

/// Run the agent oracle and return its parsed result document.
pub fn run_oracle(scenario: &CanonicalValue) -> CanonicalValue {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/agent_oracle.py");
    let mut command = Command::new(python_executable());
    command
        .arg(script)
        .env(
            "PYTHONPATH",
            repo_root().join("src").to_string_lossy().to_string(),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn the Python oracle");
    let limits = kala_json::Limits {
        max_string_length: 1_048_576,
        max_json_bytes: 16_777_216,
        ..kala_json::Limits::protocol()
    };
    let payload = kala_json::canonical_dumps(scenario, &limits).expect("canonical scenario");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write the scenario");
    let output = child.wait_with_output().expect("wait for the oracle");
    assert!(
        output.status.success(),
        "the Python oracle failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("the oracle stdout is UTF-8");
    parse(&text)
}

/// Parse canonical JSON text.
pub fn parse(text: &str) -> CanonicalValue {
    let limits = kala_json::Limits {
        max_string_length: 1_048_576,
        max_json_bytes: 16_777_216,
        ..kala_json::Limits::protocol()
    };
    kala_json::parse_strict(text.as_bytes(), &limits)
        .unwrap_or_else(|error| panic!("parse the result document: {error}"))
}

/// A JSON string helper.
pub fn text(value: &str) -> CanonicalValue {
    CanonicalValue::Text(JsonText::new(value))
}

/// Read a field of a JSON object.
pub fn get<'a>(value: &'a CanonicalValue, key: &str) -> Option<&'a CanonicalValue> {
    match value {
        CanonicalValue::Object(entries) => entries.get(&JsonText::new(key)),
        _ => None,
    }
}

/// Read a string field of a JSON object.
pub fn text_field(value: &CanonicalValue, key: &str) -> String {
    get(value, key)
        .and_then(|item| item.as_text())
        .map(|item| item.to_string_lossy())
        .unwrap_or_default()
}

/// Read an optional string field of a JSON object.
pub fn optional_text_field(value: &CanonicalValue, key: &str) -> Option<String> {
    get(value, key)
        .and_then(|item| item.as_text())
        .map(|item| item.to_string_lossy())
}

/// Read a boolean field of a JSON object.
pub fn bool_field(value: &CanonicalValue, key: &str) -> bool {
    matches!(get(value, key), Some(CanonicalValue::Bool(true)))
}

/// Read a list of strings from a JSON array field.
pub fn string_list(value: &CanonicalValue, key: &str) -> Vec<String> {
    match get(value, key) {
        Some(CanonicalValue::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_text()
                    .map(|value| value.to_string_lossy())
                    .unwrap_or_default()
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Read a number field of a JSON object.
pub fn float_field(value: &CanonicalValue, key: &str) -> Option<f64> {
    match get(value, key) {
        Some(CanonicalValue::Float(number)) => Some(*number),
        Some(CanonicalValue::Integer(number)) => number.to_string().parse::<f64>().ok(),
        _ => None,
    }
}

/// Read an integer field of a JSON object.
pub fn int_field(value: &CanonicalValue, key: &str) -> Option<i64> {
    match get(value, key) {
        Some(CanonicalValue::Integer(number)) => number.to_string().parse::<i64>().ok(),
        Some(CanonicalValue::Float(number)) => Some(*number as i64),
        _ => None,
    }
}

/// A temporary directory that removes itself.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Create a temporary directory with a unique name.
    pub fn new(prefix: &str) -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{sequence}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create the temporary directory");
        Self { path }
    }

    /// The path of the directory.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Build a JSON object.
pub fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    let mut map = CanonicalObject::new();
    for (key, value) in entries {
        map.insert(JsonText::new(key), value);
    }
    CanonicalValue::Object(map)
}

/// Replace the timing fields of a result document.
///
/// The duration of a tool call is a measurement, not a decision, so it is
/// the one field the differential cannot compare byte for byte.
pub fn normalize_timings(document: &CanonicalValue) -> CanonicalValue {
    let CanonicalValue::Object(entries) = document else {
        return document.clone();
    };
    let mut normalized = CanonicalObject::new();
    for (key, value) in entries {
        if key.to_string_lossy() == "cases" {
            normalized.insert(
                key.clone(),
                match &value {
                    CanonicalValue::Array(items) => {
                        CanonicalValue::Array(items.iter().map(normalize_case).collect::<Vec<_>>())
                    }
                    other => (*other).clone(),
                },
            );
            continue;
        }
        normalized.insert(key.clone(), value.clone());
    }
    CanonicalValue::Object(normalized)
}

/// Replace the timing fields of one case.
fn normalize_case(case: &CanonicalValue) -> CanonicalValue {
    let CanonicalValue::Object(entries) = case else {
        return case.clone();
    };
    let mut normalized = CanonicalObject::new();
    for (key, value) in entries {
        if key.to_string_lossy() == "result" {
            normalized.insert(key.clone(), normalize_result(value));
            continue;
        }
        if key.to_string_lossy() == "calls" {
            normalized.insert(key.clone(), normalize_calls(value));
            continue;
        }
        if key.to_string_lossy() == "ledger" {
            normalized.insert(key.clone(), normalize_ledger(value));
            continue;
        }
        normalized.insert(key.clone(), value.clone());
    }
    CanonicalValue::Object(normalized)
}

/// Sort the execution reports by their call identifier.
///
/// The reports are appended when a call finishes, so their order is a race
/// between the workers of a layer on both sides. The *set* of calls is the
/// decision, not the order they finished in.
fn normalize_calls(calls: &CanonicalValue) -> CanonicalValue {
    let CanonicalValue::Array(items) = calls else {
        return calls.clone();
    };
    let mut sorted = items.clone();
    sorted.sort_by_key(|item| {
        get(item, "call_id")
            .and_then(|value| value.as_text())
            .map(|value| value.to_string_lossy())
            .unwrap_or_default()
    });
    CanonicalValue::Array(sorted)
}

/// Replace the measured duration of the budget ledger.
fn normalize_ledger(ledger: &CanonicalValue) -> CanonicalValue {
    let CanonicalValue::Object(entries) = ledger else {
        return ledger.clone();
    };
    let mut normalized = CanonicalObject::new();
    for (key, value) in entries {
        if matches!(key.to_string_lossy().as_str(), "usage" | "reserved") {
            normalized.insert(key.clone(), normalize_usage(value));
            continue;
        }
        normalized.insert(key.clone(), value.clone());
    }
    CanonicalValue::Object(normalized)
}

/// Replace the measured duration of one usage record.
fn normalize_usage(usage: &CanonicalValue) -> CanonicalValue {
    let CanonicalValue::Object(entries) = usage else {
        return usage.clone();
    };
    let mut normalized = CanonicalObject::new();
    for (key, value) in entries {
        if key.to_string_lossy() == "duration_seconds" {
            normalized.insert(key.clone(), CanonicalValue::Float(0.0));
            continue;
        }
        normalized.insert(key.clone(), value.clone());
    }
    CanonicalValue::Object(normalized)
}

/// Replace the timing fields of one run result.
fn normalize_result(result: &CanonicalValue) -> CanonicalValue {
    let CanonicalValue::Object(entries) = result else {
        return result.clone();
    };
    let mut normalized = CanonicalObject::new();
    for (key, value) in entries {
        if key.to_string_lossy() == "outcomes" {
            normalized.insert(
                key.clone(),
                match &value {
                    CanonicalValue::Array(items) => CanonicalValue::Array(
                        items.iter().map(normalize_outcome).collect::<Vec<_>>(),
                    ),
                    other => (*other).clone(),
                },
            );
            continue;
        }
        normalized.insert(key.clone(), value.clone());
    }
    CanonicalValue::Object(normalized)
}

/// Replace the duration of one outcome.
fn normalize_outcome(outcome: &CanonicalValue) -> CanonicalValue {
    let CanonicalValue::Object(entries) = outcome else {
        return outcome.clone();
    };
    let mut normalized = CanonicalObject::new();
    for (key, value) in entries {
        if key.to_string_lossy() == "duration_ms" {
            normalized.insert(key.clone(), CanonicalValue::Float(0.0));
            continue;
        }
        normalized.insert(key.clone(), value.clone());
    }
    CanonicalValue::Object(normalized)
}
