//! Live Python differential oracle for every contract model and construction
//! script.
//!
//! Python is used only by this test. The oracle imports the live repository
//! package and serializes its `model_dump(mode="json")` output; the Rust side
//! compares wire bytes, sorted/ASCII canonical bytes and SHA-256 digests.

use std::path::PathBuf;
use std::process::Command;

use kala_contracts::{
    AnalysisReading, AnalysisScript, AnalysisStep, ApprovalRequest, ArtifactRef, AssemblyStep,
    ComponentScript, ComponentStep, ConstructionScript, ContractModel, DesignRevision, DesignSpec,
    DomainPackageManifest, EditStep, EvaluationMetric, EvaluationResult, PlanStep, Provenance,
    Requirement, RunBudget, ToolCall, ToolDescriptor, ToolResult, WorkflowDefinition,
    WorkflowTransition, load_analysis_json_bytes, load_analysis_json_str,
    load_construction_json_bytes, load_construction_json_str, load_reading_json_bytes,
    load_reading_json_str, model_json_limits,
};
use kala_json::{CanonicalValue, parse_strict};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn python_executable() -> PathBuf {
    if let Ok(path) = std::env::var("KALA_PYTHON") {
        return PathBuf::from(path);
    }
    let venv = repository_root().join(".venv/bin/python");
    if venv.is_file() {
        return venv;
    }
    PathBuf::from("python3")
}

