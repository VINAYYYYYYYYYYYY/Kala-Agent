//! Native contract tests for declarations, parameters, provenance, capability
//! denial and registry dispatch.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use kala_contracts::{
    ApprovalKind, ContractModel, EvaluationMetric, StepStatus, WorkflowDefinition, WorkflowStatus,
};
use kala_domains::building::{
    BUILDING_PACKAGE, BUILDING_SCHEMA_DIGEST, BUILDING_STEPS, BUILDING_WORKFLOW,
    BuildingParameters, PARAMETER_SPECS as BUILDING_SPECS,
};
use kala_domains::mechanical::{
    BRACKET_SCHEMA_DIGEST, BracketParameters, GEOMETRY_STEP_ID, MATERIALS, MECHANICAL_PACKAGE,
    MECHANICAL_STEPS, MECHANICAL_WORKFLOW, MaterialKey, PARAMETER_SPECS as MECHANICAL_SPECS,
    STRUCTURAL_STEP_ID, material_reference, violations_for,
};
use kala_domains::wing::{
    PARAMETER_SPECS as WING_SPECS, WING_PACKAGE, WING_SCHEMA_DIGEST, WING_STEPS, WING_WORKFLOW,
    WingLoadCase, WingParameters,
};
use kala_domains::{
    CapabilityGrantPolicy, CapabilityRequest, CatalogStep, DomainCallError, DomainError,
    DomainEvaluator, DomainName, DomainPackage, DomainPackageApi, DomainPackageParts,
    DomainParameters, EvaluationContext, EvaluatorImplementation, ParameterDefault, ParameterModel,
    ParameterSpec, clamp_unit_interval, evaluate, manifests, package, package_for,
    reference_context, reference_evaluation_epoch, require_capabilities, require_digest,
    require_identifier, require_name, run_reference_workflow,
};
use kala_json::CanonicalValue;

fn columns(domain: DomainName) -> (Vec<&'static str>, Vec<ParameterSpec>) {
    match domain {
        DomainName::Mechanical => (
            BracketParameters::FIELD_NAMES.to_vec(),
            MECHANICAL_SPECS.clone(),
        ),
        DomainName::Building => (
            BuildingParameters::FIELD_NAMES.to_vec(),
            BUILDING_SPECS.clone(),
        ),
        DomainName::Wing => (WingParameters::FIELD_NAMES.to_vec(), WING_SPECS.clone()),
    }
}

fn mechanical_parts(package_id: &str) -> DomainPackageParts<BracketParameters> {
    DomainPackageParts {
        package_id: package_id.to_owned(),
        version: MECHANICAL_PACKAGE.version().to_owned(),
        description: MECHANICAL_PACKAGE.description().to_owned(),
        disclaimer: MECHANICAL_PACKAGE.disclaimer().to_owned(),
        workflow: MECHANICAL_WORKFLOW.clone(),
        steps: MECHANICAL_STEPS.clone(),
        evaluators: vec![
            Box::new(kala_domains::mechanical::GeometryEvaluator),
            Box::new(kala_domains::mechanical::MaterialEvaluator),
            Box::new(kala_domains::mechanical::LoadCaseEvaluator),
            Box::new(kala_domains::mechanical::StructuralEvaluator),
        ],
        schema_digests: MECHANICAL_PACKAGE.schema_digests().to_vec(),
        required_capabilities: Vec::new(),
    }
}

fn error_class<T>(result: Result<T, DomainError>) -> &'static str {
    match result {
        Ok(_) => panic!("expected a domain error"),
        Err(error) => error.class_name(),
    }
}

#[test]
fn parameter_tables_match_models_and_defaults() {
    let packages: [(DomainName, &dyn DomainPackageApi); 3] = [
        (DomainName::Mechanical, &*MECHANICAL_PACKAGE),
        (DomainName::Building, &*BUILDING_PACKAGE),
        (DomainName::Wing, &*WING_PACKAGE),
    ];
    for (domain, package) in packages {
        let (fields, specs) = columns(domain);
        let names: Vec<&str> = specs.iter().map(ParameterSpec::name).collect();
        assert_eq!(fields, names);
        for spec in &specs {
            assert!(!spec.unit().is_empty());
            assert!(!spec.description().is_empty());
            assert!(spec.minimum().is_finite());
            assert!(spec.maximum().is_finite());
            assert_eq!(
                package.schema_digests(),
                &[match domain {
                    DomainName::Mechanical => &*BRACKET_SCHEMA_DIGEST,
                    DomainName::Building => &*BUILDING_SCHEMA_DIGEST,
                    DomainName::Wing => &*WING_SCHEMA_DIGEST,
                }
                .to_owned()]
            );
        }
        assert_eq!(package.step_ids().len(), package.steps().len());
        assert_eq!(
            package.workflow().available_step_ids().len(),
            package.step_ids().len()
        );
    }
    assert_eq!(
        MECHANICAL_PACKAGE.schema_digests(),
        std::slice::from_ref(&*BRACKET_SCHEMA_DIGEST)
    );
    assert_eq!(
        BUILDING_PACKAGE.schema_digests(),
        std::slice::from_ref(&*BUILDING_SCHEMA_DIGEST)
    );
    assert_eq!(
        WING_PACKAGE.schema_digests(),
        std::slice::from_ref(&*WING_SCHEMA_DIGEST)
    );
    assert_eq!(
        MECHANICAL_STEPS[3].checkpoint_kind(),
        Some(ApprovalKind::PreCommit)
    );
    assert_eq!(
        BUILDING_STEPS[2].checkpoint_kind(),
        Some(ApprovalKind::PreCommit)
    );
    assert_eq!(
        WING_STEPS[2].checkpoint_kind(),
        Some(ApprovalKind::PreCommit)
    );
    assert_eq!(
        MECHANICAL_WORKFLOW.transitions()[3].checkpoint_type(),
        Some(ApprovalKind::PreCommit)
    );
}

