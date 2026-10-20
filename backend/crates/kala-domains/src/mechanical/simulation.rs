//! Deterministic closed-form reference simulation for the bracket component.

use crate::mechanical::parameters::{BracketParameters, MaterialReference, material_reference_for};
use crate::spec::{UNBOUNDED_SAFETY_FACTOR, clamp_unit_interval};

/// Effective hole-area fraction.
pub const HOLE_AREA_FRACTION: f64 = std::f64::consts::PI / 4.0;
/// Cubic millimetres per cubic metre.
pub const MILLIMETRE_CUBED_TO_METRE_CUBED: f64 = 1.0e-9;
/// Newtons per kilonewton.
pub const KILONEWTON_TO_NEWTON: f64 = 1_000.0;
/// Lever arm as a fraction of the length.
pub const LEVER_ARM_FRACTION: f64 = 0.5;
/// Bending stress coefficient.
pub const BENDING_SIGMA: f64 = 1.0;
/// Shear stress coefficient.
pub const SHEAR_TAU: f64 = 0.5;
/// Von Mises shear coefficient.
pub const VON_MISES_SHEAR_COEFFICIENT: f64 = 3.0;
/// Section modulus coefficient.
pub const SECTION_MODULUS_COEFFICIENT: f64 = 6.0;
/// Thickness score multiple.
pub const THICKNESS_SCORE_MULTIPLE: f64 = 2.0;
/// Stress term weight.
pub const STRESS_TERM_WEIGHT: f64 = 0.5;
/// Thickness term weight.
pub const THICKNESS_TERM_WEIGHT: f64 = 0.2;
/// Utilisation term weight.
pub const UTILISATION_TERM_WEIGHT: f64 = 0.3;

/// Reference quantities produced by [`simulate_bracket`].
#[derive(Clone, Debug, PartialEq)]
pub struct BracketSimulation {
    /// Gross plate area in mm^2.
    pub gross_area_mm2: f64,
    /// Hole area in mm^2.
    pub hole_area_mm2: f64,
    /// Net area in mm^2.
    pub net_area_mm2: f64,
    /// Volume in mm^3.
    pub volume_mm3: f64,
    /// Mass in kg.
    pub mass_kg: f64,
    /// Applied load in N.
    pub load_n: f64,
    /// Load per introduction point in N.
    pub load_per_point_n: f64,
    /// Lever arm in mm.
    pub lever_arm_mm: f64,
    /// Bending moment in N*mm.
    pub bending_moment_n_mm: f64,
    /// Section modulus in mm^3.
    pub section_modulus_mm3: f64,
    /// Bending stress in MPa.
    pub bending_stress_mpa: f64,
    /// Shear stress in MPa.
    pub shear_stress_mpa: f64,
    /// Combined surrogate stress in MPa.
    pub stress_mpa: f64,
    /// Utilisation ratio.
    pub utilisation: f64,
    /// Safety factor.
    pub safety_factor: f64,
    /// Material web beside the holes in mm.
    pub material_web_mm: f64,
    /// Edge margin in mm.
    pub edge_margin_mm: f64,
    /// Bounded safety score.
    pub safety_score: f64,
}

/// Return deterministic reference quantities for a bracket-like component.
pub fn simulate_bracket(parameters: &BracketParameters) -> BracketSimulation {
    let material: MaterialReference = material_reference_for(parameters.material())
        .expect("a validated material key always has declared constants");
    let gross_area_mm2 = parameters.length_mm() * parameters.width_mm();
    let hole_area_mm2 = HOLE_AREA_FRACTION
        * parameters.hole_count() as f64
        * (parameters.hole_diameter_mm() * parameters.hole_diameter_mm());
    let net_area_mm2 = gross_area_mm2 - hole_area_mm2;
    let volume_mm3 = net_area_mm2 * parameters.thickness_mm();
    let mass_kg = volume_mm3 * MILLIMETRE_CUBED_TO_METRE_CUBED * material.density_kg_m3;
    let load_n = parameters.load_kn() * KILONEWTON_TO_NEWTON;
    let load_per_point_n = load_n / parameters.load_points() as f64;
    let lever_arm_mm = LEVER_ARM_FRACTION * parameters.length_mm();
    let bending_moment_n_mm = load_n * lever_arm_mm;
    let section_modulus_mm3 = parameters.width_mm()
        * (parameters.thickness_mm() * parameters.thickness_mm())
        / SECTION_MODULUS_COEFFICIENT;
    let bending_stress_mpa = BENDING_SIGMA * bending_moment_n_mm / section_modulus_mm3;
    let shear_stress_mpa = SHEAR_TAU * load_n / (parameters.width_mm() * parameters.thickness_mm());
    let stress_mpa = (bending_stress_mpa * bending_stress_mpa
        + VON_MISES_SHEAR_COEFFICIENT * (shear_stress_mpa * shear_stress_mpa))
        .sqrt();
    let utilisation = stress_mpa / material.yield_strength_mpa;
    let safety_factor = if stress_mpa == 0.0 {
        UNBOUNDED_SAFETY_FACTOR
    } else {
        material.yield_strength_mpa / stress_mpa
    };
    let material_web_mm =
        parameters.length_mm() - parameters.hole_count() as f64 * parameters.hole_diameter_mm();
    let edge_margin_mm = material_web_mm / 2.0;
    let safety_score = safety_score(parameters, safety_factor, utilisation);
    BracketSimulation {
        gross_area_mm2,
        hole_area_mm2,
        net_area_mm2,
        volume_mm3,
        mass_kg,
        load_n,
        load_per_point_n,
        lever_arm_mm,
        bending_moment_n_mm,
        section_modulus_mm3,
        bending_stress_mpa,
        shear_stress_mpa,
        stress_mpa,
        utilisation,
        safety_factor,
        material_web_mm,
        edge_margin_mm,
        safety_score,
    }
}

fn safety_score(parameters: &BracketParameters, safety_factor: f64, utilisation: f64) -> f64 {
    let stress_term = clamp_unit_interval(safety_factor / parameters.target_safety_factor())
        .expect("bounded finite score term");
    let thickness_term = clamp_unit_interval(
        parameters.thickness_mm() / (THICKNESS_SCORE_MULTIPLE * parameters.minimum_thickness_mm()),
    )
    .expect("bounded finite score term");
    let utilisation_term =
        clamp_unit_interval(1.0 - utilisation).expect("bounded finite score term");
    clamp_unit_interval(
        STRESS_TERM_WEIGHT * stress_term
            + THICKNESS_TERM_WEIGHT * thickness_term
            + UTILISATION_TERM_WEIGHT * utilisation_term,
    )
    .expect("bounded finite score")
}
