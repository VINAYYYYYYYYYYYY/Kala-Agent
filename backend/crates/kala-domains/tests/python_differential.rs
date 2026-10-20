//! Live-Python differential tests for the native domain layer.
//!
//! The oracle requires the repository virtual environment
//! (`<repo>/.venv/bin/python`, overridable through `KALA_PYTHON`). A missing
//! oracle fails the test loudly instead of silently skipping the gate. Python
//! is used only here; the crate never invokes it.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use kala_contracts::{
    ContractModel, DomainPackageManifest, EvaluationResult, StepStatus, WorkflowStatus,
};
use kala_domains::building::{
    BUILDING_PACKAGE, BuildingParameters, BuildingSimulation, simulate_building,
    violations_for as building_violations,
};
use kala_domains::mechanical::{
    BRACKET_SCHEMA_DIGEST, BracketParameters, BracketSimulation, MECHANICAL_PACKAGE,
    MECHANICAL_STEPS, STRUCTURAL_STEP_ID, simulate_bracket,
    violations_for as mechanical_violations,
};
use kala_domains::wing::{
    WING_PACKAGE, WingParameters, WingSimulation, simulate_wing, violations_for as wing_violations,
};
use kala_domains::{
    CapabilityGrantPolicy, CapabilityRequest, CatalogStep, DomainCallError, DomainError,
    DomainEvaluator, DomainName, DomainPackage, DomainPackageApi, DomainPackageParts,
    EvaluationContext, EvaluatorImplementation, ParameterModel, ReferenceWorkflowRun, StepOutcome,
    Timestamp, canonical_json, require_capabilities, run_reference_workflow,
};
use kala_json::CanonicalValue;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SURROGATE_CAPABILITY: &str = "mechanical.surrogate.read";
const PACKAGE_CAPABILITY: &str = "mechanical.reference.run";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    manifest_dir()
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
        "live Python oracle requires {} (set KALA_PYTHON to override)",
        venv.display()
    );
    venv
}

