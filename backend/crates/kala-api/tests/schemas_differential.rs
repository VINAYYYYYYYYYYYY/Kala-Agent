//! The live Python differential for the API schemas.
//!
//! The oracle (`tests/oracle_schemas.py`) builds the **live** Pydantic models
//! of `kala_agent.api.schemas` and a FastAPI application that carries the
//! **real** error handlers of `kala_agent.api.errors`, replays a deterministic
//! corpus of payloads through them, and writes both the corpus and the answers
//! to disk. This test replays the very same payloads through the native models
//! and compares:
//!
//! * the canonical JSON of every accepted object,
//! * the wire JSON of every accepted object, which is what FastAPI writes,
//! * every rejection: the kind, the location, the message, the input and the
//!   context Pydantic reports,
//! * the body of every 422 the live API answers, which is the sanitized
//!   envelope the error handlers write.
//!
//! Python is a test oracle only. The crate itself never starts an interpreter.

use std::path::{Path, PathBuf};
use std::process::Command;

use kala_api::errors;
use kala_api::schemas::{self, ApiModel};
use kala_api::validation::{Context, Failures, Issue, Location};
use kala_json::CanonicalValue;
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

/// The answer of one validation: the canonical JSON and the wire JSON of an
/// accepted object, or the failures of a refused one.
type Answer = Result<(String, String), Failures>;

/// The canonical JSON of a value, as the Python source writes it.
///
/// The limits are the ones a request body is parsed with, because the Python
/// source writes the canonical form of a rejected input with `json.dumps`,
/// which applies no protocol limit.
fn canonical(value: &CanonicalValue) -> String {
    kala_json::canonical_dumps(value, &schemas::request_body_limits())
        .expect("a canonical document")
}

/// Validate one payload with the native model of the named kind.
///
/// The first answer is the one of the model itself, which is what
/// `model_validate` reports. The second is the one of the body layer of the
/// framework, which is what a rejected body answers with.
fn validate(model: &str, value: &CanonicalValue) -> (Answer, Answer) {
    macro_rules! native {
        ($model:ty) => {{
            let model_answer = match <$model>::from_value(value) {
                Ok(instance) => match instance.to_canonical_json() {
                    Ok(canonical) => Ok((canonical, instance.to_wire_json())),
                    Err(reason) => Err(vec![Issue::new(
                        "canonicalization",
                        Vec::new(),
                        reason,
                        value,
                    )]),
                },
                Err(failures) => Err(failures),
            };
            let body_answer = match <$model>::from_body(Some(value)) {
                Ok(body) => Ok((
                    body.to_canonical_json().unwrap_or_default(),
                    body.to_wire_json(),
                )),
                Err(failures) => Err(failures),
            };
            (model_answer, body_answer)
        }};
    }
    match model {
        "EvaluateDomainRequest" => native!(schemas::EvaluateDomainRequest),
        "CreateRevisionRequest" => native!(schemas::CreateRevisionRequest),
        "PrepareRevisionRequest" => native!(schemas::PrepareRevisionRequest),
        "RunMechanicalWorkflowRequest" => native!(schemas::RunMechanicalWorkflowRequest),
        "CheckpointDecisionRequest" => native!(schemas::CheckpointDecisionRequest),
        "HealthResponse" => native!(schemas::HealthResponse),
        "DomainListResponse" => native!(schemas::DomainListResponse),
        "EvaluationResponse" => native!(schemas::EvaluationResponse),
        "RevisionListResponse" => native!(schemas::RevisionListResponse),
        "CheckpointListResponse" => native!(schemas::CheckpointListResponse),
        "RunResponse" => native!(schemas::RunResponse),
        other => panic!("the corpus names an unknown model: {other}"),
    }
}

