//! Native general CAD foundation for Kala Agent.
//!
//! This crate ports the complete Studio v1 construction compiler,
//! measurement/check layer, tessellation/STL layer, STEP export/import,
//! hidden-line views and exact-area section cuts to native Rust over a narrow,
//! owned C++ adapter to OpenCascade.
//!
//! The division of responsibility is deliberate:
//!
//! * **Rust** owns contract validation (through the accepted `kala-contracts`
//!   crate), construction semantics, ordering, units, refusal messages,
//!   budgets, measurement labels and STEP policy.
//! * **C++** owns only the third-party kernel calls and RAII for
//!   `TopoDS_Shape`, meshes and STEP buffers. It never formats product text.
//!
//! General exact B-Rep work always uses the real native OpenCascade kernel.
//! There is no analytic approximation path, no Python/OCP subprocess, no
//! product sidecar and no build-time Python discovery. Building with
//! `--no-default-features` removes the kernel entirely and reports
//! `design_available() == false`; it does not silently substitute geometry.
//!
//! # Threading and shape lifetimes
//!
//! [`Shape`] owns one C++ `TopoDS_Shape` and is deliberately not `Send` or
//! `Sync`. Independent shapes may be compiled, measured, tessellated and
//! exported on different threads. STEP export/import is serialised behind one
//! private mutex because OCCT stores STEP unit/schema settings in a global
//! registry; the crate never races those settings with itself.
//!
//! # Capability
//!
//! [`probe_capability`] performs a bounded end-to-end self-check (compile a
//! rectangle pad, measure it, tessellate it, export STEP, re-import and
//! measure the result) before it reports `available = true`. A build that
//! links the library but cannot execute the kernel therefore does not
//! advertise a working design pipeline.

#![forbid(unsafe_op_in_unsafe_fn)]

mod bounds;
mod drawing;
mod error;
mod mesh_data;
mod step_common;

pub use bounds::{
    BREP_VALID_TOLERANCE, Bounds, Check, CheckReport, CheckValue, ENVELOPE_TOLERANCE_MM,
    Measurement, SINGLE_SOLID_TOLERANCE, TopologyCounts, build_checks,
};
pub use drawing::{
    Bounds2D, DEFAULT_CHORD_TOLERANCE_MM, DEFAULT_SECTION_CHORD_TOLERANCE_MM, DEFAULT_VIEW_NAMES,
    Drawing, MAX_POINTS_PER_VIEW, MAX_SECTION_POINTS, Point2, Polyline, Section, SectionLoop,
    SectionPlane, ViewDrawing, ViewName,
};
pub use error::{
    BoundsError, CompileError, CompileErrorKind, DrawingError, DrawingErrorKind, KernelError,
    MeasurementError, MeshError, StepError, StepImportError, StepImportErrorKind,
};
pub use mesh_data::{
    DEFAULT_ANGULAR_DEFLECTION_RAD, DEFAULT_MESH_DEFLECTION_MM, MAX_MESH_TRIANGLES,
    MAX_MESH_VERTICES, MESH_ENGINE, MeshData, MeshOptions, STL_HEADER_FILL, STL_HEADER_SIZE,
    stl_bytes,
};
pub use step_common::{
    DEFAULT_MAX_STEP_BYTES, FIXED_STEP_TIMESTAMP, MAX_STEP_IMPORT_BYTES, MAX_VIEW_COORDINATE_MM,
    MAX_WARNING_LENGTH, MIN_STEP_IMPORT_BYTES, STEP_HEADER, STEP_IMPORT_MAX_BYTES_ENV,
    StepImportOptions, StepWriteOptions, normalise_step_bytes, resolve_step_import_max_bytes,
    resolve_step_import_max_bytes_from_env, validate_step_import_max_bytes,
};

// Frozen construction constants. They mirror the contract/compiler source and
// are shared by the native and explicit-unavailable builds.
/// Default wall-clock compile budget in seconds.
///
/// A 500-step design (a 2 000 x 2 000 x 5 mm plate with 249 through holes)
/// compiles and measures in 16.9 s on this host, and the 202-step 100-hole
/// plate in 2.5 s. Sixty seconds covers the whole budget with margin while
/// still refusing a runaway compile.
pub const DEFAULT_COMPILE_TIMEOUT_SECONDS: f64 = 60.0;
/// Component operations the v1 compiler can lower.
pub const SUPPORTED_COMPONENT_OPS: [&str; 4] = ["sketch", "extrude", "revolve", "pocket"];
/// Assembly operations the v1 compiler can lower.
pub const SUPPORTED_ASSEMBLY_OPS: [&str; 2] = ["place", "fuse"];
/// The exact refusal prefix for every unsupported operation.
pub const UNSUPPORTED_PREFIX: &str = "unsupported by the Studio OCP compiler v1: ";
/// Extra cut depth beyond the through-all body extent in millimetres.
pub const THROUGH_ALL_MARGIN_MM: f64 = 1.0;

#[cfg(feature = "native")]
mod native;
#[cfg(feature = "native")]
pub use native::{
    COARSENED_WARNING, COARSER_ANGULAR_DEFLECTION_RAD, COARSER_MESH_DEFLECTION_MM,
    CompilationResult, CompileOptions, CompiledComponent, Shape, StepImportOutcome,
    compile_json_bytes, compile_json_str, compile_script, compile_script_with_options,
    default_section_offset, hlr_views, hlr_views_with_options, import_step_view, measure_and_check,
    measure_shape, section, section_with_options, step_bytes, step_bytes_with_options, step_shape,
    tessellate, topology,
};

