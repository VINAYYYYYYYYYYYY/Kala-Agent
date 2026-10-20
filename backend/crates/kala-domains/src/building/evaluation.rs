//! Bounded evaluators for the building domain package.

use kala_contracts::{EvaluationMetric, EvaluationResult};

use crate::building::parameters::{
    BuildingParameters, CODE_AUTHORITY_FINDING, CONDUCTIVITY_UNIT, CUBIC_METRE_UNIT,
    DEGREE_DAY_UNIT, ENERGY_UNIT, INTENSITY_UNIT, METRE_UNIT, SQUARE_METRE_UNIT, TEMPERATURE_UNIT,
    THERMAL_SURROGATE_FINDING, TRANSMITTANCE_UNIT,
};
use crate::building::simulation::{BuildingSimulation, simulate_building};
use crate::context::EvaluationContext;
use crate::error::DomainError;
use crate::package::DomainEvaluator;
use crate::provenance::EvaluatorImplementation;
use crate::spec::NON_AUTHORITATIVE_FINDING;

/// Storey catalog step id.
pub const STOREY_STEP_ID: &str = "building.storey-model";
/// Area catalog step id.
pub const AREA_STEP_ID: &str = "building.area-model";
/// Thermal catalog step id.
pub const THERMAL_STEP_ID: &str = "building.thermal-envelope";

/// Hard constraint identifiers declared by the building package.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConstraintViolation {
    /// Stack height above the placeholder bound.
    TotalHeightAbovePlaceholderBound,
    /// Storey area below the placeholder minimum.
    StoreyAreaBelowPlaceholderMinimum,
    /// Heating intensity above the placeholder bound.
    HeatingIntensityAbovePlaceholderBound,
    /// Envelope conductivity is zero.
    EnvelopeConductivityZero,
}

impl ConstraintViolation {
    /// The declared violation identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TotalHeightAbovePlaceholderBound => "total_height_above_placeholder_bound",
            Self::StoreyAreaBelowPlaceholderMinimum => "storey_area_below_placeholder_minimum",
            Self::HeatingIntensityAbovePlaceholderBound => {
                "heating_intensity_above_placeholder_bound"
            }
            Self::EnvelopeConductivityZero => "envelope_conductivity_zero",
        }
    }

    /// The finding text reported for this violation.
    pub const fn finding(self) -> &'static str {
        match self {
            Self::TotalHeightAbovePlaceholderBound => {
                "Hard constraint failed: total_height_above_placeholder_bound; the stack height is above the declared placeholder height bound, which is not a code height limit."
            }
            Self::StoreyAreaBelowPlaceholderMinimum => {
                "Hard constraint failed: storey_area_below_placeholder_minimum; the storey area is below the declared placeholder minimum area."
            }
            Self::HeatingIntensityAbovePlaceholderBound => {
                "Hard constraint failed: heating_intensity_above_placeholder_bound; the surrogate heating intensity is above the declared placeholder bound."
            }
            Self::EnvelopeConductivityZero => {
                "Hard constraint failed: envelope_conductivity_zero; the declared envelope has no transmittance, so no thermal evaluation is meaningful."
            }
        }
    }
}

/// Return the finding text reported for a hard constraint violation.
pub fn violation_finding(violation: ConstraintViolation) -> &'static str {
    violation.finding()
}

/// Return every hard constraint the given parameters violate, in order.
pub fn violations_for(parameters: &BuildingParameters) -> Vec<ConstraintViolation> {
    let simulation = simulate_building(parameters);
    violations_for_simulation(parameters, &simulation)
}

fn violations_for_simulation(
    parameters: &BuildingParameters,
    simulation: &BuildingSimulation,
) -> Vec<ConstraintViolation> {
    let mut violations = Vec::new();
    if simulation.total_height_m > parameters.maximum_height_m() {
        violations.push(ConstraintViolation::TotalHeightAbovePlaceholderBound);
    }
    if simulation.storey_area_m2 < parameters.minimum_storey_area_m2() {
        violations.push(ConstraintViolation::StoreyAreaBelowPlaceholderMinimum);
    }
    if simulation.heating_intensity_kwh_m2 > parameters.maximum_heating_intensity_kwh_m2() {
        violations.push(ConstraintViolation::HeatingIntensityAbovePlaceholderBound);
    }
    if simulation.conductance_total_w_k <= 0.0 {
        violations.push(ConstraintViolation::EnvelopeConductivityZero);
    }
    violations
}