/// The context of a failure, as the canonical JSON of a mapping.
fn context_json(context: &[(String, Context)]) -> String {
    let object: Vec<(kala_json::JsonText, CanonicalValue)> = context
        .iter()
        .map(|(name, value)| {
            (
                kala_json::JsonText::new(name),
                match value {
                    Context::Integer(number) => CanonicalValue::from(*number),
                    Context::Text(text) => CanonicalValue::from(text.as_str()),
                },
            )
        })
        .collect();
    canonical(&CanonicalValue::Object(object.into_iter().collect()))
}

/// Compare one rejection with the failure the Python source reports.
///
/// The undeclared fields of a body are reported in the order the input carries
/// them by the Python source, and in the order the value model keeps them by
/// the native layer, which sorts the keys of an object by code point. When
/// that is the only difference, the case is counted as a known ordering
/// difference and not as a mismatch.
fn compare_failures(
    name: &str,
    expected: &Value,
    failures: &[Issue],
    ordering_differences: &mut usize,
) -> Option<String> {
    let expected_list = expected
        .as_array()
        .expect("the Python failures are an array");
    if expected_list.len() != failures.len() {
        return Some(format!(
            "{name}: python reported {} failures, the native layer reported {}",
            expected_list.len(),
            failures.len()
        ));
    }
    let mut mismatch = None;
    for (index, (expected_failure, failure)) in expected_list.iter().zip(failures).enumerate() {
        let kind = expected_failure["type"].as_str().unwrap_or_default();
        if kind != failure.kind {
            mismatch = Some(format!(
                "{name}[{index}]: python kind={kind} native kind={}",
                failure.kind
            ));
            break;
        }
        let location: Vec<String> = expected_failure["loc"]
            .as_array()
            .expect("the location is an array")
            .iter()
            .map(|part| part.as_str().unwrap_or_default().to_owned())
            .collect();
        if location != failure.location_text() {
            mismatch = Some(format!(
                "{name}[{index}]: python location={location:?} native location={:?}",
                failure.location_text()
            ));
            break;
        }
        let message = expected_failure["msg"].as_str().unwrap_or_default();
        if message != failure.message {
            mismatch = Some(format!(
                "{name}[{index}]: python message={message:?} native message={:?}",
                failure.message
            ));
            break;
        }
        let expected_input = expected_failure["input_canonical"]
            .as_str()
            .expect("the oracle records the canonical input");
        let native_input = canonical(&failure.input);
        if expected_input != native_input {
            mismatch = Some(format!(
                "{name}[{index}]: python input={expected_input:?} native input={native_input:?}"
            ));
            break;
        }
        let expected_context = expected_failure.get("ctx");
        match expected_context {
            None | Some(Value::Null) => {
                if !failure.context.is_empty() {
                    mismatch = Some(format!(
                        "{name}[{index}]: the Python source reports no context, the native layer \
                         reports {:?}",
                        failure.context
                    ));
                    break;
                }
            }
            Some(context) => {
                let expected_context = serde_json::to_string(context).expect("a JSON context");
                let native_context = context_json(&failure.context);
                if expected_context != native_context {
                    mismatch = Some(format!(
                        "{name}[{index}]: python context={expected_context} native \
                         context={native_context}"
                    ));
                    break;
                }
            }
        }
    }
    let mismatch = mismatch?;
    // The only difference the native layer may have is the order of the
    // undeclared fields.
    let mut python_keys: Vec<String> = expected_list
        .iter()
        .map(|failure| {
            format!(
                "{}|{}",
                failure["type"].as_str().unwrap_or_default(),
                failure["loc"]
                    .as_array()
                    .map(|parts| parts
                        .iter()
                        .map(|part| part.as_str().unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join("."))
                    .unwrap_or_default()
            )
        })
        .collect();
    let mut native_keys: Vec<String> = failures
        .iter()
        .map(|failure| format!("{}|{}", failure.kind, failure.location_text().join(".")))
        .collect();
    python_keys.sort();
    native_keys.sort();
    if python_keys == native_keys {
        *ordering_differences += 1;
        return None;
    }
    Some(mismatch)
}

/// The location of a failure, as the transport renders it.
fn transport_location(failure: &Issue) -> Vec<String> {
    let mut location = vec!["body".to_owned()];
    location.extend(failure.location.iter().map(|part| match part {
        Location::Field(name) => name.clone(),
        Location::Index(index) => index.to_string(),
    }));
    location
}

/// Compare two wire documents.
///
/// The field order of a model is part of the contract, so the keys of every
/// model are compared in order. The parameters of a request are free values:
/// the Python source keeps the order of the input, and the native value model
/// keeps the keys sorted by code point, so their order is not compared.
fn same_shape(expected: &Value, native: &Value, free_form: bool) -> bool {
    if free_form {
        return expected == native;
    }
    match (expected, native) {
        (Value::Object(left), Value::Object(right)) => {
            let left_keys: Vec<&String> = left.keys().collect();
            let right_keys: Vec<&String> = right.keys().collect();
            if left_keys != right_keys {
                return false;
            }
            left.iter()
                .zip(right.iter())
                .all(|((name, left), (_, right))| same_shape(left, right, name == "parameters"))
        }
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| same_shape(left, right, false))
        }
        (left, right) => left == right,
    }
}

