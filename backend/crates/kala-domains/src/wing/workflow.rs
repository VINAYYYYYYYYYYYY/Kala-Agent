//! Workflow definition, catalog steps, and reference run for the wing package.

use std::sync::LazyLock;

use kala_contracts::{ApprovalKind, ContractModel, EvaluationResult, WorkflowDefinition};
use kala_json::CanonicalValue;

use crate::context::{EvaluationContext, reference_context};
use crate::error::DomainError;
use crate::package::{DomainPackage, DomainPackageParts};
use crate::wing::evaluation::{
    AERODYNAMIC_STEP_ID, AerodynamicSurrogateEvaluator, LOAD_STEP_ID, PLANFORM_STEP_ID,
    PlanformGeometryEvaluator, SpanLoadEvaluator,
};
use crate::wing::parameters::{WING_SCHEMA_DIGEST, WingParameters};
use crate::workflow::{ReferenceWorkflowRun, run_reference_workflow};

/// Package identifier.
pub const PACKAGE_ID: &str = "wing.parametric-tapered";
/// Package version.
pub const PACKAGE_VERSION: &str = "0.1.0";
/// Package description.
pub const PACKAGE_DESCRIPTION: &str = "Wing reference package for span, chord, taper, and span load evaluation using geometric and aerodynamic surrogates that are not certified.";
/// Package disclaimer.
pub const PACKAGE_DISCLAIMER: &str = "This package performs surrogate arithmetic only. It is not a lifting-line or panel method, not CFD, not a structural or fatigue analysis, and it certifies no airworthiness, stability, or control property.";
/// Workflow definition identifier.
pub const WORKFLOW_DEFINITION_ID: &str = "wing.surrogate-evaluation";
/// Workflow version.
pub const WORKFLOW_VERSION: &str = "0.1.0";
/// Workflow name.
pub const WORKFLOW_NAME: &str = "Parametric wing surrogate evaluation";
/// Reference subject identifier.
pub const REFERENCE_SUBJECT_ID: &str = "wing-reference";

const PLANFORM_INPUTS: &[&str] = &[
    "span_m",
    "root_chord_m",
    "tip_chord_m",
    "sweep_deg",
    "dihedral_deg",
    "thickness_ratio",
    "spanwise_stations",
    "maximum_aspect_ratio",
];
const LOAD_INPUTS: &[&str] = &[
    "load_case",
    "span_load_kn_m",
    "maximum_root_bending_moment_kn_m",
];
const AERODYNAMIC_INPUTS: &[&str] = &["span_m", "root_chord_m", "tip_chord_m", "maximum_mass_kg"];

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
                ["specified", "sized", "loaded", "assessed"]
                    .into_iter()
                    .map(CanonicalValue::text)
                    .collect(),
            ),
        ),
        (
            "transitions",
            CanonicalValue::Array(vec![
                transition(
                    "wing.planform",
                    "specified",
                    "sized",
                    PLANFORM_STEP_ID,
                    None,
                ),
                transition("wing.load", "sized", "loaded", LOAD_STEP_ID, None),
                transition(
                    "wing.aerodynamics",
                    "loaded",
                    "assessed",
                    AERODYNAMIC_STEP_ID,
                    Some(ApprovalKind::PreCommit),
                ),
            ]),
        ),
        (
            "available_step_ids",
            CanonicalValue::Array(
                [PLANFORM_STEP_ID, LOAD_STEP_ID, AERODYNAMIC_STEP_ID]
                    .into_iter()
                    .map(CanonicalValue::text)
                    .collect(),
            ),
        ),
        ("max_replanning_attempts", CanonicalValue::Integer(1.into())),
    ]);
    WorkflowDefinition::from_value(&payload).expect("the declared workflow satisfies the contract")
}

/// The wing workflow definition.
pub static WING_WORKFLOW: LazyLock<WorkflowDefinition> = LazyLock::new(workflow_definition);

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

/// The wing catalog steps.
pub static WING_STEPS: LazyLock<Vec<crate::spec::CatalogStep>> = LazyLock::new(|| {
    vec![
        step(
            PLANFORM_STEP_ID,
            "Planform geometry",
            "Integrate the declared chord distribution into a reference area and enforce the declared maximum aspect ratio.",
            PLANFORM_INPUTS,
            &[
                "planform.taper_ratio",
                "planform.reference_area",
                "planform.aspect_ratio",
                "planform.mean_chord",
                "planform.sweep_magnitude",
            ],
            None,
        ),
        step(
            LOAD_STEP_ID,
            "Span load",
            "Integrate the declared span load into a surrogate root bending moment and enforce the declared maximum moment.",
            LOAD_INPUTS,
            &["load.span_load", "load.root_bending_moment"],
            None,
        ),
        step(
            AERODYNAMIC_STEP_ID,
            "Aerodynamic surrogate",
            "Report the finite-wing lift and induced drag surrogates and enforce the declared reference mass bound. Results are not certified.",
            AERODYNAMIC_INPUTS,
            &[
                "aero.lift_curve_slope",
                "aero.induced_drag_factor",
                "aero.quarter_chord_sweep_magnitude",
                "mass.reference",
                "score.geometry",
            ],
            Some(ApprovalKind::PreCommit),
        ),
    ]
});

/// The wing domain package.
pub static WING_PACKAGE: LazyLock<DomainPackage<WingParameters>> = LazyLock::new(|| {
    DomainPackage::new(DomainPackageParts {
        package_id: PACKAGE_ID.to_owned(),
        version: PACKAGE_VERSION.to_owned(),
        description: PACKAGE_DESCRIPTION.to_owned(),
        disclaimer: PACKAGE_DISCLAIMER.to_owned(),
        workflow: WING_WORKFLOW.clone(),
        steps: WING_STEPS.clone(),
        evaluators: vec![
            Box::new(PlanformGeometryEvaluator),
            Box::new(SpanLoadEvaluator),
            Box::new(AerodynamicSurrogateEvaluator),
        ],
        schema_digests: vec![WING_SCHEMA_DIGEST.clone()],
        required_capabilities: Vec::new(),
    })
    .expect("the wing package declaration is internally consistent")
});

/// The reference parameter model.
pub static REFERENCE_PARAMETERS: LazyLock<WingParameters> = LazyLock::new(WingParameters::default);

/// The reference workflow step order.
pub static REFERENCE_WORKFLOW_STEP_IDS: LazyLock<Vec<String>> =
    LazyLock::new(|| WING_PACKAGE.step_ids().to_vec());

/// Run one declared wing catalog step against typed parameters.
pub fn evaluate_wing(
    step_id: &str,
    context: &EvaluationContext,
    parameters: &WingParameters,
) -> Result<EvaluationResult, DomainError> {
    WING_PACKAGE.evaluate(step_id, context, parameters)
}

/// Run the wing reference workflow deterministically.
pub fn run_wing_reference_workflow(
    parameters: Option<&WingParameters>,
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
        &WING_PACKAGE,
        context,
        parameters,
        WING_PACKAGE.step_ids(),
        None,
    )
}

/// The workflow definition's canonical wire value.
pub fn workflow_payload() -> CanonicalValue {
    WING_WORKFLOW
        .to_canonical_value()
        .expect("the workflow is valid")
}