/// Return metrics that keep the non-authoritative placeholder visible.
pub fn placeholder_code_metrics(
    context: &EvaluationContext,
) -> Result<Vec<EvaluationMetric>, DomainError> {
    Ok(vec![
        context.metric("code.version_is_placeholder", 1.0, None, Some(true))?,
        context.metric("code.compliance_determined", 0.0, None, Some(false))?,
    ])
}

fn implementation_for(
    type_name: &str,
    step_id: &str,
) -> Result<EvaluatorImplementation, DomainError> {
    EvaluatorImplementation::native("building", step_id, type_name, &[])
}

/// Storey stack geometry evaluated against the placeholder height bound.
#[derive(Clone, Copy, Debug, Default)]
pub struct StoreyModelEvaluator;

impl DomainEvaluator<BuildingParameters> for StoreyModelEvaluator {
    fn step_id(&self) -> &str {
        STOREY_STEP_ID
    }
    fn required_capabilities(&self) -> &[String] {
        &[]
    }
    fn implementation(&self) -> Result<EvaluatorImplementation, DomainError> {
        implementation_for("StoreyModelEvaluator", STOREY_STEP_ID)
    }
    fn evaluate(
        &self,
        context: &EvaluationContext,
        parameters: &BuildingParameters,
    ) -> Result<EvaluationResult, DomainError> {
        let simulation = simulate_building(parameters);
        let height_ok = simulation.total_height_m <= parameters.maximum_height_m();
        let mut metrics = vec![
            context.metric("storey.count", parameters.storey_count() as f64, None, None)?,
            context.metric(
                "storey.height",
                parameters.storey_height_m(),
                Some(METRE_UNIT),
                None,
            )?,
            context.metric(
                "storey.total_height",
                simulation.total_height_m,
                Some(METRE_UNIT),
                Some(height_ok),
            )?,
            context.metric(
                "storey.maximum_height",
                parameters.maximum_height_m(),
                Some(METRE_UNIT),
                Some(height_ok),
            )?,
            context.metric(
                "storey.total_volume",
                simulation.total_volume_m3,
                Some(CUBIC_METRE_UNIT),
                None,
            )?,
        ];
        metrics.extend(placeholder_code_metrics(context)?);
        let mut findings = vec![
            NON_AUTHORITATIVE_FINDING.to_owned(),
            CODE_AUTHORITY_FINDING.to_owned(),
        ];
        if !height_ok {
            findings.push(
                violation_finding(ConstraintViolation::TotalHeightAbovePlaceholderBound).to_owned(),
            );
        }
        context.result(self.step_id(), height_ok, None, &metrics, &findings)
    }
}

/// Storey and envelope areas evaluated against the placeholder minimum.
#[derive(Clone, Copy, Debug, Default)]
pub struct AreaModelEvaluator;

impl DomainEvaluator<BuildingParameters> for AreaModelEvaluator {
    fn step_id(&self) -> &str {
        AREA_STEP_ID
    }
    fn required_capabilities(&self) -> &[String] {
        &[]
    }
    fn implementation(&self) -> Result<EvaluatorImplementation, DomainError> {
        implementation_for("AreaModelEvaluator", AREA_STEP_ID)
    }
    fn evaluate(
        &self,
        context: &EvaluationContext,
        parameters: &BuildingParameters,
    ) -> Result<EvaluationResult, DomainError> {
        let simulation = simulate_building(parameters);
        let area_ok = simulation.storey_area_m2 >= parameters.minimum_storey_area_m2();
        let mut metrics = vec![
            context.metric(
                "area.storey_gross",
                simulation.storey_area_m2,
                Some(SQUARE_METRE_UNIT),
                Some(area_ok),
            )?,
            context.metric(
                "area.minimum_storey",
                parameters.minimum_storey_area_m2(),
                Some(SQUARE_METRE_UNIT),
                Some(area_ok),
            )?,
            context.metric(
                "area.total_floor",
                simulation.total_floor_area_m2,
                Some(SQUARE_METRE_UNIT),
                None,
            )?,
            context.metric(
                "area.wall",
                simulation.wall_area_m2,
                Some(SQUARE_METRE_UNIT),
                None,
            )?,
            context.metric(
                "area.opaque_wall",
                simulation.opaque_wall_area_m2,
                Some(SQUARE_METRE_UNIT),
                None,
            )?,
            context.metric(
                "area.window",
                simulation.window_area_m2,
                Some(SQUARE_METRE_UNIT),
                None,
            )?,
            context.metric(
                "area.roof",
                simulation.roof_area_m2,
                Some(SQUARE_METRE_UNIT),
                None,
            )?,
            context.metric(
                "area.window_ratio",
                parameters.window_to_wall_ratio(),
                None,
                None,
            )?,
        ];
        metrics.extend(placeholder_code_metrics(context)?);
        let mut findings = vec![
            NON_AUTHORITATIVE_FINDING.to_owned(),
            CODE_AUTHORITY_FINDING.to_owned(),
        ];
        if !area_ok {
            findings.push(
                violation_finding(ConstraintViolation::StoreyAreaBelowPlaceholderMinimum)
                    .to_owned(),
            );
        }
        context.result(self.step_id(), area_ok, None, &metrics, &findings)
    }
}

