//! Canonical primitive dimensions, kinds, and typed specifications.
//!
//! Mirrors `kala_agent.studio.geometry.primitives`: "no parameters supplied"
//! resolves in one place so the mesh builders and any future exact exporter
//! agree on defaults and bounds.

use serde::{Deserialize, Serialize};

use crate::GeometryError;
use crate::limits::validate_finite_positive;
use crate::params::PrimitiveParameters;

/// Default box edge length in millimetres.
pub const DEFAULT_BOX_SIZE_MM: f64 = 10.0;
/// Default cylinder radius in millimetres.
pub const DEFAULT_CYLINDER_RADIUS_MM: f64 = 5.0;
/// Default cylinder height in millimetres.
pub const DEFAULT_CYLINDER_HEIGHT_MM: f64 = 10.0;
/// Default sphere radius in millimetres.
pub const DEFAULT_SPHERE_RADIUS_MM: f64 = 5.0;

const BOX_DIMENSIONS: &[&str] = &["size_x", "size_y", "size_z"];
const CYLINDER_DIMENSIONS: &[&str] = &["radius", "height"];
const SPHERE_DIMENSIONS: &[&str] = &["radius"];

/// The supported analytic primitive kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrimitiveKind {
    Box,
    Cylinder,
    Sphere,
}

impl PrimitiveKind {
    /// Return the Python-compatible lowercase kind name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Box => "box",
            Self::Cylinder => "cylinder",
            Self::Sphere => "sphere",
        }
    }
}

impl std::fmt::Display for PrimitiveKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<&str> for PrimitiveKind {
    type Error = GeometryError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "box" => Ok(Self::Box),
            "cylinder" => Ok(Self::Cylinder),
            "sphere" => Ok(Self::Sphere),
            _ => Err(GeometryError::new(format!("unsupported kind: {value}"))),
        }
    }
}

impl std::str::FromStr for PrimitiveKind {
    type Err = GeometryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value)
    }
}

/// Validated dimension values for one primitive, in Python key order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DimensionValues {
    Box {
        size_x: f64,
        size_y: f64,
        size_z: f64,
    },
    Cylinder {
        radius: f64,
        height: f64,
    },
    Sphere {
        radius: f64,
    },
}

/// Return the dimension parameter names for `kind`, in Python dict order.
///
/// # Errors
///
/// Returns `unsupported kind: ...` for an unknown kind.
pub fn dimension_keys(kind: &str) -> Result<&'static [&'static str], GeometryError> {
    match kind {
        "box" => Ok(BOX_DIMENSIONS),
        "cylinder" => Ok(CYLINDER_DIMENSIONS),
        "sphere" => Ok(SPHERE_DIMENSIONS),
        other => Err(GeometryError::new(format!("unsupported kind: {other}"))),
    }
}

/// Resolve and validate all dimensions for `kind`.
///
/// Missing values take the same defaults the Python preview mesh uses; every
/// supplied value is checked through the shared finite/positive guard. JSON
/// booleans, floats for integer-only fields, strings, nulls, arrays, and
/// objects are refused by [`crate::ParameterValue`] before this function is
/// reached.
///
/// # Errors
///
/// Returns the Python validation message for an unknown kind or an invalid
/// dimension.
pub fn resolve_dimension_parameters(
    kind: &str,
    parameters: &PrimitiveParameters,
) -> Result<DimensionValues, GeometryError> {
    match kind {
        "box" => Ok(DimensionValues::Box {
            size_x: resolve_dimension(parameters, "size_x", DEFAULT_BOX_SIZE_MM)?,
            size_y: resolve_dimension(parameters, "size_y", DEFAULT_BOX_SIZE_MM)?,
            size_z: resolve_dimension(parameters, "size_z", DEFAULT_BOX_SIZE_MM)?,
        }),
        "cylinder" => Ok(DimensionValues::Cylinder {
            radius: resolve_dimension(parameters, "radius", DEFAULT_CYLINDER_RADIUS_MM)?,
            height: resolve_dimension(parameters, "height", DEFAULT_CYLINDER_HEIGHT_MM)?,
        }),
        "sphere" => Ok(DimensionValues::Sphere {
            radius: resolve_dimension(parameters, "radius", DEFAULT_SPHERE_RADIUS_MM)?,
        }),
        other => Err(GeometryError::new(format!("unsupported kind: {other}"))),
    }
}

