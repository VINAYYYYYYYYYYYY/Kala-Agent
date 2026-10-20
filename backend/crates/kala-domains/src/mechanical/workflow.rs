//! Workflow definition, catalog steps, and reference run for the mechanical
//! package.

use std::sync::LazyLock;

use kala_contracts::{ApprovalKind, ContractModel, EvaluationResult, WorkflowDefinition};
use kala_json::CanonicalValue;

use crate::context::{EvaluationContext, reference_context};
use crate::error::DomainError;
use crate::mechanical::evaluation::{
    GEOMETRY_STEP_ID, GeometryEvaluator, LOAD_STEP_ID, LoadCaseEvaluator, MATERIAL_STEP_ID,
    MaterialEvaluator, STRUCTURAL_STEP_ID, StructuralEvaluator,
};
use crate::mechanical::parameters::{BRACKET_SCHEMA_DIGEST, BracketParameters};
use crate::package::{DomainPackage, DomainPackageParts};
use crate::workflow::{ReferenceWorkflowRun, run_reference_workflow};

/// Package identifier.
pub const PACKAGE_ID: &str = "mechanical.bracket";
/// Package version.
pub const PACKAGE_VERSION: &str = "0.1.0";
/// Package description.
pub const PACKAGE_DESCRIPTION: &str = "Mechanical reference package for a bracket-like component: dimensions, material, and a static load case evaluated by bounded closed-form surrogates.";
/// Package disclaimer.
pub const PACKAGE_DISCLAIMER: &str = "This package performs arithmetic surrogates only. It is not CAD or CAE, not finite element analysis, not a fastener or tolerance analysis, and not a certification of fitness for any purpose.";
/// Workflow definition identifier.
pub const WORKFLOW_DEFINITION_ID: &str = "mechanical.bracket-evaluation";
/// Workflow version.
pub const WORKFLOW_VERSION: &str = "0.1.0";
/// Workflow name.
pub const WORKFLOW_NAME: &str = "Bracket-like component reference evaluation";
/// Reference subject identifier.
pub const REFERENCE_SUBJECT_ID: &str = "bracket-reference";

const SPECIMEN_INPUTS: &[&str] = &[
    "length_mm",
    "width_mm",
    "thickness_mm",
    "hole_diameter_mm",
    "hole_count",
    "material",
];
const LOAD_INPUTS: &[&str] = &["load_kn", "load_points"];
const CONSTRAINT_INPUTS: &[&str] = &[
    "minimum_thickness_mm",
    "maximum_stress_mpa",
    "target_safety_factor",
];

fn transition(
    transition_id: &str,
    from_state: &str,
    to_state: &str,
    step_id: &str,
    checkpoint_type: Option<ApprovalKind>,
) -> CanonicalValue {
    CanonicalValue::object([
        ("transition_id", CanonicalValue::text(transition_id)),
        ("from_state", CanonicalValue::text(from_state)),
        ("to_state", CanonicalValue::text(to_state)),
        ("step_id", CanonicalValue::text(step_id)),
        (
            "checkpoint_type",
            checkpoint_type.map_or(CanonicalValue::Null, |kind| {
                CanonicalValue::text(kind.as_str())
            }),
        ),
    ])
}

fn workflow_definition() -> WorkflowDefinition {
    let payload = CanonicalValue::object([
        (
            "definition_id",
            CanonicalValue::text(WORKFLOW_DEFINITION_ID),
        ),
        ("version", CanonicalValue::text(WORKFLOW_VERSION)),
        ("name", CanonicalValue::text(WORKFLOW_NAME)),
        ("description", CanonicalValue::text(PACKAGE_DISCLAIMER)),
        ("initial_state", CanonicalValue::text("specified")),
        (
            "states",
            CanonicalValue::Array(
                [
                    "specified",
                    "sectioned",
                    "material-graded",
                    "load-cased",
                    "assessed",
                ]
                .into_iter()
                .map(CanonicalValue::text)
                .collect(),
            ),
        ),
        (
            "transitions",
            CanonicalValue::Array(vec![
                transition(
                    "mechanical.section",
                    "specified",
                    "sectioned",
                    GEOMETRY_STEP_ID,
                    None,
                ),
                transition(
                    "mechanical.grade",
                    "sectioned",
                    "material-graded",
                    MATERIAL_STEP_ID,
                    None,
                ),
                transition(
                    "mechanical.load",
                    "material-graded",
                    "load-cased",
                    LOAD_STEP_ID,
                    None,
                ),
                transition(
                    "mechanical.assess",
                    "load-cased",
                    "assessed",
                    STRUCTURAL_STEP_ID,
                    Some(ApprovalKind::PreCommit),
                ),
            ]),
        ),
        (
            "available_step_ids",
            CanonicalValue::Array(
                [
                    GEOMETRY_STEP_ID,
                    MATERIAL_STEP_ID,
                    LOAD_STEP_ID,
                    STRUCTURAL_STEP_ID,
                ]
                .into_iter()
                .map(CanonicalValue::text)
                .collect(),
            ),
        ),
        ("max_replanning_attempts", CanonicalValue::Integer(1.into())),
    ]);
    WorkflowDefinition::from_value(&payload).expect("the declared workflow satisfies the contract")
}

