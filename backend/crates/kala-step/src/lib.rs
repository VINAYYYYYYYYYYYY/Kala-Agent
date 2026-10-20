//! Native exact analytic B-rep STEP export for Kala Studio primitives.
//!
//! This crate replaces the Python `kala_agent.studio.geometry.step` module
//! (OCP `BRepPrimAPI_MakeBox` / `MakeCylinder` / `MakeSphere` plus
//! `STEPControl_Writer`) with a self-contained Rust writer. It produces real
//! ISO 10303-21 exchange files whose solid is an exact analytic B-rep:
//!
//! * a box of six planar faces, twelve line edges and eight shared vertices,
//! * a cylinder of one cylindrical lateral face and two planar caps with
//!   full-circle edges and a seam line, and
//! * a sphere of one closed spherical face,
//!
//! with millimetre units and an origin-centred body (`box` extents
//! `+/- size / 2`, cylinder axis `+Z` over `height`, sphere at the origin).
//!
//! There is **no** tessellation, no mesh substitute, no Python/OCP process or
//! sidecar, and no C or C++ dependency in this build: the crate needs only the
//! Rust standard library plus the reviewed `kala-geometry` parameter/validation
//! layer and `serde_json`. The generated completeness is enforced by a native
//! boundary audit and independently checked by the test-only OpenCascade
//! re-import in `tests/ocp_oracle.py`.
//!
//! Dimensions, defaults, bounds and validation messages come from
//! [`kala_geometry`], so an invalid export fails with exactly the Python
//! message before any geometry is written.
//!
//! # Capability
//!
//! [`probe_capability`] reports `available = true` only when the built-in
//! bounded self-check succeeds. Because the writer has no optional native
//! dependency, a service build that links this crate always ships the exact
//! B-rep exporter; an adapter still keeps its own unavailable branch for
//! builds that do not link this crate.
//!
//! # Example
//!
//! ```
//! use kala_step::{PrimitiveKind, export_step};
//! use serde_json::json;
//!
//! let bytes = export_step(PrimitiveKind::Box, Some(&json!({"size_x": 20.0}))).unwrap();
//! assert!(bytes.starts_with(b"ISO-10303-21;"));
//! ```

#![forbid(unsafe_code)]

mod error;
mod model;
mod primitives;
mod serialize;

use std::path::Path;

use kala_geometry::resolve_dimension_parameters;
use serde_json::Value;

pub use error::StepError;
pub use serialize::{ISO_PREFIX, ISO_TERMINATOR, ISO_TRAILER};

pub use kala_geometry::{DimensionValues, PrimitiveKind, PrimitiveParameters};

/// The Python `STEP_OUTPUT_MODE` value: an exact B-rep, never a mesh.
pub const STEP_OUTPUT_MODE: &str = "exact_brep";

/// The exporter identity reported by [`StepCapability`].
pub const STEP_EXPORTER_NAME: &str = "kala-step native analytic B-rep writer";

/// Upper bound for one emitted document.
///
/// The largest supported primitive emits under 8 KiB; the cap turns a future
/// construction bug into a typed error instead of an unbounded response body.
pub const MAX_STEP_BYTES: usize = 64 * 1024;

/// Whether a STEP exporter is usable in this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepCapability {
    /// True only when this build can produce a verified STEP document.
    pub available: bool,
    /// The exporter identity, or `None` when unavailable.
    pub exporter_name: Option<&'static str>,
}

/// The typed exporter seam a service adapter implements.
///
/// The signature mirrors the Python entry point: a primitive kind, the raw
/// route parameters, and the work directory. This build never writes to
/// `workdir` — the document is assembled in memory, so there is no temporary
/// file to clean up — but the parameter is part of the contract so a service
/// can forward it unchanged.
pub trait StepExporter: Send + Sync {
    /// Report whether this exporter can produce STEP bytes.
    fn capability(&self) -> StepCapability;

    /// Serialise one primitive to a STEP byte string.
    ///
    /// # Errors
    ///
    /// Returns [`StepError::Invalid`] for dimensions the Python validators
    /// would refuse and [`StepError::Export`] when the writer cannot finish.
    fn export_primitive(
        &self,
        kind: PrimitiveKind,
        parameters: Option<&Value>,
        workdir: &Path,
    ) -> Result<Vec<u8>, StepError>;
}

/// The production exporter: the native analytic writer in this crate.
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeStepExporter;

impl StepExporter for NativeStepExporter {
    fn capability(&self) -> StepCapability {
        probe_capability()
    }

    fn export_primitive(
        &self,
        kind: PrimitiveKind,
        parameters: Option<&Value>,
        _workdir: &Path,
    ) -> Result<Vec<u8>, StepError> {
        export_step_primitive(kind, parameters)
    }
}

/// Report the STEP capability of this build.
///
/// Runs the bounded self-check in [`self_check`]; the exporter is reported
/// unavailable if the self-check fails, so a service never advertises a
/// capability it cannot deliver.
#[must_use]
pub fn probe_capability() -> StepCapability {
    match self_check() {
        Ok(()) => StepCapability {
            available: true,
            exporter_name: Some(STEP_EXPORTER_NAME),
        },
        Err(_) => StepCapability {
            available: false,
            exporter_name: None,
        },
    }
}