fn resolve_dimension(
    parameters: &PrimitiveParameters,
    name: &str,
    default: f64,
) -> Result<f64, GeometryError> {
    match parameters.get(name) {
        Some(value) => value.as_dimension(name),
        None => validate_finite_positive(name, default),
    }
}

/// A fully typed primitive specification with explicit tessellation.
///
/// Unlike [`PrimitiveParameters`], this type cannot carry booleans, strings,
/// or floats where counts are required; it is the ergonomic API for Rust
/// callers. The builders still re-check dimension and tessellation bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PrimitiveSpec {
    Box {
        size: [f64; 3],
        subdivisions: u32,
    },
    Cylinder {
        radius: f64,
        height: f64,
        radial_segments: u32,
        height_segments: u32,
    },
    Sphere {
        radius: f64,
        rings: u32,
        sectors: u32,
    },
}

impl PrimitiveSpec {
    /// Default box: 10 mm cube with one quad per face.
    #[must_use]
    pub fn default_box() -> Self {
        Self::Box {
            size: [DEFAULT_BOX_SIZE_MM; 3],
            subdivisions: crate::limits::DEFAULT_BOX_EDGE_SUBDIVISIONS,
        }
    }

    /// Default cylinder: radius 5 mm, height 10 mm, 48 radial wedges.
    #[must_use]
    pub fn default_cylinder() -> Self {
        Self::Cylinder {
            radius: DEFAULT_CYLINDER_RADIUS_MM,
            height: DEFAULT_CYLINDER_HEIGHT_MM,
            radial_segments: crate::limits::DEFAULT_CYLINDER_RADIAL_SEGMENTS,
            height_segments: crate::limits::DEFAULT_CYLINDER_HEIGHT_SEGMENTS,
        }
    }

    /// Default sphere: radius 5 mm, 24 rings by 32 sectors.
    #[must_use]
    pub fn default_sphere() -> Self {
        Self::Sphere {
            radius: DEFAULT_SPHERE_RADIUS_MM,
            rings: crate::limits::DEFAULT_SPHERE_RINGS,
            sectors: crate::limits::DEFAULT_SPHERE_SECTORS,
        }
    }

    /// Return the kind of this specification.
    #[must_use]
    pub fn kind(&self) -> PrimitiveKind {
        match self {
            Self::Box { .. } => PrimitiveKind::Box,
            Self::Cylinder { .. } => PrimitiveKind::Cylinder,
            Self::Sphere { .. } => PrimitiveKind::Sphere,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_keys_match_python_order() {
        assert_eq!(dimension_keys("box").unwrap(), BOX_DIMENSIONS);
        assert_eq!(dimension_keys("cylinder").unwrap(), CYLINDER_DIMENSIONS);
        assert_eq!(dimension_keys("sphere").unwrap(), SPHERE_DIMENSIONS);
        assert_eq!(
            dimension_keys("torus").unwrap_err().message(),
            "unsupported kind: torus"
        );
    }

    #[test]
    fn defaults_resolve_without_parameters() {
        match resolve_dimension_parameters("box", &PrimitiveParameters::new()).unwrap() {
            DimensionValues::Box {
                size_x,
                size_y,
                size_z,
            } => assert_eq!([size_x, size_y, size_z], [10.0, 10.0, 10.0]),
            _ => panic!("box resolved to the wrong kind"),
        }
    }

    #[test]
    fn kind_round_trips() {
        for kind in [
            PrimitiveKind::Box,
            PrimitiveKind::Cylinder,
            PrimitiveKind::Sphere,
        ] {
            assert_eq!(kind.as_str().parse::<PrimitiveKind>().unwrap(), kind);
            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                format!("\"{}\"", kind.as_str())
            );
        }
        assert_eq!(
            "torus".parse::<PrimitiveKind>().unwrap_err().message(),
            "unsupported kind: torus"
        );
    }
}
