//! Triangle mesh storage, limits and binary STL serialisation.
//!
//! These types are kernel-independent. The native tessellator fills them from
//! real OpenCascade triangulations; the STL writer is a byte-for-byte port of
//! the Python `mesh.py` writer (little-endian `f32` normal/vertex pairs and a
//! `u16` attribute per triangle).
//!
//! The Python source never promises byte parity for tessellated meshes across
//! OCCT versions. Triangle order follows the shape's face exploration order;
//! that order is deterministic for a given kernel version but is not a
//! cross-version format contract.

use crate::error::MeshError;

/// Default linear deflection in millimetres.
pub const DEFAULT_MESH_DEFLECTION_MM: f64 = 0.1;
/// Default angular deflection in radians.
pub const DEFAULT_ANGULAR_DEFLECTION_RAD: f64 = 0.5;
/// The engine name carried by the Python result.
pub const MESH_ENGINE: &str = "ocp";
/// The fixed 80-byte STL header fill.
pub const STL_HEADER_FILL: &[u8] = b"kala-studio design mesh, binary STL export";
/// Binary STL header size.
pub const STL_HEADER_SIZE: usize = 80;
/// Hard resource ceiling for one mesh.
pub const MAX_MESH_VERTICES: usize = 200_000;
/// Hard resource ceiling for one mesh.
pub const MAX_MESH_TRIANGLES: usize = 400_000;

/// Tessellation settings. The limits are enforced before any array grows past
/// them; a mesh is refused rather than truncated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshOptions {
    /// Linear deflection in millimetres, strictly positive and finite.
    pub deflection_mm: f64,
    /// Angular deflection in radians, strictly positive and finite.
    pub angular_deflection_rad: f64,
    /// Maximum total vertex count.
    pub max_vertices: usize,
    /// Maximum total triangle count.
    pub max_triangles: usize,
}

impl Default for MeshOptions {
    fn default() -> Self {
        Self {
            deflection_mm: DEFAULT_MESH_DEFLECTION_MM,
            angular_deflection_rad: DEFAULT_ANGULAR_DEFLECTION_RAD,
            max_vertices: MAX_MESH_VERTICES,
            max_triangles: MAX_MESH_TRIANGLES,
        }
    }
}

/// A triangle mesh stored as three parallel flat arrays.
///
/// `positions` and `normals` are `3 * vertex_count` long; `indices` is
/// `3 * triangle_count` long and zero-based.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshData {
    /// Flat `x, y, z` positions in millimetres.
    pub positions: Vec<f64>,
    /// Flat `nx, ny, nz` unit normals, one per position.
    pub normals: Vec<f64>,
    /// Zero-based triangle corners.
    pub indices: Vec<u32>,
    /// The linear deflection used.
    pub deflection_mm: f64,
    /// The angular deflection used.
    pub angular_deflection_rad: f64,
}

impl MeshData {
    /// The number of vertices.
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// The number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Whether the flat arrays are internally consistent and every index is in
    /// range. Used by the STL writer to fail safely instead of panicking on a
    /// hand-built malformed mesh.
    pub fn is_well_formed(&self) -> bool {
        self.positions.len() % 3 == 0
            && self.normals.len() == self.positions.len()
            && self.indices.len() % 3 == 0
            && self
                .indices
                .iter()
                .all(|index| (*index as usize) < self.vertex_count())
    }
}

/// Serialise `mesh` as a deterministic binary STL byte string.
///
/// The header is padded with spaces to exactly 80 bytes (and truncated if the
/// caller supplies a longer header). The triangle count is little-endian
/// `u32`; each triangle is a face normal computed from the winding, three
/// little-endian `f32` vertices, and a zero `u16` attribute.
///
/// The source function returns raw bytes and relies on Python exceptions for a
/// malformed hand-built mesh. The native function returns a typed error for the
/// same malformed input and never panics; bounded tessellator output cannot
/// reach those errors.
pub fn stl_bytes(mesh: &MeshData, header: &[u8]) -> Result<Vec<u8>, MeshError> {
    if !mesh.is_well_formed() {
        return Err(MeshError::new(
            "the mesh arrays are inconsistent or an index is outside the vertex array",
        ));
    }
    let triangle_count = mesh.triangle_count();
    let triangle_count_u32 = u32::try_from(triangle_count)
        .map_err(|_| MeshError::new("the mesh has too many triangles for a binary STL"))?;

    let mut fill = Vec::with_capacity(STL_HEADER_SIZE);
    fill.extend_from_slice(header);
    fill.resize(STL_HEADER_SIZE, b' ');
    fill.truncate(STL_HEADER_SIZE);

    let capacity = triangle_count
        .checked_mul(50)
        .and_then(|bytes| bytes.checked_add(STL_HEADER_SIZE + 4))
        .ok_or_else(|| MeshError::new("the mesh is too large to serialise as a binary STL"))?;
    let mut output = Vec::with_capacity(capacity);
    output.extend_from_slice(&fill);
    output.extend_from_slice(&triangle_count_u32.to_le_bytes());

    for triangle in mesh.indices.chunks_exact(3) {
        let i0 = triangle[0] as usize;
        let i1 = triangle[1] as usize;
        let i2 = triangle[2] as usize;
        let a = (
            mesh.positions[i0 * 3],
            mesh.positions[i0 * 3 + 1],
            mesh.positions[i0 * 3 + 2],
        );
        let b = (
            mesh.positions[i1 * 3],
            mesh.positions[i1 * 3 + 1],
            mesh.positions[i1 * 3 + 2],
        );
        let c = (
            mesh.positions[i2 * 3],
            mesh.positions[i2 * 3 + 1],
            mesh.positions[i2 * 3 + 2],
        );
        let u = (b.0 - a.0, b.1 - a.1, b.2 - a.2);
        let v = (c.0 - a.0, c.1 - a.1, c.2 - a.2);
        let mut normal = (
            u.1 * v.2 - u.2 * v.1,
            u.2 * v.0 - u.0 * v.2,
            u.0 * v.1 - u.1 * v.0,
        );
        let magnitude_sq = normal.0 * normal.0 + normal.1 * normal.1 + normal.2 * normal.2;
        if magnitude_sq > 0.0 {
            let inverse = 1.0 / magnitude_sq.sqrt();
            normal = (normal.0 * inverse, normal.1 * inverse, normal.2 * inverse);
        }
        for value in [
            normal.0, normal.1, normal.2, a.0, a.1, a.2, b.0, b.1, b.2, c.0, c.1, c.2,
        ] {
            output.extend_from_slice(&(value as f32).to_le_bytes());
        }
        output.extend_from_slice(&0u16.to_le_bytes());
    }
    Ok(output)
}
