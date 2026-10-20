//! Exact kernel measurement and honesty checks.
//!
//! Every reported number is read from OpenCascade: `BRepGProp` for volume,
//! area and centre of mass; `BRepBndLib::AddOptimal` for bounds;
//! `BRepCheck_Analyzer` for validity; `TopExp_Explorer` for the solid count.
//! There is no closed form and no fallback estimate.

#![allow(unsafe_code)]

use crate::bounds::{Bounds, Measurement, TopologyCounts};
use crate::error::{KernelError, MeasurementError};
use crate::native::ffi;
use crate::native::ffi::{FfiError, FfiMeasurement, FfiPublishState, FfiTopology};
use crate::native::shape::Shape;

/// Measure a shape with the exact kernel property and bounds algorithms.
pub fn measure_shape(shape: &Shape) -> Result<Measurement, MeasurementError> {
    if shape.is_null() {
        return Err(MeasurementError::new(
            "the shape is null and cannot be measured",
        ));
    }
    let mut out = FfiMeasurement::default();
    let mut error = FfiError::default();
    // SAFETY: shape is a live handle; both out-pointers are valid for the call.
    let status = unsafe { ffi::kala_shape_measure(shape.as_ptr(), &mut out, &mut error) };
    match status {
        ffi::OK => {
            let bounds = Bounds::new(out.min, out.max)
                .map_err(|bounds| MeasurementError::new(bounds.message()))?;
            Ok(Measurement {
                volume_mm3: out.volume,
                surface_area_mm2: out.area,
                centroid_mm: out.centroid,
                bounds,
                valid: out.valid != 0,
                solid_count: out.solid_count.max(0) as usize,
            })
        }
        ffi::VOID_BOX => Err(MeasurementError::new(
            "the shape has no finite bounding box",
        )),
        status => Err(MeasurementError::new(format!(
            "the geometry kernel could not measure the shape: {}",
            ffi::exception_class(status, &error)
        ))),
    }
}

/// Measure a shape and build its checks in one call.
pub fn measure_and_check(
    shape: &Shape,
    expected_envelope: Option<&Bounds>,
    expect_single_solid: bool,
) -> Result<(Measurement, crate::bounds::CheckReport), MeasurementError> {
    let measurement = measure_shape(shape)?;
    let report = crate::bounds::build_checks(&measurement, expected_envelope, expect_single_solid);
    Ok((measurement, report))
}

/// Occurrence counts for shells, faces, edges and vertices.
///
/// This is an additive topology summary over the same exact shape. It never
/// changes the source `Measurement` fields.
pub fn topology(shape: &Shape) -> Result<TopologyCounts, MeasurementError> {
    if shape.is_null() {
        return Err(MeasurementError::new(
            "the shape is null and cannot be measured",
        ));
    }
    let mut out = FfiTopology::default();
    let mut error = FfiError::default();
    // SAFETY: shape is a live handle; both out-pointers are valid for the call.
    let status = unsafe { ffi::kala_shape_topology(shape.as_ptr(), &mut out, &mut error) };
    if status != ffi::OK {
        return Err(MeasurementError::new(format!(
            "the geometry kernel could not measure the shape: {}",
            ffi::exception_class(status, &error)
        )));
    }
    Ok(TopologyCounts {
        solids: out.solids.max(0) as usize,
        shells: out.shells.max(0) as usize,
        faces: out.faces.max(0) as usize,
        edges: out.edges.max(0) as usize,
        vertices: out.vertices.max(0) as usize,
    })
}

/// Publishability fields used by the compiler before any result is returned.
pub(crate) struct PublishState {
    pub(crate) is_null: bool,
    pub(crate) solid_count: usize,
    pub(crate) volume: f64,
    pub(crate) valid: bool,
}

pub(crate) fn publish_state(shape: &Shape) -> Result<PublishState, KernelError> {
    let mut out = FfiPublishState::default();
    let mut error = FfiError::default();
    // SAFETY: shape is a live handle; both out-pointers are valid for the call.
    let status = unsafe { ffi::kala_shape_publish_state(shape.as_ptr(), &mut out, &mut error) };
    if status != ffi::OK {
        return Err(KernelError::new(ffi::exception_class(status, &error)));
    }
    Ok(PublishState {
        is_null: out.is_null != 0,
        solid_count: out.solid_count.max(0) as usize,
        volume: out.volume,
        valid: out.valid != 0,
    })
}

pub(crate) fn volume(shape: &Shape) -> Result<f64, KernelError> {
    let mut out = 0.0;
    let mut error = FfiError::default();
    // SAFETY: shape is a live handle; the out-pointer is valid for the call.
    let status = unsafe { ffi::kala_shape_volume(shape.as_ptr(), &mut out, &mut error) };
    if status != ffi::OK {
        return Err(KernelError::new(ffi::exception_class(status, &error)));
    }
    Ok(out)
}

/// The optimal kernel bounding box, or `None` when the kernel reports a void
/// box (no finite bounds).
pub(crate) fn optimal_box(shape: &Shape) -> Result<Option<Bounds>, KernelError> {
    optimal_box_ex(shape, true)
}

/// The optimal kernel bounding box with an explicit triangulation policy.
///
/// `measure.py` asks for the triangulation-aware box (`use_triangulation =
/// true`); `importer.py` deliberately asks for the exact B-Rep envelope
/// (`false`) so a display triangulation can never move an imported bound.
pub(crate) fn optimal_box_ex(
    shape: &Shape,
    use_triangulation: bool,
) -> Result<Option<Bounds>, KernelError> {
    let mut out = [0.0_f64; 6];
    let mut error = FfiError::default();
    // SAFETY: shape is a live handle; the out-pointer is valid for the call.
    let status = unsafe {
        ffi::kala_optimal_box_ex(
            shape.as_ptr(),
            if use_triangulation { 1 } else { 0 },
            out.as_mut_ptr(),
            &mut error,
        )
    };
    match status {
        ffi::OK => Ok(Some(
            Bounds::new([out[0], out[1], out[2]], [out[3], out[4], out[5]])
                .map_err(|bounds| KernelError::new(bounds.message()))?,
        )),
        ffi::VOID_BOX => Ok(None),
        status => Err(KernelError::new(ffi::exception_class(status, &error))),
    }
}