/// Transmission-only envelope evaluation against the placeholder intensity
/// bound.
#[derive(Clone, Copy, Debug, Default)]
pub struct ThermalEnvelopeEvaluator;

impl DomainEvaluator<BuildingParameters> for ThermalEnvelopeEvaluator {
    fn step_id(&self) -> &str {
        THERMAL_STEP_ID
    }
    fn required_capabilities(&self) -> &[String] {
        &[]
    }
    fn implementation(&self) -> Result<EvaluatorImplementation, DomainError> {
        implementation_for("ThermalEnvelopeEvaluator", THERMAL_STEP_ID)
    }
    fn evaluate(
        &self,
        context: &EvaluationContext,
        parameters: &BuildingParameters,
    ) -> Result<EvaluationResult, DomainError> {
        let simulation = simulate_building(parameters);
        let intensity_ok =
            simulation.heating_intensity_kwh_m2 <= parameters.maximum_heating_intensity_kwh_m2();
        let conductance_ok = simulation.conductance_total_w_k > 0.0;
        let mut metrics = vec![
            context.metric(
                "envelope.conductance_wall",
                simulation.conductance_wall_w_k,
                Some(TRANSMITTANCE_UNIT),
                None,
            )?,
            context.metric(
                "envelope.conductance_window",
                simulation.conductance_window_w_k,
                Some(TRANSMITTANCE_UNIT),
                None,
            )?,
            context.metric(
                "envelope.conductance_roof",
                simulation.conductance_roof_w_k,
                Some(TRANSMITTANCE_UNIT),
                None,
            )?,
            context.metric(
                "envelope.conductance_total",
                simulation.conductance_total_w_k,
                Some(TRANSMITTANCE_UNIT),
                Some(conductance_ok),
            )?,
            context.metric(
                "envelope.conductance_per_floor_area",
                simulation.conductance_per_floor_area_w_m2_k,
                Some(CONDUCTIVITY_UNIT),
                None,
            )?,
            context.metric(
                "envelope.designed_wall_conductivity",
                parameters.wall_conductivity_w_m2k(),
                Some(CONDUCTIVITY_UNIT),
                None,
            )?,
            context.metric(
                "envelope.temperature_difference",
                simulation.temperature_difference_k,
                Some(TEMPERATURE_UNIT),
                None,
            )?,
            context.metric(
                "envelope.degree_days",
                parameters.heating_degree_days_k(),
                Some(DEGREE_DAY_UNIT),
                None,
            )?,
            context.metric(
                "envelope.annual_heating",
                simulation.annual_heating_kwh,
                Some(ENERGY_UNIT),
                None,
            )?,
            context.metric(
                "envelope.heating_intensity",
                simulation.heating_intensity_kwh_m2,
                Some(INTENSITY_UNIT),
                Some(intensity_ok),
            )?,
            context.metric(
                "envelope.maximum_heating_intensity",
                parameters.maximum_heating_intensity_kwh_m2(),
                Some(INTENSITY_UNIT),
                Some(intensity_ok),
            )?,
            context.metric("envelope.score", simulation.envelope_score, None, None)?,
        ];
        metrics.extend(placeholder_code_metrics(context)?);
        let mut findings = vec![
            NON_AUTHORITATIVE_FINDING.to_owned(),
            CODE_AUTHORITY_FINDING.to_owned(),
            THERMAL_SURROGATE_FINDING.to_owned(),
        ];
        if !intensity_ok {
            findings.push(
                violation_finding(ConstraintViolation::HeatingIntensityAbovePlaceholderBound)
                    .to_owned(),
            );
        }
        if !conductance_ok {
            findings
                .push(violation_finding(ConstraintViolation::EnvelopeConductivityZero).to_owned());
        }
        context.result(
            self.step_id(),
            intensity_ok && conductance_ok,
            Some(simulation.envelope_score),
            &metrics,
            &findings,
        )
    }
}