#[cfg(not(feature = "native"))]
mod unavailable;
#[cfg(not(feature = "native"))]
pub use unavailable::{
    COARSENED_WARNING, COARSER_ANGULAR_DEFLECTION_RAD, COARSER_MESH_DEFLECTION_MM,
    CompilationResult, CompileOptions, CompiledComponent, Shape, StepImportOutcome,
    compile_json_bytes, compile_json_str, compile_script, compile_script_with_options,
    default_section_offset, hlr_views, hlr_views_with_options, import_step_view, measure_and_check,
    measure_shape, section, section_with_options, step_bytes, step_bytes_with_options, step_shape,
    tessellate, topology,
};

/// Re-export the accepted construction model used by the compiler entry
/// points. Consumers do not need a second, incompatible script type.
pub use kala_contracts::{
    AnalysisScript, ComponentScript, ComponentStep, ConstructionScript, EditStep, Text,
};

use std::sync::OnceLock;

/// What the built library can actually do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capability {
    /// True only when the end-to-end kernel self-check passed.
    pub available: bool,
    /// `native-occt` or `none`.
    pub backend: &'static str,
    /// The linked OpenCascade version, or `none`.
    pub kernel: String,
    /// Why the kernel is unavailable, when it is.
    pub reason: Option<String>,
}

static CAPABILITY: OnceLock<Capability> = OnceLock::new();

/// Probe the built library with a bounded, real kernel self-check.
///
/// The result is computed once per process. A missing native SDK at build time
/// is represented by the explicit unavailable build; a broken runtime link or
/// a kernel that cannot compile/measure/mesh/export the probe is reported as
/// unavailable with the exact failure message.
pub fn probe_capability() -> Capability {
    CAPABILITY.get_or_init(detect).clone()
}

/// Whether the full design pipeline (compile, measure, mesh, STEP) is ready.
pub fn design_available() -> bool {
    probe_capability().available
}

#[cfg(feature = "native")]
fn detect() -> Capability {
    let kernel = native::ffi::occt_version();
    match native_self_check() {
        Ok(()) => Capability {
            available: true,
            backend: "native-occt",
            kernel,
            reason: None,
        },
        Err(reason) => Capability {
            available: false,
            backend: "native-occt",
            kernel,
            reason: Some(reason),
        },
    }
}

#[cfg(not(feature = "native"))]
fn detect() -> Capability {
    Capability {
        available: false,
        backend: "none",
        kernel: "none".to_owned(),
        reason: Some(
            "kala-cad was built without the native OpenCascade kernel; rebuild with the \
             `native` feature and a configured OCCT SDK"
                .to_owned(),
        ),
    }
}

#[cfg(feature = "native")]
fn native_self_check() -> Result<(), String> {
    const PROBE_SCRIPT: &str = r#"{
        "summary": "kala-cad capability probe",
        "components": [{
            "name": "probe",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 10.0, "height": 20.0},
                {"op": "extrude", "length": 30.0}
            ]
        }],
        "assembly": []
    }"#;
    let script = kala_contracts::load_construction_json_str(PROBE_SCRIPT)
        .map_err(|error| format!("the probe script was refused: {error}"))?;
    let compiled = compile_script(&script).map_err(|error| error.to_string())?;
    let measured = measure_shape(&compiled.assembly_shape).map_err(|error| error.to_string())?;
    if !measured.valid || measured.solid_count != 1 {
        return Err("the probe shape is not one valid solid".to_owned());
    }
    if (measured.volume_mm3 - 6000.0).abs() > 6000.0 * 1e-6 {
        return Err(format!(
            "the probe volume was {}, expected 6000",
            measured.volume_mm3
        ));
    }
    let mesh = tessellate(&compiled.assembly_shape, MeshOptions::default())
        .map_err(|error| error.to_string())?;
    if mesh.triangle_count() == 0
        || !mesh.positions.iter().all(|value| value.is_finite())
        || !mesh.normals.iter().all(|value| value.is_finite())
    {
        return Err("the probe mesh is empty or non-finite".to_owned());
    }
    let step = step_bytes(&compiled.assembly_shape).map_err(|error| error.to_string())?;
    if !step.starts_with(b"ISO-10303-21") {
        return Err("the probe STEP export is not ISO-10303-21".to_owned());
    }
    // Exercise the full view-only import path, not just the shape reader.
    let view = import_step_view(&step, crate::StepImportOptions::default())
        .map_err(|error| error.to_string())?;
    if view.solids != 1 || !view.warnings.is_empty() || view.mesh.triangle_count() == 0 {
        return Err("the probe STEP view import is not one displayable solid".to_owned());
    }
    let imported_measurement = measure_shape(&view.shape).map_err(|error| error.to_string())?;
    if !imported_measurement.valid || imported_measurement.solid_count != 1 {
        return Err("the probe STEP re-import is not one valid solid".to_owned());
    }
    if (imported_measurement.volume_mm3 - measured.volume_mm3).abs() > 1e-6 * 6000.0 {
        return Err(format!(
            "the probe STEP volume changed from {} to {}",
            measured.volume_mm3, imported_measurement.volume_mm3
        ));
    }
    for axis in 0..3 {
        if (imported_measurement.bounds.min_mm[axis] - measured.bounds.min_mm[axis]).abs() > 1e-6
            || (imported_measurement.bounds.max_mm[axis] - measured.bounds.max_mm[axis]).abs()
                > 1e-6
        {
            return Err("the probe STEP bounds changed".to_owned());
        }
    }
    Ok(())
}
