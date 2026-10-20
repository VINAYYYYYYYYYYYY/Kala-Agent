//! Native STEP export, view-only import and the kernel-held STEP settings.
//!
//! Both directions use `STEPControl` with the `STEPControl_AsIs` model type and
//! an explicit millimetre cascade unit. Data is exchanged through in-memory
//! `std::stringstream` objects in the adapter, so no script-controlled path
//! ever reaches the host filesystem.
//!
//! OpenCascade keeps STEP unit/schema settings in a process-global registry.
//! Every export/import in this crate is serialised behind one private mutex so
//! two caller threads cannot race those settings. Equivalent toggles are
//! documented in the crate README.
//!
//! STEP bytes are **not** a byte-parity contract: the writer embeds a
//! generation timestamp and its own entity numbering. [`normalise_step_bytes`]
//! applies the exact replacement the compatibility corpus uses (the first
//! `FILE_NAME` timestamp and the per-process translator counter), which makes
//! two exports of the same geometry comparable without hiding a geometry
//! difference. Geometry, units, solid count, volume and bounds are compared by
//! tests; timestamps and ordering may differ between kernels and exports.
//!
//! [`import_step_view`] is the view-only half of ADR-0011: it reads bounded
//! provider bytes, tessellates them inside the existing mesh budget with one
//! coarser retry, and reports the exact B-Rep envelope plus honest warnings. It
//! never modifies or re-exports the shape.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Mutex, MutexGuard};

use crate::bounds::Bounds;
use crate::error::{StepError, StepImportError, StepImportErrorKind};
use crate::mesh_data::{
    DEFAULT_ANGULAR_DEFLECTION_RAD, DEFAULT_MESH_DEFLECTION_MM, MeshData, MeshOptions,
};
use crate::native::ffi;
use crate::native::ffi::FfiError;
use crate::native::measure;
use crate::native::mesh;
use crate::native::shape::Shape;
use crate::step_common::{
    MAX_VIEW_COORDINATE_MM, MAX_WARNING_LENGTH, STEP_HEADER, StepImportOptions,
    normalise_step_bytes, validate_step_import_max_bytes,
};

/// The coarse retry linear deflection, matching `importer.py`.
pub const COARSER_MESH_DEFLECTION_MM: f64 = 0.5;
/// The coarse retry angular deflection, matching `importer.py`.
pub const COARSER_ANGULAR_DEFLECTION_RAD: f64 = 0.8;
/// The warning appended after the single coarser tessellation retry.
pub const COARSENED_WARNING: &str = "the imported shape exceeded the default tessellation budget; it was retried once with a coarser 0.5 mm deflection";

static STEP_LOCK: Mutex<()> = Mutex::new(());

fn step_guard() -> MutexGuard<'static, ()> {
    STEP_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One view-only import: the display mesh, exact envelope and honest warnings.
#[derive(Debug)]
pub struct StepImportOutcome {
    /// The display triangulation.
    pub mesh: MeshData,
    /// The exact imported B-Rep envelope (no triangulation influence).
    pub bounds: Bounds,
    /// Number of `TopAbs_SOLID` sub-shapes.
    pub solids: usize,
    /// Display warnings; never a silent substitution.
    pub warnings: Vec<String>,
    /// The imported exact shape, for a drawing/section consumer.
    pub shape: Shape,
    /// Whether the single coarser tessellation retry was used.
    pub coarsened: bool,
}