/// Whether two rejected bodies are the same body.
///
/// The detail lines of the undeclared fields are in the order the input
/// carries them by the Python source and in the order the value model keeps
/// them by the native layer, so those lines are compared as a set and the
/// other lines are compared in order.
fn same_envelope(python_body: &str, native_body: &str) -> bool {
    if python_body == native_body {
        return true;
    }
    let Ok(python) = serde_json::from_str::<Value>(python_body) else {
        return false;
    };
    let Ok(native) = serde_json::from_str::<Value>(native_body) else {
        return false;
    };
    if python["error"] != native["error"] {
        return false;
    }
    let Some(python_details) = python["details"].as_array() else {
        return false;
    };
    let Some(native_details) = native["details"].as_array() else {
        return false;
    };
    if python_details.len() != native_details.len() {
        return false;
    }
    fn split(details: &[Value]) -> (Vec<String>, Vec<String>) {
        let mut ordered: Vec<String> = Vec::new();
        let mut undeclared: Vec<String> = Vec::new();
        for detail in details {
            let text = detail.as_str().unwrap_or_default();
            if text.contains(": extra_forbidden: ") {
                undeclared.push(text.to_owned());
            } else {
                ordered.push(text.to_owned());
            }
        }
        (ordered, undeclared)
    }
    let (python_ordered, python_undeclared) = split(python_details);
    let (native_ordered, native_undeclared) = split(native_details);
    if python_ordered != native_ordered {
        return false;
    }
    let mut python_undeclared = python_undeclared;
    let mut native_undeclared = native_undeclared;
    python_undeclared.sort_unstable();
    native_undeclared.sort_unstable();
    python_undeclared == native_undeclared
}

/// Whether two wire documents are the same document.
fn same_wire(python_wire: &str, native_wire: &str) -> bool {
    if python_wire == native_wire {
        return true;
    }
    let Ok(left) = serde_json::from_str::<Value>(python_wire) else {
        return false;
    };
    let Ok(right) = serde_json::from_str::<Value>(native_wire) else {
        return false;
    };
    same_shape(&left, &right, false)
}

