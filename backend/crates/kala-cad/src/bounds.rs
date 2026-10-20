//! Declared and measured bounds, measurement rows and honesty checks.
//!
//! This is the kernel-independent part of the Python `design/measure.py`
//! module: `Bounds`, `Measurement`, `Check`, `CheckReport`,
//! `build_checks` and `measure_and_check`'s check construction. The measured
//! numbers themselves always come from the native kernel in
//! [`crate::measure_shape`]; nothing here invents a volume, area or envelope.

use std::fmt;

use crate::error::BoundsError;

/// The declared-vs-measured envelope tolerance in millimetres.
pub const ENVELOPE_TOLERANCE_MM: f64 = 0.01;
/// The exact single-solid tolerance (a count, not millimetres).
pub const SINGLE_SOLID_TOLERANCE: f64 = 0.0;
/// The exact B-Rep validity tolerance (a boolean, not millimetres).
pub const BREP_VALID_TOLERANCE: f64 = 0.0;

/// One check value: a boolean, a number, or an inclusive `(min, max)` pair.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CheckValue {
    /// A boolean expectation/measurement.
    Bool(bool),
    /// A scalar expectation/measurement.
    Float(f64),
    /// A pair of scalar values (used for an axis envelope).
    Pair(f64, f64),
}

impl CheckValue {
    /// Return the boolean payload, if this is a boolean value.
    pub fn as_bool(self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(value),
            _ => None,
        }
    }

    /// Return the float payload, if this is a scalar value.
    pub fn as_float(self) -> Option<f64> {
        match self {
            Self::Float(value) => Some(value),
            _ => None,
        }
    }

    /// Return the pair payload, if this is a pair value.
    pub fn as_pair(self) -> Option<(f64, f64)> {
        match self {
            Self::Pair(low, high) => Some((low, high)),
            _ => None,
        }
    }
}

impl fmt::Display for CheckValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(value) => write!(formatter, "{value}"),
            Self::Float(value) => write!(formatter, "{value}"),
            Self::Pair(low, high) => write!(formatter, "({low}, {high})"),
        }
    }
}

/// An axis-aligned box in millimetres, measured or declared.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    /// Minimum corner along X, Y, Z.
    pub min_mm: [f64; 3],
    /// Maximum corner along X, Y, Z.
    pub max_mm: [f64; 3],
}

impl Bounds {
    /// Validate a three-axis box.
    ///
    /// The error messages match the Python dataclass: non-finite values,
    /// and a minimum above the maximum, are refused before any kernel call.
    pub fn new(min_mm: [f64; 3], max_mm: [f64; 3]) -> Result<Self, BoundsError> {
        Self::from_slices(&min_mm, &max_mm)
    }

    /// Validate a box from slices, including the Python length check.
    ///
    /// The frozen Python `Bounds` accepts arbitrary sequences and rejects a
    /// length other than three with "bounds require three minimum and three
    /// maximum values". A fixed-array constructor cannot express that case,
    /// so this entry point preserves it for JSON/session adapters.
    pub fn from_slices(min_mm: &[f64], max_mm: &[f64]) -> Result<Self, BoundsError> {
        if min_mm.len() != 3 || max_mm.len() != 3 {
            return Err(BoundsError::new(
                "bounds require three minimum and three maximum values",
            ));
        }
        for value in min_mm.iter().chain(max_mm.iter()) {
            if !value.is_finite() {
                return Err(BoundsError::new("bounds must be finite numbers"));
            }
        }
        for axis in 0..3 {
            if min_mm[axis] > max_mm[axis] {
                return Err(BoundsError::new("bounds minimum exceeds maximum"));
            }
        }
        Ok(Self {
            min_mm: [min_mm[0], min_mm[1], min_mm[2]],
            max_mm: [max_mm[0], max_mm[1], max_mm[2]],
        })
    }

    /// The box size along X, Y and Z.
    pub fn size_mm(&self) -> [f64; 3] {
        [
            self.max_mm[0] - self.min_mm[0],
            self.max_mm[1] - self.min_mm[1],
            self.max_mm[2] - self.min_mm[2],
        ]
    }
}

