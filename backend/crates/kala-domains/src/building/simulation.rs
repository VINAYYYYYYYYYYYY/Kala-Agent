//! Deterministic reference simulation for the building envelope.

use crate::building::parameters::BuildingParameters;
use crate::spec::clamp_unit_interval;

/// Hours per day.
pub const HOURS_PER_DAY: f64 = 24.0;
/// Watts per kilowatt.
pub const WATT_TO_KILOWATT: f64 = 1.0e-3;
/// Reference transmittance per floor area.
pub const REFERENCE_TRANSMITTANCE_PER_FLOOR_AREA: f64 = 0.9;
/// Glazing score ratio.
pub const GLAZING_SCORE_RATIO: f64 = 0.5;
/// UA term weight.
pub const UA_TERM_WEIGHT: f64 = 0.45;
/// Intensity term weight.
pub const INTENSITY_TERM_WEIGHT: f64 = 0.35;
/// Glazing term weight.
pub const GLAZING_TERM_WEIGHT: f64 = 0.20;
/// No-penalty term.
pub const NO_PENALTY_TERM: f64 = 1.0;

/// Reference quantities produced by [`simulate_building`].
#[derive(Clone, Debug, PartialEq)]
pub struct BuildingSimulation {
    /// Area of one storey.
    pub storey_area_m2: f64,
    /// Total floor area.
    pub total_floor_area_m2: f64,
    /// Total height.
    pub total_height_m: f64,
    /// Total volume.
    pub total_volume_m3: f64,
    /// Wall area.
    pub wall_area_m2: f64,
    /// Window area.
    pub window_area_m2: f64,
    /// Opaque wall area.
    pub opaque_wall_area_m2: f64,
    /// Roof area.
    pub roof_area_m2: f64,
    /// Wall conductance.
    pub conductance_wall_w_k: f64,
    /// Window conductance.
    pub conductance_window_w_k: f64,
    /// Roof conductance.
    pub conductance_roof_w_k: f64,
    /// Total conductance.
    pub conductance_total_w_k: f64,
    /// Conductance per floor area.
    pub conductance_per_floor_area_w_m2_k: f64,
    /// Temperature difference.
    pub temperature_difference_k: f64,
    /// Transmission loss.
    pub transmission_loss_w_k: f64,
    /// Annual heating energy.
    pub annual_heating_kwh: f64,
    /// Heating intensity.
    pub heating_intensity_kwh_m2: f64,
    /// Bounded envelope score.
    pub envelope_score: f64,
}

/// Return deterministic reference quantities for a storey stack.
pub fn simulate_building(parameters: &BuildingParameters) -> BuildingSimulation {
    let storey_count = parameters.storey_count() as f64;
    let storey_area_m2 = parameters.gross_floor_area_m2();
    let total_floor_area_m2 = storey_area_m2 * storey_count;
    let total_height_m = parameters.storey_height_m() * storey_count;
    let total_volume_m3 = total_floor_area_m2 * parameters.storey_height_m();
    let wall_area_m2 = parameters.perimeter_m() * parameters.storey_height_m() * storey_count;
    let window_area_m2 = wall_area_m2 * parameters.window_to_wall_ratio();
    let opaque_wall_area_m2 = wall_area_m2 - window_area_m2;
    let roof_area_m2 = storey_area_m2;
    let conductance_wall_w_k = parameters.wall_conductivity_w_m2k() * opaque_wall_area_m2;
    let conductance_window_w_k = parameters.window_conductivity_w_m2k() * window_area_m2;
    let conductance_roof_w_k = parameters.roof_conductivity_w_m2k() * roof_area_m2;
    let conductance_total_w_k =
        conductance_wall_w_k + conductance_window_w_k + conductance_roof_w_k;
    let conductance_per_floor_area_w_m2_k = conductance_total_w_k / total_floor_area_m2;
    let temperature_difference_k =
        parameters.design_indoor_temperature_c() - parameters.design_outdoor_temperature_c();
    let transmission_loss_w_k = conductance_total_w_k * temperature_difference_k;
    let annual_heating_kwh = transmission_loss_w_k
        * parameters.heating_degree_days_k()
        * HOURS_PER_DAY
        * WATT_TO_KILOWATT;
    let heating_intensity_kwh_m2 = annual_heating_kwh / total_floor_area_m2;
    let envelope_score = envelope_score(
        parameters,
        conductance_per_floor_area_w_m2_k,
        heating_intensity_kwh_m2,
    );
    BuildingSimulation {
        storey_area_m2,
        total_floor_area_m2,
        total_height_m,
        total_volume_m3,
        wall_area_m2,
        window_area_m2,
        opaque_wall_area_m2,
        roof_area_m2,
        conductance_wall_w_k,
        conductance_window_w_k,
        conductance_roof_w_k,
        conductance_total_w_k,
        conductance_per_floor_area_w_m2_k,
        temperature_difference_k,
        transmission_loss_w_k,
        annual_heating_kwh,
        heating_intensity_kwh_m2,
        envelope_score,
    }
}

fn envelope_score(
    parameters: &BuildingParameters,
    conductance_per_floor_area_w_m2_k: f64,
    heating_intensity_kwh_m2: f64,
) -> f64 {
    let conductance_term = if conductance_per_floor_area_w_m2_k <= 0.0 {
        NO_PENALTY_TERM
    } else {
        clamp_unit_interval(
            REFERENCE_TRANSMITTANCE_PER_FLOOR_AREA / conductance_per_floor_area_w_m2_k,
        )
        .expect("bounded finite score term")
    };
    let intensity_term = if heating_intensity_kwh_m2 <= 0.0 {
        NO_PENALTY_TERM
    } else {
        clamp_unit_interval(
            parameters.maximum_heating_intensity_kwh_m2() / heating_intensity_kwh_m2,
        )
        .expect("bounded finite score term")
    };
    let glazing_term =
        clamp_unit_interval(1.0 - parameters.window_to_wall_ratio() / GLAZING_SCORE_RATIO)
            .expect("bounded finite score term");
    clamp_unit_interval(
        UA_TERM_WEIGHT * conductance_term
            + INTENSITY_TERM_WEIGHT * intensity_term
            + GLAZING_TERM_WEIGHT * glazing_term,
    )
    .expect("bounded finite score")
}
