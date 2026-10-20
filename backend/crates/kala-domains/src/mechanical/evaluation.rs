//! Bounded evaluators for the mechanical domain package.

use kala_contracts::EvaluationResult;

use crate::context::EvaluationContext;
use crate::error::DomainError;
use crate::mechanical::parameters::{
    AREA_UNIT, BracketParameters, DENSITY_UNIT, EXPANSION_UNIT, FORCE_UNIT, LENGTH_UNIT, MASS_UNIT,
    MATERIALS, MODULUS_UNIT, MOMENT_UNIT, STRESS_UNIT, VOLUME_UNIT, material_reference_for,
};
use crate::mechanical::simulation::{BracketSimulation, simulate_bracket};
use crate::package::DomainEvaluator;
use crate::provenance::EvaluatorImplementation;
use crate::spec::NON_AUTHORITATIVE_FINDING;

/// Geometry catalog step id.
pub const GEOMETRY_STEP_ID: &str = "mechanical.geometry-check";
/// Material catalog step id.
pub const MATERIAL_STEP_ID: &str = "mechanical.material-check";
/// Load catalog step id.
pub const LOAD_STEP_ID: &str = "mechanical.load-check";
/// Structural catalog step id.
pub const STRUCTURAL_STEP_ID: &str = "mechanical.constraint-evaluation";

/// Maximum load per introduction point in kN.
pub const MAX_LOAD_PER_POINT_KN: f64 = 150.0;
/// Material-reference finding.
pub const MATERIAL_REFERENCE_FINDING: &str = "Material constants are illustrative placeholders with a declared reference status; they are not certified material data.";
/// Safety-factor warning finding.
pub const SAFETY_FACTOR_FINDING: &str = "Surrogate safety factor is below the declared target; this is a margin warning, not a hard constraint failure.";

/// Hard constraint identifiers declared by the mechanical package.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConstraintViolation {
    /// Thickness below the declared minimum.
    ThicknessBelowMinimum,
    /// Surrogate stress above the declared maximum.
    StressAboveMaximum,
    /// Edge margin below the declared minimum.
    EdgeMarginBelowMinimum,
    /// Load per point above the package bound.
    LoadPerPointAboveBound,
}

impl ConstraintViolation {
    /// The declared violation identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ThicknessBelowMinimum => "thickness_below_minimum",
            Self::StressAboveMaximum => "stress_above_maximum",
            Self::EdgeMarginBelowMinimum => "edge_margin_below_minimum",
            Self::LoadPerPointAboveBound => "load_per_point_above_bound",
        }
    }

    /// The finding text reported for this violation.
    pub const fn finding(self) -> &'static str {
        match self {
            Self::ThicknessBelowMinimum => {
                "Hard constraint failed: thickness_below_minimum; the plate thickness is below the declared minimum thickness."
            }
            Self::StressAboveMaximum => {
                "Hard constraint failed: stress_above_maximum; the surrogate stress is above the declared maximum stress."
            }
            Self::EdgeMarginBelowMinimum => {
                "Hard constraint failed: edge_margin_below_minimum; the hole pattern leaves less material beside it than the declared minimum edge margin."
            }
            Self::LoadPerPointAboveBound => {
                "Hard constraint failed: load_per_point_above_bound; the load per introduction point is above the declared package bound."
            }
        }
    }
}

/// Return the finding text reported for a hard constraint violation.
pub fn violation_finding(violation: ConstraintViolation) -> &'static str {
    violation.finding()
}

/// Return every hard constraint the given parameters violate, in order.
pub fn violations_for(parameters: &BracketParameters) -> Vec<ConstraintViolation> {
    let simulation = simulate_bracket(parameters);
    violations_for_simulation(parameters, &simulation)
}

fn violations_for_simulation(
    parameters: &BracketParameters,
    simulation: &BracketSimulation,
) -> Vec<ConstraintViolation> {
    let mut violations = Vec::new();
    if parameters.thickness_mm() < parameters.minimum_thickness_mm() {
        violations.push(ConstraintViolation::ThicknessBelowMinimum);
    }
    if simulation.stress_mpa > parameters.maximum_stress_mpa() {
        violations.push(ConstraintViolation::StressAboveMaximum);
    }
    if simulation.edge_margin_mm < parameters.minimum_edge_margin_mm() {
        violations.push(ConstraintViolation::EdgeMarginBelowMinimum);
    }
    if simulation.load_per_point_n > MAX_LOAD_PER_POINT_KN * 1_000.0 {
        violations.push(ConstraintViolation::LoadPerPointAboveBound);
    }
    violations
}

fn implementation_for(
    type_name: &str,
    step_id: &str,
) -> Result<EvaluatorImplementation, DomainError> {
    EvaluatorImplementation::native("mechanical", step_id, type_name, &[])
}

/// Plate section and minimum-thickness evaluation.
#[derive(Clone, Copy, Debug, Default)]
pub struct GeometryEvaluator;