/// Serialise a non-null exact shape as an ISO-10303-21 STEP document.
///
/// The writer transfers the shape `AsIs` in millimetres. The result is checked
/// for the ISO magic and refused otherwise; no non-STEP bytes are returned as
/// a successful export.
pub fn step_bytes(shape: &Shape) -> Result<Vec<u8>, StepError> {
    if shape.is_null() {
        return Err(StepError::new("the STEP export needs a non-null shape"));
    }
    let _guard = step_guard();
    let mut error = FfiError::default();
    // SAFETY: shape is a live handle; the returned buffer is owned by the
    // adapter and the out-error pointer is valid for the call.
    let raw = unsafe { ffi::kala_step_export(shape.as_ptr(), &mut error) };
    let Some(buffer) = NonNull::new(raw) else {
        return Err(match error.code {
            ffi::NOT_DONE => StepError::new("the STEP writer refused the shape"),
            ffi::NULL_RESULT => StepError::new("the STEP writer did not finish serialising"),
            ffi::INVALID_ARGUMENT => {
                StepError::new("the STEP writer could not select millimetre units")
            }
            status => StepError::new(format!(
                "the geometry kernel could not export STEP: {}",
                ffi::exception_class(status, &error)
            )),
        });
    };
    let buffer = BufferGuard(buffer);
    // SAFETY: the guard owns a live buffer handle.
    let size = unsafe { ffi::kala_buffer_size(buffer.0.as_ptr()) };
    if size < 0 {
        return Err(StepError::new("the STEP writer returned an invalid buffer"));
    }
    let mut data = vec![0_u8; size as usize];
    let mut copy_error = FfiError::default();
    // SAFETY: the destination is exactly the reported buffer size, the buffer
    // is live, and the out-error pointer is valid.
    let status = unsafe {
        ffi::kala_buffer_copy(
            buffer.0.as_ptr(),
            data.as_mut_ptr(),
            data.len(),
            &mut copy_error,
        )
    };
    if status != ffi::OK {
        return Err(StepError::new(
            "the STEP writer returned an unreadable buffer",
        ));
    }
    if !data.starts_with(STEP_HEADER) {
        return Err(StepError::new(
            "the STEP writer produced a non-ISO file; refusing to ship it",
        ));
    }
    Ok(data)
}

/// Serialise a shape with explicit [`crate::StepWriteOptions`].
pub fn step_bytes_with_options(
    shape: &Shape,
    options: crate::step_common::StepWriteOptions,
) -> Result<Vec<u8>, StepError> {
    let data = step_bytes(shape)?;
    Ok(if options.normalise {
        normalise_step_bytes(&data)
    } else {
        data
    })
}

/// Read a STEP document and return the transferred exact shape.
///
/// The cascade unit is set to millimetres. A reader that reports no roots or a
/// null shape is a failure, never an empty successful import. Messages are the
/// exact ones `importer.py` raises for the same refusal class.
///
/// This low-level reader does **not** enforce a byte ceiling; untrusted bytes
/// must go through [`import_step_view`], which applies the configured limit
/// before calling it.
pub fn step_shape(data: &[u8]) -> Result<Shape, StepError> {
    if data.is_empty() {
        return Err(StepError::new("the STEP file is empty"));
    }
    let _guard = step_guard();
    let mut error = FfiError::default();
    // SAFETY: the input slice is valid for the call and the out-error pointer
    // is valid; the adapter copies the bytes before returning.
    let raw = unsafe { ffi::kala_step_import(data.as_ptr(), data.len(), &mut error) };
    if raw.is_null() {
        return Err(match error.code {
            ffi::NOT_DONE => StepError::new("the STEP reader refused the file"),
            ffi::NULL_RESULT => StepError::new("the STEP file carries no transferable root"),
            ffi::NO_TRIANGLES => StepError::new("the STEP reader transferred no shape"),
            ffi::VOID_BOX => StepError::new("the STEP file produced an empty shape"),
            ffi::INVALID_ARGUMENT => StepError::new("the STEP reader failed"),
            status => StepError::new(format!(
                "the STEP reader failed: {}",
                ffi::exception_class(status, &error)
            )),
        });
    }
    Shape::from_owned_raw(raw)
        .map_err(|failure| StepError::new(format!("the STEP reader produced no shape: {failure}")))
}