/// Every kernel-measured number for one exact shape.
#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    /// Exact kernel volume in cubic millimetres.
    pub volume_mm3: f64,
    /// Exact kernel surface area in square millimetres.
    pub surface_area_mm2: f64,
    /// Kernel centre of mass in millimetres.
    pub centroid_mm: [f64; 3],
    /// Optimal axis-aligned bounds in millimetres.
    pub bounds: Bounds,
    /// `BRepCheck_Analyzer` validity.
    pub valid: bool,
    /// Number of `TopAbs_SOLID` sub-shapes.
    pub solid_count: usize,
}

/// Occurrence counts for one exact shape.
///
/// The Python source exposes only `solid_count`; this additive summary exists
/// because the native measurement foundation is also the topology source for
/// later drawing/session consumers. Counts use `TopExp_Explorer`, the same
/// occurrence semantics as the source's solid count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TopologyCounts {
    /// Number of `TopAbs_SOLID` occurrences.
    pub solids: usize,
    /// Number of `TopAbs_SHELL` occurrences.
    pub shells: usize,
    /// Number of `TopAbs_FACE` occurrences.
    pub faces: usize,
    /// Number of `TopAbs_EDGE` occurrences.
    pub edges: usize,
    /// Number of `TopAbs_VERTEX` occurrences.
    pub vertices: usize,
}

/// One declared-versus-measured check row.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    /// Stable row id (`brep_valid`, `single_solid`, `envelope_x`, ...).
    pub id: String,
    /// Human label used by the UI.
    pub label: String,
    /// Declared value.
    pub expected: CheckValue,
    /// Kernel or topology value.
    pub measured: CheckValue,
    /// Allowed difference (0 for exact checks).
    pub tolerance: f64,
    /// `bool`, `solid` or `mm`.
    pub unit: String,
    /// Whether this row passed.
    pub passed: bool,
}

/// The ordered checks and the single verification verdict.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckReport {
    /// Ordered rows: validity, single-solid (optional), envelope X/Y/Z.
    pub checks: Vec<Check>,
    /// True only when every row passed.
    pub verified: bool,
}

/// Build the mandatory checks and the `verified` verdict.
///
/// `brep_valid` is always present. `single_solid` is present when the caller
/// expects one part. The three envelope checks are present only when a
/// declared envelope exists; no envelope is invented for a raw script.
pub fn build_checks(
    measurement: &Measurement,
    expected_envelope: Option<&Bounds>,
    expect_single_solid: bool,
) -> CheckReport {
    let mut checks = vec![Check {
        id: "brep_valid".to_owned(),
        label: "B-Rep valid".to_owned(),
        expected: CheckValue::Bool(true),
        measured: CheckValue::Bool(measurement.valid),
        tolerance: BREP_VALID_TOLERANCE,
        unit: "bool".to_owned(),
        passed: measurement.valid,
    }];
    if expect_single_solid {
        checks.push(Check {
            id: "single_solid".to_owned(),
            label: "Single solid".to_owned(),
            expected: CheckValue::Float(1.0),
            measured: CheckValue::Float(measurement.solid_count as f64),
            tolerance: SINGLE_SOLID_TOLERANCE,
            unit: "solid".to_owned(),
            passed: measurement.solid_count == 1,
        });
    }
    if let Some(envelope) = expected_envelope {
        for (axis, name) in ["x", "y", "z"].into_iter().enumerate() {
            let expected = (envelope.min_mm[axis], envelope.max_mm[axis]);
            let measured = (
                measurement.bounds.min_mm[axis],
                measurement.bounds.max_mm[axis],
            );
            let passed = (expected.0 - measured.0).abs() <= ENVELOPE_TOLERANCE_MM
                && (expected.1 - measured.1).abs() <= ENVELOPE_TOLERANCE_MM;
            checks.push(Check {
                id: format!("envelope_{name}"),
                label: format!("Envelope {}", name.to_uppercase()),
                expected: CheckValue::Pair(expected.0, expected.1),
                measured: CheckValue::Pair(measured.0, measured.1),
                tolerance: ENVELOPE_TOLERANCE_MM,
                unit: "mm".to_owned(),
                passed,
            });
        }
    }
    let verified = checks.iter().all(|check| check.passed);
    CheckReport { checks, verified }
}
