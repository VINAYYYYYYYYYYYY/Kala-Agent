//! The typed STEP capability seam and the parked native primitive geometry.
//!
//! ADR-0010 parks the native geometry crates: `kala-geometry` and
//! `kala-step` are **not** default dependencies of this crate, and the
//! default build contains no native geometry at all. The supervised Python
//! geometry worker is the front door's geometry source; see
//! [`crate::geometry_backend`].
//!
//! What stays in every build is the seam itself: [`StepExporter`],
//! [`StepCapability`] and [`UnavailableStepExporter`], so a build without the
//! parked feature keeps a truthful `available = false` capability and a real
//! `step_unavailable` 503 branch instead of fabricating STEP bytes.
//!
//! With the non-default `native-primitives` feature the reviewed native
//! analytic mesh and B-rep writer are linked again and
//! [`crate::native_backend::NativePrimitivesBackend`] serves the primitive
//! routes. Nothing is deleted.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::schemas::PrimitiveKind;

#[cfg(feature = "native-primitives")]
use crate::json::JsonWriteError;
#[cfg(feature = "native-primitives")]
use kala_geometry::{GeometryError, Mesh};

/// The Python `STEP_UNAVAILABLE_MESSAGE` top-level text is preserved in
/// shape, but the native build names the native capability rather than an
/// optional Python package that this binary will never load.
pub const STEP_UNAVAILABLE_MESSAGE: &str = "STEP export requires a native B-rep exporter, which is not integrated in this studio build. The studio never fabricates a STEP representation; native STEP export is an explicit pending dependency.";

/// Whether a STEP exporter is present in this process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepCapability {
    pub available: bool,
    pub exporter_name: Option<String>,
}

/// A typed STEP export failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepError {
    /// The dimensions the Python validators would refuse (`ValueError`).
    Invalid(String),
    /// The capability is not integrated in this build.
    Unavailable(String),
    /// A native exporter could not produce a shape.
    Export(String),
}

impl std::fmt::Display for StepError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) | Self::Unavailable(message) | Self::Export(message) => {
                formatter.write_str(message)
            }
        }
    }
}

/// The production seam a STEP exporter implements.
pub trait StepExporter: Send + Sync {
    fn capability(&self) -> StepCapability;

    fn export_primitive(
        &self,
        kind: PrimitiveKind,
        parameters: Option<&Value>,
        workdir: &Path,
    ) -> Result<Vec<u8>, StepError>;
}

/// The exporter a default build links, or the truthful unavailable seam when
/// the `native-primitives` feature is disabled.
#[must_use]
pub fn default_step_exporter() -> Arc<dyn StepExporter> {
    #[cfg(feature = "native-primitives")]
    {
        Arc::new(NativeStepExporter)
    }
    #[cfg(not(feature = "native-primitives"))]
    {
        Arc::new(UnavailableStepExporter)
    }
}

/// The production adapter over the reviewed native analytic B-rep writer.
#[cfg(feature = "native-primitives")]
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeStepExporter;

#[cfg(feature = "native-primitives")]
impl StepExporter for NativeStepExporter {
    fn capability(&self) -> StepCapability {
        let capability = kala_step::probe_capability();
        StepCapability {
            available: capability.available,
            exporter_name: capability.exporter_name.map(str::to_owned),
        }
    }

    fn export_primitive(
        &self,
        kind: PrimitiveKind,
        parameters: Option<&Value>,
        workdir: &Path,
    ) -> Result<Vec<u8>, StepError> {
        // The native writer validates through the same reviewed
        // `kala-geometry` layer, so an invalid dimension keeps the exact
        // Python `ValueError` text and the route's 422 branch. `workdir` is
        // part of the source contract; the native writer assembles the
        // document in memory and never touches it.
        let _ = workdir;
        let kind = kala_step::PrimitiveKind::try_from(kind.as_str())
            .map_err(|error| StepError::Invalid(error.message().to_owned()))?;
        kala_step::export_step_primitive(kind, parameters).map_err(|error| match error {
            kala_step::StepError::Invalid(inner) => StepError::Invalid(inner.message().to_owned()),
            kala_step::StepError::Export(message) => StepError::Export(message),
        })
    }
}

/// The truthful alternate exporter: no capability, no bytes.
#[derive(Debug, Default)]
pub struct UnavailableStepExporter;