/// Read bounded STEP bytes, tessellate them for display and report the view.
///
/// The shape is never modified and never re-exported. The caller owns the
/// original bytes and their content digest.
///
/// Raises:
/// * [`StepImportErrorKind::Malformed`] for empty bytes, a bad header, a reader
///   refusal, no transferable root/shape, a non-finite or out-of-range
///   envelope;
/// * [`StepImportErrorKind::TooLarge`] when the bytes exceed the ceiling;
/// * [`StepImportErrorKind::TooComplex`] when the tessellation exceeds the mesh
///   budget even after the single coarser retry.
pub fn import_step_view(
    data: &[u8],
    options: StepImportOptions,
) -> Result<StepImportOutcome, StepImportError> {
    if data.is_empty() {
        return Err(StepImportError::new(
            StepImportErrorKind::Malformed,
            "the STEP file is empty",
        ));
    }
    let limit = validate_step_import_max_bytes(options.max_step_bytes)?;
    if data.len() as u64 > limit {
        return Err(StepImportError::new(
            StepImportErrorKind::TooLarge,
            format!("the STEP file must not exceed {limit} bytes"),
        ));
    }
    if !data.starts_with(STEP_HEADER) {
        return Err(StepImportError::new(
            StepImportErrorKind::Malformed,
            "the file does not start with an ISO-10303-21 header",
        ));
    }

    let shape = step_shape(data)
        .map_err(|error| StepImportError::new(StepImportErrorKind::Malformed, error.message()))?;

    let base = MeshOptions {
        deflection_mm: DEFAULT_MESH_DEFLECTION_MM,
        angular_deflection_rad: DEFAULT_ANGULAR_DEFLECTION_RAD,
        max_vertices: options.max_vertices,
        max_triangles: options.max_triangles,
    };
    let (display_mesh, coarsened) = match mesh::tessellate(&shape, base) {
        Ok(mesh) => (mesh, false),
        Err(_first_error) => {
            let coarse = MeshOptions {
                deflection_mm: COARSER_MESH_DEFLECTION_MM,
                angular_deflection_rad: COARSER_ANGULAR_DEFLECTION_RAD,
                ..base
            };
            match mesh::tessellate(&shape, coarse) {
                Ok(mesh) => (mesh, true),
                Err(coarse_error) => {
                    // Python re-raises the *second* (coarser) attempt's message.
                    return Err(StepImportError::new(
                        StepImportErrorKind::TooComplex,
                        coarse_error.message(),
                    ));
                }
            }
        }
    };

    let bounds = measure::optimal_box_ex(&shape, false)
        .map_err(|_| {
            StepImportError::new(
                StepImportErrorKind::Malformed,
                "the STEP shape has no finite bounding box",
            )
        })?
        .ok_or_else(|| {
            StepImportError::new(
                StepImportErrorKind::Malformed,
                "the STEP shape has no finite bounding box",
            )
        })?;
    let coordinates = [
        bounds.min_mm[0],
        bounds.min_mm[1],
        bounds.min_mm[2],
        bounds.max_mm[0],
        bounds.max_mm[1],
        bounds.max_mm[2],
    ];
    if coordinates.iter().any(|value| !value.is_finite()) {
        return Err(StepImportError::new(
            StepImportErrorKind::Malformed,
            "the STEP shape has no finite bounding box",
        ));
    }
    if coordinates
        .iter()
        .any(|value| value.abs() > MAX_VIEW_COORDINATE_MM)
    {
        return Err(StepImportError::new(
            StepImportErrorKind::Malformed,
            "the STEP shape is outside the display coordinate range",
        ));
    }
    Bounds::new(bounds.min_mm, bounds.max_mm).map_err(|_| {
        StepImportError::new(
            StepImportErrorKind::Malformed,
            "the STEP shape has an invalid bounding box",
        )
    })?;

    let solids = shape.solid_count().map_err(|error| {
        StepImportError::new(
            StepImportErrorKind::Malformed,
            format!("the STEP file could not be imported: {error}"),
        )
    })?;
    let valid = shape.is_valid().unwrap_or(false);
    let mut warnings = Vec::new();
    if solids == 0 {
        warnings.push("the STEP file contains no solids; only its surfaces are shown".to_owned());
    }
    if !valid {
        warnings.push(
            "the imported shape did not pass the kernel validity check; \
             it is displayed exactly as exported"
                .to_owned(),
        );
    }
    if coarsened {
        warnings.push(COARSENED_WARNING.to_owned());
    }
    let warnings = warnings
        .into_iter()
        .map(|warning| warning.chars().take(MAX_WARNING_LENGTH).collect())
        .collect();

    Ok(StepImportOutcome {
        mesh: display_mesh,
        bounds,
        solids,
        warnings,
        shape,
        coarsened,
    })
}

struct BufferGuard(NonNull<c_void>);

impl Drop for BufferGuard {
    fn drop(&mut self) {
        // SAFETY: unique owner, freed once.
        unsafe { ffi::kala_buffer_free(self.0.as_ptr()) };
    }
}