fn run_oracle(request: &Value) -> Value {
    let script = manifest_dir().join("tests/oracle/domains_oracle.py");
    let mut child = Command::new(python_executable())
        .arg(&script)
        .env("PYTHONHASHSEED", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("failed to spawn {}: {error}", script.display()));
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(request.to_string().as_bytes())
        .expect("write oracle input");
    let output = child.wait_with_output().expect("wait for oracle");
    assert!(
        output.status.success(),
        "oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "oracle did not return JSON: {error}\nstdout: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn canonical_from_serde(value: &Value) -> CanonicalValue {
    kala_json::strict_json_loads(value.to_string().as_bytes()).expect("oracle JSON is canonical")
}

fn bits(value: f64) -> String {
    format!("{:016x}", value.to_bits())
}

macro_rules! assert_simulation_bits {
    ($simulation:expr, $oracle:expr, $($field:ident),+ $(,)?) => {
        $(
            assert_eq!(
                bits($simulation.$field),
                $oracle[stringify!($field)]
                    .as_str()
                    .expect("oracle field bits"),
                "simulation field {}",
                stringify!($field)
            );
        )+
    };
}

#[derive(Clone, Debug)]
struct Case {
    domain: DomainName,
    values: Value,
    subject: String,
    evaluated_at: Option<String>,
}

fn context_for(case: &Case) -> EvaluationContext {
    match case.evaluated_at.as_deref() {
        None => EvaluationContext::reference(&case.subject).unwrap(),
        Some(text) => {
            EvaluationContext::new(&case.subject, Timestamp::parse(text).unwrap()).unwrap()
        }
    }
}

fn model_wire(case: &Case) -> (String, CanonicalValue) {
    let value = canonical_from_serde(&case.values);
    match case.domain {
        DomainName::Mechanical => {
            let parameters = BracketParameters::from_value(&value).unwrap();
            (
                parameters.to_wire_json_ascii().unwrap(),
                parameters.to_canonical_value().unwrap(),
            )
        }
        DomainName::Building => {
            let parameters = BuildingParameters::from_value(&value).unwrap();
            (
                parameters.to_wire_json_ascii().unwrap(),
                parameters.to_canonical_value().unwrap(),
            )
        }
        DomainName::Wing => {
            let parameters = WingParameters::from_value(&value).unwrap();
            (
                parameters.to_wire_json_ascii().unwrap(),
                parameters.to_canonical_value().unwrap(),
            )
        }
    }
}

fn simulation_mechanical(case: &Case) -> BracketSimulation {
    simulate_bracket(&BracketParameters::from_value(&canonical_from_serde(&case.values)).unwrap())
}

fn simulation_building(case: &Case) -> BuildingSimulation {
    simulate_building(&BuildingParameters::from_value(&canonical_from_serde(&case.values)).unwrap())
}

fn simulation_wing(case: &Case) -> WingSimulation {
    simulate_wing(&WingParameters::from_value(&canonical_from_serde(&case.values)).unwrap())
}

fn workflow_for(case: &Case) -> ReferenceWorkflowRun {
    let value = canonical_from_serde(&case.values);
    let context = context_for(case);
    match case.domain {
        DomainName::Mechanical => {
            let parameters = BracketParameters::from_value(&value).unwrap();
            run_reference_workflow(
                &MECHANICAL_PACKAGE,
                &context,
                &parameters,
                MECHANICAL_PACKAGE.step_ids(),
                None,
            )
            .unwrap()
        }
        DomainName::Building => {
            let parameters = BuildingParameters::from_value(&value).unwrap();
            run_reference_workflow(
                &BUILDING_PACKAGE,
                &context,
                &parameters,
                BUILDING_PACKAGE.step_ids(),
                None,
            )
            .unwrap()
        }
        DomainName::Wing => {
            let parameters = WingParameters::from_value(&value).unwrap();
            run_reference_workflow(
                &WING_PACKAGE,
                &context,
                &parameters,
                WING_PACKAGE.step_ids(),
                None,
            )
            .unwrap()
        }
    }
}

fn check_evaluation(evaluation: &EvaluationResult, oracle: &Value) {
    assert_eq!(
        evaluation.to_canonical_json().unwrap(),
        oracle["canonical"].as_str().unwrap(),
        "evaluation bytes"
    );
    assert_eq!(
        evaluation.evaluation_id(),
        oracle["evaluation_id"].as_str().unwrap()
    );
    assert_eq!(
        evaluation.subject_id(),
        oracle["subject_id"].as_str().unwrap()
    );
    assert_eq!(evaluation.passed(), oracle["passed"].as_bool().unwrap());
    assert_eq!(
        evaluation.metrics().len(),
        oracle["metrics"].as_array().unwrap().len()
    );
    assert_eq!(
        evaluation.findings().len(),
        oracle["findings"].as_array().unwrap().len()
    );
    assert_eq!(
        oracle["evaluated_at"].as_str().unwrap(),
        evaluation.evaluated_at().to_wire_string()
    );
}

fn check_workflow(run: &ReferenceWorkflowRun, oracle: &Value) {
    assert_eq!(run.status.as_str(), oracle["status"].as_str().unwrap());
    assert_eq!(run.passed(), oracle["passed"].as_bool().unwrap());
    assert_eq!(
        run.blocked_step_id.as_deref(),
        oracle["blocked_step_id"].as_str()
    );
    assert_eq!(
        run.ordered_step_ids,
        oracle["ordered_step_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        run.completed_step_ids,
        oracle["completed_step_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(run.definition_id, oracle["definition_id"].as_str().unwrap());
    assert_eq!(
        run.definition_version,
        oracle["definition_version"].as_str().unwrap()
    );
    assert_eq!(run.package_id, oracle["package_id"].as_str().unwrap());
    assert_eq!(
        run.findings(),
        oracle["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    );
    match (run.score(), oracle["score_bits"].as_str()) {
        (None, None) => {}
        (Some(score), Some(expected)) => assert_eq!(bits(score), expected),
        (left, right) => panic!("score mismatch: {left:?} vs {right:?}"),
    }
    let outcomes = oracle["outcomes"].as_array().unwrap();
    assert_eq!(run.outcomes.len(), outcomes.len());
    for (outcome, expected) in run.outcomes.iter().zip(outcomes) {
        assert_eq!(outcome.step_id, expected["step_id"].as_str().unwrap());
        assert_eq!(
            outcome.status.as_str(),
            expected["status"].as_str().unwrap()
        );
        assert_eq!(outcome.reason.as_deref(), expected["reason"].as_str());
        let evaluation = outcome.evaluation.as_ref().expect("executed outcome");
        check_evaluation(evaluation, &expected["evaluation"]);
    }
}

fn check_package_structures(oracle: &Value) {
    let packages: [(DomainName, &dyn DomainPackageApi); 3] = [
        (DomainName::Mechanical, &*MECHANICAL_PACKAGE),
        (DomainName::Building, &*BUILDING_PACKAGE),
        (DomainName::Wing, &*WING_PACKAGE),
    ];
    for (domain, package) in packages {
        let expected = &oracle["packages"][domain.as_str()];
        assert_eq!(
            package.description(),
            expected["description"].as_str().unwrap()
        );
        assert_eq!(
            package.disclaimer(),
            expected["disclaimer"].as_str().unwrap()
        );
        assert_eq!(
            package.required_capabilities(),
            &expected["required_capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            package.schema_digests(),
            &expected["schema_digests"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            package
                .steps()
                .iter()
                .map(|step| step.step_id().to_owned())
                .collect::<Vec<_>>(),
            expected["step_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        );
        let steps = CanonicalValue::Array(
            package
                .steps()
                .iter()
                .map(CatalogStep::as_canonical_value)
                .collect(),
        );
        assert_eq!(
            canonical_json(&steps).unwrap(),
            canonical_json(&canonical_from_serde(&expected["step_payloads"])).unwrap()
        );
        assert_eq!(
            canonical_json(
                &package
                    .workflow()
                    .to_canonical_value()
                    .expect("workflow canonical value")
            )
            .unwrap(),
            canonical_json(&canonical_from_serde(&expected["workflow_payload"])).unwrap()
        );
        let manifest = package.manifest();
        assert_eq!(manifest.package_id(), package.package_id());
        assert_eq!(manifest.digest().to_hex(), package.digest());
        assert!(
            manifest
                .schema_digests()
                .iter()
                .any(|digest| digest.to_hex() == package.schema_digests()[0])
        );
    }
    assert_eq!(
        &*BRACKET_SCHEMA_DIGEST,
        MECHANICAL_PACKAGE.schema_digests()[0].as_str()
    );
}

fn reference_cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for (domain, values) in [
        (DomainName::Mechanical, json!({})),
        (DomainName::Building, json!({})),
        (DomainName::Wing, json!({})),
    ] {
        cases.push(Case {
            domain,
            values,
            subject: format!("subject-{}", cases.len()),
            evaluated_at: None,
        });
    }
    let boundary = [
        (
            DomainName::Mechanical,
            json!({"thickness_mm": 0.5, "load_kn": 0.0, "hole_count": 0, "minimum_edge_margin_mm": 0.0}),
        ),
        (
            DomainName::Mechanical,
            json!({"thickness_mm": 25.0, "load_kn": 200.0, "hole_count": 12, "hole_diameter_mm": 1.0}),
        ),
        (
            DomainName::Mechanical,
            json!({"thickness_mm": 1.0, "load_kn": 50.0}),
        ),
        (DomainName::Mechanical, json!({"material": "steel-s355"})),
        (
            DomainName::Mechanical,
            json!({"material": "titanium-ti6al4v", "load_points": 1, "hole_count": 3, "hole_diameter_mm": 2.0, "minimum_edge_margin_mm": 3.0}),
        ),
        (
            DomainName::Building,
            json!({"storey_count": 1, "heating_degree_days_k": 1.0, "window_to_wall_ratio": 0.0}),
        ),
        (
            DomainName::Building,
            json!({"storey_count": 200, "heating_degree_days_k": 40000.0, "window_to_wall_ratio": 0.9}),
        ),
        (
            DomainName::Building,
            json!({"wall_conductivity_w_m2k": 0.0, "window_conductivity_w_m2k": 0.0, "roof_conductivity_w_m2k": 0.0}),
        ),
        (
            DomainName::Building,
            json!({"storey_count": 100, "gross_floor_area_m2": 10.0, "minimum_storey_area_m2": 20.0, "maximum_height_m": 400.0}),
        ),
        (
            DomainName::Building,
            json!({"storey_count": 200, "storey_height_m": 6.0, "maximum_height_m": 200.0}),
        ),
        (
            DomainName::Wing,
            json!({"span_m": 1.0, "root_chord_m": 1.0, "tip_chord_m": 0.5, "span_load_kn_m": 0.0, "spanwise_stations": 2}),
        ),
        (
            DomainName::Wing,
            json!({"span_m": 60.0, "root_chord_m": 1.0, "tip_chord_m": 0.5, "span_load_kn_m": 5000.0, "spanwise_stations": 64}),
        ),
        (
            DomainName::Wing,
            json!({"maximum_mass_kg": 5.0, "sweep_deg": -10.0, "dihedral_deg": -20.0}),
        ),
        (
            DomainName::Wing,
            json!({"span_m": 100.0, "root_chord_m": 0.05, "tip_chord_m": 0.01}),
        ),
        (
            DomainName::Wing,
            json!({"span_load_kn_m": 5000.0, "maximum_root_bending_moment_kn_m": 150.0}),
        ),
        (
            DomainName::Mechanical,
            json!({"hole_count": true, "minimum_edge_margin_mm": 0.0}),
        ),
        (
            DomainName::Mechanical,
            json!({"hole_count": "10", "thickness_mm": "8.5", "minimum_edge_margin_mm": "0", "material": "steel-s355"}),
        ),
        (
            DomainName::Building,
            json!({"storey_count": "4", "storey_height_m": "3.0"}),
        ),
        (
            DomainName::Wing,
            json!({"span_m": "12.0", "spanwise_stations": "8"}),
        ),
    ];
    for (domain, values) in boundary {
        cases.push(Case {
            domain,
            values,
            subject: format!("subject-{}", cases.len()),
            evaluated_at: None,
        });
    }
    cases
}

struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
    fn unit(&mut self) -> f64 {
        ((self.next() >> 11) as f64) / ((1u64 << 53) as f64)
    }
    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }
    fn integer(&mut self, low: i64, high: i64) -> i64 {
        low + (self.next() % ((high - low + 1) as u64)) as i64
    }
}

fn random_cases() -> Vec<Case> {
    let mut rng = Lcg::new(0x5eed_d0a3);
    let mut cases = Vec::new();
    for index in 0..15 {
        let material = match index % 3 {
            0 => "aluminium-6061-t6",
            1 => "steel-s355",
            _ => "titanium-ti6al4v",
        };
        let count = rng.integer(0, 6);
        let diameter = if count == 0 { 0.0 } else { rng.range(0.5, 5.0) };
        let mechanical = json!({
            "length_mm": rng.range(20.0, 500.0),
            "width_mm": rng.range(20.0, 300.0),
            "thickness_mm": rng.range(0.5, 25.0),
            "hole_diameter_mm": diameter,
            "hole_count": count,
            "minimum_edge_margin_mm": if count == 0 { 0.0 } else { rng.range(0.0, 5.0) },
            "material": material,
            "load_kn": rng.range(0.0, 200.0),
            "load_points": rng.integer(1, 8),
            "minimum_thickness_mm": rng.range(0.1, 10.0),
            "maximum_stress_mpa": rng.range(1.0, 2000.0),
            "target_safety_factor": rng.range(1.0, 10.0),
        });
        cases.push(Case {
            domain: DomainName::Mechanical,
            values: mechanical,
            subject: format!("random-mechanical-{index}"),
            evaluated_at: (index % 3 == 0)
                .then_some(format!("2026-05-06T07:08:09.{:06}+05:30", index * 1111)),
        });
        let indoor = rng.range(15.0, 30.0);
        let building = json!({
            "storey_count": rng.integer(1, 200),
            "storey_height_m": rng.range(2.0, 6.0),
            "gross_floor_area_m2": rng.range(10.0, 5000.0),
            "perimeter_m": rng.range(5.0, 500.0),
            "wall_conductivity_w_m2k": rng.range(0.0, 5.0),
            "window_conductivity_w_m2k": rng.range(0.0, 6.0),
            "roof_conductivity_w_m2k": rng.range(0.0, 5.0),
            "window_to_wall_ratio": rng.range(0.0, 0.9),
            "heating_degree_days_k": rng.range(1.0, 40000.0),
            "design_indoor_temperature_c": indoor,
            "design_outdoor_temperature_c": rng.range(-40.0, indoor - 0.5),
            "minimum_storey_area_m2": rng.range(1.0, 100.0),
            "maximum_height_m": rng.range(10.0, 400.0),
            "maximum_heating_intensity_kwh_m2": rng.range(10.0, 2000.0),
        });
        cases.push(Case {
            domain: DomainName::Building,
            values: building,
            subject: format!("random-building-{index}"),
            evaluated_at: None,
        });
        let span = rng.range(5.0, 80.0);
        let root = rng.range(0.2, 5.0_f64.min(span));
        let tip = rng.range(0.05, root);
        let wing = json!({
            "span_m": span,
            "root_chord_m": root,
            "tip_chord_m": tip,
            "sweep_deg": rng.range(-10.0, 75.0),
            "dihedral_deg": rng.range(-20.0, 45.0),
            "thickness_ratio": rng.range(0.03, 0.25),
            "spanwise_stations": rng.integer(2, 64),
            "load_case": if index % 2 == 0 { "reference-surface" } else { "symmetric-cruise" },
            "span_load_kn_m": rng.range(0.0, 1000.0),
            "maximum_aspect_ratio": rng.range(2.0, 60.0),
            "maximum_root_bending_moment_kn_m": rng.range(10.0, 5000.0),
            "maximum_mass_kg": rng.range(10.0, 5000.0),
        });
        cases.push(Case {
            domain: DomainName::Wing,
            values: wing,
            subject: format!("random-wing-{index}"),
            evaluated_at: None,
        });
    }
    cases
}

fn invalid_cases() -> Vec<(DomainName, Value)> {
    let mut cases = Vec::new();
    for values in [
        json!({"thickness_mm": 0.1}),
        json!({"thickness_mm": 25.1}),
        json!({"hole_count": 13}),
        json!({"hole_diameter_mm": 50.1}),
        json!({"load_kn": -1.0}),
        json!({"load_points": 9}),
        json!({"maximum_stress_mpa": 0.5}),
        json!({"material": "unobtainium"}),
        json!({"unknown_field": 1.0}),
        json!({"hole_count": 2, "hole_diameter_mm": 0.0}),
        json!({"hole_count": 12, "hole_diameter_mm": 3.0, "length_mm": 20.0}),
        json!({"hole_count": 0, "minimum_edge_margin_mm": 2.0}),
        json!({"hole_count": 2.5}),
        json!({"hole_count": "2.0_0"}),
        json!({"hole_count": "2.5"}),
        json!({"hole_count": "2e0"}),
        json!({"hole_count": "1__0"}),
        json!({"hole_count": "٢"}),
        json!({"thickness_mm": "true"}),
        json!({"thickness_mm": "nan"}),
        json!({"thickness_mm": "inf"}),
        json!({"thickness_mm": "0x10"}),
        json!({"thickness_mm": {"nested": 1}}),
        json!({"hole_count": null}),
        json!({"hole_count": {"nested": 1}}),
        json!({"material": " STEEL-S355 "}),
    ] {
        cases.push((DomainName::Mechanical, values));
    }
    let huge_integer: Value =
        serde_json::from_str(&format!("{{\"thickness_mm\":1{}}}", "0".repeat(400)))
            .expect("arbitrary precision integer");
    cases.push((DomainName::Mechanical, huge_integer));
    for values in [
        json!({"storey_count": 0}),
        json!({"storey_count": 201}),
        json!({"storey_height_m": 1.9}),
        json!({"gross_floor_area_m2": 9.0}),
        json!({"perimeter_m": 4.9}),
        json!({"window_to_wall_ratio": 0.95}),
        json!({"heating_degree_days_k": 0.5}),
        json!({"code_version": "asce-7-2022"}),
        json!({"unexpected": 1}),
        json!({"design_outdoor_temperature_c": 25.0, "design_indoor_temperature_c": 20.0}),
        json!({"storey_count": 4.5}),
        json!({"storey_height_m": "2_1.5"}),
    ] {
        cases.push((DomainName::Building, values));
    }
    for values in [
        json!({"span_m": 0.9}),
        json!({"span_m": 120.1}),
        json!({"tip_chord_m": 12.1}),
        json!({"spanwise_stations": 1}),
        json!({"spanwise_stations": 65}),
        json!({"thickness_ratio": 0.02}),
        json!({"sweep_deg": 75.1}),
        json!({"load_case": "gust"}),
        json!({"nope": 1}),
        json!({"root_chord_m": 1.0, "tip_chord_m": 2.0}),
        json!({"span_m": 1.0, "root_chord_m": 6.0, "tip_chord_m": 1.0}),
        json!({"spanwise_stations": true}),
    ] {
        cases.push((DomainName::Wing, values));
    }
    cases
}

fn check_invalid(case: &Case, oracle: &Value) {
    assert!(
        !oracle["ok"].as_bool().unwrap(),
        "oracle accepted invalid case {:?}",
        case.values
    );
    let result = match case.domain {
        DomainName::Mechanical => {
            BracketParameters::from_value(&canonical_from_serde(&case.values))
                .map(|_| ())
                .map_err(DomainCallError::Parameters)
        }
        DomainName::Building => BuildingParameters::from_value(&canonical_from_serde(&case.values))
            .map(|_| ())
            .map_err(DomainCallError::Parameters),
        DomainName::Wing => WingParameters::from_value(&canonical_from_serde(&case.values))
            .map(|_| ())
            .map_err(DomainCallError::Parameters),
    };
    let error = result.expect_err("native model accepted an invalid case");
    let DomainCallError::Parameters(error) = error else {
        panic!("expected a parameter error");
    };
    let first = &oracle["errors"][0];
    let expected_location = first["loc"]
        .as_array()
        .unwrap()
        .first()
        .and_then(|value| value.as_str());
    assert_eq!(
        error.field_name(),
        expected_location,
        "field mismatch for {:?}",
        case.values
    );
    assert_eq!(
        error.message(),
        first["msg"].as_str().unwrap(),
        "message mismatch for {:?}",
        case.values
    );
}

#[derive(Clone, Copy)]
struct CapabilityCase {
    step_capability: Option<&'static str>,
    package_capability: Option<&'static str>,
    grants: Option<&'static [&'static str]>,
}

const CAPABILITY_CASES: [CapabilityCase; 9] = [
    CapabilityCase {
        step_capability: Some(SURROGATE_CAPABILITY),
        package_capability: None,
        grants: None,
    },
    CapabilityCase {
        step_capability: Some(SURROGATE_CAPABILITY),
        package_capability: None,
        grants: Some(&[]),
    },
    CapabilityCase {
        step_capability: Some(SURROGATE_CAPABILITY),
        package_capability: None,
        grants: Some(&[SURROGATE_CAPABILITY, PACKAGE_CAPABILITY]),
    },
    CapabilityCase {
        step_capability: Some(SURROGATE_CAPABILITY),
        package_capability: None,
        grants: Some(&[SURROGATE_CAPABILITY]),
    },
    CapabilityCase {
        step_capability: None,
        package_capability: None,
        grants: Some(&[SURROGATE_CAPABILITY]),
    },
    CapabilityCase {
        step_capability: None,
        package_capability: Some(PACKAGE_CAPABILITY),
        grants: Some(&[PACKAGE_CAPABILITY]),
    },
    CapabilityCase {
        step_capability: Some(SURROGATE_CAPABILITY),
        package_capability: Some(PACKAGE_CAPABILITY),
        grants: Some(&[SURROGATE_CAPABILITY, PACKAGE_CAPABILITY]),
    },
    CapabilityCase {
        step_capability: Some(SURROGATE_CAPABILITY),
        package_capability: Some(PACKAGE_CAPABILITY),
        grants: Some(&[SURROGATE_CAPABILITY]),
    },
    CapabilityCase {
        step_capability: None,
        package_capability: None,
        grants: Some(&[]),
    },
];

struct RecordingEvaluator {
    capabilities: Vec<String>,
    calls: Arc<AtomicUsize>,
}

impl DomainEvaluator<BracketParameters> for RecordingEvaluator {
    fn step_id(&self) -> &str {
        STRUCTURAL_STEP_ID
    }
    fn required_capabilities(&self) -> &[String] {
        &self.capabilities
    }
    fn implementation(&self) -> Result<EvaluatorImplementation, DomainError> {
        EvaluatorImplementation::new(
            STRUCTURAL_STEP_ID,
            "kala_domains.mechanical.evaluation.RecordingEvaluator",
            "kala_domains.mechanical.evaluation",
            &"0".repeat(64),
            &"1".repeat(64),
            &self
                .capabilities
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )
    }
    fn evaluate(
        &self,
        context: &EvaluationContext,
        parameters: &BracketParameters,
    ) -> Result<EvaluationResult, DomainError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        kala_domains::mechanical::StructuralEvaluator.evaluate(context, parameters)
    }
}

struct RecordingPolicy {
    grants: Vec<String>,
    requests: Mutex<Vec<CapabilityRequest>>,
}

impl CapabilityGrantPolicy for RecordingPolicy {
    fn granted_capabilities(&self, request: &CapabilityRequest) -> Vec<String> {
        self.requests.lock().unwrap().push(request.clone());
        self.grants.clone()
    }
}

fn capability_package(
    case: CapabilityCase,
    calls: Arc<AtomicUsize>,
) -> DomainPackage<BracketParameters> {
    let mut steps = MECHANICAL_STEPS.clone();
    if let Some(capability) = case.step_capability {
        steps = steps
            .into_iter()
            .map(|step| {
                if step.step_id() == STRUCTURAL_STEP_ID {
                    step.with_required_capabilities(&[capability]).unwrap()
                } else {
                    step
                }
            })
            .collect();
    }
    let capabilities: Vec<String> =
        require_capabilities(case.step_capability, "catalog step capability").unwrap();
    let evaluators: Vec<Box<dyn DomainEvaluator<BracketParameters>>> = vec![
        Box::new(kala_domains::mechanical::GeometryEvaluator),
        Box::new(kala_domains::mechanical::MaterialEvaluator),
        Box::new(kala_domains::mechanical::LoadCaseEvaluator),
        Box::new(RecordingEvaluator {
            capabilities: capabilities.clone(),
            calls: calls.clone(),
        }),
    ];
    DomainPackage::new(DomainPackageParts {
        package_id: "mechanical.test-copy".to_owned(),
        version: MECHANICAL_PACKAGE.version().to_owned(),
        description: MECHANICAL_PACKAGE.description().to_owned(),
        disclaimer: MECHANICAL_PACKAGE.disclaimer().to_owned(),
        workflow: MECHANICAL_PACKAGE.workflow().clone(),
        steps,
        evaluators,
        schema_digests: MECHANICAL_PACKAGE.schema_digests().to_vec(),
        required_capabilities: case
            .package_capability
            .into_iter()
            .map(str::to_owned)
            .collect(),
    })
    .unwrap()
}

fn capability_request_payload(request: &CapabilityRequest) -> Value {
    json!({
        "package_id": request.package_id(),
        "step_ids": request.step_ids(),
        "required_capabilities": request.required_capabilities(),
    })
}

#[test]
fn live_python_domain_differential() {
    check_package_structures(&run_oracle(&json!({})));
    let mut cases = reference_cases();
    cases.extend(random_cases());
    let request_cases: Vec<Value> = cases
        .iter()
        .map(|case| {
            json!({
                "domain": case.domain.as_str(),
                "values": &case.values,
                "subject": &case.subject,
                "evaluated_at": case.evaluated_at,
            })
        })
        .collect();
    let invalid = invalid_cases();
    let invalid_requests: Vec<Value> = invalid
        .iter()
        .map(|(domain, values)| json!({"domain": domain.as_str(), "values": values}))
        .collect();
    let capability_requests: Vec<Value> = CAPABILITY_CASES
        .iter()
        .map(|case| {
            json!({
                "step_capability": case.step_capability,
                "package_capability": case.package_capability,
                "grants": case.grants,
            })
        })
        .collect();
    let oracle = run_oracle(&json!({
        "parameter_cases": request_cases,
        "validation_cases": invalid_requests,
        "capability_cases": capability_requests,
    }));
    check_package_structures(&oracle);

    let parameter_results = oracle["parameter_cases"].as_array().unwrap();
    assert_eq!(cases.len(), parameter_results.len());
    for (case, expected) in cases.iter().zip(parameter_results) {
        assert!(
            expected["ok"].as_bool().unwrap(),
            "oracle rejected {:?}: {}",
            case.values,
            expected
        );
        let (wire, canonical) = model_wire(case);
        assert_eq!(wire, expected["wire_json"].as_str().unwrap());
        let canonical_text = canonical_json(&canonical).unwrap();
        let digest = Sha256::digest(canonical_text.as_bytes());
        assert_eq!(
            format!("{digest:x}"),
            expected["canonical_digest"].as_str().unwrap()
        );
        match case.domain {
            DomainName::Mechanical => {
                assert_simulation_bits!(
                    simulation_mechanical(case),
                    expected["simulation"],
                    gross_area_mm2,
                    hole_area_mm2,
                    net_area_mm2,
                    volume_mm3,
                    mass_kg,
                    load_n,
                    load_per_point_n,
                    lever_arm_mm,
                    bending_moment_n_mm,
                    section_modulus_mm3,
                    bending_stress_mpa,
                    shear_stress_mpa,
                    stress_mpa,
                    utilisation,
                    safety_factor,
                    material_web_mm,
                    edge_margin_mm,
                    safety_score,
                );
            }
            DomainName::Building => {
                assert_simulation_bits!(
                    simulation_building(case),
                    expected["simulation"],
                    storey_area_m2,
                    total_floor_area_m2,
                    total_height_m,
                    total_volume_m3,
                    wall_area_m2,
                    window_area_m2,
                    opaque_wall_area_m2,
                    roof_area_m2,
                    conductance_wall_w_k,
                    conductance_window_w_k,
                    conductance_roof_w_k,
                    conductance_total_w_k,
                    conductance_per_floor_area_w_m2_k,
                    temperature_difference_k,
                    transmission_loss_w_k,
                    annual_heating_kwh,
                    heating_intensity_kwh_m2,
                    envelope_score,
                );
            }
            DomainName::Wing => {
                assert_simulation_bits!(
                    simulation_wing(case),
                    expected["simulation"],
                    taper_ratio,
                    reference_area_m2,
                    mean_chord_m,
                    aspect_ratio,
                    structural_volume_m3,
                    mass_kg,
                    span_load_n_m,
                    root_bending_moment_kn_m,
                    lift_curve_slope_per_rad,
                    induced_drag_factor,
                    induced_drag_coefficient,
                    quarter_chord_sweep_rad,
                    dihedral_rad,
                    geometry_score,
                );
            }
        }
        let violations: Vec<&str> = match case.domain {
            DomainName::Mechanical => mechanical_violations(
                &BracketParameters::from_value(&canonical_from_serde(&case.values)).unwrap(),
            )
            .iter()
            .map(|violation| violation.as_str())
            .collect(),
            DomainName::Building => building_violations(
                &BuildingParameters::from_value(&canonical_from_serde(&case.values)).unwrap(),
            )
            .iter()
            .map(|violation| violation.as_str())
            .collect(),
            DomainName::Wing => wing_violations(
                &WingParameters::from_value(&canonical_from_serde(&case.values)).unwrap(),
            )
            .iter()
            .map(|violation| violation.as_str())
            .collect(),
        };
        assert_eq!(
            violations,
            expected["violations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap())
                .collect::<Vec<_>>()
        );
        let context = context_for(case);
        let evaluations = match case.domain {
            DomainName::Mechanical => {
                let parameters =
                    BracketParameters::from_value(&canonical_from_serde(&case.values)).unwrap();
                MECHANICAL_PACKAGE
                    .step_ids()
                    .iter()
                    .map(|step_id| {
                        MECHANICAL_PACKAGE
                            .evaluate(step_id, &context, &parameters)
                            .unwrap()
                    })
                    .collect::<Vec<_>>()
            }
            DomainName::Building => {
                let parameters =
                    BuildingParameters::from_value(&canonical_from_serde(&case.values)).unwrap();
                BUILDING_PACKAGE
                    .step_ids()
                    .iter()
                    .map(|step_id| {
                        BUILDING_PACKAGE
                            .evaluate(step_id, &context, &parameters)
                            .unwrap()
                    })
                    .collect::<Vec<_>>()
            }
            DomainName::Wing => {
                let parameters =
                    WingParameters::from_value(&canonical_from_serde(&case.values)).unwrap();
                WING_PACKAGE
                    .step_ids()
                    .iter()
                    .map(|step_id| {
                        WING_PACKAGE
                            .evaluate(step_id, &context, &parameters)
                            .unwrap()
                    })
                    .collect::<Vec<_>>()
            }
        };
        let expected_evaluations = expected["evaluations"].as_array().unwrap();
        assert_eq!(evaluations.len(), expected_evaluations.len());
        for (evaluation, oracle_evaluation) in evaluations.iter().zip(expected_evaluations) {
            check_evaluation(evaluation, oracle_evaluation);
        }
        check_workflow(&workflow_for(case), &expected["workflow"]);
    }

    let validation_results = oracle["validation_cases"].as_array().unwrap();
    assert_eq!(invalid.len(), validation_results.len());
    for ((domain, values), expected) in invalid.iter().zip(validation_results) {
        check_invalid(
            &Case {
                domain: *domain,
                values: values.clone(),
                subject: "invalid".to_owned(),
                evaluated_at: None,
            },
            expected,
        );
    }

    let capability_results = oracle["capability_cases"].as_array().unwrap();
    assert_eq!(CAPABILITY_CASES.len(), capability_results.len());
    for (case, expected) in CAPABILITY_CASES.iter().zip(capability_results) {
        let calls = Arc::new(AtomicUsize::new(0));
        let package = capability_package(*case, calls.clone());
        let context = EvaluationContext::reference("capability-subject").unwrap();
        let parameters = BracketParameters::default();
        let policy = case.grants.map(|grants| RecordingPolicy {
            grants: grants.iter().map(|value| (*value).to_owned()).collect(),
            requests: Mutex::new(Vec::new()),
        });
        let result = run_reference_workflow(
            &package,
            &context,
            &parameters,
            package.step_ids(),
            policy
                .as_ref()
                .map(|policy| policy as &dyn CapabilityGrantPolicy),
        );
        let called = calls.load(Ordering::SeqCst);
        assert_eq!(
            called,
            expected["calls"].as_array().unwrap().len(),
            "evaluation calls"
        );
        match (result, expected["ok"].as_bool().unwrap()) {
            (Ok(run), true) => {
                assert!(run.passed());
                assert_eq!(
                    policy
                        .as_ref()
                        .map(|policy| policy.requests.lock().unwrap().len())
                        .unwrap_or(0),
                    expected["requests"].as_array().unwrap().len()
                );
                if let Some(policy) = &policy {
                    let requests = policy.requests.lock().unwrap();
                    for (request, expected_request) in requests
                        .iter()
                        .zip(expected["requests"].as_array().unwrap())
                    {
                        assert_eq!(
                            canonical_json(&canonical_from_serde(&capability_request_payload(
                                request
                            )))
                            .unwrap(),
                            canonical_json(&canonical_from_serde(expected_request)).unwrap()
                        );
                    }
                }
            }
            (Err(error), false) => {
                assert_eq!(called, 0, "a denied run must not evaluate");
                assert_eq!(
                    error.class_name(),
                    expected["error_class"].as_str().unwrap()
                );
                assert_eq!(error.message(), expected["message"].as_str().unwrap());
            }
            (result, expected_ok) => {
                panic!("capability decision mismatch: {result:?} vs ok={expected_ok}")
            }
        }
    }
}

#[test]
fn legacy_fixture_and_native_provenance_are_distinct() {
    let fixture_path = manifest_dir().join("../../fixtures/corpus/v1/domain_provenance.json");
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(&fixture_path).expect("read provenance fixture"))
            .expect("provenance fixture JSON");
    assert_eq!(
        fixture["implementation_digest_version"],
        "evaluator-implementation-digest-v2"
    );
    let packages = fixture["packages"].as_array().unwrap();
    let mut by_domain: BTreeMap<&str, &Value> = BTreeMap::new();
    for package in packages {
        by_domain.insert(package["domain"].as_str().unwrap(), package);
    }
    for domain in DomainName::ALL {
        let legacy = by_domain[domain.as_str()];
        let record = kala_domains::provenance_compatibility(domain).unwrap();
        assert_eq!(
            record.legacy_python_package_digest,
            legacy["package_digest"].as_str().unwrap()
        );
        assert_eq!(
            record.legacy_python_schema_digests,
            legacy["schema_digests"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap())
                .collect::<Vec<_>>()
        );
        for digest in &record.implementation_identities {
            let expected = legacy["evaluator_implementations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["step_id"].as_str().unwrap() == digest.step_id)
                .expect("legacy implementation");
            assert_eq!(
                digest.legacy_python_digest,
                expected["digest"].as_str().unwrap()
            );
            assert_ne!(digest.legacy_python_digest, digest.native_rust_digest);
        }
        assert!(record.schema_digests_preserved);
        assert!(!record.implementation_identity_equivalent);
        assert!(record.requires_reparation_and_reapproval);
        assert_ne!(
            record.legacy_python_package_digest,
            record.native_rust_package_digest
        );
        let manifest = kala_domains::package_for(domain).manifest();
        assert_eq!(
            manifest.digest().to_hex(),
            record.native_rust_package_digest
        );
        assert_ne!(
            manifest.digest().to_hex(),
            legacy["manifest"]["digest"].as_str().unwrap()
        );
    }
}

#[test]
fn domain_package_manifest_is_a_contract_manifest() {
    fn assert_manifest(manifest: &DomainPackageManifest) {
        assert!(manifest.digest().to_hex().len() == 64);
        assert!(manifest.signer_id().is_none());
        assert!(manifest.signature_digest().is_none());
    }
    assert_manifest(MECHANICAL_PACKAGE.manifest());
    assert_manifest(BUILDING_PACKAGE.manifest());
    assert_manifest(WING_PACKAGE.manifest());
}

fn model_json_wire(
    domain: DomainName,
    source: &str,
) -> Result<(String, String), kala_domains::ParameterError> {
    fn finish<P: ParameterModel>(parameters: P) -> (String, String) {
        let wire = parameters.to_wire_json().unwrap();
        let canonical = parameters.to_canonical_value().unwrap();
        let digest = Sha256::digest(canonical_json(&canonical).unwrap().as_bytes());
        (wire, format!("{digest:x}"))
    }
    match domain {
        DomainName::Mechanical => BracketParameters::from_json_str(source).map(finish),
        DomainName::Building => BuildingParameters::from_json_str(source).map(finish),
        DomainName::Wing => WingParameters::from_json_str(source).map(finish),
    }
}

fn model_json_cases() -> Vec<(DomainName, String)> {
    let mechanical = [
        r#"{"width_mm":30,"width_mm":40}"#,
        r#"{"width_mm":"30","width_mm":40}"#,
        r#"{"width_mm":{"a":1},"width_mm":40}"#,
        r#"{"width_mm":30,"width\u005fmm":40}"#,
        r#"{"thickness_mm":5,"thickness_mm":"8.5"}"#,
        r#"{"material":"steel-s355","material":"titanium-ti6al4v"}"#,
        r#"{"thickness_mm":5,"thickness_mm":6,"hole_count":2,"hole_count":3}"#,
        r#"{"thickness_mm":5,"thickness\u005fmm":6}"#,
        r#"{"width_mm":30,"\ud83d\ude00":1}"#,
        r#"{"$serde_json::private::Number":"6","thickness_mm":6}"#,
        r#"{"thickness_mm":{"$serde_json::private::Number":"6"}}"#,
        r#"{"thickness_mm":5,"$serde_json::private::Number":"6","$serde_json::private::Number":"7"}"#,
        r#"{"thickness_mm":NaN}"#,
        r#"{"thickness_mm":Infinity}"#,
        r#"{"thickness_mm":-Infinity}"#,
        r#"{"hole_count":1e400}"#,
        r#"{"width_mm":40,"width_mm":{"a":1}}"#,
        r#"{"thickness_mm":5,"thickness_mm":null}"#,
        r#"{"thickness_mm":5,"thickness_mm":"x"}"#,
        r#"{"width_mm":30,"nested":{"a":1,"a":2}}"#,
        r#"{"thickness_mm":"\ud83d\ude00"}"#,
        r#"{"thickness_mm":"\ud800"}"#,
        "[]",
        r#"{"thickness_mm":5,}"#,
        r#"{"thickness_mm":5} trailing"#,
    ];
    let building = [
        r#"{"storey_count":4,"storey_count":5}"#,
        r#"{"storey_height_m":"3.0","storey_height_m":4.0}"#,
        r#"{"gross_floor_area_m2":250.0,"gross_floor_area_m2":"300.0"}"#,
        r#"{"storey_count":4,"storey_count":0}"#,
        r#"{"unexpected":1,"unexpected":2}"#,
    ];
    let wing = [
        r#"{"span_m":10,"span_m":20}"#,
        r#"{"spanwise_stations":8,"spanwise_stations":"10"}"#,
        r#"{"load_case":"reference-surface","load_case":"symmetric-cruise"}"#,
        r#"{"span_m":20,"span_m":0.5}"#,
        r#"{"span_m":20,"span_m":"21.5"}"#,
    ];
    mechanical
        .into_iter()
        .map(|source| (DomainName::Mechanical, source.to_owned()))
        .chain(
            building
                .into_iter()
                .map(|source| (DomainName::Building, source.to_owned())),
        )
        .chain(
            wing.into_iter()
                .map(|source| (DomainName::Wing, source.to_owned())),
        )
        .collect()
}

fn evaluation_with_score(score: Option<f64>, index: usize) -> EvaluationResult {
    let value = serde_json::json!({
        "evaluation_id": format!("evaluation-{index}"),
        "subject_id": "score-audit",
        "passed": true,
        "score": score,
        "metrics": [],
        "findings": [],
        "artifact_ids": [],
        "evaluated_at": "2026-01-01T00:00:00Z",
    });
    EvaluationResult::from_value(&canonical_from_serde(&value)).unwrap()
}

fn run_with_scores(scores: &[Option<f64>]) -> ReferenceWorkflowRun {
    let outcomes: Vec<StepOutcome> = scores
        .iter()
        .enumerate()
        .map(|(index, score)| StepOutcome {
            step_id: format!("step-{index}"),
            status: StepStatus::Succeeded,
            evaluation: Some(evaluation_with_score(*score, index)),
            reason: None,
        })
        .collect();
    ReferenceWorkflowRun {
        package_id: "score.audit".to_owned(),
        definition_id: "score.audit".to_owned(),
        definition_version: "1".to_owned(),
        status: WorkflowStatus::Completed,
        ordered_step_ids: outcomes
            .iter()
            .map(|outcome| outcome.step_id.clone())
            .collect(),
        completed_step_ids: outcomes
            .iter()
            .map(|outcome| outcome.step_id.clone())
            .collect(),
        outcomes,
        blocked_step_id: None,
    }
}

fn score_matrix() -> Vec<Vec<Option<f64>>> {
    let mut cases = vec![
        vec![
            Some(0.356107),
            Some(0.301885),
            Some(0.49567),
            Some(0.042336),
        ],
        vec![
            Some(0.042336),
            Some(0.49567),
            Some(0.301885),
            Some(0.356107),
        ],
        vec![Some(0.1); 10],
        vec![Some(0.0), Some(1.0)],
        vec![Some(0.123456), Some(0.123457)],
        vec![],
        vec![None],
        vec![Some(0.0), None, Some(1.0), None],
        vec![Some(1.0); 37],
    ];
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for length in [1usize, 2, 3, 5, 8, 13, 21, 34, 55, 73] {
        let mut values = Vec::with_capacity(length);
        for index in 0..length {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            if index % 7 == 3 {
                values.push(None);
            } else {
                let value = (state >> 11) as f64 / (1u64 << 53) as f64;
                values.push(Some(value));
            }
        }
        cases.push(values);
    }
    cases
}

fn model_json_bytes_wire(
    domain: DomainName,
    source: &[u8],
) -> Result<String, kala_domains::ParameterError> {
    fn finish<P: ParameterModel>(parameters: P) -> String {
        parameters.to_wire_json().unwrap()
    }
    match domain {
        DomainName::Mechanical => BracketParameters::from_json_bytes(source).map(finish),
        DomainName::Building => BuildingParameters::from_json_bytes(source).map(finish),
        DomainName::Wing => WingParameters::from_json_bytes(source).map(finish),
    }
}

#[test]
fn live_python_model_json_and_score_differential() {
    let model_cases = model_json_cases();
    let score_cases = score_matrix();
    let oracle = run_oracle(&json!({
        "model_json_cases": model_cases
            .iter()
            .map(|(domain, text)| json!({"domain": domain.as_str(), "text": text}))
            .collect::<Vec<_>>(),
        "score_cases": score_cases
            .iter()
            .map(|scores| json!({"scores": scores}))
            .collect::<Vec<_>>(),
    }));
    let model_results = oracle["model_json_cases"].as_array().unwrap();
    assert_eq!(model_cases.len(), model_results.len());
    for ((domain, source), expected) in model_cases.iter().zip(model_results) {
        let native = model_json_wire(*domain, source);
        let bytes_native = model_json_bytes_wire(*domain, source.as_bytes());
        if expected["ok"].as_bool().unwrap() {
            let (wire, digest) = native.unwrap_or_else(|error| {
                panic!("native rejected {source:?}: {error}");
            });
            assert_eq!(wire, expected["wire_json"].as_str().unwrap(), "{source}");
            assert_eq!(
                digest,
                expected["canonical_digest"].as_str().unwrap(),
                "{source}"
            );
            assert_eq!(
                bytes_native.unwrap_or_else(|error| {
                    panic!("native byte entry point rejected {source:?}: {error}")
                }),
                wire,
                "{source}"
            );
        } else {
            let error = native.expect_err("native accepted an oracle-rejected case");
            assert_eq!(
                error.field_name(),
                expected["errors"][0]["loc"]
                    .as_array()
                    .unwrap()
                    .first()
                    .and_then(|value| value.as_str()),
                "{source}"
            );
            if error.field_name().is_some() {
                assert_eq!(
                    error.message(),
                    expected["errors"][0]["msg"].as_str().unwrap(),
                    "{source}"
                );
            } else {
                assert_eq!(
                    error.kind().pydantic_type(),
                    expected["errors"][0]["type"].as_str().unwrap(),
                    "{source}"
                );
            }
            assert!(bytes_native.is_err(), "{source}");
        }
    }

    let score_results = oracle["score_cases"].as_array().unwrap();
    assert_eq!(score_cases.len(), score_results.len());
    for (scores, expected) in score_cases.iter().zip(score_results) {
        let run = run_with_scores(scores);
        match (run.score(), expected["score_bits"].as_str()) {
            (None, None) => {}
            (Some(score), Some(bits)) => {
                assert_eq!(format!("{:016x}", score.to_bits()), bits, "{scores:?}");
            }
            (left, right) => panic!("score mismatch: {left:?} vs {right:?}"),
        }
    }
}