impl StepExporter for UnavailableStepExporter {
    fn capability(&self) -> StepCapability {
        StepCapability {
            available: false,
            exporter_name: None,
        }
    }

    fn export_primitive(
        &self,
        _kind: PrimitiveKind,
        _parameters: Option<&Value>,
        _workdir: &Path,
    ) -> Result<Vec<u8>, StepError> {
        Err(StepError::Unavailable(STEP_UNAVAILABLE_MESSAGE.to_owned()))
    }
}

#[cfg(feature = "native-primitives")]
/// Build a validated analytic mesh from route parameters.
///
/// # Errors
///
/// Returns the Python validation text, or a flag for the legacy
/// `OverflowError` branch when an integer dimension cannot become an `f64`.
pub fn build_mesh(
    kind: PrimitiveKind,
    parameters: Option<&Value>,
) -> Result<Mesh, GeometryRouteError> {
    if has_overflowing_dimension(kind, parameters) {
        return Err(GeometryRouteError::Overflow);
    }
    kala_geometry::build_primitive_json(kind.as_str(), parameters)
        .map_err(GeometryRouteError::Invalid)
}

/// Geometry failures as the service observes them.
#[derive(Debug)]
#[cfg(feature = "native-primitives")]
pub enum GeometryRouteError {
    /// Python raises `ValueError`, mapped to a 422 route error.
    Invalid(GeometryError),
    /// Python raises `OverflowError` for a too-wide integer dimension; the
    /// original route returns a sanitized 500.
    Overflow,
}

#[cfg(feature = "native-primitives")]
/// Detect the Python `float(huge_int)` overflow branch before the geometry
/// crate turns it into a validation error.
fn has_overflowing_dimension(kind: PrimitiveKind, parameters: Option<&Value>) -> bool {
    let Some(Value::Object(object)) = parameters else {
        return false;
    };
    let names: &[&str] = match kind {
        PrimitiveKind::Box => &["size_x", "size_y", "size_z"],
        PrimitiveKind::Cylinder => &["radius", "height"],
        PrimitiveKind::Sphere => &["radius"],
    };
    for name in names {
        let Some(Value::Number(number)) = object.get(*name) else {
            continue;
        };
        let text = number.to_string();
        if text.bytes().any(|byte| matches!(byte, b'.' | b'e' | b'E')) {
            continue;
        }
        if text.trim_start_matches('-').len() > 309 {
            return true;
        }
        if text.parse::<f64>().is_ok_and(|value| value.is_infinite()) {
            return true;
        }
    }
    false
}

#[cfg(feature = "native-primitives")]
/// Serialise the Python `PreviewResponse` body for a mesh.
///
/// # Errors
///
/// Returns [`JsonWriteError`] if a non-finite float reaches the response.
pub fn preview_body(mesh: &Mesh) -> Result<Vec<u8>, JsonWriteError> {
    let bounds = kala_geometry::compute_bounds(mesh.positions()).map_err(|_| JsonWriteError)?;
    let mut out = String::with_capacity(mesh.positions().len() * 8);
    out.push_str("{\"positions\":");
    write_float_array(&mut out, mesh.positions())?;
    out.push_str(",\"normals\":");
    write_float_array(&mut out, mesh.normals())?;
    out.push_str(",\"indices\":[");
    for (index, value) in mesh.indices().iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&value.to_string());
    }
    out.push_str("],\"bounds\":{\"min\":");
    write_float_triple(&mut out, bounds.min)?;
    out.push_str(",\"max\":");
    write_float_triple(&mut out, bounds.max)?;
    out.push_str("},\"engine\":\"");
    out.push_str(kala_geometry::MESH_ENGINE);
    out.push_str("\",\"warnings\":[\"");
    out.push_str(kala_geometry::ANALYTIC_WARNING);
    out.push_str("\"]}");
    Ok(out.into_bytes())
}

#[cfg(feature = "native-primitives")]
fn write_float_array(out: &mut String, values: &[f64]) -> Result<(), JsonWriteError> {
    out.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&pydantic_float(*value)?);
    }
    out.push(']');
    Ok(())
}

