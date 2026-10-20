//! Shared helpers for the native memory integration tests.

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kala_contracts::{ContractModel, Text, Timestamp, model_json_limits};
use kala_json::{CanonicalValue, canonical_dumps, parse_strict};
use kala_memory::{
    AddOptions, GetOptions, ListOptions, ListWorkingOptions, MemoryError, MemoryManager,
    MemoryManagerConfig, MemoryRecord, MemoryRepository, PromoteOptions, PromoteTarget,
};
use kala_persistence::Database;
use num_bigint::BigInt;
use num_traits::ToPrimitive;

pub fn text(value: &str) -> CanonicalValue {
    CanonicalValue::text(value)
}

pub fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::object(entries)
}

pub fn array(values: Vec<CanonicalValue>) -> CanonicalValue {
    CanonicalValue::Array(values)
}

pub fn int(value: i64) -> CanonicalValue {
    CanonicalValue::Integer(BigInt::from(value))
}

pub fn get<'a>(value: &'a CanonicalValue, key: &str) -> Option<&'a CanonicalValue> {
    match value {
        CanonicalValue::Object(object) => object.get(&kala_json::JsonText::new(key)),
        _ => None,
    }
}

pub fn value_text(value: &CanonicalValue) -> Option<Text> {
    value
        .as_text()
        .map(|text| Text::from_json_text(text.clone()))
}

pub fn required_text(value: &CanonicalValue, key: &str) -> Text {
    get(value, key)
        .and_then(value_text)
        .unwrap_or_else(|| panic!("missing text field {key}"))
}

pub fn optional_text(value: &CanonicalValue, key: &str) -> Option<Text> {
    match get(value, key) {
        None | Some(CanonicalValue::Null) => None,
        Some(value) => Some(value_text(value).unwrap_or_else(|| panic!("bad text field {key}"))),
    }
}

pub fn optional_bool(value: &CanonicalValue, key: &str) -> bool {
    bool_or_default(value, key, false)
}

pub fn bool_or_default(value: &CanonicalValue, key: &str, default: bool) -> bool {
    match get(value, key) {
        None | Some(CanonicalValue::Null) => default,
        Some(CanonicalValue::Bool(value)) => *value,
        Some(_) => panic!("bad bool field {key}"),
    }
}

pub fn optional_int(value: &CanonicalValue, key: &str) -> Option<BigInt> {
    match get(value, key) {
        None | Some(CanonicalValue::Null) => None,
        Some(CanonicalValue::Integer(value)) => Some(value.clone()),
        Some(_) => panic!("bad integer field {key}"),
    }
}

pub fn optional_timestamp(value: &CanonicalValue, key: &str) -> Option<Timestamp> {
    match get(value, key) {
        None | Some(CanonicalValue::Null) => None,
        Some(value) => Some(
            Timestamp::from_value(value).unwrap_or_else(|_| panic!("bad timestamp field {key}")),
        ),
    }
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("backend")
        .parent()
        .expect("repo")
        .to_path_buf()
}

pub fn python_executable() -> PathBuf {
    if let Ok(value) = std::env::var("KALA_MEMORY_PYTHON") {
        return PathBuf::from(value);
    }
    let candidate = repo_root().join(".venv/bin/python");
    assert!(
        candidate.exists(),
        "live Python oracle missing: {} (set KALA_MEMORY_PYTHON)",
        candidate.display()
    );
    candidate
}