impl DomainEvaluator<BracketParameters> for GeometryEvaluator {
    fn step_id(&self) -> &str {
        GEOMETRY_STEP_ID
    }
    fn required_capabilities(&self) -> &[String] {
        &[]
    }
    fn implementation(&self) -> Result<EvaluatorImplementation, DomainError> {
        implementation_for("GeometryEvaluator", GEOMETRY_STEP_ID)
    }
    fn evaluate(
        &self,
        context: &EvaluationContext,
        parameters: &BracketParameters,
    ) -> Result<EvaluationResult, DomainError> {
        let simulation = simulate_bracket(parameters);
        let thickness_ok = parameters.thickness_mm() >= parameters.minimum_thickness_mm();
        let metrics = vec![
            context.metric(
                "plate.length",
                parameters.length_mm(),
                Some(LENGTH_UNIT),
                None,
            )?,
            context.metric(
                "plate.width",
                parameters.width_mm(),
                Some(LENGTH_UNIT),
                None,
            )?,
            context.metric(
                "plate.thickness",
                parameters.thickness_mm(),
                Some(LENGTH_UNIT),
                Some(thickness_ok),
            )?,
            context.metric(
                "plate.minimum_thickness",
                parameters.minimum_thickness_mm(),
                Some(LENGTH_UNIT),
                None,
            )?,
            context.metric(
                "section.gross_area",
                simulation.gross_area_mm2,
                Some(AREA_UNIT),
                None,
            )?,
            context.metric(
                "section.hole_area",
                simulation.hole_area_mm2,
                Some(AREA_UNIT),
                None,
            )?,
            context.metric(
                "section.net_area",
                simulation.net_area_mm2,
                Some(AREA_UNIT),
                None,
            )?,
            context.metric(
                "section.volume",
                simulation.volume_mm3,
                Some(VOLUME_UNIT),
                None,
            )?,
            context.metric("mass", simulation.mass_kg, Some(MASS_UNIT), None)?,
        ];
        let mut findings = vec![NON_AUTHORITATIVE_FINDING.to_owned()];
        if !thickness_ok {
            findings.push(violation_finding(ConstraintViolation::ThicknessBelowMinimum).to_owned());
        }
        context.result(self.step_id(), thickness_ok, None, &metrics, &findings)
    }
}

/// Declared material constants and the hole-pattern edge margin.
#[derive(Clone, Copy, Debug, Default)]
pub struct MaterialEvaluator;

impl DomainEvaluator<BracketParameters> for MaterialEvaluator {
    fn step_id(&self) -> &str {
        MATERIAL_STEP_ID
    }
    fn required_capabilities(&self) -> &[String] {
        &[]
    }
    fn implementation(&self) -> Result<EvaluatorImplementation, DomainError> {
        implementation_for("MaterialEvaluator", MATERIAL_STEP_ID)
    }
    fn evaluate(
        &self,
        context: &EvaluationContext,
        parameters: &BracketParameters,
    ) -> Result<EvaluationResult, DomainError> {
        let material = material_reference_for(parameters.material())?;
        let simulation = simulate_bracket(parameters);
        let margin_ok = simulation.edge_margin_mm >= parameters.minimum_edge_margin_mm();
        let metrics = vec![
            context.metric(
                "material.density",
                material.density_kg_m3,
                Some(DENSITY_UNIT),
                None,
            )?,
            context.metric(
                "material.yield_strength",
                material.yield_strength_mpa,
                Some(STRESS_UNIT),
                None,
            )?,
            context.metric(
                "material.elastic_modulus",
                material.elastic_modulus_gpa,
                Some(MODULUS_UNIT),
                None,
            )?,
            context.metric(
                "material.thermal_expansion",
                material.thermal_expansion_per_kelvin,
                Some(EXPANSION_UNIT),
                None,
            )?,
            context.metric(
                "material.declared_count",
                MATERIALS.len() as f64,
                None,
                None,
            )?,
            context.metric(
                "pattern.material_web",
                simulation.material_web_mm,
                Some(LENGTH_UNIT),
                None,
            )?,
            context.metric(
                "pattern.edge_margin",
                simulation.edge_margin_mm,
                Some(LENGTH_UNIT),
                Some(margin_ok),
            )?,
            context.metric(
                "pattern.minimum_edge_margin",
                parameters.minimum_edge_margin_mm(),
                Some(LENGTH_UNIT),
                None,
            )?,
        ];
        let mut findings = vec![
            NON_AUTHORITATIVE_FINDING.to_owned(),
            MATERIAL_REFERENCE_FINDING.to_owned(),
        ];
        if !margin_ok {
            findings
                .push(violation_finding(ConstraintViolation::EdgeMarginBelowMinimum).to_owned());
        }
        context.result(self.step_id(), margin_ok, None, &metrics, &findings)
    }
}