/// The mechanical workflow definition.
pub static MECHANICAL_WORKFLOW: LazyLock<WorkflowDefinition> = LazyLock::new(workflow_definition);

fn step(
    step_id: &str,
    name: &str,
    description: &str,
    inputs: &[&str],
    metrics: &[&str],
    checkpoint_kind: Option<ApprovalKind>,
) -> crate::spec::CatalogStep {
    crate::spec::CatalogStep::new(
        step_id,
        name,
        description,
        inputs,
        metrics,
        &[],
        0.0,
        checkpoint_kind,
        1,
    )
    .expect("declared catalog step")
}

/// The mechanical catalog steps.
pub static MECHANICAL_STEPS: LazyLock<Vec<crate::spec::CatalogStep>> = LazyLock::new(|| {
    vec![
        step(
            GEOMETRY_STEP_ID,
            "Plate section check",
            "Compute the declared plate section and enforce the minimum thickness constraint.",
            SPECIMEN_INPUTS,
            &[
                "plate.length",
                "plate.width",
                "plate.thickness",
                "plate.minimum_thickness",
                "section.net_area",
                "section.volume",
                "mass",
            ],
            None,
        ),
        step(
            MATERIAL_STEP_ID,
            "Declared material check",
            "Report the declared material constants and enforce the hole pattern edge margin constraint.",
            &[
                "material",
                "hole_count",
                "hole_diameter_mm",
                "minimum_edge_margin_mm",
            ],
            &[
                "material.density",
                "material.yield_strength",
                "pattern.edge_margin",
            ],
            None,
        ),
        step(
            LOAD_STEP_ID,
            "Load case check",
            "Report the applied load and enforce the per-point load bound.",
            LOAD_INPUTS,
            &["load.applied", "load.per_point", "load.bending_moment"],
            None,
        ),
        step(
            STRUCTURAL_STEP_ID,
            "Surrogate stress and constraint evaluation",
            "Evaluate the surrogate stress score and enforce the maximum stress and minimum thickness hard constraints.",
            CONSTRAINT_INPUTS,
            &["stress.surrogate", "margin.safety_factor", "score.safety"],
            Some(ApprovalKind::PreCommit),
        ),
    ]
});

/// The mechanical domain package.
pub static MECHANICAL_PACKAGE: LazyLock<DomainPackage<BracketParameters>> = LazyLock::new(|| {
    DomainPackage::new(DomainPackageParts {
        package_id: PACKAGE_ID.to_owned(),
        version: PACKAGE_VERSION.to_owned(),
        description: PACKAGE_DESCRIPTION.to_owned(),
        disclaimer: PACKAGE_DISCLAIMER.to_owned(),
        workflow: MECHANICAL_WORKFLOW.clone(),
        steps: MECHANICAL_STEPS.clone(),
        evaluators: vec![
            Box::new(GeometryEvaluator),
            Box::new(MaterialEvaluator),
            Box::new(LoadCaseEvaluator),
            Box::new(StructuralEvaluator),
        ],
        schema_digests: vec![BRACKET_SCHEMA_DIGEST.clone()],
        required_capabilities: Vec::new(),
    })
    .expect("the mechanical package declaration is internally consistent")
});

/// The reference parameter model.
pub static REFERENCE_PARAMETERS: LazyLock<BracketParameters> =
    LazyLock::new(BracketParameters::default);

/// The reference workflow step order.
pub static REFERENCE_WORKFLOW_STEP_IDS: LazyLock<Vec<String>> =
    LazyLock::new(|| MECHANICAL_PACKAGE.step_ids().to_vec());

/// Run one declared mechanical catalog step against typed parameters.
pub fn evaluate_bracket(
    step_id: &str,
    context: &EvaluationContext,
    parameters: &BracketParameters,
) -> Result<EvaluationResult, DomainError> {
    MECHANICAL_PACKAGE.evaluate(step_id, context, parameters)
}

/// Run the mechanical reference workflow deterministically.
pub fn run_bracket_reference_workflow(
    parameters: Option<&BracketParameters>,
    context: Option<&EvaluationContext>,
) -> Result<ReferenceWorkflowRun, DomainError> {
    let default_context;
    let context = match context {
        Some(context) => context,
        None => {
            default_context = reference_context(REFERENCE_SUBJECT_ID)?;
            &default_context
        }
    };
    let parameters = parameters.unwrap_or(&REFERENCE_PARAMETERS);
    run_reference_workflow(
        &MECHANICAL_PACKAGE,
        context,
        parameters,
        MECHANICAL_PACKAGE.step_ids(),
        None,
    )
}

/// The workflow definition's canonical wire value.
pub fn workflow_payload() -> CanonicalValue {
    MECHANICAL_WORKFLOW
        .to_canonical_value()
        .expect("the workflow is valid")
}
