//! Mechanical domain package: a bracket-like component reference evaluation.

pub mod evaluation;
pub mod parameters;
pub mod simulation;
pub mod workflow;

pub use evaluation::{
    ConstraintViolation, GEOMETRY_STEP_ID, GeometryEvaluator, LOAD_STEP_ID, LoadCaseEvaluator,
    MATERIAL_REFERENCE_FINDING, MATERIAL_STEP_ID, MAX_LOAD_PER_POINT_KN, MaterialEvaluator,
    SAFETY_FACTOR_FINDING, STRUCTURAL_STEP_ID, StructuralEvaluator, violation_finding,
    violations_for,
};
pub use parameters::{
    AREA_UNIT, BRACKET_SCHEMA_DIGEST, BracketParameters, DENSITY_UNIT, EXPANSION_UNIT, FORCE_UNIT,
    ILLUSTRATIVE_MATERIAL_STATUS, LENGTH_UNIT, MASS_UNIT, MATERIALS, MODULUS_UNIT, MOMENT_UNIT,
    MaterialKey, MaterialReference, PARAMETER_NAMES, PARAMETER_SPECS, RATIO_UNIT, STRESS_UNIT,
    VOLUME_UNIT, material_reference, material_reference_for,
};
pub use simulation::{BracketSimulation, simulate_bracket};
pub use workflow::{
    MECHANICAL_PACKAGE, MECHANICAL_STEPS, MECHANICAL_WORKFLOW, PACKAGE_DESCRIPTION,
    PACKAGE_DISCLAIMER, PACKAGE_ID, PACKAGE_VERSION, REFERENCE_PARAMETERS, REFERENCE_SUBJECT_ID,
    REFERENCE_WORKFLOW_STEP_IDS, WORKFLOW_DEFINITION_ID, WORKFLOW_NAME, WORKFLOW_VERSION,
    evaluate_bracket, run_bracket_reference_workflow,
};
