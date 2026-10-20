//! Live Python differential for the ConstructionScript lowering.
//!
//! The oracle is the repository's own `kala_agent_solidedge.compile`, run in
//! an isolated interpreter against the live repository sources. The Rust side
//! lowers the same scripts and compares the two results byte for byte, as
//! canonical JSON. Refusals are compared message for message.
//!
//! The corpus is the shared parity fixture
//! (`tests/fixtures/construction_parity.json`, read-only) plus the cases of
//! the Python compile tests, which the oracle rebuilds from the live test
//! module's own helpers.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use kala_contracts::{
    ContractModel, load_analysis_json_str, load_construction_json_str, model_json_limits,
};
use kala_json::{CanonicalObject, CanonicalValue, JsonText, parse_strict};
use kala_solidedge_client::{BridgeOp, compile_operations};

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
    let root = repository_root();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/compile_oracle.py");
    let search = std::env::join_paths([
        root.join("adapters/solidedge/src"),
        root.join("adapters/solidedge-bridge/src"),
        root.join("src"),
    ])
    .expect("a search path");
    let output = Command::new(python_executable())
        .arg("-I")
        .arg(&script)
        .arg(&root)
        .env("PYTHONPATH", search)
        .output()
        .expect("run the Python oracle");
    assert!(
        output.status.success(),
        "the Python oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn field<'a>(object: &'a CanonicalObject, name: &str) -> Option<&'a CanonicalValue> {
    object.get(&JsonText::new(name))
}

fn text(object: &CanonicalObject, name: &str) -> Option<String> {
    match field(object, name) {
        Some(CanonicalValue::Text(value)) => value.to_utf8().ok(),
        _ => None,
    }
}

fn is_true(object: &CanonicalObject, name: &str) -> bool {
    matches!(field(object, name), Some(CanonicalValue::Bool(true)))
}

fn canonical(value: &CanonicalValue) -> String {
    kala_json::canonical_dumps(value, &model_json_limits()).expect("canonical JSON")
}

fn canonical_operations(operations: &[BridgeOp]) -> String {
    let value = CanonicalValue::Array(
        operations
            .iter()
            .map(BridgeOp::to_canonical_value)
            .collect::<Vec<_>>(),
    );
    canonical(&value)
}

/// The scripts of the shared parity fixture, keyed by their case id.
fn fixture_scripts() -> BTreeMap<String, String> {
    let path = repository_root().join("tests/fixtures/construction_parity.json");
    let bytes = std::fs::read(&path).expect("read the parity fixture");
    let value = parse_strict(&bytes, &model_json_limits()).expect("the parity fixture is JSON");
    let CanonicalValue::Object(root) = &value else {
        panic!("the parity fixture is not an object");
    };
    let Some(CanonicalValue::Array(cases)) = field(root, "cases") else {
        panic!("the parity fixture has no cases");
    };
    let mut scripts = BTreeMap::new();
    for case in cases {
        let CanonicalValue::Object(case) = case else {
            panic!("a parity case is not an object");
        };
        let id = text(case, "id").expect("a parity case id");
        let Some(CanonicalValue::Object(script)) = field(case, "script") else {
            panic!("a parity case has no script");
        };
        scripts.insert(id, canonical(&CanonicalValue::Object(script.clone())));
    }
    scripts
}

#[test]
fn live_python_lowering_differential() {
    let output = oracle_output();
    let root = parse_strict(&output, &model_json_limits()).expect("the oracle prints JSON");
    let CanonicalValue::Object(root) = &root else {
        panic!("the oracle root is not an object");
    };
    let Some(CanonicalValue::Array(cases)) = field(root, "cases") else {
        panic!("the oracle printed no cases");
    };
    assert!(
        cases.len() >= 54,
        "expected the whole corpus, got {} cases",
        cases.len()
    );

    let fixture = fixture_scripts();
    assert_eq!(fixture.len(), 28, "the shared parity fixture changed size");

    let mut compiled = 0_usize;
    let mut refused = 0_usize;
    for case in cases {
        let CanonicalValue::Object(case) = case else {
            panic!("an oracle case is not an object");
        };
        let id = text(case, "id").expect("an oracle case id");
        let expected_script = text(case, "script").expect("an oracle case script");

        // The fixture cases are read from the shared fixture on this side, so
        // the differential really covers the repository's own corpus.
        let script_text = match id.strip_prefix("fixture:") {
            Some(name) => {
                let fixture_script = fixture
                    .get(name)
                    .unwrap_or_else(|| panic!("{id}: the fixture case is missing"));
                let script =
                    load_construction_json_str(fixture_script).expect("a valid fixture script");
                assert_eq!(
                    script
                        .to_canonical_json_with(&model_json_limits())
                        .expect("canonical JSON"),
                    expected_script,
                    "{id}: the fixture script and the oracle script differ"
                );
                fixture_script.clone()
            }
            None => expected_script.clone(),
        };

        let script = load_construction_json_str(&script_text)
            .unwrap_or_else(|error| panic!("{id}: the script did not load: {}", error.message()));
        let analysis = text(case, "analysis")
            .map(|value| load_analysis_json_str(&value).expect("a valid analysis script"));
        let fillets = match field(case, "fillets") {
            Some(CanonicalValue::Object(map)) => {
                let mut radii = BTreeMap::new();
                for (name, value) in map {
                    let name = name.to_utf8().expect("a fillet name");
                    let CanonicalValue::Float(radius) = value else {
                        panic!("{id}: a fillet radius is not a number");
                    };
                    radii.insert(name, *radius);
                }
                Some(radii)
            }
            _ => None,
        };

        let outcome = compile_operations(&script, analysis.as_ref(), fillets.as_ref());
        if is_true(case, "ok") {
            let operations =
                outcome.unwrap_or_else(|error| panic!("{id}: Rust refused: {}", error.message()));
            assert_eq!(
                canonical_operations(&operations),
                text(case, "canonical").expect("an oracle canonical lowering"),
                "{id}: the lowering differs"
            );
            compiled += 1;
        } else {
            let error = outcome.expect_err(&format!("{id}: Rust accepted what Python refused"));
            assert_eq!(
                error.message(),
                text(case, "message").expect("an oracle refusal message"),
                "{id}: the refusal message differs"
            );
            refused += 1;
        }
    }

    assert_eq!(compiled + refused, cases.len());
    assert!(refused >= 7, "expected the refusal cases, got {refused}");
    assert!(
        compiled >= 40,
        "expected the compiled cases, got {compiled}"
    );
}
