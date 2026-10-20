//! Bounded evaluators for the wing domain package.

use kala_contracts::EvaluationResult;

use crate::context::EvaluationContext;
use crate::error::DomainError;
use crate::package::DomainEvaluator;
use crate::provenance::EvaluatorImplementation;
use crate::wing::parameters::{
    ANGLE_UNIT, AREA_UNIT, LENGTH_UNIT, MASS_UNIT, NON_CERTIFIED_FINDING, PER_RADIAN_UNIT,
    ROOT_MOMENT_UNIT, SPAN_LOAD_UNIT, STATION_UNIT, TAPER_FINDING, VOLUME_UNIT, WingParameters,
};
use crate::wing::simulation::simulate_wing;

/// Planform catalog step id.
pub const PLANFORM_STEP_ID: &str = "wing.planform-geometry";
/// Load catalog step id.
pub const LOAD_STEP_ID: &str = "wing.span-load";
/// Aerodynamic catalog step id.
pub const AERODYNAMIC_STEP_ID: &str = "wing.aerodynamic-surrogate";

/// Hard constraint identifiers declared by the wing package.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConstraintViolation {
    /// Aspect ratio above the declared maximum.
    AspectRatioAboveMaximum,
    /// Root bending moment above the declared maximum.
    RootBendingMomentAboveMaximum,
    /// Mass above the declared maximum.
    MassAboveMaximum,
}

impl ConstraintViolation {
    /// The declared violation identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AspectRatioAboveMaximum => "aspect_ratio_above_maximum",
            Self::RootBendingMomentAboveMaximum => "root_bending_moment_above_maximum",
            Self::MassAboveMaximum => "mass_above_maximum",
        }
    }

    /// The finding text reported for this violation.
    pub const fn finding(self) -> &'static str {
        match self {
            Self::AspectRatioAboveMaximum => {
                "Hard constraint failed: aspect_ratio_above_maximum; the reference aspect ratio is above the declared maximum."
            }
            Self::RootBendingMomentAboveMaximum => {
                "Hard constraint failed: root_bending_moment_above_maximum; the surrogate root bending moment is above the declared maximum."
            }
            Self::MassAboveMaximum => {
                "Hard constraint failed: mass_above_maximum; the reference mass is above the declared maximum."
            }
        }
    }
}

/// Return the finding text reported for a hard constraint violation.
pub fn violation_finding(violation: ConstraintViolation) -> &'static str {
    violation.finding()
}

/// Return every hard constraint the given parameters violate, in order.
pub fn violations_for(parameters: &WingParameters) -> Vec<ConstraintViolation> {
    let simulation = simulate_wing(parameters);
    let mut violations = Vec::new();
    if simulation.aspect_ratio > parameters.maximum_aspect_ratio() {
        violations.push(ConstraintViolation::AspectRatioAboveMaximum);
    }
    if simulation.root_bending_moment_kn_m > parameters.maximum_root_bending_moment_kn_m() {
        violations.push(ConstraintViolation::RootBendingMomentAboveMaximum);
    }
    if simulation.mass_kg > parameters.maximum_mass_kg() {
        violations.push(ConstraintViolation::MassAboveMaximum);
    }
    violations
}

fn implementation_for(
    type_name: &str,
    step_id: &str,
) -> Result<EvaluatorImplementation, DomainError> {
    EvaluatorImplementation::native("wing", step_id, type_name, &[])
}

/// Span, chord, taper, and reference aspect ratio against the declared bound.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlanformGeometryEvaluator;

impl DomainEvaluator<WingParameters> for PlanformGeometryEvaluator {
    fn step_id(&self) -> &str {
        PLANFORM_STEP_ID
    }
    fn required_capabilities(&self) -> &[String] {
        &[]
    }
    fn implementation(&self) -> Result<EvaluatorImplementation, DomainError> {
        implementation_for("PlanformGeometryEvaluator", PLANFORM_STEP_ID)
    }
    fn evaluate(
        &self,
        context: &EvaluationContext,
        parameters: &WingParameters,
    ) -> Result<EvaluationResult, DomainError> {
        let simulation = simulate_wing(parameters);
        let aspect_ok = simulation.aspect_ratio <= parameters.maximum_aspect_ratio();
        let metrics = vec![
            context.metric(
                "planform.span",
                parameters.span_m(),
                Some(LENGTH_UNIT),
                None,
            )?,
            context.metric(
                "planform.root_chord",
                parameters.root_chord_m(),
                Some(LENGTH_UNIT),
                None,
            )?,
            context.metric(
                "planform.tip_chord",
                parameters.tip_chord_m(),
                Some(LENGTH_UNIT),
                None,
            )?,
            context.metric("planform.taper_ratio", simulation.taper_ratio, None, None)?,
            context.metric(
                "planform.mean_chord",
                simulation.mean_chord_m,
                Some(LENGTH_UNIT),
                None,
            )?,
            context.metric(
                "planform.reference_area",
                simulation.reference_area_m2,
                Some(AREA_UNIT),
                None,
            )?,
            context.metric(
                "planform.aspect_ratio",
                simulation.aspect_ratio,
                None,
                Some(aspect_ok),
            )?,
            context.metric(
                "planform.maximum_aspect_ratio",
                parameters.maximum_aspect_ratio(),
                None,
                Some(aspect_ok),
            )?,
            context.metric(
                "planform.sweep_magnitude",
                parameters.sweep_deg().abs(),
                Some(ANGLE_UNIT),
                None,
            )?,
            context.metric(
                "planform.dihedral_magnitude",
                parameters.dihedral_deg().abs(),
                Some(ANGLE_UNIT),
                None,
            )?,
            context.metric(
                "planform.thickness_ratio",
                parameters.thickness_ratio(),
                None,
                None,
            )?,
            context.metric(
                "planform.reference_volume",
                simulation.structural_volume_m3,
                Some(VOLUME_UNIT),
                None,
            )?,
            context.metric(
                "planform.spanwise_stations",
                parameters.spanwise_stations() as f64,
                Some(STATION_UNIT),
                None,
            )?,
        ];
        let mut findings = vec![NON_CERTIFIED_FINDING.to_owned(), TAPER_FINDING.to_owned()];
        if !aspect_ok {
            findings
                .push(violation_finding(ConstraintViolation::AspectRatioAboveMaximum).to_owned());
        }
        context.result(self.step_id(), aspect_ok, None, &metrics, &findings)
    }
}