/// Applied load and the load introduced at each point.
#[derive(Clone, Copy, Debug, Default)]
pub struct LoadCaseEvaluator;

impl DomainEvaluator<BracketParameters> for LoadCaseEvaluator {
    fn step_id(&self) -> &str {
        LOAD_STEP_ID
    }
    fn required_capabilities(&self) -> &[String] {
        &[]
    }
    fn implementation(&self) -> Result<EvaluatorImplementation, DomainError> {
        implementation_for("LoadCaseEvaluator", LOAD_STEP_ID)
    }
    fn evaluate(
        &self,
        context: &EvaluationContext,
        parameters: &BracketParameters,
    ) -> Result<EvaluationResult, DomainError> {
        let simulation = simulate_bracket(parameters);
        let load_per_point_ok = simulation.load_per_point_n <= MAX_LOAD_PER_POINT_KN * 1_000.0;
        let metrics = vec![
            context.metric("load.applied", parameters.load_kn(), Some("kN"), None)?,
            context.metric(
                "load.applied_newtons",
                simulation.load_n,
                Some(FORCE_UNIT),
                None,
            )?,
            context.metric("load.points", parameters.load_points() as f64, None, None)?,
            context.metric(
                "load.per_point",
                simulation.load_per_point_n,
                Some(FORCE_UNIT),
                Some(load_per_point_ok),
            )?,
            context.metric(
                "load.lever_arm",
                simulation.lever_arm_mm,
                Some(LENGTH_UNIT),
                None,
            )?,
            context.metric(
                "load.bending_moment",
                simulation.bending_moment_n_mm,
                Some(MOMENT_UNIT),
                None,
            )?,
            context.metric(
                "load.bound_per_point",
                MAX_LOAD_PER_POINT_KN,
                Some("kN"),
                None,
            )?,
        ];
        let mut findings = vec![NON_AUTHORITATIVE_FINDING.to_owned()];
        if !load_per_point_ok {
            findings
                .push(violation_finding(ConstraintViolation::LoadPerPointAboveBound).to_owned());
        }
        context.result(self.step_id(), load_per_point_ok, None, &metrics, &findings)
    }
}

/// Surrogate stress evaluation enforcing the declared hard constraints.
#[derive(Clone, Copy, Debug, Default)]
pub struct StructuralEvaluator;

impl DomainEvaluator<BracketParameters> for StructuralEvaluator {
    fn step_id(&self) -> &str {
        STRUCTURAL_STEP_ID
    }
    fn required_capabilities(&self) -> &[String] {
        &[]
    }
    fn implementation(&self) -> Result<EvaluatorImplementation, DomainError> {
        implementation_for("StructuralEvaluator", STRUCTURAL_STEP_ID)
    }
    fn evaluate(
        &self,
        context: &EvaluationContext,
        parameters: &BracketParameters,
    ) -> Result<EvaluationResult, DomainError> {
        let simulation = simulate_bracket(parameters);
        let stress_ok = simulation.stress_mpa <= parameters.maximum_stress_mpa();
        let thickness_ok = parameters.thickness_mm() >= parameters.minimum_thickness_mm();
        let safety_ok = simulation.safety_factor >= parameters.target_safety_factor();
        let metrics = vec![
            context.metric(
                "section.section_modulus",
                simulation.section_modulus_mm3,
                Some("mm^3"),
                None,
            )?,
            context.metric(
                "stress.bending",
                simulation.bending_stress_mpa,
                Some(STRESS_UNIT),
                None,
            )?,
            context.metric(
                "stress.shear",
                simulation.shear_stress_mpa,
                Some(STRESS_UNIT),
                None,
            )?,
            context.metric(
                "stress.surrogate",
                simulation.stress_mpa,
                Some(STRESS_UNIT),
                Some(stress_ok),
            )?,
            context.metric(
                "stress.maximum",
                parameters.maximum_stress_mpa(),
                Some(STRESS_UNIT),
                Some(stress_ok),
            )?,
            context.metric("margin.utilisation", simulation.utilisation, None, None)?,
            context.metric(
                "margin.safety_factor",
                simulation.safety_factor,
                None,
                Some(safety_ok),
            )?,
            context.metric(
                "margin.target_safety_factor",
                parameters.target_safety_factor(),
                None,
                None,
            )?,
            context.metric("score.safety", simulation.safety_score, None, None)?,
        ];
        let mut findings = vec![NON_AUTHORITATIVE_FINDING.to_owned()];
        if !stress_ok {
            findings.push(violation_finding(ConstraintViolation::StressAboveMaximum).to_owned());
        }
        if !thickness_ok {
            findings.push(violation_finding(ConstraintViolation::ThicknessBelowMinimum).to_owned());
        }
        if !safety_ok {
            findings.push(SAFETY_FACTOR_FINDING.to_owned());
        }
        context.result(
            self.step_id(),
            stress_ok && thickness_ok,
            Some(simulation.safety_score),
            &metrics,
            &findings,
        )
    }
}
