//! Analytic parametric triangle meshes for studio preview primitives.
//!
//! The construction order, winding correction, normal accumulation, and
//! floating-point operations mirror `kala_agent.studio.geometry.meshes`
//! exactly. Every mesh is an origin-centred, outward-wound, watertight
//! tessellation; it is a preview representation, not a B-rep or NURBS result.

use std::collections::BTreeMap;
use std::f64::consts::PI;

use serde_json::Value;

use crate::GeometryError;
use crate::limits::{
    DEFAULT_BOX_EDGE_SUBDIVISIONS, DEFAULT_CYLINDER_HEIGHT_SEGMENTS,
    DEFAULT_CYLINDER_RADIAL_SEGMENTS, DEFAULT_SPHERE_RINGS, DEFAULT_SPHERE_SECTORS,
    MAX_BOX_EDGE_SUBDIVISIONS, MAX_CYLINDER_HEIGHT_SEGMENTS, MAX_CYLINDER_RADIAL_SEGMENTS,
    MAX_SPHERE_RINGS, MAX_SPHERE_SECTORS, MIN_CYLINDER_HEIGHT_SEGMENTS,
    MIN_CYLINDER_RADIAL_SEGMENTS, MIN_SPHERE_RINGS, MIN_SPHERE_SECTORS, check_resource_bounds,
    validate_finite_positive, validate_int_in_range,
};
use crate::params::PrimitiveParameters;
use crate::primitives::{DimensionValues, PrimitiveSpec, resolve_dimension_parameters};

/// Engine identifier used by studio preview responses.
pub const MESH_ENGINE: &str = "analytic";
/// Warning attached to every analytic preview response.
pub const ANALYTIC_WARNING: &str =
    "analytic tessellation is a preview representation; not CAD-grade NURBS or B-rep";

/// A triangle mesh stored as three parallel flat arrays.
///
/// Positions and normals are `f64` with three components per vertex; indices
/// are `u32` triples. The fields are private so a mesh cannot be constructed
/// with mismatched array lengths or out-of-range indices by accident.
#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    positions: Vec<f64>,
    normals: Vec<f64>,
    indices: Vec<u32>,
}

impl Mesh {
    /// Construct a mesh from already-computed parallel arrays.
    ///
    /// This performs the same resource-cap check as the builders plus the
    /// structural checks a native consumer needs (matching normal length,
    /// complete triangle triples, indices inside the vertex array).
    ///
    /// # Errors
    ///
    /// Returns a sanitized error when an array is structurally invalid or the
    /// vertex/triangle caps are exceeded.
    pub fn from_parts(
        positions: Vec<f64>,
        normals: Vec<f64>,
        indices: Vec<u32>,
    ) -> Result<Self, GeometryError> {
        if positions.len() % 3 != 0 {
            return Err(GeometryError::new(
                "the mesh positions array is not a multiple of 3",
            ));
        }
        if normals.len() != positions.len() {
            return Err(GeometryError::new(
                "the mesh normals array must match the positions array",
            ));
        }
        if indices.len() % 3 != 0 {
            return Err(GeometryError::new(
                "the mesh indices array is not a multiple of 3",
            ));
        }
        let vertex_count = positions.len() / 3;
        let triangle_count = indices.len() / 3;
        if indices
            .iter()
            .any(|index| usize::try_from(*index).map_or(true, |index| index >= vertex_count))
        {
            return Err(GeometryError::new(
                "the mesh index is outside the vertex array",
            ));
        }
        check_resource_bounds(vertex_count, triangle_count)?;
        Ok(Self {
            positions,
            normals,
            indices,
        })
    }

    /// Create an empty mesh. Writing it produces an 84-byte empty binary STL.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            positions: Vec::new(),
            normals: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Flat `x, y, z` position array in millimetres.
    #[must_use]
    pub fn positions(&self) -> &[f64] {
        &self.positions
    }

    /// Flat `x, y, z` per-vertex smooth normal array.
    #[must_use]
    pub fn normals(&self) -> &[f64] {
        &self.normals
    }

