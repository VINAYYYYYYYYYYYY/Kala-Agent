//! Building domain package: storey, area, and thermal envelope evaluation.

pub mod evaluation;
pub mod parameters;
pub mod simulation;
pub mod workflow;

pub use evaluation::{
    AREA_STEP_ID, AreaModelEvaluator, ConstraintViolation, STOREY_STEP_ID, StoreyModelEvaluator,
    THERMAL_STEP_ID, ThermalEnvelopeEvaluator, placeholder_code_metrics, violation_finding,
    violations_for,
};
pub use parameters::{
    BUILDING_SCHEMA_DIGEST, BuildingCodeVersion, BuildingParameters, CODE_AUTHORITY_FINDING,
    CONDUCTIVITY_UNIT, CUBIC_METRE_UNIT, DEGREE_DAY_UNIT, ENERGY_UNIT, INTENSITY_UNIT, METRE_UNIT,
    NON_AUTHORITATIVE_CODE_VERSION, PARAMETER_SPECS, RATIO_UNIT, SQUARE_METRE_UNIT,
    TEMPERATURE_UNIT, THERMAL_SURROGATE_FINDING, TRANSMITTANCE_UNIT,
};
pub use simulation::{BuildingSimulation, simulate_building};
pub use workflow::{
    BUILDING_PACKAGE, BUILDING_STEPS, BUILDING_WORKFLOW, PACKAGE_DESCRIPTION, PACKAGE_DISCLAIMER,
    PACKAGE_ID, PACKAGE_VERSION, REFERENCE_PARAMETERS, REFERENCE_SUBJECT_ID,
    REFERENCE_WORKFLOW_STEP_IDS, WORKFLOW_DEFINITION_ID, WORKFLOW_NAME, WORKFLOW_VERSION,
    evaluate_building, run_building_reference_workflow,
};