#[test]
fn the_native_models_match_the_live_python_schemas() {
    let Some(interpreter) = python() else {
        eprintln!(
            "SKIPPED: the repository Python interpreter was not found; the live differential \
             did not run. Set KALA_PYTHON to run it."
        );
        return;
    };
    let work = tempfile::tempdir().expect("a temporary directory");
    let corpus_path = work.path().join("corpus.json");
    let results_path = work.path().join("results.json");
    let oracle = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle_schemas.py");
    let output = Command::new(&interpreter)
        .arg("-I")
        .arg(&oracle)
        .arg(&corpus_path)
        .arg(&results_path)
        .env("PYTHONPATH", Path::new(REPO).join("src"))
        .output()
        .expect("the oracle runs");
    assert!(
        output.status.success(),
        "the oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let corpus_text = std::fs::read_to_string(&corpus_path).expect("the corpus is written");
    let corpus: Value = serde_json::from_str(&corpus_text).expect("the corpus is JSON");
    let results_text = std::fs::read_to_string(&results_path).expect("the results are written");
    let results: Value = serde_json::from_str(&results_text).expect("the results are JSON");

    let cases = corpus["cases"]
        .as_array()
        .expect("the corpus carries cases");
    let answers = results["results"]
        .as_array()
        .expect("the answers are an array");
    assert_eq!(cases.len(), answers.len(), "every case has an answer");

    let mut mismatches: Vec<String> = Vec::new();
    let mut accepted = 0usize;
    let mut rejected = 0usize;
    let mut ordering_differences = 0usize;
    for (case, expected) in cases.iter().zip(answers.iter()) {
        let name = case["name"].as_str().unwrap_or("<unnamed>");
        let model = case["model"].as_str().unwrap_or("<unnamed>");
        let payload = serde_json::to_string(&case["payload"]).expect("the payload is JSON");
        let value = kala_json::parse_strict(payload.as_bytes(), &schemas::request_body_limits())
            .expect("the payload is JSON");
        let python_accepted = expected["ok"].as_bool().expect("the answer is an answer");
        let python_body = expected["body"].as_str().unwrap_or_default();

        let (model_answer, body_answer) = validate(model, &value);
        match model_answer {
            Ok((canonical_json, wire_json)) => {
                if !python_accepted {
                    mismatches.push(format!(
                        "{name}: the Python source accepts the payload, the native layer refuses it"
                    ));
                    continue;
                }
                accepted += 1;
                let python_canonical = expected["canonical"].as_str().unwrap_or_default();
                if python_canonical != canonical_json {
                    mismatches.push(format!(
                        "{name}: python canonical={python_canonical:?} native canonical={canonical_json:?}"
                    ));
                }
                let python_wire = expected["wire"].as_str().unwrap_or_default();
                if !same_wire(python_wire, &wire_json) {
                    mismatches.push(format!(
                        "{name}: python wire={python_wire:?} native wire={wire_json:?}"
                    ));
                }
                if !same_wire(python_body, &wire_json) {
                    mismatches.push(format!(
                        "{name}: python body={python_body:?} native body={wire_json:?}"
                    ));
                }
            }
            Err(failures) => {
                if python_accepted {
                    mismatches.push(format!(
                        "{name}: the Python source refuses the payload, the native layer accepts it: \
                         {failures:?}"
                    ));
                    continue;
                }
                rejected += 1;
                if let Some(mismatch) = compare_failures(
                    name,
                    &expected["errors"],
                    &failures,
                    &mut ordering_differences,
                ) {
                    mismatches.push(mismatch);
                    continue;
                }
                let body_failures = body_answer.err().unwrap_or_default();
                let transport: Vec<errors::ValidationFailure> = body_failures
                    .iter()
                    .map(|failure| errors::ValidationFailure {
                        location: transport_location(failure),
                        message: Some(failure.message.clone()),
                        kind: Some(failure.kind.clone()),
                    })
                    .collect();
                let body = errors::handle_request_validation(Some(&transport));
                let native_body = String::from_utf8(body.ordered_bytes()).expect("a UTF-8 body");
                if !same_envelope(python_body, &native_body) {
                    mismatches.push(format!(
                        "{name}: python body={python_body:?} native body={native_body:?}"
                    ));
                }
            }
        }
    }
    eprintln!(
        "differential cases: {} (accepted {accepted}, rejected {rejected}, {ordering_differences} \
         with the order of the undeclared fields)",
        cases.len()
    );

    assert!(
        mismatches.is_empty(),
        "{} mismatches out of {} cases:\n{}",
        mismatches.len(),
        cases.len(),
        mismatches.join("\n")
    );
}