    /// Flat triangle index array.
    #[must_use]
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// Number of vertices.
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Number of triangles.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// Axis-aligned bounding box in the mesh's native coordinate frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

/// A millimetre-rounded spatial vertex key.
pub type PositionKey = [i64; 3];
/// An undirected edge keyed by two spatially welded endpoints.
pub type PositionEdge = (PositionKey, PositionKey);

/// Return the axis-aligned bounding box of a flat positions array.
///
/// An empty array returns the origin-zero box, matching Python. Unlike the
/// Python helper (which raises from `min()` on a malformed array), a
/// non-multiple-of-three array is refused explicitly.
///
/// # Errors
///
/// Returns [`GeometryError`] when `positions.len() % 3 != 0`.
pub fn compute_bounds(positions: &[f64]) -> Result<Bounds, GeometryError> {
    if positions.is_empty() {
        return Ok(Bounds {
            min: [0.0; 3],
            max: [0.0; 3],
        });
    }
    if positions.len() % 3 != 0 {
        return Err(GeometryError::new(
            "the mesh positions array is not a multiple of 3",
        ));
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for vertex in positions.chunks_exact(3) {
        for axis in 0..3 {
            // Update only on a strict comparison. Python's min()/max() keep
            // the first value on a tie, which matters for signed zero.
            if vertex[axis] < min[axis] {
                min[axis] = vertex[axis];
            }
            if vertex[axis] > max[axis] {
                max[axis] = vertex[axis];
            }
        }
    }
    Ok(Bounds { min, max })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn signed_face_normal(
    ax: f64,
    ay: f64,
    az: f64,
    bx: f64,
    by: f64,
    bz: f64,
    cx: f64,
    cy: f64,
    cz: f64,
) -> (f64, f64, f64) {
    let ux = bx - ax;
    let uy = by - ay;
    let uz = bz - az;
    let vx = cx - ax;
    let vy = cy - ay;
    let vz = cz - az;
    (uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx)
}

fn vertex(positions: &[f64], index: u32) -> (f64, f64, f64) {
    let base = index as usize * 3;
    (positions[base], positions[base + 1], positions[base + 2])
}

fn enforce_outward(positions: &[f64], indices: &mut Vec<u32>, i0: u32, i1: u32, i2: u32) {
    if i0 == i1 || i1 == i2 || i0 == i2 {
        return;
    }
    let (ax, ay, az) = vertex(positions, i0);
    let (bx, by, bz) = vertex(positions, i1);
    let (cx, cy, cz) = vertex(positions, i2);
    let (nx, ny, nz) = signed_face_normal(ax, ay, az, bx, by, bz, cx, cy, cz);
    let magnitude_sq = nx * nx + ny * ny + nz * nz;
    if magnitude_sq <= 0.0 {
        return;
    }
    let centroid_x = (ax + bx + cx) / 3.0;
    let centroid_y = (ay + by + cy) / 3.0;
    let centroid_z = (az + bz + cz) / 3.0;
    let length =
        (centroid_x * centroid_x + centroid_y * centroid_y + centroid_z * centroid_z).sqrt();
    if length == 0.0 {
        return;
    }
    let out_dot = (nx * centroid_x + ny * centroid_y + nz * centroid_z) / length;
    if out_dot < 0.0 {
        indices.extend([i0, i2, i1]);
    } else {
        indices.extend([i0, i1, i2]);
    }
}

fn vertex_normals(positions: &[f64], indices: &[u32]) -> Vec<f64> {
    let mut normals = vec![0.0; positions.len()];
    for triangle in indices.chunks_exact(3) {
        let (i0, i1, i2) = (triangle[0], triangle[1], triangle[2]);
        let (ax, ay, az) = vertex(positions, i0);
        let (bx, by, bz) = vertex(positions, i1);
        let (cx, cy, cz) = vertex(positions, i2);
        let (nx, ny, nz) = signed_face_normal(ax, ay, az, bx, by, bz, cx, cy, cz);
        for index in [i0, i1, i2] {
            let base = index as usize * 3;
            normals[base] += nx;
            normals[base + 1] += ny;
            normals[base + 2] += nz;
        }
    }
    for vertex_index in 0..positions.len() / 3 {
        let base = vertex_index * 3;
        let nx = normals[base];
        let ny = normals[base + 1];
        let nz = normals[base + 2];
        let length = (nx * nx + ny * ny + nz * nz).sqrt();
        if length > 0.0 {
            normals[base] = nx / length;
            normals[base + 1] = ny / length;
            normals[base + 2] = nz / length;
        }
    }
    normals
}

fn finalize(positions: Vec<f64>, indices: Vec<u32>) -> Result<Mesh, GeometryError> {
    if positions.len() % 3 != 0 {
        return Err(GeometryError::new(
            "the mesh positions array is not a multiple of 3",
        ));
    }
    let normals = vertex_normals(&positions, &indices);
    check_resource_bounds(positions.len() / 3, indices.len() / 3)?;
    Ok(Mesh {
        positions,
        normals,
        indices,
    })
}

/// Build a parametric axis-aligned box centred on the origin.
///
/// # Errors
///
/// Returns the Python validation message for an invalid dimension or
/// subdivision count.
pub fn build_box(
    size_x: f64,
    size_y: f64,
    size_z: f64,
    subdivisions: u32,
) -> Result<Mesh, GeometryError> {
    let sx = validate_finite_positive("size_x", size_x)?;
    let sy = validate_finite_positive("size_y", size_y)?;
    let sz = validate_finite_positive("size_z", size_z)?;
    let n = validate_int_in_range(
        "subdivisions",
        i128::from(subdivisions),
        1,
        MAX_BOX_EDGE_SUBDIVISIONS,
    )?;

    let mut positions: Vec<f64> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let half_x = sx / 2.0;
    let half_y = sy / 2.0;
    let half_z = sz / 2.0;

    let faces: [(usize, usize, usize, f64); 6] = [
        (0, 1, 2, half_z),
        (0, 1, 2, -half_z),
        (0, 2, 1, half_y),
        (0, 2, 1, -half_y),
        (1, 2, 0, half_x),
        (1, 2, 0, -half_x),
    ];

    for (u_axis, v_axis, c_axis, c_value) in faces {
        let half_u = match u_axis {
            0 => half_x,
            1 => half_y,
            _ => half_z,
        };
        let half_v = match v_axis {
            0 => half_x,
            1 => half_y,
            _ => half_z,
        };
        let base_index = u32::try_from(positions.len() / 3)
            .expect("box vertex count is bounded by the resource cap");
        for row in 0..=n {
            let v = (f64::from(row) / f64::from(n)) * 2.0 - 1.0;
            for column in 0..=n {
                let u = (f64::from(column) / f64::from(n)) * 2.0 - 1.0;
                let mut coordinates = [0.0_f64; 3];
                coordinates[u_axis] = u * half_u;
                coordinates[v_axis] = v * half_v;
                coordinates[c_axis] = c_value;
                positions.extend(coordinates);
            }
        }
        for row in 0..n {
            for column in 0..n {
                let v0 = base_index + row * (n + 1) + column;
                let v1 = v0 + 1;
                let v2 = v0 + (n + 1);
                let v3 = v2 + 1;
                enforce_outward(&positions, &mut indices, v0, v1, v2);
                enforce_outward(&positions, &mut indices, v1, v3, v2);
            }
        }
    }

    finalize(positions, indices)
}

/// Build a closed Z-axis cylinder centred on the origin.
///
/// # Errors
///
/// Returns the Python validation message for an invalid dimension, radial
/// segment count, or height segment count.
pub fn build_cylinder(
    radius: f64,
    height: f64,
    radial_segments: u32,
    height_segments: u32,
) -> Result<Mesh, GeometryError> {
    let r = validate_finite_positive("radius", radius)?;
    let h = validate_finite_positive("height", height)?;
    let radial = validate_int_in_range(
        "radial_segments",
        i128::from(radial_segments),
        MIN_CYLINDER_RADIAL_SEGMENTS,
        MAX_CYLINDER_RADIAL_SEGMENTS,
    )?;
    let rings = validate_int_in_range(
        "height_segments",
        i128::from(height_segments),
        MIN_CYLINDER_HEIGHT_SEGMENTS,
        MAX_CYLINDER_HEIGHT_SEGMENTS,
    )?;
    let ring_count = rings + 1;
    let radial_count = radial + 1;

    let mut positions: Vec<f64> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let half_h = h / 2.0;

    let mut ring_first = vec![0_u32; ring_count as usize];
    for ring_index in 0..ring_count {
        ring_first[ring_index as usize] =
            u32::try_from(positions.len() / 3).expect("cylinder vertex count is bounded");
        let z = -half_h + (f64::from(ring_index) / f64::from(rings)) * h;
        for segment in 0..radial_count {
            let theta = (f64::from(segment) / f64::from(radial)) * 2.0 * PI;
            let cos_t = theta.cos();
            let sin_t = theta.sin();
            positions.extend([r * cos_t, r * sin_t, z]);
        }
    }

    for ring_index in 0..rings {
        for segment in 0..radial {
            let i0 = ring_first[ring_index as usize] + segment;
            let i1 = i0 + 1;
            let i2 = ring_first[ring_index as usize + 1] + segment;
            let i3 = i2 + 1;
            enforce_outward(&positions, &mut indices, i0, i1, i2);
            enforce_outward(&positions, &mut indices, i1, i3, i2);
        }
    }

    let top_center_index =
        u32::try_from(positions.len() / 3).expect("cylinder vertex count is bounded");
    positions.extend([0.0, 0.0, half_h]);
    let top_ring_first =
        u32::try_from(positions.len() / 3).expect("cylinder vertex count is bounded");
    for segment in 0..radial_count {
        let theta = (f64::from(segment) / f64::from(radial)) * 2.0 * PI;
        let cos_t = theta.cos();
        let sin_t = theta.sin();
        positions.extend([r * cos_t, r * sin_t, half_h]);
    }
    for segment in 0..radial {
        let i0 = top_ring_first + segment;
        let i1 = i0 + 1;
        enforce_outward(&positions, &mut indices, top_center_index, i0, i1);
    }

    let bottom_center_index =
        u32::try_from(positions.len() / 3).expect("cylinder vertex count is bounded");
    positions.extend([0.0, 0.0, -half_h]);
    let bottom_ring_first =
        u32::try_from(positions.len() / 3).expect("cylinder vertex count is bounded");
    for segment in 0..radial_count {
        let theta = (f64::from(segment) / f64::from(radial)) * 2.0 * PI;
        let cos_t = theta.cos();
        let sin_t = theta.sin();
        positions.extend([r * cos_t, r * sin_t, -half_h]);
    }
    for segment in 0..radial {
        let i0 = bottom_ring_first + segment;
        let i1 = i0 + 1;
        enforce_outward(&positions, &mut indices, bottom_center_index, i1, i0);
    }

    finalize(positions, indices)
}

/// Build a UV sphere centred on the origin.
///
/// # Errors
///
/// Returns the Python validation message for an invalid radius, ring count, or
/// sector count.
pub fn build_sphere(radius: f64, rings: u32, sectors: u32) -> Result<Mesh, GeometryError> {
    let r = validate_finite_positive("radius", radius)?;
    let ring_count = validate_int_in_range(
        "rings",
        i128::from(rings),
        MIN_SPHERE_RINGS,
        MAX_SPHERE_RINGS,
    )?;
    let sector_count = validate_int_in_range(
        "sectors",
        i128::from(sectors),
        MIN_SPHERE_SECTORS,
        MAX_SPHERE_SECTORS,
    )?;

    let mut positions: Vec<f64> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let south_pole = 0_u32;
    positions.extend([0.0, 0.0, -r]);

    let ring_count_usize = ring_count as usize;
    let mut ring_first = vec![0_u32; ring_count_usize + 1];
    ring_first[ring_count_usize] = south_pole;

    for ring_index in 1..ring_count {
        ring_first[ring_index as usize] =
            u32::try_from(positions.len() / 3).expect("sphere vertex count is bounded");
        let phi = PI * f64::from(ring_index) / f64::from(ring_count);
        let sin_phi = phi.sin();
        let cos_phi = phi.cos();
        for sector_index in 0..=sector_count {
            let theta = 2.0 * PI * f64::from(sector_index) / f64::from(sector_count);
            let cos_t = theta.cos();
            let sin_t = theta.sin();
            positions.extend([r * sin_phi * cos_t, r * sin_phi * sin_t, r * cos_phi]);
        }
    }

    let north_pole = u32::try_from(positions.len() / 3).expect("sphere vertex count is bounded");
    positions.extend([0.0, 0.0, r]);
    ring_first[0] = north_pole;

    for ring_index in 1..ring_count_usize.saturating_sub(1) {
        let ring_first_top = ring_first[ring_index];
        let ring_first_next = ring_first[ring_index + 1];
        for sector_index in 0..sector_count {
            let v0 = ring_first_top + sector_index;
            let v1 = v0 + 1;
            let v2 = ring_first_next + sector_index;
            let v3 = v2 + 1;
            enforce_outward(&positions, &mut indices, v0, v1, v2);
            enforce_outward(&positions, &mut indices, v1, v3, v2);
        }
    }

    let first_ring_first = ring_first[1];
    for sector_index in 0..sector_count {
        let i0 = first_ring_first + sector_index;
        let i1 = i0 + 1;
        enforce_outward(&positions, &mut indices, north_pole, i0, i1);
    }

    let last_ring_first = ring_first[ring_count_usize - 1];
    for sector_index in 0..sector_count {
        let i0 = last_ring_first + sector_index;
        let i1 = i0 + 1;
        enforce_outward(&positions, &mut indices, south_pole, i1, i0);
    }

    finalize(positions, indices)
}

/// Build one primitive from a string kind and a validated parameter mapping.
///
/// Unknown parameters are a validation error, never silently ignored.
///
/// # Errors
///
/// Returns the Python validation message for an empty/unknown kind, unknown
/// parameter name, invalid dimension, or invalid tessellation count.
pub fn build_primitive(
    kind: &str,
    parameters: &PrimitiveParameters,
) -> Result<Mesh, GeometryError> {
    if kind.is_empty() {
        return Err(GeometryError::new("kind must be a non-empty string"));
    }
    match kind {
        "box" => {
            if !parameters
                .unknown_keys(&["size_x", "size_y", "size_z", "subdivisions"])
                .is_empty()
            {
                return Err(GeometryError::new(
                    "box parameters must be a subset of size_x, size_y, size_z, subdivisions",
                ));
            }
            let dims = resolve_dimension_parameters("box", parameters)?;
            let subdivisions = match parameters.get("subdivisions") {
                Some(value) => value.as_count("subdivisions", 1, MAX_BOX_EDGE_SUBDIVISIONS)?,
                None => DEFAULT_BOX_EDGE_SUBDIVISIONS,
            };
            match dims {
                DimensionValues::Box {
                    size_x,
                    size_y,
                    size_z,
                } => build_box(size_x, size_y, size_z, subdivisions),
                _ => unreachable!("box resolved to a non-box dimension set"),
            }
        }
        "cylinder" => {
            if !parameters
                .unknown_keys(&["radius", "height", "radial_segments", "height_segments"])
                .is_empty()
            {
                return Err(GeometryError::new(
                    "cylinder parameters must be a subset of \
                     radius, height, radial_segments, height_segments",
                ));
            }
            let dims = resolve_dimension_parameters("cylinder", parameters)?;
            let radial_segments = match parameters.get("radial_segments") {
                Some(value) => value.as_count(
                    "radial_segments",
                    MIN_CYLINDER_RADIAL_SEGMENTS,
                    MAX_CYLINDER_RADIAL_SEGMENTS,
                )?,
                None => DEFAULT_CYLINDER_RADIAL_SEGMENTS,
            };
            let height_segments = match parameters.get("height_segments") {
                Some(value) => value.as_count(
                    "height_segments",
                    MIN_CYLINDER_HEIGHT_SEGMENTS,
                    MAX_CYLINDER_HEIGHT_SEGMENTS,
                )?,
                None => DEFAULT_CYLINDER_HEIGHT_SEGMENTS,
            };
            match dims {
                DimensionValues::Cylinder { radius, height } => {
                    build_cylinder(radius, height, radial_segments, height_segments)
                }
                _ => unreachable!("cylinder resolved to a non-cylinder dimension set"),
            }
        }
        "sphere" => {
            if !parameters
                .unknown_keys(&["radius", "rings", "sectors"])
                .is_empty()
            {
                return Err(GeometryError::new(
                    "sphere parameters must be a subset of radius, rings, sectors",
                ));
            }
            let dims = resolve_dimension_parameters("sphere", parameters)?;
            let rings = match parameters.get("rings") {
                Some(value) => value.as_count("rings", MIN_SPHERE_RINGS, MAX_SPHERE_RINGS)?,
                None => DEFAULT_SPHERE_RINGS,
            };
            let sectors = match parameters.get("sectors") {
                Some(value) => value.as_count("sectors", MIN_SPHERE_SECTORS, MAX_SPHERE_SECTORS)?,
                None => DEFAULT_SPHERE_SECTORS,
            };
            match dims {
                DimensionValues::Sphere { radius } => build_sphere(radius, rings, sectors),
                _ => unreachable!("sphere resolved to a non-sphere dimension set"),
            }
        }
        other => Err(GeometryError::new(format!("unsupported kind: {other}"))),
    }
}

/// Build one primitive from a string kind and an optional JSON parameter value.
///
/// `None` and JSON `null` both mean "use every default". The value keeps JSON
/// integer, float, and boolean kinds through [`PrimitiveParameters`].
///
/// # Errors
///
/// Returns an adapter error for a non-object parameter value or the same
/// validation errors as [`build_primitive`].
pub fn build_primitive_json(kind: &str, parameters: Option<&Value>) -> Result<Mesh, GeometryError> {
    let params = match parameters {
        Some(value) => PrimitiveParameters::from_json_value(value)?,
        None => PrimitiveParameters::new(),
    };
    build_primitive(kind, &params)
}

/// Build one primitive from a fully typed [`PrimitiveSpec`].
///
/// # Errors
///
/// Returns the Python validation message for an out-of-range dimension or
/// tessellation count.
pub fn build_spec(spec: &PrimitiveSpec) -> Result<Mesh, GeometryError> {
    match *spec {
        PrimitiveSpec::Box { size, subdivisions } => {
            build_box(size[0], size[1], size[2], subdivisions)
        }
        PrimitiveSpec::Cylinder {
            radius,
            height,
            radial_segments,
            height_segments,
        } => build_cylinder(radius, height, radial_segments, height_segments),
        PrimitiveSpec::Sphere {
            radius,
            rings,
            sectors,
        } => build_sphere(radius, rings, sectors),
    }
}

/// Return a map keyed on ordered vertex-index pairs counting edge uses.
#[must_use]
pub fn welded_edges(mesh: &Mesh) -> BTreeMap<(u32, u32), u32> {
    let mut counts = BTreeMap::new();
    for triangle in mesh.indices.chunks_exact(3) {
        let (i0, i1, i2) = (triangle[0], triangle[1], triangle[2]);
        for edge in [(i0, i1), (i1, i2), (i2, i0)] {
            *counts.entry(edge).or_insert(0) += 1;
        }
    }
    counts
}

/// Return a map keyed on directed, millimetre-rounded spatial edges.
#[must_use]
pub fn welded_edges_by_position(mesh: &Mesh) -> BTreeMap<(PositionKey, PositionKey), u32> {
    let mut counts = BTreeMap::new();
    for triangle in mesh.indices.chunks_exact(3) {
        let (i0, i1, i2) = (triangle[0], triangle[1], triangle[2]);
        for (first, second) in [(i0, i1), (i1, i2), (i2, i0)] {
            let key = (
                position_key(mesh.positions(), first),
                position_key(mesh.positions(), second),
            );
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    counts
}

/// Return a symmetric position-keyed map counting total uses per undirected
/// edge. A watertight manifold has every edge used exactly twice.
#[must_use]
pub fn welded_edges_by_position_pairs(mesh: &Mesh) -> BTreeMap<PositionEdge, u32> {
    let directed = welded_edges_by_position(mesh);
    let mut paired = BTreeMap::new();
    for ((a, b), count) in directed {
        let key = if a <= b { (a, b) } else { (b, a) };
        *paired.entry(key).or_insert(0) += count;
    }
    paired
}

fn position_key(positions: &[f64], index: u32) -> PositionKey {
    let base = index as usize * 3;
    [
        (positions[base] * 1000.0).round_ties_even() as i64,
        (positions[base + 1] * 1000.0).round_ties_even() as i64,
        (positions[base + 2] * 1000.0).round_ties_even() as i64,
    ]
}

/// Return the signed tetrahedral volume of a mesh from the origin.
#[must_use]
pub fn signed_volume(mesh: &Mesh) -> f64 {
    let mut total = 0.0;
    let positions = mesh.positions();
    for triangle in mesh.indices.chunks_exact(3) {
        let (ax, ay, az) = vertex(positions, triangle[0]);
        let (bx, by, bz) = vertex(positions, triangle[1]);
        let (cx, cy, cz) = vertex(positions, triangle[2]);
        total +=
            (ax * (by * cz - bz * cy) + ay * (bz * cx - bx * cz) + az * (bx * cy - by * cx)) / 6.0;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_bounds_handles_empty_and_malformed_arrays() {
        assert_eq!(
            compute_bounds(&[]).unwrap(),
            Bounds {
                min: [0.0; 3],
                max: [0.0; 3],
            }
        );
        assert_eq!(
            compute_bounds(&[1.0, 2.0]).unwrap_err().message(),
            "the mesh positions array is not a multiple of 3"
        );
    }

    #[test]
    fn mesh_rejects_structural_mismatches() {
        assert_eq!(
            Mesh::from_parts(vec![0.0, 0.0], vec![0.0, 0.0], Vec::new())
                .unwrap_err()
                .message(),
            "the mesh positions array is not a multiple of 3"
        );
        assert_eq!(
            Mesh::from_parts(vec![0.0; 3], vec![0.0; 6], Vec::new())
                .unwrap_err()
                .message(),
            "the mesh normals array must match the positions array"
        );
        assert_eq!(
            Mesh::from_parts(vec![0.0; 3], vec![0.0; 3], vec![0, 1])
                .unwrap_err()
                .message(),
            "the mesh indices array is not a multiple of 3"
        );
        assert_eq!(
            Mesh::from_parts(vec![0.0; 3], vec![0.0; 3], vec![0, 1, 1])
                .unwrap_err()
                .message(),
            "the mesh index is outside the vertex array"
        );
    }

    #[test]
    fn empty_mesh_is_writeable() {
        let mesh = Mesh::empty();
        assert_eq!(mesh.vertex_count(), 0);
        assert_eq!(mesh.triangle_count(), 0);
    }

    #[test]
    fn position_keys_round_ties_to_even_like_python() {
        let positions = [
            0.0005, 0.0, 0.0, // 0.5 -> 0
            0.0015, 0.0, 0.0, // 1.5 -> 2
            -0.0005, 0.0, 0.0, // -0.5 -> 0
            -0.0015, 0.0, 0.0, // -1.5 -> -2
        ];
        assert_eq!(position_key(&positions, 0), [0, 0, 0]);
        assert_eq!(position_key(&positions, 1), [2, 0, 0]);
        assert_eq!(position_key(&positions, 2), [0, 0, 0]);
        assert_eq!(position_key(&positions, 3), [-2, 0, 0]);
    }
}
