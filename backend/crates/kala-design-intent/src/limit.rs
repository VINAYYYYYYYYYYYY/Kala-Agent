//! Studio dimension limits shared by the templates and the intent calls.
//!
//! Port of the constants and `validate_finite_positive` from
//! `kala_agent.studio.geometry.limits`. The geometry contract in
//! `kala_contracts` has a different five-metre ceiling
//! (`MAX_DIMENSION_MM = 5000`), so these studio limits are deliberately kept
//! separate instead of re-exporting the contract constant.

/// Largest declared studio dimension in millimetres.
pub const MAX_DIMENSION_MM: f64 = 2_000.0;

/// Smallest declared studio dimension in millimetres.
pub const MIN_DIMENSION_MM: f64 = 0.1;

/// Return a finite, positive value within the studio millimetre bounds.
///
/// The message vocabulary and range are exactly
/// `kala_agent.studio.geometry.limits.validate_finite_positive`. Rust has no
/// `bool`-is-an-`int` coercion, so a boolean argument is refused by the type
/// system before this function is reached.
pub(crate) fn validate_finite_positive(name: &str, value: f64) -> Result<f64, String> {
    if !value.is_finite() {
        return Err(format!("{name} must be a finite number"));
    }
    if value < MIN_DIMENSION_MM || value > MAX_DIMENSION_MM {
        return Err(format!("{name} must be between 0.1 and 2000.0 millimetres"));
    }
    Ok(value)
}
