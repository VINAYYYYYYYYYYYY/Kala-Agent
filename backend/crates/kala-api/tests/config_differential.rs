//! The live Python differential for the local configuration.
//!
//! The oracle (`tests/oracle_config.py`) exercises the **live**
//! `kala_agent.config.Settings` model of the repository and records either the
//! validated settings or the exact Pydantic failures: the error kind, the
//! field, and the message. This test validates the very same values with the
//! native port and compares the answers field by field and failure by
//! failure.
//!
//! Python is a test oracle only. The crate itself never starts an
//! interpreter.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use kala_api::config::{SettingValue, error_kind, settings_from_env, settings_from_values};
use serde_json::Value;

const REPO: &str = "/home/v9i/Projects/Kala-Agent";

fn python() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("KALA_PYTHON") {
        let path = PathBuf::from(explicit);
        return path.exists().then_some(path);
    }
    let path = Path::new(REPO).join(".venv/bin/python");
    path.exists().then_some(path)
}

fn tagged(value: &Value) -> SettingValue {
    match value["kind"].as_str().unwrap_or("text") {
        "text" => SettingValue::Text(value["value"].as_str().unwrap_or("").to_owned()),
        "integer" => SettingValue::Integer(value["value"].as_i64().unwrap_or_default()),
        "float" => SettingValue::Float(
            value["value"]
                .as_str()
                .unwrap_or("0")
                .parse::<f64>()
                .unwrap_or(f64::NAN),
        ),
        "boolean" => SettingValue::Boolean(value["value"].as_bool().unwrap_or_default()),
        _ => SettingValue::Null,
    }
}

fn float_text(value: f64) -> String {
    kala_json::format_float(value).unwrap_or_else(|_| "nan".to_owned())
}

