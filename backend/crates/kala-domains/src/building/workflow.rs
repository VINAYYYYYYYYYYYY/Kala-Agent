//! Workflow definition, catalog steps, and reference run for the building
//! package.

use std::sync::LazyLock;

use kala_contracts::{ApprovalKind, ContractModel, EvaluationResult, WorkflowDefinition};
use kala_json::CanonicalValue;

use crate::building::evaluation::{
    AREA_STEP_ID, AreaModelEvaluator, STOREY_STEP_ID, StoreyModelEvaluator, THERMAL_STEP_ID,
    ThermalEnvelopeEvaluator,
};
use crate::building::parameters::{BUILDING_SCHEMA_DIGEST, BuildingParameters};
use crate::context::{EvaluationContext, reference_context};
use crate::error::DomainError;
use crate::package::{DomainPackage, DomainPackageParts};
use crate::workflow::{ReferenceWorkflowRun, run_reference_workflow};

/// Package identifier.
pub const PACKAGE_ID: &str = "building.storey-envelope";
/// Package version.
pub const PACKAGE_VERSION: &str = "0.1.0";
/// Package description.
pub const PACKAGE_DESCRIPTION: &str = "Building reference package for storey, area, and thermal envelope evaluation using a transmission-only surrogate and a non-authoritative code placeholder.";
/// Package disclaimer.
pub const PACKAGE_DISCLAIMER: &str = "This package performs arithmetic surrogates only. It contains no building code, determines no compliance, no occupancy classification, and no safety approval, and it is not an energy model.";
/// Workflow definition identifier.
pub const WORKFLOW_DEFINITION_ID: &str = "building.envelope-evaluation";
/// Workflow version.
pub const WORKFLOW_VERSION: &str = "0.1.0";
/// Workflow name.
pub const WORKFLOW_NAME: &str = "Storey and thermal envelope reference evaluation";
/// Reference subject identifier.
pub const REFERENCE_SUBJECT_ID: &str = "building-reference";

const STOREY_INPUTS: &[&str] = &["storey_count", "storey_height_m", "maximum_height_m"];
const AREA_INPUTS: &[&str] = &[
    "gross_floor_area_m2",
    "perimeter_m",
    "window_to_wall_ratio",
    "minimum_storey_area_m2",
];
const THERMAL_INPUTS: &[&str] = &[
    "wall_conductivity_w_m2k",
    "window_conductivity_w_m2k",
    "roof_conductivity_w_m2k",
    "heating_degree_days_k",
    "design_indoor_temperature_c",
    "design_outdoor_temperature_c",
    "maximum_heating_intensity_kwh_m2",
    "code_version",
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
                ["specified", "storeyed", "planned", "assessed"]
                    .into_iter()
                    .map(CanonicalValue::text)
                    .collect(),
            ),
        ),
        (
            "transitions",
            CanonicalValue::Array(vec![
                transition(
                    "building.storey",
                    "specified",
                    "storeyed",
                    STOREY_STEP_ID,
                    None,
                ),
                transition("building.area", "storeyed", "planned", AREA_STEP_ID, None),
                transition(
                    "building.envelope",
                    "planned",
                    "assessed",
                    THERMAL_STEP_ID,
                    Some(ApprovalKind::PreCommit),
                ),
            ]),
        ),
        (
            "available_step_ids",
            CanonicalValue::Array(
                [STOREY_STEP_ID, AREA_STEP_ID, THERMAL_STEP_ID]
                    .into_iter()
                    .map(CanonicalValue::text)
                    .collect(),
            ),
        ),
        ("max_replanning_attempts", CanonicalValue::Integer(1.into())),
    ]);
    WorkflowDefinition::from_value(&payload).expect("the declared workflow satisfies the contract")
}

/// The building workflow definition.
pub static BUILDING_WORKFLOW: LazyLock<WorkflowDefinition> = LazyLock::new(workflow_definition);

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

/// The building catalog steps.
pub static BUILDING_STEPS: LazyLock<Vec<crate::spec::CatalogStep>> = LazyLock::new(|| {
    vec![
        step(
            STOREY_STEP_ID,
            "Storey model",
            "Build the storey stack and enforce the declared placeholder height bound.",
            STOREY_INPUTS,
            &[
                "storey.count",
                "storey.height",
                "storey.total_height",
                "storey.total_volume",
            ],
            None,
        ),
        step(
            AREA_STEP_ID,
            "Area model",
            "Derive storey, floor, and envelope areas and enforce the declared placeholder minimum storey area.",
            AREA_INPUTS,
            &[
                "area.storey_gross",
                "area.total_floor",
                "area.wall",
                "area.window",
                "area.roof",
            ],
            None,
        ),
        step(
            THERMAL_STEP_ID,
            "Thermal envelope surrogate",
            "Compute the transmission-only heating surrogate against the declared placeholder intensity bound, with the code reference marked non-authoritative.",
            THERMAL_INPUTS,
            &[
                "envelope.conductance_total",
                "envelope.annual_heating",
                "envelope.heating_intensity",
                "envelope.score",
                "code.version_is_placeholder",
            ],
            Some(ApprovalKind::PreCommit),
        ),
    ]
});

/// The building domain package.
pub static BUILDING_PACKAGE: LazyLock<DomainPackage<BuildingParameters>> = LazyLock::new(|| {
    DomainPackage::new(DomainPackageParts {
        package_id: PACKAGE_ID.to_owned(),
        version: PACKAGE_VERSION.to_owned(),
        description: PACKAGE_DESCRIPTION.to_owned(),
        disclaimer: PACKAGE_DISCLAIMER.to_owned(),
        workflow: BUILDING_WORKFLOW.clone(),
        steps: BUILDING_STEPS.clone(),
        evaluators: vec![
            Box::new(StoreyModelEvaluator),
            Box::new(AreaModelEvaluator),
            Box::new(ThermalEnvelopeEvaluator),
        ],
        schema_digests: vec![BUILDING_SCHEMA_DIGEST.clone()],
        required_capabilities: Vec::new(),
    })
    .expect("the building package declaration is internally consistent")
});

/// The reference parameter model.
pub static REFERENCE_PARAMETERS: LazyLock<BuildingParameters> =
    LazyLock::new(BuildingParameters::default);

/// The reference workflow step order.
pub static REFERENCE_WORKFLOW_STEP_IDS: LazyLock<Vec<String>> =
    LazyLock::new(|| BUILDING_PACKAGE.step_ids().to_vec());

/// Run one declared building catalog step against typed parameters.
pub fn evaluate_building(
    step_id: &str,
    context: &EvaluationContext,
    parameters: &BuildingParameters,
) -> Result<EvaluationResult, DomainError> {
    BUILDING_PACKAGE.evaluate(step_id, context, parameters)
}

/// Run the building reference workflow deterministically.
pub fn run_building_reference_workflow(
    parameters: Option<&BuildingParameters>,
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
        &BUILDING_PACKAGE,
        context,
        parameters,
        BUILDING_PACKAGE.step_ids(),
        None,
    )
}

/// The workflow definition's canonical wire value.
pub fn workflow_payload() -> CanonicalValue {
    BUILDING_WORKFLOW
        .to_canonical_value()
        .expect("the workflow is valid")
}