#[test]
fn mechanical_parameters_accept_pydantic_coercions_and_reject_boundaries() {
    let default = BracketParameters::default();
    assert_eq!(default, *kala_domains::mechanical::REFERENCE_PARAMETERS);
    for material in ["aluminium-6061-t6", "steel-s355", "titanium-ti6al4v"] {
        let reference = material_reference(material).unwrap();
        assert!(!reference.label.is_empty());
        assert!(reference.density_kg_m3 > 0.0);
        assert!(reference.yield_strength_mpa > 0.0);
        assert!(reference.reference_status.starts_with("illustrative"));
    }
    assert_eq!(MATERIALS.len(), 3);
    assert_eq!(
        material_reference("unobtainium").unwrap_err().message(),
        "material 'unobtainium' is not declared by this package"
    );
    let steel = BracketParameters::from_json_str(
        r#"{"material":"steel-s355","hole_count":"4","thickness_mm":"8.5"}"#,
    )
    .unwrap();
    assert_eq!(steel.material(), MaterialKey::SteelS355);
    assert_eq!(steel.hole_count(), 4);
    assert_eq!(steel.thickness_mm(), 8.5);
    assert_eq!(
        BracketParameters::from_json_str(r#"{"hole_count":true,"minimum_edge_margin_mm":0}"#)
            .unwrap()
            .hole_count(),
        1
    );
    for source in [
        r#"{"thickness_mm":0.1}"#,
        r#"{"thickness_mm":25.1}"#,
        r#"{"hole_count":13}"#,
        r#"{"hole_count":2.5}"#,
        r#"{"hole_count":"2e0"}"#,
        r#"{"hole_count":"2.0_0"}"#,
        r#"{"hole_count":"٢"}"#,
        r#"{"material":"STEEL-S355"}"#,
        r#"{"thickness_mm":"nan"}"#,
        r#"{"thickness_mm":"inf"}"#,
        r#"{"thickness_mm":1e400}"#,
        r#"{"unknown":1}"#,
        r#"{"hole_count":2,"hole_diameter_mm":0}"#,
        r#"{"hole_count":0,"minimum_edge_margin_mm":2}"#,
    ] {
        assert!(
            BracketParameters::from_json_str(source).is_err(),
            "{source}"
        );
    }
    let values = CanonicalValue::object([
        ("thickness_mm", CanonicalValue::Float(f64::NAN)),
        ("load_kn", CanonicalValue::Float(f64::INFINITY)),
    ]);
    assert!(BracketParameters::from_value(&values).is_err());
}

#[test]
fn building_and_wing_parameter_invariants_and_material_variants_are_enforced() {
    let building = BuildingParameters::default();
    assert_eq!(building.storey_count(), 4);
    assert!(
        BuildingParameters::from_json_str(
            r#"{"design_outdoor_temperature_c":25,"design_indoor_temperature_c":20}"#
        )
        .is_err()
    );
    assert!(BuildingParameters::from_json_str(r#"{"window_to_wall_ratio":0.95}"#).is_err());
    assert_eq!(
        BuildingParameters::from_json_str(r#"{"storey_count":"4","storey_height_m":"3.0"}"#)
            .unwrap(),
        building
    );
    let wing = WingParameters::default();
    assert_eq!(wing.load_case(), WingLoadCase::ReferenceSurface);
    assert!(WingParameters::from_json_str(r#"{"root_chord_m":1.0,"tip_chord_m":2.0}"#).is_err());
    assert!(
        WingParameters::from_json_str(r#"{"span_m":1.0,"root_chord_m":6.0,"tip_chord_m":1.0}"#)
            .is_err()
    );
    assert_eq!(
        WingParameters::from_json_str(r#"{"spanwise_stations":"8","span_m":"12.0"}"#).unwrap(),
        wing
    );
    assert_eq!(
        WingLoadCase::parse("symmetric-cruise"),
        Some(WingLoadCase::SymmetricCruise)
    );
    assert_eq!(WingLoadCase::parse("gust"), None);
    assert!(!MATERIALS[1].label.is_empty());
}

#[test]
fn shared_declarations_reject_invalid_names_ranges_and_capabilities() {
    assert_eq!(
        require_name("  plate.length ", "name").unwrap(),
        "plate.length"
    );
    assert!(require_name("Not A Name", "name").is_err());
    assert!(require_identifier("not a subject", "subject_id").is_err());
    assert!(require_digest("not-a-digest", "schema digest").is_err());
    let normalized =
        require_capabilities(["b.capability", "a.capability", "b.capability"], "granted").unwrap();
    assert_eq!(normalized, vec!["a.capability", "b.capability"]);
    assert_eq!(
        require_capabilities(Vec::<&str>::new(), "granted").unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        error_class(
            ParameterSpec::new(
                "width",
                "mm",
                10.0,
                1.0,
                ParameterDefault::Float(0.0),
                "broken"
            )
            .map(|_| ())
        ),
        "DomainParameterError"
    );
    assert!(
        CatalogStep::new(
            "mechanical.broken",
            "Broken",
            "Broken descriptor",
            &["Not A Metric"],
            &[],
            &[],
            0.0,
            None,
            1,
        )
        .is_err()
    );
    assert!(
        CatalogStep::new(
            "mechanical.broken",
            "Broken",
            "Broken descriptor",
            &[],
            &[],
            &[],
            f64::NAN,
            None,
            1,
        )
        .is_err()
    );
    assert!(
        CatalogStep::new(
            "mechanical.broken",
            "Broken",
            "Broken descriptor",
            &[],
            &[],
            &[],
            0.0,
            None,
            0,
        )
        .is_err()
    );
    assert_eq!(
        error_class(kala_domains::parse_checkpoint_kind("export.typo").map(|_| ())),
        "DomainParameterError"
    );
    assert_eq!(clamp_unit_interval(-5.0).unwrap(), 0.0);
    assert_eq!(clamp_unit_interval(5.0).unwrap(), 1.0);
    assert_eq!(clamp_unit_interval(0.1234564).unwrap(), 0.123456);
    assert_eq!(clamp_unit_interval(0.1234565).unwrap(), 0.123456);
    assert_eq!(clamp_unit_interval(0.1234575).unwrap(), 0.123457);
    assert_eq!(clamp_unit_interval(1e-7).unwrap(), 0.0);
    assert_eq!(
        clamp_unit_interval(f64::NEG_INFINITY)
            .unwrap_err()
            .class_name(),
        "DomainParameterError"
    );
    assert_eq!(
        clamp_unit_interval(f64::NAN).unwrap_err().message(),
        "bounded terms must be finite numbers"
    );
}

#[test]
fn packages_expose_declared_steps_evaluators_and_manifests() {
    for package in [
        &*MECHANICAL_PACKAGE as &dyn DomainPackageApi,
        &*BUILDING_PACKAGE,
        &*WING_PACKAGE,
    ] {
        assert_eq!(package.manifest().package_id(), package.package_id());
        assert_eq!(package.manifest().digest().to_hex(), package.digest());
        assert!(package.manifest().schema_digests().len() >= 2);
        let mut implementation_digests: Vec<&str> = package
            .evaluator_implementations()
            .iter()
            .map(|implementation| implementation.digest())
            .collect();
        implementation_digests.sort_unstable();
        let mut manifest_digests: Vec<String> = package
            .manifest()
            .schema_digests()
            .iter()
            .map(|digest| digest.to_hex())
            .collect();
        manifest_digests.sort();
        assert!(manifest_digests.windows(2).all(|pair| pair[0] < pair[1]));
        for digest in implementation_digests {
            assert!(manifest_digests.iter().any(|value| value == digest));
        }
        assert_eq!(
            package.manifest_schema_digests().len(),
            manifest_digests.len()
        );
        assert!(package.required_capabilities().is_empty());
        assert!(package.manifest().required_capabilities().is_empty());
        assert!(package.manifest().signer_id().is_none());
        assert!(package.manifest().signature_digest().is_none());
        assert_eq!(
            package.workflow().definition_id(),
            package.manifest().workflow_definition_ids()[0].as_str()
        );
        assert_eq!(package.steps().len(), package.step_ids().len());
    }
    assert!(MECHANICAL_PACKAGE.manifest().digest().to_hex().len() == 64);
    assert_ne!(
        MECHANICAL_PACKAGE.digest(),
        kala_domains::legacy::MECHANICAL_PACKAGE_DIGEST
    );
    assert_eq!(
        MECHANICAL_PACKAGE
            .evaluator_implementations()
            .iter()
            .map(|item| item.step_id())
            .collect::<Vec<_>>(),
        {
            let mut ids = MECHANICAL_PACKAGE.step_ids().to_vec();
            ids.sort();
            ids
        }
    );
    assert_eq!(
        MECHANICAL_PACKAGE
            .evaluate(
                GEOMETRY_STEP_ID,
                &reference_context("x").unwrap(),
                &BracketParameters::default()
            )
            .unwrap()
            .subject_id(),
        "x"
    );
}

#[derive(Clone)]
struct SubstitutedEvaluator {
    calls: Arc<AtomicUsize>,
}

impl DomainEvaluator<BracketParameters> for SubstitutedEvaluator {
    fn step_id(&self) -> &str {
        STRUCTURAL_STEP_ID
    }
    fn required_capabilities(&self) -> &[String] {
        &[]
    }
    fn implementation(&self) -> Result<EvaluatorImplementation, DomainError> {
        EvaluatorImplementation::new(
            STRUCTURAL_STEP_ID,
            "kala_domains.mechanical.evaluation.SubstitutedEvaluator",
            "kala_domains.mechanical.evaluation",
            &"a".repeat(64),
            &"b".repeat(64),
            &[],
        )
    }
    fn evaluate(
        &self,
        context: &EvaluationContext,
        parameters: &BracketParameters,
    ) -> Result<kala_contracts::EvaluationResult, DomainError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        kala_domains::mechanical::StructuralEvaluator.evaluate(context, parameters)
    }
}

fn substituted_package() -> DomainPackage<BracketParameters> {
    let mut parts = mechanical_parts("mechanical.test-copy");
    parts.evaluators[3] = Box::new(SubstitutedEvaluator {
        calls: Arc::new(AtomicUsize::new(0)),
    });
    DomainPackage::new(parts).unwrap()
}

#[test]
fn evaluator_substitution_changes_identity_but_not_numbers() {
    let substituted = substituted_package();
    assert_ne!(substituted.digest(), MECHANICAL_PACKAGE.digest());
    assert_ne!(
        substituted.manifest().digest().to_hex(),
        MECHANICAL_PACKAGE.manifest().digest().to_hex()
    );
    for implementation in MECHANICAL_PACKAGE.evaluator_implementations() {
        let custom = substituted
            .evaluator_implementations()
            .iter()
            .find(|item| item.step_id() == implementation.step_id())
            .unwrap();
        if implementation.step_id() == STRUCTURAL_STEP_ID {
            assert_ne!(implementation.digest(), custom.digest());
        } else {
            assert_eq!(implementation.digest(), custom.digest());
        }
    }
    let context = reference_context("domain-test-subject").unwrap();
    let parameters = BracketParameters::default();
    assert_eq!(
        substituted
            .evaluate(STRUCTURAL_STEP_ID, &context, &parameters)
            .unwrap()
            .to_canonical_json()
            .unwrap(),
        MECHANICAL_PACKAGE
            .evaluate(STRUCTURAL_STEP_ID, &context, &parameters)
            .unwrap()
            .to_canonical_json()
            .unwrap()
    );
}

#[test]
fn inconsistent_package_declarations_are_refused() {
    assert!(DomainPackage::new(mechanical_parts("mechanical.test-copy")).is_ok());

    let mut no_steps = mechanical_parts("mechanical.test-copy");
    no_steps.steps = Vec::new();
    assert_eq!(
        error_class(DomainPackage::new(no_steps)),
        "DomainPackageError"
    );

    let mut no_evaluators = mechanical_parts("mechanical.test-copy");
    no_evaluators.evaluators = Vec::new();
    assert_eq!(
        error_class(DomainPackage::new(no_evaluators)),
        "DomainPackageError"
    );

    let mut no_schema = mechanical_parts("mechanical.test-copy");
    no_schema.schema_digests = Vec::new();
    assert_eq!(
        error_class(DomainPackage::new(no_schema)),
        "DomainPackageError"
    );

    let mut bad_schema = mechanical_parts("mechanical.test-copy");
    bad_schema.schema_digests = vec!["not-a-digest".to_owned()];
    assert_eq!(
        error_class(DomainPackage::new(bad_schema)),
        "DomainParameterError"
    );

    let mut short_steps = mechanical_parts("mechanical.test-copy");
    short_steps.steps = MECHANICAL_STEPS[..2].to_vec();
    assert_eq!(
        error_class(DomainPackage::new(short_steps)),
        "DomainPackageError"
    );

    let mut short_evaluators = mechanical_parts("mechanical.test-copy");
    short_evaluators.evaluators.truncate(2);
    assert_eq!(
        error_class(DomainPackage::new(short_evaluators)),
        "DomainPackageError"
    );

    let mut bad_package_caps = mechanical_parts("mechanical.test-copy");
    bad_package_caps.required_capabilities = vec!["Not A Capability".to_owned()];
    assert_eq!(
        error_class(DomainPackage::new(bad_package_caps)),
        "DomainParameterError"
    );

    let mut duplicate_steps = mechanical_parts("mechanical.test-copy");
    duplicate_steps.steps = vec![
        MECHANICAL_STEPS[0].clone(),
        MECHANICAL_STEPS[0].clone(),
        MECHANICAL_STEPS[1].clone(),
        MECHANICAL_STEPS[2].clone(),
        MECHANICAL_STEPS[3].clone(),
    ];
    assert_eq!(
        error_class(DomainPackage::new(duplicate_steps)),
        "DomainPackageError"
    );

    let mut duplicate_evaluators = mechanical_parts("mechanical.test-copy");
    duplicate_evaluators
        .evaluators
        .push(Box::new(kala_domains::mechanical::GeometryEvaluator));
    assert_eq!(
        error_class(DomainPackage::new(duplicate_evaluators)),
        "DomainPackageError"
    );

    let mismatch = DomainPackage::new(DomainPackageParts {
        package_id: "mechanical.test-copy".to_owned(),
        version: MECHANICAL_PACKAGE.version().to_owned(),
        description: MECHANICAL_PACKAGE.description().to_owned(),
        disclaimer: MECHANICAL_PACKAGE.disclaimer().to_owned(),
        workflow: MECHANICAL_WORKFLOW.clone(),
        steps: MECHANICAL_STEPS
            .iter()
            .map(|step| {
                if step.step_id() == STRUCTURAL_STEP_ID {
                    step.with_required_capabilities(&["mechanical.surrogate.read"])
                        .unwrap()
                } else {
                    step.clone()
                }
            })
            .collect(),
        evaluators: vec![
            Box::new(kala_domains::mechanical::GeometryEvaluator),
            Box::new(kala_domains::mechanical::MaterialEvaluator),
            Box::new(kala_domains::mechanical::LoadCaseEvaluator),
            Box::new(kala_domains::mechanical::StructuralEvaluator),
        ],
        schema_digests: MECHANICAL_PACKAGE.schema_digests().to_vec(),
        required_capabilities: Vec::new(),
    });
    assert_eq!(
        mismatch.err().expect("capability mismatch").message(),
        "mechanical.constraint-evaluation must declare the same capabilities on its catalog step and on its evaluator"
    );

    assert!(
        MECHANICAL_PACKAGE
            .catalog_step("mechanical.not-declared")
            .is_err()
    );
    assert!(
        MECHANICAL_PACKAGE
            .evaluator("mechanical.not-declared")
            .is_err()
    );
}

#[test]
fn checkpoint_declarations_must_match_the_workflow_transition() {
    fn workflow_with(step_id: &str, checkpoint: Option<ApprovalKind>) -> WorkflowDefinition {
        let mut value = MECHANICAL_WORKFLOW.to_canonical_value().unwrap();
        let CanonicalValue::Object(object) = &mut value else {
            panic!("workflow object");
        };
        let key = kala_json::JsonText::new("transitions");
        let Some(CanonicalValue::Array(transitions)) = object.get_mut(&key) else {
            panic!("transitions array");
        };
        for transition in transitions {
            let CanonicalValue::Object(fields) = transition else {
                panic!("transition object");
            };
            if fields
                .get(&kala_json::JsonText::new("step_id"))
                .and_then(CanonicalValue::as_text)
                .and_then(|text| text.to_utf8().ok())
                .as_deref()
                == Some(step_id)
            {
                fields.insert(
                    kala_json::JsonText::new("checkpoint_type"),
                    checkpoint.map_or(CanonicalValue::Null, |kind| {
                        CanonicalValue::text(kind.as_str())
                    }),
                );
            }
        }
        WorkflowDefinition::from_value(&value).unwrap()
    }
    fn workflow_without_transition(step_id: &str) -> WorkflowDefinition {
        let mut value = MECHANICAL_WORKFLOW.to_canonical_value().unwrap();
        let CanonicalValue::Object(object) = &mut value else {
            panic!("workflow object");
        };
        let key = kala_json::JsonText::new("transitions");
        let Some(CanonicalValue::Array(transitions)) = object.get_mut(&key) else {
            panic!("transitions array");
        };
        transitions.retain(|transition| {
            let CanonicalValue::Object(fields) = transition else {
                return false;
            };
            fields
                .get(&kala_json::JsonText::new("step_id"))
                .and_then(CanonicalValue::as_text)
                .and_then(|text| text.to_utf8().ok())
                .as_deref()
                != Some(step_id)
        });
        // Keep every declared state reachable without a transition for the
        // tested step, so the package reaches the checkpoint check rather than
        // failing workflow validation first.
        transitions.push(CanonicalValue::object([
            ("transition_id", CanonicalValue::text("mechanical.bypass")),
            ("from_state", CanonicalValue::text("load-cased")),
            ("to_state", CanonicalValue::text("assessed")),
            ("step_id", CanonicalValue::text(GEOMETRY_STEP_ID)),
            ("checkpoint_type", CanonicalValue::Null),
        ]));
        WorkflowDefinition::from_value(&value).unwrap()
    }
    let package_with = |workflow: WorkflowDefinition, steps: Vec<CatalogStep>| {
        let mut parts = mechanical_parts("mechanical.test-copy");
        parts.workflow = workflow;
        parts.steps = steps;
        DomainPackage::new(parts)
    };
    assert_eq!(
        package_with(
            workflow_with(STRUCTURAL_STEP_ID, None),
            MECHANICAL_STEPS.clone()
        )
        .err()
        .expect("checkpoint mismatch")
        .message(),
        "mechanical.constraint-evaluation declares a checkpoint kind that its workflow transition does not carry"
    );
    assert_eq!(
        package_with(
            MECHANICAL_WORKFLOW.clone(),
            MECHANICAL_STEPS
                .iter()
                .map(|step| {
                    if step.step_id() == STRUCTURAL_STEP_ID {
                        step.with_checkpoint_kind(None)
                    } else {
                        step.clone()
                    }
                })
                .collect(),
        )
        .err()
        .expect("checkpoint mismatch")
        .message(),
        "mechanical.constraint-evaluation declares a checkpoint kind that its workflow transition does not carry"
    );
    assert_eq!(
        package_with(
            workflow_without_transition(STRUCTURAL_STEP_ID),
            MECHANICAL_STEPS.clone()
        )
        .err()
        .expect("missing transition")
        .message(),
        "mechanical.constraint-evaluation declares a checkpoint that no workflow transition carries"
    );
    for checkpoint in [
        ApprovalKind::PlanApproval,
        ApprovalKind::Export,
        ApprovalKind::Egress,
    ] {
        assert!(
            package_with(
                workflow_with(STRUCTURAL_STEP_ID, Some(checkpoint)),
                MECHANICAL_STEPS.clone()
            )
            .is_err()
        );
        assert!(
            package_with(
                MECHANICAL_WORKFLOW.clone(),
                MECHANICAL_STEPS
                    .iter()
                    .map(|step| {
                        if step.step_id() == GEOMETRY_STEP_ID {
                            step.with_checkpoint_kind(Some(checkpoint))
                        } else {
                            step.clone()
                        }
                    })
                    .collect()
            )
            .is_err()
        );
    }
    let agreeing = package_with(
        workflow_with(GEOMETRY_STEP_ID, Some(ApprovalKind::PlanApproval)),
        MECHANICAL_STEPS
            .iter()
            .map(|step| {
                if step.step_id() == GEOMETRY_STEP_ID {
                    step.with_checkpoint_kind(Some(ApprovalKind::PlanApproval))
                } else {
                    step.clone()
                }
            })
            .collect(),
    )
    .unwrap();
    assert_ne!(agreeing.digest(), MECHANICAL_PACKAGE.digest());
}

#[derive(Default)]
struct GrantPolicy {
    granted: Vec<String>,
    requests: Mutex<Vec<CapabilityRequest>>,
}

impl CapabilityGrantPolicy for GrantPolicy {
    fn granted_capabilities(&self, request: &CapabilityRequest) -> Vec<String> {
        self.requests.lock().unwrap().push(request.clone());
        self.granted.clone()
    }
}

struct RecordingCapabilityEvaluator {
    capabilities: Vec<String>,
    calls: Arc<AtomicUsize>,
}

impl DomainEvaluator<BracketParameters> for RecordingCapabilityEvaluator {
    fn step_id(&self) -> &str {
        STRUCTURAL_STEP_ID
    }
    fn required_capabilities(&self) -> &[String] {
        &self.capabilities
    }
    fn implementation(&self) -> Result<EvaluatorImplementation, DomainError> {
        EvaluatorImplementation::new(
            STRUCTURAL_STEP_ID,
            "kala_domains.mechanical.evaluation.RecordingCapabilityEvaluator",
            "kala_domains.mechanical.evaluation",
            &"c".repeat(64),
            &"d".repeat(64),
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
    ) -> Result<kala_contracts::EvaluationResult, DomainError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        kala_domains::mechanical::StructuralEvaluator.evaluate(context, parameters)
    }
}

#[test]
fn capability_denial_happens_before_any_evaluator() {
    let calls = Arc::new(AtomicUsize::new(0));
    let capable = {
        let mut parts = mechanical_parts("mechanical.test-copy");
        parts.steps = parts
            .steps
            .into_iter()
            .map(|step| {
                if step.step_id() == STRUCTURAL_STEP_ID {
                    step.with_required_capabilities(&["mechanical.surrogate.read"])
                        .unwrap()
                } else {
                    step
                }
            })
            .collect();
        parts.evaluators[3] = Box::new(RecordingCapabilityEvaluator {
            capabilities: vec!["mechanical.surrogate.read".to_owned()],
            calls: calls.clone(),
        });
        DomainPackage::new(parts).unwrap()
    };
    assert_eq!(
        capable
            .catalog_step(STRUCTURAL_STEP_ID)
            .unwrap()
            .required_capabilities(),
        ["mechanical.surrogate.read"]
    );
    assert_eq!(
        capable.evaluator_implementations()[0].required_capabilities(),
        ["mechanical.surrogate.read"]
    );
    assert!(kala_domains::enforce_capability_request(&capable, capable.step_ids(), None).is_err());
    let context = reference_context("capability-subject").unwrap();
    let parameters = BracketParameters::default();
    let denied = run_reference_workflow(&capable, &context, &parameters, capable.step_ids(), None)
        .unwrap_err();
    assert_eq!(denied.class_name(), "DomainCapabilityError");
    assert_eq!(
        denied.message(),
        "mechanical.test-copy declares capabilities that no policy granted: mechanical.surrogate.read"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    for grants in [
        vec![],
        vec![
            "mechanical.surrogate.read".to_owned(),
            "mechanical.extra".to_owned(),
        ],
    ] {
        let policy = GrantPolicy {
            granted: grants,
            requests: Mutex::new(Vec::new()),
        };
        let error = run_reference_workflow(
            &capable,
            &context,
            &parameters,
            capable.step_ids(),
            Some(&policy),
        )
        .unwrap_err();
        assert_eq!(error.class_name(), "DomainCapabilityError");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    let policy = GrantPolicy {
        granted: vec!["mechanical.surrogate.read".to_owned()],
        requests: Mutex::new(Vec::new()),
    };
    let run = run_reference_workflow(
        &capable,
        &context,
        &parameters,
        capable.step_ids(),
        Some(&policy),
    )
    .unwrap();
    assert!(run.passed());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let requests = policy.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].required_capabilities(),
        ["mechanical.surrogate.read"]
    );
    assert_eq!(requests[0].package_id(), "mechanical.test-copy");

    let surplus = run_reference_workflow(
        &MECHANICAL_PACKAGE,
        &context,
        &parameters,
        MECHANICAL_PACKAGE.step_ids(),
        Some(&GrantPolicy {
            granted: vec!["mechanical.extra".to_owned()],
            requests: Mutex::new(Vec::new()),
        }),
    )
    .unwrap_err();
    assert_eq!(
        surplus.message(),
        "the granted capabilities are not the capabilities mechanical.bracket declares: mechanical.extra"
    );
    assert!(CapabilityRequest::new("mechanical.test-copy", &[], &[]).is_err());
    assert!(
        CapabilityRequest::new(
            "mechanical.test-copy",
            &["mechanical.geometry-check".to_owned()],
            &["Not A Capability".to_owned()],
        )
        .is_err()
    );
}

#[test]
fn reference_workflow_validates_steps_and_stops_at_the_first_failure() {
    let package = &*MECHANICAL_PACKAGE;
    let context = reference_context("workflow-subject").unwrap();
    assert_eq!(
        run_reference_workflow(package, &context, &BracketParameters::default(), &[], None)
            .unwrap_err()
            .class_name(),
        "DomainParameterError"
    );
    assert_eq!(
        run_reference_workflow(
            package,
            &context,
            &BracketParameters::default(),
            &[GEOMETRY_STEP_ID.to_owned(), GEOMETRY_STEP_ID.to_owned()],
            None,
        )
        .unwrap_err()
        .message(),
        "reference workflow steps must be unique"
    );
    assert_eq!(
        run_reference_workflow(
            package,
            &context,
            &BracketParameters::default(),
            &["mechanical.not-declared".to_owned()],
            None,
        )
        .unwrap_err()
        .class_name(),
        "DomainStepError"
    );
    let thin = BracketParameters::from_json_str(r#"{"thickness_mm":1.0}"#).unwrap();
    let thin_run =
        run_reference_workflow(package, &context, &thin, package.step_ids(), None).unwrap();
    assert_eq!(thin_run.status, WorkflowStatus::Failed);
    assert!(!thin_run.passed());
    assert_eq!(thin_run.blocked_step_id.as_deref(), Some(GEOMETRY_STEP_ID));
    assert_eq!(thin_run.outcomes.len(), 1);
    assert_eq!(thin_run.outcomes[0].status, StepStatus::Failed);
    assert!(
        thin_run.outcomes[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("hard constraint failure reported by eval.mechanical.geometry-check")
    );
    assert!(
        thin_run
            .findings()
            .iter()
            .any(|finding| finding.contains("thickness_below_minimum"))
    );

    let stressed = BracketParameters::from_json_str(r#"{"load_kn":200.0}"#).unwrap();
    let stressed_run =
        run_reference_workflow(package, &context, &stressed, package.step_ids(), None).unwrap();
    assert_eq!(
        stressed_run.blocked_step_id.as_deref(),
        Some(STRUCTURAL_STEP_ID)
    );
    assert_eq!(stressed_run.outcomes.len(), 4);
    assert_eq!(stressed_run.completed_step_ids.len(), 3);
    assert!(stressed_run.score().is_some());
    assert!(
        violations_for(&stressed)
            .iter()
            .any(|violation| violation.as_str() == "stress_above_maximum")
    );

    let unloaded = BracketParameters::from_json_str(r#"{"load_kn":0.0}"#).unwrap();
    let unloaded_run =
        run_reference_workflow(package, &context, &unloaded, package.step_ids(), None).unwrap();
    assert!(unloaded_run.passed());
    assert_eq!(unloaded_run.score(), Some(1.0));
    assert_eq!(
        unloaded_run
            .evaluations()
            .last()
            .unwrap()
            .metrics()
            .iter()
            .find(|metric| metric.name() == "margin.safety_factor")
            .unwrap()
            .value(),
        1.0e6
    );
}

#[test]
fn evaluation_context_is_utc_normalized_and_ids_are_deterministic() {
    let epoch = reference_evaluation_epoch();
    assert_eq!(epoch.to_wire_string(), "2026-01-01T00:00:00Z");
    let context = EvaluationContext::new(
        "  subject.one  ",
        kala_domains::Timestamp::parse("2026-05-06T07:08:09.123456+05:30").unwrap(),
    )
    .unwrap();
    assert_eq!(context.subject_id(), "subject.one");
    assert_eq!(
        context.evaluated_at().to_wire_string(),
        "2026-05-06T01:38:09.123456Z"
    );
    assert_eq!(
        context.evaluation_id("mechanical.geometry-check").unwrap(),
        "eval.mechanical.geometry-check"
    );
    let custom = EvaluationContext::with_prefix("subject", epoch.clone(), "case").unwrap();
    assert_eq!(
        custom.evaluation_id("mechanical.geometry-check").unwrap(),
        "case.mechanical.geometry-check"
    );
    assert!(EvaluationContext::new("not a subject", epoch).is_err());

    let metric = context
        .metric("plate.length", 120.0, Some("mm"), None)
        .unwrap();
    assert_eq!(metric.value(), 120.0);
    assert_eq!(metric.name(), "plate.length");
    assert_eq!(metric.unit().unwrap().as_str(), Some("mm"));
    let result = context
        .result(
            "mechanical.geometry-check",
            true,
            Some(1.0),
            &[metric],
            &["finding one".to_owned()],
        )
        .unwrap();
    assert_eq!(result.evaluation_id(), "eval.mechanical.geometry-check");
    assert_eq!(result.subject_id(), "subject.one");
    assert_eq!(
        result.evaluated_at().to_wire_string(),
        "2026-05-06T01:38:09.123456Z"
    );
    assert!(
        context
            .metric("negative", -1.0, None, None)
            .unwrap_err()
            .class_name()
            .eq("DomainPackageError")
    );
    let duplicate_metric = context.metric("same", 1.0, None, None).unwrap();
    assert!(
        context
            .result(
                "mechanical.geometry-check",
                true,
                None,
                &[duplicate_metric.clone(), duplicate_metric],
                &[],
            )
            .is_err()
    );
    assert!(
        context
            .result(
                "mechanical.geometry-check",
                true,
                None,
                &[],
                &["same".to_owned(), "same".to_owned()],
            )
            .is_err()
    );
    assert!(
        context
            .result("mechanical.geometry-check", true, Some(2.0), &[], &[])
            .is_err()
    );
}

#[test]
fn registry_exposes_all_three_domains_and_default_dispatch() {
    assert_eq!(DomainName::ALL.len(), 3);
    assert_eq!(DomainName::parse("building"), Some(DomainName::Building));
    assert_eq!(DomainName::parse("absent"), None);
    let names: Vec<&str> = kala_domains::packages()
        .iter()
        .map(|package| package.domain())
        .collect();
    assert_eq!(names, vec!["building", "mechanical", "wing"]);
    assert_eq!(manifests().len(), 3);
    assert_eq!(
        package("mechanical").unwrap().package_id(),
        "mechanical.bracket"
    );
    assert!(package("absent").is_none());
    let context = reference_context("registry-subject").unwrap();
    for domain in DomainName::ALL {
        let parameters = DomainParameters::defaults(domain);
        let canonical = parameters.to_wire_value().to_canonical_value().unwrap();
        let result = evaluate(
            domain,
            package_for(domain).step_ids()[0].as_str(),
            &context,
            &canonical,
        )
        .unwrap();
        assert_eq!(result.subject_id(), "registry-subject");
        assert_eq!(
            kala_domains::evaluate_typed(
                &parameters,
                package_for(domain).step_ids()[0].as_str(),
                &context
            )
            .unwrap()
            .to_canonical_json()
            .unwrap(),
            result.to_canonical_json().unwrap()
        );
    }
    let bad = DomainParameters::from_value(
        DomainName::Mechanical,
        &CanonicalValue::object([("unknown", CanonicalValue::Bool(true))]),
    );
    assert!(matches!(bad, Err(DomainCallError::Parameters(_))));
    assert!(matches!(
        evaluate(
            DomainName::Mechanical,
            "mechanical.not-declared",
            &context,
            &CanonicalValue::Object(Default::default())
        ),
        Err(DomainCallError::Domain(DomainError::Step(_)))
    ));
    assert_eq!(
        BUILDING_PACKAGE.workflow().definition_id(),
        BUILDING_WORKFLOW.definition_id()
    );
    assert_eq!(
        WING_PACKAGE.workflow().definition_id(),
        WING_WORKFLOW.definition_id()
    );
    assert_eq!(kala_domains::NATIVE_IMPLEMENTATION_LANGUAGE, "rust");
}

#[test]
fn unused_helper_types_are_still_publicly_constructible() {
    let metric = EvaluationMetric::from_value(&CanonicalValue::object([
        ("name", CanonicalValue::text("a.metric")),
        ("value", CanonicalValue::Integer(1.into())),
        ("unit", CanonicalValue::Null),
        ("passed", CanonicalValue::Null),
    ]))
    .unwrap();
    assert_eq!(metric.name(), "a.metric");
    let request = CapabilityRequest::new(
        "mechanical.test-copy",
        &["mechanical.geometry-check".to_owned()],
        &[],
    )
    .unwrap();
    assert!(request.step_ids().len() == 1);
}

#[test]
fn serializer_tag_shaped_keys_and_arbitrary_integers_are_plain_data() {
    let tagged_key =
        BracketParameters::from_json_str(r#"{"$serde_json::private::Number":1}"#).unwrap_err();
    assert_eq!(
        tagged_key.field_name(),
        Some("$serde_json::private::Number")
    );
    assert_eq!(
        tagged_key.kind(),
        kala_domains::ParameterErrorKind::ExtraForbidden
    );
    let tagged_value = CanonicalValue::object([(
        "thickness_mm",
        CanonicalValue::object([("$serde_json::private::Number", CanonicalValue::text("8.0"))]),
    )]);
    let tagged_value = BracketParameters::from_value(&tagged_value).unwrap_err();
    assert_eq!(tagged_value.field_name(), Some("thickness_mm"));
    assert_eq!(tagged_value.message(), "Input should be a valid number");

    let not_finite = CanonicalValue::object([("hole_count", CanonicalValue::Float(f64::NAN))]);
    let not_finite = BracketParameters::from_value(&not_finite).unwrap_err();
    assert_eq!(not_finite.field_name(), Some("hole_count"));
    assert_eq!(
        not_finite.kind(),
        kala_domains::ParameterErrorKind::FiniteNumber
    );
    assert_eq!(not_finite.message(), "Input should be a finite number");

    let huge = CanonicalValue::Integer(num_bigint::BigInt::from(10).pow(400));
    let object = CanonicalValue::object([("hole_count", huge.clone())]);
    let error = BracketParameters::from_value(&object).unwrap_err();
    assert_eq!(error.field_name(), Some("hole_count"));
    assert_eq!(error.message(), "Input should be less than or equal to 12");

    let huge_float = CanonicalValue::object([("thickness_mm", huge.clone())]);
    let huge_float = BracketParameters::from_value(&huge_float).unwrap_err();
    assert_eq!(huge_float.field_name(), Some("thickness_mm"));
    assert_eq!(huge_float.message(), "Input should be a valid number");

    let canonical = CanonicalValue::object([("big", huge)]);
    let digest = kala_domains::canonical_digest(&canonical).unwrap();
    assert_eq!(digest.len(), 64);
    assert!(
        kala_domains::canonical_json(&canonical)
            .unwrap()
            .contains("100000000000000000000000000000000000000000")
    );
}

#[test]
fn model_json_input_matches_pydantic_duplicate_and_surrogate_rules() {
    // Duplicate declared keys use last-wins, through both entry points.
    let duplicate = BracketParameters::from_json_str(r#"{"width_mm":30,"width_mm":40}"#).unwrap();
    assert_eq!(duplicate.width_mm(), 40.0);
    assert_eq!(
        BracketParameters::from_json_bytes(br#"{"width_mm":"30","width_mm":"40"}"#)
            .unwrap()
            .width_mm(),
        40.0
    );
    assert_eq!(
        BracketParameters::from_json_str(r#"{"thickness_mm":{"nested":1},"thickness_mm":8.5}"#)
            .unwrap()
            .thickness_mm(),
        8.5
    );

    // Escaped spellings of the same key collapse to the same entry.
    assert_eq!(
        BracketParameters::from_json_str(r#"{"width_mm":30,"width\u005fmm":40}"#)
            .unwrap()
            .width_mm(),
        40.0
    );
    assert_eq!(
        BuildingParameters::from_json_str(r#"{"storey_count":4,"storey_count":5}"#)
            .unwrap()
            .storey_count(),
        5
    );
    assert_eq!(
        WingParameters::from_json_str(r#"{"span_m":10,"span_m":"20"}"#)
            .unwrap()
            .span_m(),
        20.0
    );

    // Surrogate pairs are code points, and the ordinary extra-field contract
    // names the decoded key. Lone surrogate escapes are invalid JSON.
    let surrogate =
        BracketParameters::from_json_str(r#"{"width_mm":30,"\ud83d\ude00":1}"#).unwrap_err();
    assert_eq!(surrogate.field_name(), Some("😀"));
    assert_eq!(surrogate.message(), "Extra inputs are not permitted");
    let lone = BracketParameters::from_json_str(r#"{"thickness_mm":"\ud800"}"#).unwrap_err();
    assert_eq!(lone.field_name(), None);
    assert_eq!(lone.kind(), kala_domains::ParameterErrorKind::JsonInvalid);

    // Serializer-tag-shaped keys and nested objects remain plain data.
    let private = BracketParameters::from_json_str(
        r#"{"$serde_json::private::Number":"6","thickness_mm":6}"#,
    )
    .unwrap_err();
    assert_eq!(private.field_name(), Some("$serde_json::private::Number"));
    let tagged = BracketParameters::from_json_str(
        r#"{"thickness_mm":{"$serde_json::private::Number":"6"}}"#,
    )
    .unwrap_err();
    assert_eq!(tagged.field_name(), Some("thickness_mm"));
    let nested =
        BracketParameters::from_json_str(r#"{"width_mm":30,"nested":{"a":1,"a":2}}"#).unwrap_err();
    assert_eq!(nested.field_name(), Some("nested"));

    // Non-finite literals are accepted by Pydantic's JSON parser and then
    // rejected by the declared field range, not by the parser.
    for source in [
        r#"{"thickness_mm":NaN}"#,
        r#"{"thickness_mm":Infinity}"#,
        r#"{"thickness_mm":1e400}"#,
    ] {
        let error = BracketParameters::from_json_str(source).unwrap_err();
        assert_eq!(error.field_name(), Some("thickness_mm"));
        assert_eq!(error.message(), "Input should be less than or equal to 25");
    }
    let negative = BracketParameters::from_json_str(r#"{"thickness_mm":-Infinity}"#).unwrap_err();
    assert_eq!(
        negative.message(),
        "Input should be greater than or equal to 0.5"
    );
}
