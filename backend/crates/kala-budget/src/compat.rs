//! Compatibility helpers carried by the decimal/budget corpus.
//!
//! The corpus's decimal fixture also freezes `kala_agent.domains`
//! `clamp_unit_interval`, so the budget cell validates it here to consume the
//! whole 85-case fixture. A dedicated domain crate can own the production
//! version later without changing these semantics.

use std::str::FromStr;

/// Error raised for non-finite bounded terms, mirroring `DomainParameterError`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("bounded terms must be finite numbers")]
pub struct DomainParameterError;

/// `kala_agent.domains.clamp_unit_interval(value)`.
///
/// Clamps into `[0, 1]` and rounds to six decimal places using Python's
/// `round(value, 6)` (correctly rounded, ties to even).
pub fn clamp_unit_interval(value: f64) -> Result<f64, DomainParameterError> {
    if !value.is_finite() {
        return Err(DomainParameterError);
    }
    // Python `round(min(1.0, max(0.0, value)), 6)`; `max` keeps its first
    // argument on ties, which matters for negative zero.
    let lower = if value > 0.0 { value } else { 0.0 };
    let clamped = if lower < 1.0 { lower } else { 1.0 };
    round_to_digits(clamped, 6).ok_or(DomainParameterError)
}

/// Round to `digits` decimal places with correct rounding, ties to even
/// (Rust's fixed formatting is correctly rounded; parsing it back is exact).
pub(crate) fn round_to_digits(value: f64, digits: usize) -> Option<f64> {
    let text = format!("{value:.*}", digits);
    f64::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_and_rounds_like_the_domain_oracle() {
        assert_eq!(clamp_unit_interval(0.0).unwrap(), 0.0);
        assert_eq!(clamp_unit_interval(1.0).unwrap(), 1.0);
        assert_eq!(clamp_unit_interval(-1.0).unwrap(), 0.0);
        assert_eq!(clamp_unit_interval(2.0).unwrap(), 1.0);
        assert_eq!(clamp_unit_interval(0.1234564).unwrap(), 0.123456);
        assert_eq!(clamp_unit_interval(0.1234565).unwrap(), 0.123456);
        assert_eq!(clamp_unit_interval(0.1234575).unwrap(), 0.123457);
        assert_eq!(clamp_unit_interval(0.9999995).unwrap(), 1.0);
        assert_eq!(clamp_unit_interval(1e-7).unwrap(), 0.0);
        assert_eq!(
            clamp_unit_interval(f64::INFINITY).unwrap_err(),
            DomainParameterError
        );
        assert_eq!(
            clamp_unit_interval(f64::NAN).unwrap_err(),
            DomainParameterError
        );
    }
}