fn oracle_output() -> Vec<u8> {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/contracts_oracle.py");
    let output = Command::new(python_executable())
        .arg(script)
        .env("PYTHONPATH", repository_root().join("src"))
        .output()
        .expect("run Python oracle");
    assert!(
        output.status.success(),
        "Python oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn value<'a>(object: &'a kala_json::CanonicalObject, name: &str) -> Option<&'a CanonicalValue> {
    object.get(&kala_json::JsonText::new(name))
}

fn text(object: &kala_json::CanonicalObject, name: &str) -> Option<String> {
    let CanonicalValue::Text(value) = value(object, name)? else {
        return None;
    };
    value.to_utf8().ok()
}

fn flag(object: &kala_json::CanonicalObject, name: &str) -> bool {
    matches!(value(object, name), Some(CanonicalValue::Bool(true)))
}

#[derive(Debug)]
struct Snapshot {
    wire_ascii: String,
    wire_utf8: Option<String>,
    canonical: String,
    digest: String,
}

fn snapshot<T: ContractModel>(model: &T) -> Snapshot {
    Snapshot {
        wire_ascii: model.to_wire_json_ascii(),
        wire_utf8: model.to_wire_json().ok(),
        canonical: model
            .to_canonical_json_with(&model_json_limits())
            .expect("canonical JSON"),
        digest: model
            .canonical_digest_with(&model_json_limits())
            .expect("canonical digest")
            .to_hex(),
    }
}

fn validated<T: ContractModel>(input_json: &str, entry: &str) -> Result<Snapshot, String> {
    let model = if entry == "bytes" {
        T::from_json_bytes(input_json.as_bytes())
    } else {
        T::from_json_str(input_json)
    }
    .map_err(|error| error.to_string())?;
    let first = snapshot(&model);
    // Cross-reader check: the wire JSON must be valid contract input again and
    // reproduce identical wire and canonical bytes.
    let reparsed = parse_strict(first.wire_ascii.as_bytes(), &model_json_limits())
        .map_err(|error| error.to_string())?;
    let roundtrip = T::from_value(&reparsed).map_err(|error| error.to_string())?;
    let second = snapshot(&roundtrip);
    assert_eq!(first.wire_ascii, second.wire_ascii, "wire round trip");
    assert_eq!(first.canonical, second.canonical, "canonical round trip");
    Ok(first)
}

fn validate(
    model: &str,
    loader: Option<&str>,
    entry: &str,
    input_json: &str,
) -> Result<Snapshot, String> {
    let use_bytes = entry == "bytes";
    match (model, loader) {
        ("ConstructionScript", Some("load_construction")) => {
            let model = if use_bytes {
                load_construction_json_bytes(input_json.as_bytes())
            } else {
                load_construction_json_str(input_json)
            }
            .map_err(|error| error.message().to_owned())?;
            Ok(snapshot(&model))
        }
        ("AnalysisScript", Some("load_analysis")) => {
            let model = if use_bytes {
                load_analysis_json_bytes(input_json.as_bytes())
            } else {
                load_analysis_json_str(input_json)
            }
            .map_err(|error| error.message().to_owned())?;
            Ok(snapshot(&model))
        }
        ("AnalysisReading", Some("load_reading")) => {
            let model = if use_bytes {
                load_reading_json_bytes(input_json.as_bytes())
            } else {
                load_reading_json_str(input_json)
            }
            .map_err(|error| error.message().to_owned())?;
            Ok(snapshot(&model))
        }
        ("Requirement", _) => validated::<Requirement>(input_json, entry),
        ("DesignSpec", _) => validated::<DesignSpec>(input_json, entry),
        ("DomainPackageManifest", _) => validated::<DomainPackageManifest>(input_json, entry),
        ("ArtifactRef", _) => validated::<ArtifactRef>(input_json, entry),
        ("DesignRevision", _) => validated::<DesignRevision>(input_json, entry),
        ("PlanStep", _) => validated::<PlanStep>(input_json, entry),
        ("WorkflowTransition", _) => validated::<WorkflowTransition>(input_json, entry),
        ("WorkflowDefinition", _) => validated::<WorkflowDefinition>(input_json, entry),
        ("ToolDescriptor", _) => validated::<ToolDescriptor>(input_json, entry),
        ("ToolCall", _) => validated::<ToolCall>(input_json, entry),
        ("ToolResult", _) => validated::<ToolResult>(input_json, entry),
        ("EvaluationMetric", _) => validated::<EvaluationMetric>(input_json, entry),
        ("EvaluationResult", _) => validated::<EvaluationResult>(input_json, entry),
        ("ApprovalRequest", _) => validated::<ApprovalRequest>(input_json, entry),
        ("RunBudget", _) => validated::<RunBudget>(input_json, entry),
        ("Provenance", _) => validated::<Provenance>(input_json, entry),
        ("ComponentStep", _) => validated::<ComponentStep>(input_json, entry),
        ("ComponentScript", _) => validated::<ComponentScript>(input_json, entry),
        ("AssemblyStep", _) => validated::<AssemblyStep>(input_json, entry),
        ("ConstructionScript", _) => validated::<ConstructionScript>(input_json, entry),
        ("AnalysisStep", _) => validated::<AnalysisStep>(input_json, entry),
        ("AnalysisScript", _) => validated::<AnalysisScript>(input_json, entry),
        ("EditStep", _) => validated::<EditStep>(input_json, entry),
        ("AnalysisReading", _) => validated::<AnalysisReading>(input_json, entry),
        _ => panic!("unknown oracle model {model}"),
    }
}

#[test]
fn live_python_contract_differential() {
    let output = oracle_output();
    let root = parse_strict(&output, &model_json_limits()).expect("oracle JSON");
    let CanonicalValue::Object(root) = &root else {
        panic!("oracle root is not an object");
    };
    let Some(CanonicalValue::Array(cases)) = value(root, "cases") else {
        panic!("oracle cases missing");
    };
    assert!(cases.len() > 100, "expected a meaningful corpus");
    for case in cases {
        let CanonicalValue::Object(case) = case else {
            panic!("case is not an object");
        };
        let id = text(case, "id").expect("case id");
        let model = text(case, "model").expect("case model");
        let loader = text(case, "loader");
        let entry = text(case, "entry").unwrap_or_else(|| "str".to_owned());
        let input_json = text(case, "input_json").expect("case input");
        let result = validate(&model, loader.as_deref(), &entry, &input_json);
        if flag(case, "ok") {
            let snapshot = result.unwrap_or_else(|error| panic!("{id}: Rust refused: {error}"));
            assert_eq!(
                snapshot.wire_ascii,
                text(case, "wire_ascii").expect("wire_ascii"),
                "{id}: wire_ascii"
            );
            if let Some(expected) = text(case, "wire_utf8") {
                assert_eq!(
                    snapshot.wire_utf8.as_deref(),
                    Some(expected.as_str()),
                    "{id}: wire_utf8"
                );
            }
            assert_eq!(
                snapshot.canonical,
                text(case, "canonical").expect("canonical"),
                "{id}: canonical"
            );
            assert_eq!(
                snapshot.digest,
                text(case, "digest").expect("digest"),
                "{id}: digest"
            );
        } else {
            let error = result.expect_err(&format!("{id}: Rust accepted but Python refused"));
            if let Some(expected) = text(case, "message") {
                assert_eq!(error, expected, "{id}: construction error message");
            }
        }
    }
}

#[test]
fn oracle_transport_preserves_raw_duplicate_text() {
    let output = oracle_output();
    let root = parse_strict(&output, &model_json_limits()).expect("oracle JSON");
    let CanonicalValue::Object(root) = &root else {
        panic!("oracle root is not an object");
    };
    let Some(CanonicalValue::Array(cases)) = value(root, "cases") else {
        panic!("oracle cases missing");
    };
    let mut duplicate_surrogate_str = false;
    let mut duplicate_surrogate_bytes = false;
    let mut escaped_private_spelling = false;
    let mut nonadjacent_constraints = false;
    let mut bytes_loader = false;
    for case in cases {
        let CanonicalValue::Object(case) = case else {
            panic!("case is not an object");
        };
        let input = text(case, "input_json").expect("case input");
        let entry = text(case, "entry").unwrap_or_else(|| "str".to_owned());
        let loader = text(case, "loader");
        if input.contains(r#""id":"spec","id":"last""#) && input.contains(r#""text":"\ud800""#) {
            match entry.as_str() {
                "bytes" => duplicate_surrogate_bytes = true,
                _ => duplicate_surrogate_str = true,
            }
        }
        if input.contains(r#""\u0024serde_json::private::Number""#) {
            escaped_private_spelling = true;
        }
        if input.contains(r#""constraints":["a","b","a"]"#) {
            nonadjacent_constraints = true;
        }
        if loader.as_deref() == Some("load_construction") && entry == "bytes" {
            bytes_loader = true;
        }
    }
    assert!(
        duplicate_surrogate_str,
        "missing str duplicate+surrogate case"
    );
    assert!(
        duplicate_surrogate_bytes,
        "missing bytes duplicate+surrogate case"
    );
    assert!(
        escaped_private_spelling,
        "missing escaped private-marker spelling case"
    );
    assert!(
        nonadjacent_constraints,
        "missing nonadjacent duplicate constraints case"
    );
    assert!(bytes_loader, "missing construction loader bytes case");
}