#[cfg(feature = "native-primitives")]
/// Float text for a Pydantic `response_model` body.
///
/// FastAPI serialises a `response_model` through pydantic-core, whose float
/// text is the Rust `serde_json`/ryu form (decimal down to `1e-5`, `1e-6`
/// scientific without zero padding, `1e+16` with a sign). This is *not*
/// CPython `repr`, which is why the preview fixture digest differs from a
/// `json.dumps` body. Verified against the live Pydantic oracle over random
/// finite bit patterns.
fn pydantic_float(value: f64) -> Result<String, JsonWriteError> {
    if !value.is_finite() {
        return Err(JsonWriteError);
    }
    serde_json::to_string(&value).map_err(|_| JsonWriteError)
}

#[cfg(feature = "native-primitives")]
fn write_float_triple(out: &mut String, values: [f64; 3]) -> Result<(), JsonWriteError> {
    out.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&pydantic_float(*value)?);
    }
    out.push(']');
    Ok(())
}

/// Serialise a mesh as deterministic binary STL.
#[must_use]
#[cfg(feature = "native-primitives")]
pub fn stl_bytes(mesh: &Mesh) -> Vec<u8> {
    kala_geometry::write_binary_stl(mesh)
}

/// Resolve the Python `_resolve_workdir` value.
#[must_use]
#[cfg(feature = "native-primitives")]
pub fn resolve_step_workdir(explicit_data_dir: Option<&str>) -> std::path::PathBuf {
    if let Some(value) = explicit_data_dir {
        if !value.is_empty() {
            return std::path::PathBuf::from(value);
        }
    }
    std::env::temp_dir().join("kala-studio-work")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "native-primitives")]
    #[test]
    fn preview_body_matches_geometry_values() {
        let mesh = build_mesh(PrimitiveKind::Box, None).unwrap();
        let body = String::from_utf8(preview_body(&mesh).unwrap()).unwrap();
        assert!(body.starts_with("{\"positions\":[-5.0,-5.0,5.0"));
        assert!(body.contains("\"engine\":\"analytic\""));
        assert!(body.ends_with("]}"));
        assert!(!body.contains("-0.0"));
    }

    #[cfg(feature = "native-primitives")]
    #[test]
    fn overflow_branch_is_detected_separately() {
        let huge = "9".repeat(400);
        let value: Value =
            serde_json::from_str(&format!("{{\"size_x\":{huge},\"size_y\":1,\"size_z\":1}}"))
                .unwrap();
        assert!(matches!(
            build_mesh(PrimitiveKind::Box, Some(&value)),
            Err(GeometryRouteError::Overflow)
        ));
        let float_value: Value =
            serde_json::from_str(r#"{"size_x":1e400,"size_y":1,"size_z":1}"#).unwrap();
        match build_mesh(PrimitiveKind::Box, Some(&float_value)) {
            Err(GeometryRouteError::Invalid(error)) => {
                assert_eq!(error.message(), "size_x must be a finite number");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn unavailable_exporter_is_truthful() {
        let exporter = UnavailableStepExporter;
        assert!(!exporter.capability().available);
        assert!(matches!(
            exporter.export_primitive(PrimitiveKind::Box, None, Path::new("/tmp")),
            Err(StepError::Unavailable(_))
        ));
    }

    #[cfg(feature = "native-primitives")]
    #[test]
    fn native_exporter_is_the_default_and_emits_real_step() {
        let exporter = default_step_exporter();
        let capability = exporter.capability();
        assert!(
            capability.available,
            "the linked native writer must report capable"
        );
        assert!(
            capability
                .exporter_name
                .as_deref()
                .is_some_and(|name| name.contains("kala-step")),
            "unexpected exporter identity: {capability:?}"
        );
        let bytes = exporter
            .export_primitive(PrimitiveKind::Box, None, Path::new("/tmp"))
            .expect("the default exporter writes a box");
        assert!(bytes.starts_with(b"ISO-10303-21;"));
        assert!(
            bytes
                .windows("MANIFOLD_SOLID_BREP".len())
                .any(|window| window == b"MANIFOLD_SOLID_BREP")
        );
        // Invalid dimensions keep the Python validation text for the 422 path.
        let invalid = exporter.export_primitive(
            PrimitiveKind::Box,
            Some(&serde_json::json!({"size_x": 0.01})),
            Path::new("/tmp"),
        );
        match invalid {
            Err(StepError::Invalid(message)) => {
                assert_eq!(message, "size_x must be between 0.1 and 2000.0 millimetres");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
