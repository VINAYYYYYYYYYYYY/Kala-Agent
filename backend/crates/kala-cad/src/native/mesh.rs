//! Real OpenCascade tessellation with bounded output.
//!
//! The mesher is `BRepMesh_IncrementalMesh` with the source deflection and
//! angular settings. Faces are read back individually, positions are moved by
//! each face's `TopLoc_Location`, normals are the kernel's per-vertex normals
//! (flipped for reversed faces), and triangle winding is swapped for reversed
//! faces exactly as `design/mesh.py` does.
//!
//! The adapter enforces the vertex/triangle limits while it is still reading
//! faces, so a pathological shape is refused before Rust allocates an
//! unbounded array. A mesh is never truncated.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::NonNull;

use crate::error::MeshError;
use crate::mesh_data::{MeshData, MeshOptions};
use crate::native::ffi;
use crate::native::ffi::FfiError;
use crate::native::shape::Shape;

/// Triangulate `shape` into bounded positions, normals and indices.
///
/// Raises [`MeshError`] for a null shape, a non-positive/non-finite tolerance,
/// a failed kernel tessellation, a face with no triangulation, a mesh budget
/// overflow, or a mesh with no triangles.
pub fn tessellate(shape: &Shape, options: MeshOptions) -> Result<MeshData, MeshError> {
    if !options.deflection_mm.is_finite() || options.deflection_mm <= 0.0 {
        return Err(MeshError::new(
            "the mesh deflection must be a positive finite number",
        ));
    }
    if !options.angular_deflection_rad.is_finite() || options.angular_deflection_rad <= 0.0 {
        return Err(MeshError::new(
            "the angular deflection must be a positive finite number",
        ));
    }
    if shape.is_null() {
        return Err(MeshError::new("the mesh needs a non-null shape"));
    }

    let max_vertices = i64::try_from(options.max_vertices).unwrap_or(i64::MAX);
    let max_triangles = i64::try_from(options.max_triangles).unwrap_or(i64::MAX);
    let mut error = FfiError::default();
    // SAFETY: shape is a live handle; the adapter owns the returned mesh and
    // the out-error pointer is valid for the call.
    // Parallel meshing is disabled deliberately: the source never promises
    // thread interleaving or byte parity, and a single-threaded tessellation
    // keeps a session's kernel work deterministic and free of internal worker
    // pools. Triangle counts and geometry are the same algorithm.
    let raw = unsafe {
        ffi::kala_mesh_create(
            shape.as_ptr(),
            options.deflection_mm,
            options.angular_deflection_rad,
            0,
            max_vertices,
            max_triangles,
            &mut error,
        )
    };
    let Some(mesh) = NonNull::new(raw) else {
        return Err(match error.code {
            ffi::NOT_DONE => MeshError::new("the geometry kernel could not tessellate the shape"),
            ffi::NULL_RESULT => MeshError::new("a face of the shape has no triangulation"),
            ffi::NORMALS_FAILED => MeshError::new(format!(
                "the geometry kernel could not compute mesh normals: {}",
                error.text()
            )),
            ffi::OVER_VERTEX => MeshError::new(format!(
                "the mesh would exceed the {} vertex limit; refusing to truncate it",
                options.max_vertices
            )),
            ffi::OVER_TRIANGLE => MeshError::new(format!(
                "the mesh would exceed the {} triangle limit; refusing to truncate it",
                options.max_triangles
            )),
            ffi::NO_TRIANGLES => MeshError::new("the geometry kernel produced no mesh triangles"),
            status => MeshError::new(format!(
                "the geometry kernel could not tessellate the shape: {}",
                ffi::exception_class(status, &error)
            )),
        });
    };

    let guard = MeshGuard(mesh);
    // SAFETY: the guard owns a live mesh handle.
    let vertex_count = unsafe { ffi::kala_mesh_vertex_count(guard.0.as_ptr()) };
    // SAFETY: the guard owns a live mesh handle.
    let triangle_count = unsafe { ffi::kala_mesh_triangle_count(guard.0.as_ptr()) };
    if vertex_count < 0 || triangle_count < 0 {
        return Err(MeshError::new(
            "the geometry kernel returned an invalid mesh",
        ));
    }
    let vertex_count = vertex_count as usize;
    let triangle_count = triangle_count as usize;
    let (Some(position_len), Some(index_len)) =
        (vertex_count.checked_mul(3), triangle_count.checked_mul(3))
    else {
        return Err(MeshError::new(
            "the geometry kernel returned an oversized mesh",
        ));
    };

    let mut positions = vec![0.0_f64; position_len];
    let mut normals = vec![0.0_f64; position_len];
    let mut indices = vec![0_u32; index_len];
    let mut copy_error = FfiError::default();
    // SAFETY: all buffers are exactly the size the adapter reads from the live
    // mesh handle, and the out-error pointer is valid.
    let status = unsafe {
        ffi::kala_mesh_copy(
            guard.0.as_ptr(),
            positions.as_mut_ptr(),
            positions.len(),
            normals.as_mut_ptr(),
            normals.len(),
            indices.as_mut_ptr(),
            indices.len(),
            &mut copy_error,
        )
    };
    if status != ffi::OK {
        return Err(MeshError::new(
            "the geometry kernel could not copy the mesh",
        ));
    }

    Ok(MeshData {
        positions,
        normals,
        indices,
        deflection_mm: options.deflection_mm,
        angular_deflection_rad: options.angular_deflection_rad,
    })
}

struct MeshGuard(NonNull<c_void>);

impl Drop for MeshGuard {
    fn drop(&mut self) {
        // SAFETY: unique owner, freed once.
        unsafe { ffi::kala_mesh_free(self.0.as_ptr()) };
    }
}
