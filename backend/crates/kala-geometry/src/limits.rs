//! Finite bounds and resource caps for studio geometry requests.
//!
//! These constants and validators mirror
//! `kala_agent.studio.geometry.limits`. Every public builder normalises
//! user-controlled dimensions and tessellation counts through this module so
//! non-finite, non-positive, out-of-range, boolean, or wrong-kind values are
//! refused with the sanitized Python message before any math runs.

use crate::GeometryError;

/// Dimensions are millimetres; two metres is the hard preview ceiling.
pub const MAX_DIMENSION_MM: f64 = 2000.0;
/// Smallest accepted dimension in millimetres.
pub const MIN_DIMENSION_MM: f64 = 0.1;

/// Maximum box quads per edge.
pub const MAX_BOX_EDGE_SUBDIVISIONS: u32 = 64;
/// Default box quads per edge.
pub const DEFAULT_BOX_EDGE_SUBDIVISIONS: u32 = 1;

/// Maximum radial wedges for a cylinder.
pub const MAX_CYLINDER_RADIAL_SEGMENTS: u32 = 256;
/// Minimum radial wedges for a cylinder.
pub const MIN_CYLINDER_RADIAL_SEGMENTS: u32 = 6;
/// Default radial wedges for a cylinder.
pub const DEFAULT_CYLINDER_RADIAL_SEGMENTS: u32 = 48;

/// Maximum vertical cylinder segments.
pub const MAX_CYLINDER_HEIGHT_SEGMENTS: u32 = 64;
/// Minimum vertical cylinder segments.
pub const MIN_CYLINDER_HEIGHT_SEGMENTS: u32 = 1;
/// Default vertical cylinder segments.
pub const DEFAULT_CYLINDER_HEIGHT_SEGMENTS: u32 = 1;

/// Maximum sphere latitude rings.
pub const MAX_SPHERE_RINGS: u32 = 96;
/// Minimum sphere latitude rings.
pub const MIN_SPHERE_RINGS: u32 = 3;
/// Default sphere latitude rings.
pub const DEFAULT_SPHERE_RINGS: u32 = 24;

/// Maximum sphere longitude sectors.
pub const MAX_SPHERE_SECTORS: u32 = 128;
/// Minimum sphere longitude sectors.
pub const MIN_SPHERE_SECTORS: u32 = 4;
/// Default sphere longitude sectors.
pub const DEFAULT_SPHERE_SECTORS: u32 = 32;

/// Hard per-mesh vertex ceiling.
pub const MAX_MESH_VERTICES: usize = 200_000;
/// Hard per-mesh triangle ceiling.
pub const MAX_MESH_TRIANGLES: usize = 400_000;

/// Return a finite, positive dimension within the millimetre bounds.
///
/// Non-finite and out-of-range values produce the same sanitized messages as
/// Python's `validate_finite_positive`; the submitted value is never echoed.
///
/// # Errors
///
/// Returns [`GeometryError`] when `value` is not finite or is outside
/// `[MIN_DIMENSION_MM, MAX_DIMENSION_MM]`.
pub fn validate_finite_positive(name: &str, value: f64) -> Result<f64, GeometryError> {
    if !value.is_finite() {
        return Err(GeometryError::new(format!(
            "{name} must be a finite number"
        )));
    }
    if value < MIN_DIMENSION_MM || value > MAX_DIMENSION_MM {
        return Err(GeometryError::new(format!(
            "{name} must be between 0.1 and 2000.0 millimetres"
        )));
    }
    Ok(value)
}

/// Return an integer count bounded by an inclusive range.
///
/// The Python validator first checks the JSON kind (integer, not boolean and
/// not float), then the range. The caller is responsible for the kind check;
/// [`crate::ParameterValue::as_count`] performs it for JSON input. This
/// function performs the range check and converts to the `u32` used by the
/// builders.
///
/// # Errors
///
/// Returns [`GeometryError`] when `value` is outside `[minimum, maximum]`.
pub fn validate_int_in_range(
    name: &str,
    value: i128,
    minimum: u32,
    maximum: u32,
) -> Result<u32, GeometryError> {
    if value < i128::from(minimum) || value > i128::from(maximum) {
        return Err(GeometryError::new(format!(
            "{name} must be between {minimum} and {maximum}"
        )));
    }
    // The range is a u32 range and `value` is inside it, so this conversion
    // cannot truncate.
    Ok(value as u32)
}

pub(crate) fn check_resource_bounds(
    vertex_count: usize,
    triangle_count: usize,
) -> Result<(), GeometryError> {
    if vertex_count > MAX_MESH_VERTICES {
        return Err(GeometryError::new(format!(
            "the requested primitive would produce {vertex_count} vertices; \
             the limit is {MAX_MESH_VERTICES}"
        )));
    }
    if triangle_count > MAX_MESH_TRIANGLES {
        return Err(GeometryError::new(format!(
            "the requested primitive would produce {triangle_count} triangles; \
             the limit is {MAX_MESH_TRIANGLES}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_boundaries_are_inclusive() {
        assert_eq!(
            validate_finite_positive("size_x", MIN_DIMENSION_MM).unwrap(),
            MIN_DIMENSION_MM
        );
        assert_eq!(
            validate_finite_positive("size_x", MAX_DIMENSION_MM).unwrap(),
            MAX_DIMENSION_MM
        );
        assert_eq!(
            validate_finite_positive("size_x", 0.099_999_999_999_999_99)
                .unwrap_err()
                .message(),
            "size_x must be between 0.1 and 2000.0 millimetres"
        );
        assert_eq!(
            validate_finite_positive("size_x", 2_000.000_000_000_1)
                .unwrap_err()
                .message(),
            "size_x must be between 0.1 and 2000.0 millimetres"
        );
    }

    #[test]
    fn non_finite_dimension_is_rejected() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                validate_finite_positive("radius", value)
                    .unwrap_err()
                    .message(),
                "radius must be a finite number"
            );
        }
    }

    #[test]
    fn integer_range_messages_match_python() {
        assert_eq!(
            validate_int_in_range("subdivisions", 0, 1, 64)
                .unwrap_err()
                .message(),
            "subdivisions must be between 1 and 64"
        );
        assert_eq!(
            validate_int_in_range("subdivisions", 65, 1, 64)
                .unwrap_err()
                .message(),
            "subdivisions must be between 1 and 64"
        );
        assert_eq!(
            validate_int_in_range("radial_segments", 5, 6, 256)
                .unwrap_err()
                .message(),
            "radial_segments must be between 6 and 256"
        );
    }

    #[test]
    fn resource_messages_match_python() {
        assert_eq!(
            check_resource_bounds(MAX_MESH_VERTICES + 1, 0)
                .unwrap_err()
                .message(),
            "the requested primitive would produce 200001 vertices; the limit is 200000"
        );
        assert_eq!(
            check_resource_bounds(0, MAX_MESH_TRIANGLES + 1)
                .unwrap_err()
                .message(),
            "the requested primitive would produce 400001 triangles; the limit is 400000"
        );
    }
}