/// Run the test-only Python oracle and return its exact stdout bytes.
pub fn run_oracle(document: &CanonicalValue) -> String {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle.py");
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
    let mut child = command.spawn().expect("spawn Python oracle");
    let payload = canonical(document);
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write oracle input");
    let output = child.wait_with_output().expect("wait for Python oracle");
    assert!(
        output.status.success(),
        "Python oracle failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("oracle stdout is UTF-8")
        .trim_end()
        .to_owned()
}

/// The canonical JSON encoding used for every comparison.
pub fn canonical(value: &CanonicalValue) -> String {
    canonical_dumps(value, &model_json_limits()).expect("canonical value")
}

/// Compare the native canonical value against the oracle's exact bytes.
pub fn assert_exact_oracle(native: &CanonicalValue, oracle: &str) {
    let native_bytes = canonical(native);
    if native_bytes != oracle {
        let _ = parse_strict(oracle.as_bytes(), &model_json_limits());
        panic!("native/oracle canonical bytes differ\nnative:\n{native_bytes}\noracle:\n{oracle}");
    }
}

fn error_value(error: &MemoryError) -> CanonicalValue {
    object(vec![(
        "error",
        object(vec![
            ("class", text(error.python_class_name())),
            ("message", text(&error.message())),
        ]),
    )])
}

fn record_value(record: &MemoryRecord) -> CanonicalValue {
    record
        .to_canonical_value()
        .expect("a validated memory record always converts to a canonical value")
}

fn records_value(records: Vec<MemoryRecord>) -> CanonicalValue {
    CanonicalValue::Array(records.iter().map(record_value).collect())
}

fn record_of(operation: &CanonicalValue, key: &str) -> Result<MemoryRecord, MemoryError> {
    let value = get(operation, key)
        .ok_or_else(|| MemoryError::Value(format!("operation is missing the {key} record")))?;
    MemoryRecord::from_value(value).map_err(MemoryError::Contract)
}

fn list_options(operation: &CanonicalValue) -> ListOptions {
    ListOptions {
        source: optional_text(operation, "source").map(|value| value.to_string_lossy()),
        now: optional_timestamp(operation, "now"),
        include_working: bool_or_default(operation, "include_working", true),
        durable_only: optional_bool(operation, "durable_only"),
        limit: optional_int(operation, "limit")
            .and_then(|value| value.to_i64())
            .unwrap_or(1_000),
    }
}

fn list_working_options(operation: &CanonicalValue) -> ListWorkingOptions {
    ListWorkingOptions {
        source: optional_text(operation, "source").map(|value| value.to_string_lossy()),
        now: optional_timestamp(operation, "now"),
        limit: optional_int(operation, "limit")
            .and_then(|value| value.to_i64())
            .unwrap_or(1_000),
    }
}

fn run_op(
    manager: &MemoryManager<'_>,
    repository: &MemoryRepository<'_>,
    operation: &CanonicalValue,
) -> CanonicalValue {
    let name = required_text(operation, "op").to_string_lossy();
    let outcome: Result<CanonicalValue, MemoryError> = match name.as_str() {
        "add" => {
            let record = record_of(operation, "record");
            record.and_then(|record| {
                manager
                    .add(
                        &record,
                        AddOptions {
                            durable: optional_bool(operation, "durable"),
                            promote: optional_bool(operation, "promote"),
                            now: optional_timestamp(operation, "now"),
                        },
                    )
                    .map(|stored| record_value(&stored))
            })
        }
        "try_add" => record_of(operation, "record").and_then(|record| {
            manager
                .try_add(&record, optional_timestamp(operation, "now"))
                .map(|result| result.as_dict())
        }),
        "promote" => {
            let target_value = match get(operation, "target") {
                Some(value) => value,
                None => {
                    return error_value(&MemoryError::Value(
                        "operation is missing the promote target".to_owned(),
                    ));
                }
            };
            let kind = required_text(target_value, "kind").to_string_lossy();
            let options = PromoteOptions {
                provenance_id: optional_text(operation, "provenance_id")
                    .map(|value| value.to_string_lossy()),
                now: optional_timestamp(operation, "now"),
            };
            if kind == "record" {
                match record_of(target_value, "record") {
                    Ok(record) => manager
                        .promote(PromoteTarget::Record(&record), options)
                        .map(|stored| record_value(&stored)),
                    Err(error) => Err(error),
                }
            } else {
                let memory_id = required_text(target_value, "id").to_string_lossy();
                manager
                    .promote(PromoteTarget::Id(&memory_id), options)
                    .map(|stored| record_value(&stored))
            }
        }
        "get" => {
            let memory_id = required_text(operation, "memory_id").to_string_lossy();
            manager
                .get(
                    &memory_id,
                    GetOptions {
                        now: optional_timestamp(operation, "now"),
                        include_expired: optional_bool(operation, "include_expired"),
                    },
                )
                .map(|found| match found {
                    Some(record) => record_value(&record),
                    None => CanonicalValue::Null,
                })
        }
        "list" => manager.list(list_options(operation)).map(records_value),
        "list_active" => manager
            .list_active(list_options(operation))
            .map(records_value),
        "list_durable" => manager
            .list_durable(list_options(operation))
            .map(records_value),
        "list_working" => manager
            .list_working(list_working_options(operation))
            .map(records_value),
        "detect_conflicts" => record_of(operation, "record").and_then(|record| {
            manager
                .detect_conflicts(&record, optional_timestamp(operation, "now"))
                .map(records_value)
        }),
        "discard_working" => {
            let memory_id = required_text(operation, "memory_id").to_string_lossy();
            Ok(CanonicalValue::Bool(manager.discard_working(&memory_id)))
        }
        "delete" => {
            let memory_id = required_text(operation, "memory_id").to_string_lossy();
            manager.delete(&memory_id).map(CanonicalValue::Bool)
        }
        "repository_put" => record_of(operation, "record").and_then(|record| {
            repository
                .put(&record)
                .map_err(MemoryError::Repository)
                .map(|()| CanonicalValue::Null)
        }),
        other => Err(MemoryError::Value(format!("unknown memory op {other}"))),
    };
    match outcome {
        Ok(value) => value,
        Err(error) => error_value(&error),
    }
}

/// Run one scenario against the native implementation.
pub fn memory_native(scenario: &CanonicalValue) -> CanonicalValue {
    let database = Database::in_memory().expect("in-memory database");
    let repository = MemoryRepository::new(&database);
    let max_working_records = optional_int(scenario, "max_working_records")
        .and_then(|value| value.to_i64())
        .map_or(1_000, |value| usize::try_from(value).unwrap_or(0));
    let config = MemoryManagerConfig {
        max_working_records,
        require_provenance_on_promotion: optional_bool(scenario, "require_provenance_on_promotion"),
    };
    let operations = match get(scenario, "ops") {
        Some(CanonicalValue::Array(values)) => values.clone(),
        _ => Vec::new(),
    };
    let manager = match MemoryManager::with_config(repository, config) {
        Ok(manager) => manager,
        Err(error) => {
            return object(vec![(
                "results",
                array(operations.iter().map(|_| error_value(&error)).collect()),
            )]);
        }
    };
    let results = operations
        .iter()
        .map(|operation| run_op(&manager, &repository, operation))
        .collect();
    object(vec![("results", array(results))])
}

/// Run a batch of scenarios against the native implementation.
pub fn memory_native_batch(document: &CanonicalValue) -> CanonicalValue {
    let scenarios = match get(document, "scenarios") {
        Some(CanonicalValue::Array(values)) => values.clone(),
        _ => Vec::new(),
    };
    let responses = scenarios.iter().map(memory_native).collect();
    object(vec![("scenarios", array(responses))])
}

/// The number of scenarios and operations a differential document contains.
///
/// The differential tests use this to state the size of the corpus they
/// compare, so the coverage claim is visible in the test source.
pub fn document_totals(document: &CanonicalValue) -> (usize, usize) {
    let scenarios = match get(document, "scenarios") {
        Some(CanonicalValue::Array(values)) => values.len(),
        _ => 0,
    };
    let operations = match get(document, "scenarios") {
        Some(CanonicalValue::Array(values)) => values
            .iter()
            .map(|scenario| match get(scenario, "ops") {
                Some(CanonicalValue::Array(items)) => items.len(),
                _ => 0,
            })
            .sum(),
        _ => 0,
    };
    (scenarios, operations)
}

/// Compare one scenario batch against the live Python oracle.
pub fn assert_matches_oracle(document: &CanonicalValue) {
    let oracle = run_oracle(document);
    let native = memory_native_batch(document);
    assert_exact_oracle(&native, &oracle);
}

/// Build a memory record payload for a scenario document.
#[allow(clippy::too_many_arguments)]
pub fn record_payload(
    memory_id: &str,
    content: &str,
    source: &str,
    provenance_id: Option<&str>,
    created_at: &str,
    expires_at: Option<&str>,
) -> CanonicalValue {
    let entries = vec![
        ("memory_id", text(memory_id)),
        ("content", text(content)),
        ("source", text(source)),
        (
            "provenance_id",
            provenance_id.map_or(CanonicalValue::Null, text),
        ),
        ("metadata_json", text("{}")),
        ("created_at", text(created_at)),
        ("expires_at", expires_at.map_or(CanonicalValue::Null, text)),
    ];
    object(entries)
}