/// Run the built-in exporter self-check.
///
/// Builds the default box document in memory and verifies that it is a
/// complete ISO 10303-21 file with millimetre units and an analytic closed
/// shell. Costs a few microseconds and writes nothing.
///
/// # Errors
///
/// Returns the failure the same way an export would.
pub fn self_check() -> Result<(), StepError> {
    let bytes = export_typed(PrimitiveKind::Box, &PrimitiveParameters::new())?;
    let probe = |needle: &[u8]| bytes.windows(needle.len()).any(|window| window == needle);
    for needle in [
        b"SI_UNIT(.MILLI.,.METRE.)".as_slice(),
        b"MANIFOLD_SOLID_BREP".as_slice(),
        b"CLOSED_SHELL".as_slice(),
        b"ADVANCED_FACE".as_slice(),
    ] {
        if !probe(needle) {
            return Err(StepError::Export(
                "the STEP self-check produced an unexpected document".to_owned(),
            ));
        }
    }
    Ok(())
}

/// Serialise one primitive to a STEP byte string.
///
/// Parameters are read exactly like the Python route: `None` and JSON `null`
/// mean "no parameters supplied" so the canonical defaults apply, a JSON
/// object supplies named dimensions, and booleans, strings, arrays, objects or
/// non-finite numbers in a dimension slot are refused with the Python
/// validator message.
///
/// # Errors
///
/// Returns [`StepError::Invalid`] for invalid kinds or dimensions and
/// [`StepError::Export`] when the writer cannot complete.
pub fn export_step_primitive(
    kind: PrimitiveKind,
    parameters: Option<&Value>,
) -> Result<Vec<u8>, StepError> {
    let parameters = match parameters {
        Some(value) => PrimitiveParameters::from_json_value(value)?,
        None => PrimitiveParameters::new(),
    };
    export_typed(kind, &parameters)
}

/// Serialise one primitive from already typed parameters.
///
/// This is the direct path for Rust callers and the example CLI; the values
/// still run through the shared Python-compatible validators.
///
/// # Errors
///
/// Returns [`StepError::Invalid`] for an invalid kind or dimension and
/// [`StepError::Export`] when the writer cannot complete.
pub fn export_parameters(
    kind: PrimitiveKind,
    parameters: &PrimitiveParameters,
) -> Result<Vec<u8>, StepError> {
    export_typed(kind, parameters)
}

/// Serialise one primitive named by its lowercase kind string.
///
/// # Errors
///
/// Returns [`StepError::Invalid`] with the Python `unsupported kind: ...`
/// message for an unknown kind.
pub fn export_kind_str(kind: &str, parameters: Option<&Value>) -> Result<Vec<u8>, StepError> {
    let kind = PrimitiveKind::try_from(kind).map_err(StepError::Invalid)?;
    export_step_primitive(kind, parameters)
}

/// Alias for [`export_step_primitive`] matching the Python module function
/// name used by the studio service.
///
/// # Errors
///
/// See [`export_step_primitive`].
pub fn export_step(kind: PrimitiveKind, parameters: Option<&Value>) -> Result<Vec<u8>, StepError> {
    export_step_primitive(kind, parameters)
}

fn export_typed(
    kind: PrimitiveKind,
    parameters: &PrimitiveParameters,
) -> Result<Vec<u8>, StepError> {
    let dimensions = resolve_dimension_parameters(kind.as_str(), parameters)?;
    match dimensions {
        DimensionValues::Box {
            size_x,
            size_y,
            size_z,
        } => primitives::box_model(size_x, size_y, size_z)?.into_bytes("Kala Studio box"),
        DimensionValues::Cylinder { radius, height } => {
            primitives::cylinder_model(radius, height).into_bytes("Kala Studio cylinder")
        }
        DimensionValues::Sphere { radius } => {
            primitives::sphere_model(radius).into_bytes("Kala Studio sphere")
        }
    }
}

/// Verify a finished document before it leaves the crate.
///
/// Mirrors the Python writer's prefix refusal and additionally bounds the
/// output and checks ASCII-only content, so a construction bug can never ship
/// a partial or binary-contaminated response.
pub(crate) fn check_document(bytes: &[u8]) -> Result<(), StepError> {
    if bytes.len() > MAX_STEP_BYTES {
        return Err(StepError::Export(
            "the native STEP writer produced an oversized document".to_owned(),
        ));
    }
    if !bytes.starts_with(ISO_PREFIX) {
        return Err(StepError::Export(
            "the native STEP writer produced a non-ISO file; refusing to ship it".to_owned(),
        ));
    }
    let mut trailer = ISO_TRAILER.to_vec();
    trailer.extend_from_slice(ISO_TERMINATOR);
    if !bytes.ends_with(&trailer) {
        return Err(StepError::Export(
            "the native STEP writer produced a truncated file; refusing to ship it".to_owned(),
        ));
    }
    if !bytes.iter().all(u8::is_ascii) {
        return Err(StepError::Export(
            "the native STEP writer produced non-ASCII content".to_owned(),
        ));
    }
    Ok(())
}