#[test]
fn the_native_settings_match_the_live_python_settings() {
    let Some(interpreter) = python() else {
        eprintln!(
            "SKIPPED: the repository Python interpreter was not found; the live configuration \
             differential did not run. Set KALA_PYTHON to run it."
        );
        return;
    };
    let work = tempfile::tempdir().expect("a temporary directory");
    let corpus_path = work.path().join("corpus.json");
    let results_path = work.path().join("results.json");
    let oracle = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle_config.py");
    let output = Command::new(&interpreter)
        .arg("-I")
        .arg(&oracle)
        .arg(&corpus_path)
        .arg(&results_path)
        .arg(work.path().join("work"))
        .output()
        .expect("the oracle runs");
    assert!(
        output.status.success(),
        "the oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let corpus: Value =
        serde_json::from_str(&std::fs::read_to_string(&corpus_path).expect("the corpus"))
            .expect("the corpus is JSON");
    let results: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(&results_path).expect("the results"))
            .expect("the results are JSON");

    let mut mismatches: Vec<String> = Vec::new();
    let mut compared = 0usize;

    for case in corpus["cases"].as_array().expect("the cases") {
        let name = case["name"].as_str().unwrap_or("<unnamed>");
        let mut values: BTreeMap<String, SettingValue> = BTreeMap::new();
        if let Some(map) = case["values"].as_object() {
            for (key, value) in map {
                values.insert(key.clone(), tagged(value));
            }
        }
        let expected = results
            .iter()
            .find(|item| item["name"].as_str() == Some(name))
            .expect("every case has an answer");
        compared += 1;
        compare(
            &mut mismatches,
            name,
            expected,
            settings_from_values(&values),
        );
    }

    for case in corpus["environment_cases"].as_array().expect("the cases") {
        let name = case["name"].as_str().unwrap_or("<unnamed>");
        let mut environ: BTreeMap<String, String> = BTreeMap::new();
        if let Some(map) = case["environ"].as_object() {
            for (key, value) in map {
                environ.insert(key.clone(), value.as_str().unwrap_or("").to_owned());
            }
        }
        let expected = results
            .iter()
            .find(|item| item["name"].as_str() == Some(name))
            .expect("every case has an answer");
        compared += 1;
        compare(&mut mismatches, name, expected, settings_from_env(&environ));
    }

    eprintln!("configuration differential cases: {compared}");
    assert!(
        mismatches.is_empty(),
        "{} mismatches out of {compared} cases:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

fn compare(
    mismatches: &mut Vec<String>,
    name: &str,
    expected: &Value,
    actual: Result<kala_api::config::Settings, Vec<kala_api::config::SettingsError>>,
) {
    match (expected["ok"].as_bool().unwrap_or(false), actual) {
        (true, Ok(settings)) => {
            let expected_settings = &expected["settings"];
            let mut differences: Vec<String> = Vec::new();
            let mut check = |field: &str, mine: String, theirs: &str| {
                if mine != theirs {
                    differences.push(format!("{field}: python={theirs:?} native={mine:?}"));
                }
            };
            check(
                "data_dir",
                settings.data_dir.to_string_lossy().into_owned(),
                expected_settings["data_dir"].as_str().unwrap_or(""),
            );
            check(
                "model_provider",
                settings.model_provider.clone(),
                expected_settings["model_provider"].as_str().unwrap_or(""),
            );
            check(
                "model_id",
                settings.model_id.clone(),
                expected_settings["model_id"].as_str().unwrap_or(""),
            );
            check(
                "model_endpoint",
                settings.model_endpoint.clone().unwrap_or_default(),
                expected_settings["model_endpoint"].as_str().unwrap_or(""),
            );
            check(
                "request_timeout_seconds",
                float_text(settings.request_timeout_seconds),
                &float_text(
                    expected_settings["request_timeout_seconds"]
                        .as_f64()
                        .unwrap_or_default(),
                ),
            );
            check(
                "tool_timeout_seconds",
                float_text(settings.tool_timeout_seconds),
                &float_text(
                    expected_settings["tool_timeout_seconds"]
                        .as_f64()
                        .unwrap_or_default(),
                ),
            );
            check(
                "worker_count",
                settings.worker_count.to_string(),
                &expected_settings["worker_count"]
                    .as_u64()
                    .unwrap_or_default()
                    .to_string(),
            );
            if expected_settings.get("artifact_dir").is_some() {
                check(
                    "artifact_dir",
                    settings.artifact_dir().to_string_lossy().into_owned(),
                    expected_settings["artifact_dir"].as_str().unwrap_or(""),
                );
                check(
                    "journal_path",
                    settings.journal_path().to_string_lossy().into_owned(),
                    expected_settings["journal_path"].as_str().unwrap_or(""),
                );
                check(
                    "database_path",
                    settings.database_path().to_string_lossy().into_owned(),
                    expected_settings["database_path"].as_str().unwrap_or(""),
                );
                check(
                    "transcript_dir",
                    settings.transcript_dir().to_string_lossy().into_owned(),
                    expected_settings["transcript_dir"].as_str().unwrap_or(""),
                );
            }
            if !differences.is_empty() {
                mismatches.push(format!("{name}: {}", differences.join("; ")));
            }
        }
        (false, Err(errors)) => {
            let expected_errors = expected["errors"].as_array().expect("the errors");
            if expected_errors.len() != errors.len() {
                mismatches.push(format!(
                    "{name}: python reported {} errors, native reported {}",
                    expected_errors.len(),
                    errors.len()
                ));
                return;
            }
            for (index, failure) in errors.iter().enumerate() {
                let theirs = &expected_errors[index];
                let kind = theirs["kind"].as_str().unwrap_or("");
                let location = theirs["location"].as_str().unwrap_or("");
                let message = theirs["message"].as_str().unwrap_or("");
                if failure.kind != kind
                    || failure.location != location
                    || failure.message != message
                {
                    mismatches.push(format!(
                        "{name}[{index}]: python=({kind}, {location}, {message}) \
                         native=({}, {}, {})",
                        failure.kind, failure.location, failure.message
                    ));
                }
            }
        }
        (true, Err(errors)) => {
            let rendered: Vec<String> = errors
                .iter()
                .map(|error| format!("({}, {})", error.kind, error.message))
                .collect();
            mismatches.push(format!(
                "{name}: python accepted the values, native refused them: {rendered:?}"
            ));
        }
        (false, Ok(_)) => {
            mismatches.push(format!(
                "{name}: python refused the values, native accepted them"
            ));
        }
    }
}

#[test]
fn the_error_kinds_are_the_python_kinds() {
    assert_eq!(error_kind::VALUE_ERROR, "value_error");
    assert_eq!(error_kind::STRING_TOO_SHORT, "string_too_short");
    assert_eq!(error_kind::URL_PARSING, "url_parsing");
    assert_eq!(error_kind::EXTRA_FORBIDDEN, "extra_forbidden");
}
