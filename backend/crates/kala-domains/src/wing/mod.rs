//! Wing domain package: span, chord, taper, and span load evaluation.

pub mod evaluation;
pub mod parameters;
pub mod simulation;
pub mod workflow;

pub use evaluation::{
    AERODYNAMIC_STEP_ID, AerodynamicSurrogateEvaluator, ConstraintViolation, LOAD_STEP_ID,
    PLANFORM_STEP_ID, PlanformGeometryEvaluator, SpanLoadEvaluator, violation_finding,
    violations_for,
};
pub use parameters::{
    ANGLE_UNIT, AREA_UNIT, LENGTH_UNIT, MASS_UNIT, NON_CERTIFIED_FINDING, PARAMETER_SPECS,
    PER_RADIAN_UNIT, RATIO_UNIT, ROOT_MOMENT_UNIT, SPAN_LOAD_UNIT, STATION_UNIT, TAPER_FINDING,
    VOLUME_UNIT, WING_SCHEMA_DIGEST, WingLoadCase, WingParameters,
};
pub use simulation::{WingSimulation, simulate_wing};
pub use workflow::{
    PACKAGE_DESCRIPTION, PACKAGE_DISCLAIMER, PACKAGE_ID, PACKAGE_VERSION, REFERENCE_PARAMETERS,
    REFERENCE_SUBJECT_ID, REFERENCE_WORKFLOW_STEP_IDS, WING_PACKAGE, WING_STEPS, WING_WORKFLOW,
    WORKFLOW_DEFINITION_ID, WORKFLOW_NAME, WORKFLOW_VERSION, evaluate_wing,
    run_wing_reference_workflow,
};