/// Distributed span load integrated into a surrogate root bending moment.
#[derive(Clone, Copy, Debug, Default)]
pub struct SpanLoadEvaluator;

impl DomainEvaluator<WingParameters> for SpanLoadEvaluator {
    fn step_id(&self) -> &str {
        LOAD_STEP_ID
    }
    fn required_capabilities(&self) -> &[String] {
        &[]
    }
    fn implementation(&self) -> Result<EvaluatorImplementation, DomainError> {
        implementation_for("SpanLoadEvaluator", LOAD_STEP_ID)
    }
    fn evaluate(
        &self,
        context: &EvaluationContext,
        parameters: &WingParameters,
    ) -> Result<EvaluationResult, DomainError> {
        let simulation = simulate_wing(parameters);
        let moment_ok =
            simulation.root_bending_moment_kn_m <= parameters.maximum_root_bending_moment_kn_m();
        let metrics = vec![
            context.metric(
                "load.span_load",
                parameters.span_load_kn_m(),
                Some(SPAN_LOAD_UNIT),
                None,
            )?,
            context.metric(
                "load.span_load_newtons",
                simulation.span_load_n_m,
                Some("N/m"),
                None,
            )?,
            context.metric(
                "load.root_bending_moment",
                simulation.root_bending_moment_kn_m,
                Some(ROOT_MOMENT_UNIT),
                Some(moment_ok),
            )?,
            context.metric(
                "load.maximum_root_bending_moment",
                parameters.maximum_root_bending_moment_kn_m(),
                Some(ROOT_MOMENT_UNIT),
                Some(moment_ok),
            )?,
        ];
        let mut findings = vec![NON_CERTIFIED_FINDING.to_owned()];
        if !moment_ok {
            findings.push(
                violation_finding(ConstraintViolation::RootBendingMomentAboveMaximum).to_owned(),
            );
        }
        context.result(self.step_id(), moment_ok, None, &metrics, &findings)
    }
}

/// Finite-wing lift and induced drag surrogates with the reference mass bound.
#[derive(Clone, Copy, Debug, Default)]
pub struct AerodynamicSurrogateEvaluator;

impl DomainEvaluator<WingParameters> for AerodynamicSurrogateEvaluator {
    fn step_id(&self) -> &str {
        AERODYNAMIC_STEP_ID
    }
    fn required_capabilities(&self) -> &[String] {
        &[]
    }
    fn implementation(&self) -> Result<EvaluatorImplementation, DomainError> {
        implementation_for("AerodynamicSurrogateEvaluator", AERODYNAMIC_STEP_ID)
    }
    fn evaluate(
        &self,
        context: &EvaluationContext,
        parameters: &WingParameters,
    ) -> Result<EvaluationResult, DomainError> {
        let simulation = simulate_wing(parameters);
        let mass_ok = simulation.mass_kg <= parameters.maximum_mass_kg();
        let metrics = vec![
            context.metric(
                "aero.lift_curve_slope",
                simulation.lift_curve_slope_per_rad,
                Some(PER_RADIAN_UNIT),
                None,
            )?,
            context.metric(
                "aero.induced_drag_factor",
                simulation.induced_drag_factor,
                None,
                None,
            )?,
            context.metric(
                "aero.induced_drag_coefficient",
                simulation.induced_drag_coefficient,
                None,
                None,
            )?,
            context.metric(
                "aero.quarter_chord_sweep_magnitude",
                simulation.quarter_chord_sweep_rad.abs(),
                Some("rad"),
                None,
            )?,
            context.metric(
                "aero.dihedral_magnitude",
                simulation.dihedral_rad.abs(),
                Some("rad"),
                None,
            )?,
            context.metric(
                "mass.reference",
                simulation.mass_kg,
                Some(MASS_UNIT),
                Some(mass_ok),
            )?,
            context.metric(
                "mass.maximum",
                parameters.maximum_mass_kg(),
                Some(MASS_UNIT),
                Some(mass_ok),
            )?,
            context.metric("score.geometry", simulation.geometry_score, None, None)?,
        ];
        let mut findings = vec![NON_CERTIFIED_FINDING.to_owned()];
        if !mass_ok {
            findings.push(violation_finding(ConstraintViolation::MassAboveMaximum).to_owned());
        }
        context.result(
            self.step_id(),
            mass_ok,
            Some(simulation.geometry_score),
            &metrics,
            &findings,
        )
    }
}
