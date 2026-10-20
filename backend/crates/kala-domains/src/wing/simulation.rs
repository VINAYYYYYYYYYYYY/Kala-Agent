//! Deterministic reference simulation for the parametric wing.

use crate::spec::clamp_unit_interval;
use crate::wing::parameters::WingParameters;

/// Newtons per kilonewton.
pub const KILONEWTON_TO_NEWTON: f64 = 1_000.0;
/// Elliptical root-moment coefficient.
// This is the CPython-evaluated declaration value:
// `1.0 / (math.pi * math.sqrt(3.0))`.
pub const ELLIPTICAL_ROOT_MOMENT_COEFFICIENT: f64 = 0.1837762984739307;
/// Areal mass in kg/m^2.
pub const AREAL_MASS_KG_M2: f64 = 12.0;
/// Induced-drag efficiency.
pub const INDUCED_DRAG_EFFICIENCY: f64 = 1.0;
/// Radians per degree.
pub const RADIANS_PER_DEGREE: f64 = std::f64::consts::PI / 180.0;
/// No-penalty term.
pub const NO_PENALTY_TERM: f64 = 1.0;
/// Aspect term weight.
pub const ASPECT_TERM_WEIGHT: f64 = 0.35;
/// Bending term weight.
pub const BENDING_TERM_WEIGHT: f64 = 0.4;
/// Efficiency term weight.
pub const EFFICIENCY_TERM_WEIGHT: f64 = 0.25;

/// Reference quantities produced by [`simulate_wing`].
#[derive(Clone, Debug, PartialEq)]
pub struct WingSimulation {
    /// Taper ratio.
    pub taper_ratio: f64,
    /// Reference area.
    pub reference_area_m2: f64,
    /// Mean chord.
    pub mean_chord_m: f64,
    /// Aspect ratio.
    pub aspect_ratio: f64,
    /// Structural reference volume.
    pub structural_volume_m3: f64,
    /// Reference mass.
    pub mass_kg: f64,
    /// Span load in N/m.
    pub span_load_n_m: f64,
    /// Root bending moment.
    pub root_bending_moment_kn_m: f64,
    /// Lift-curve slope per radian.
    pub lift_curve_slope_per_rad: f64,
    /// Induced-drag factor.
    pub induced_drag_factor: f64,
    /// Induced-drag coefficient.
    pub induced_drag_coefficient: f64,
    /// Quarter-chord sweep in radians.
    pub quarter_chord_sweep_rad: f64,
    /// Dihedral in radians.
    pub dihedral_rad: f64,
    /// Bounded geometry score.
    pub geometry_score: f64,
}

fn integrated_chord_length(parameters: &WingParameters) -> f64 {
    let stations = parameters.spanwise_stations();
    let mut total = 0.0;
    for index in 0..stations {
        let start_fraction = index as f64 / stations as f64;
        let end_fraction = (index + 1) as f64 / stations as f64;
        let start_chord = parameters.root_chord_m()
            + (parameters.tip_chord_m() - parameters.root_chord_m()) * start_fraction;
        let end_chord = parameters.root_chord_m()
            + (parameters.tip_chord_m() - parameters.root_chord_m()) * end_fraction;
        total += 0.5 * (start_chord + end_chord) * (end_fraction - start_fraction);
    }
    total
}

/// Return deterministic reference quantities for a parametric wing.
pub fn simulate_wing(parameters: &WingParameters) -> WingSimulation {
    let integrated_chord_m = integrated_chord_length(parameters);
    let reference_area_m2 = parameters.span_m() * integrated_chord_m;
    let mean_chord_m = integrated_chord_m;
    let aspect_ratio = parameters.span_m() * parameters.span_m() / reference_area_m2;
    let taper_ratio = parameters.tip_chord_m() / parameters.root_chord_m();
    let structural_volume_m3 = reference_area_m2 * parameters.thickness_ratio();
    let mass_kg = reference_area_m2 * AREAL_MASS_KG_M2;
    let span_load_n_m = parameters.span_load_kn_m() * KILONEWTON_TO_NEWTON;
    let root_bending_moment_kn_m = ELLIPTICAL_ROOT_MOMENT_COEFFICIENT
        * span_load_n_m
        * (parameters.span_m() * parameters.span_m())
        / KILONEWTON_TO_NEWTON;
    let lift_curve_slope_per_rad = 2.0 * std::f64::consts::PI * aspect_ratio / (aspect_ratio + 2.0);
    let induced_drag_factor = (aspect_ratio + 2.0) / (2.0 * aspect_ratio);
    let induced_drag_coefficient =
        1.0 / (std::f64::consts::PI * aspect_ratio * INDUCED_DRAG_EFFICIENCY);
    let quarter_chord_sweep_rad = parameters.sweep_deg() * RADIANS_PER_DEGREE;
    let dihedral_rad = parameters.dihedral_deg() * RADIANS_PER_DEGREE;
    let geometry_score = geometry_score(
        parameters,
        aspect_ratio,
        root_bending_moment_kn_m,
        lift_curve_slope_per_rad,
    );
    WingSimulation {
        taper_ratio,
        reference_area_m2,
        mean_chord_m,
        aspect_ratio,
        structural_volume_m3,
        mass_kg,
        span_load_n_m,
        root_bending_moment_kn_m,
        lift_curve_slope_per_rad,
        induced_drag_factor,
        induced_drag_coefficient,
        quarter_chord_sweep_rad,
        dihedral_rad,
        geometry_score,
    }
}

fn geometry_score(
    parameters: &WingParameters,
    aspect_ratio: f64,
    root_bending_moment_kn_m: f64,
    lift_curve_slope_per_rad: f64,
) -> f64 {
    let aspect_term = clamp_unit_interval(1.0 - aspect_ratio / parameters.maximum_aspect_ratio())
        .expect("bounded finite score term");
    let bending_term = if root_bending_moment_kn_m <= 0.0 {
        NO_PENALTY_TERM
    } else {
        clamp_unit_interval(
            1.0 - root_bending_moment_kn_m / parameters.maximum_root_bending_moment_kn_m(),
        )
        .expect("bounded finite score term")
    };
    let efficiency_term =
        clamp_unit_interval(lift_curve_slope_per_rad / (2.0 * std::f64::consts::PI))
            .expect("bounded finite score term");
    clamp_unit_interval(
        ASPECT_TERM_WEIGHT * aspect_term
            + BENDING_TERM_WEIGHT * bending_term
            + EFFICIENCY_TERM_WEIGHT * efficiency_term,
    )
    .expect("bounded finite score")
}
