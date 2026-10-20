//! Native analytic geometry and deterministic binary STL for Kala Studio.
//!
//! This crate is a standalone native Rust replacement for the Python
//! `kala_agent.studio.geometry` analytic mesh path. It reproduces the Python
//! source's numeric operation order for box, cylinder, and sphere
//! tessellations, its validation messages, its resource caps, and its binary
//! STL byte layout. It does **not** implement a B-rep kernel, STEP export, or
//! Solid Edge COM; those remain separate capabilities.
//!
//! # Parity scope
//!
//! * Dimensions are `f64` millimetres in `[0.1, 2000.0]`, matching the Python
//!   finite/positive validator, including rejection of booleans and
//!   non-integer tessellation counts in the parameter adapter.
//! * Tessellation counts use the Python defaults and maxima.
//! * Meshes are watertight, outward-wound triangle soups with per-vertex
//!   smooth normals accumulated in Python's iteration order.
//! * STL packing is the only place an `f64` becomes an `f32`, exactly as the
//!   Python `struct.pack("<12fH", ...)` path does.
//! * Trigonometry, square roots, and powers call the platform libm in the same
//!   order as CPython. Byte-for-byte parity is therefore guaranteed on the
//!   same host/libm (the fixture gate runs on this Linux glibc host); a
//!   different libm may differ in the last bit. See the crate README.
//!
//! Python is used only as a test oracle in `tests/`; the library never invokes
//! Python or another process.

#![forbid(unsafe_code)]

mod error;
pub mod limits;
pub mod meshes;
pub mod params;
pub mod primitives;
pub mod stl;

pub use error::GeometryError;
pub use limits::{
    DEFAULT_BOX_EDGE_SUBDIVISIONS, DEFAULT_CYLINDER_HEIGHT_SEGMENTS,
    DEFAULT_CYLINDER_RADIAL_SEGMENTS, DEFAULT_SPHERE_RINGS, DEFAULT_SPHERE_SECTORS,
    MAX_BOX_EDGE_SUBDIVISIONS, MAX_CYLINDER_HEIGHT_SEGMENTS, MAX_CYLINDER_RADIAL_SEGMENTS,
    MAX_DIMENSION_MM, MAX_MESH_TRIANGLES, MAX_MESH_VERTICES, MAX_SPHERE_RINGS, MAX_SPHERE_SECTORS,
    MIN_CYLINDER_HEIGHT_SEGMENTS, MIN_CYLINDER_RADIAL_SEGMENTS, MIN_DIMENSION_MM, MIN_SPHERE_RINGS,
    MIN_SPHERE_SECTORS, validate_finite_positive, validate_int_in_range,
};
pub use meshes::{
    ANALYTIC_WARNING, Bounds, MESH_ENGINE, Mesh, PositionEdge, PositionKey, build_box,
    build_cylinder, build_primitive, build_primitive_json, build_spec, build_sphere,
    compute_bounds, signed_volume, welded_edges, welded_edges_by_position,
    welded_edges_by_position_pairs,
};
pub use params::{ParameterValue, PrimitiveParameters};
pub use primitives::{
    DEFAULT_BOX_SIZE_MM, DEFAULT_CYLINDER_HEIGHT_MM, DEFAULT_CYLINDER_RADIUS_MM,
    DEFAULT_SPHERE_RADIUS_MM, DimensionValues, PrimitiveKind, PrimitiveSpec, dimension_keys,
    resolve_dimension_parameters,
};
pub use stl::{
    STL_BINARY_FORMAT, STL_HEADER_FILL, STL_HEADER_SIZE, STL_TRIANGLE_SIZE, TRIANGLES_PER_FACE,
    write_binary_stl,
};
