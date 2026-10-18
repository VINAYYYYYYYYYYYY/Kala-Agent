"""Studio geometry primitives and exports.

The package exposes:

- Parametric mesh builders for box, cylinder, and sphere primitives.
- A finite, bounded numeric validator for dimensions, ring counts, and
  segment counts.
- A binary STL writer for the downloadable preview path.
- A STEP writer that wraps OCP's ``BRepPrimAPI`` builders and writes an
  exact B-rep shape (and a probe that reports whether OCP is available).
"""

from kala_agent.studio.geometry.limits import (
    DEFAULT_BOX_EDGE_SUBDIVISIONS,
    DEFAULT_CYLINDER_HEIGHT_SEGMENTS,
    DEFAULT_CYLINDER_RADIAL_SEGMENTS,
    DEFAULT_SPHERE_RINGS,
    DEFAULT_SPHERE_SECTORS,
    MAX_BOX_EDGE_SUBDIVISIONS,
    MAX_CYLINDER_HEIGHT_SEGMENTS,
    MAX_CYLINDER_RADIAL_SEGMENTS,
    MAX_DIMENSION_MM,
    MAX_MESH_TRIANGLES,
    MAX_MESH_VERTICES,
    MAX_SPHERE_RINGS,
    MAX_SPHERE_SECTORS,
    MIN_CYLINDER_HEIGHT_SEGMENTS,
    MIN_CYLINDER_RADIAL_SEGMENTS,
    MIN_DIMENSION_MM,
    MIN_SPHERE_RINGS,
    MIN_SPHERE_SECTORS,
    validate_finite_positive,
    validate_int_in_range,
)
from kala_agent.studio.geometry.meshes import (
    ANALYTIC_WARNING,
    MESH_ENGINE,
    Bounds,
    Mesh,
    build_box,
    build_cylinder,
    build_primitive,
    build_sphere,
    compute_bounds,
    signed_volume,
    welded_edges,
    welded_edges_by_position,
    welded_edges_by_position_pairs,
)
from kala_agent.studio.geometry.primitives import (
    DEFAULT_BOX_SIZE_MM,
    DEFAULT_CYLINDER_HEIGHT_MM,
    DEFAULT_CYLINDER_RADIUS_MM,
    DEFAULT_SPHERE_RADIUS_MM,
    dimension_keys,
    resolve_dimension_parameters,
)
from kala_agent.studio.geometry.step import (
    OCP_MODULE_NAMES,
    STEP_OUTPUT_MODE,
    STEP_UNAVAILABLE_MESSAGE,
    StepCapability,
    StepExportError,
    StepUnavailableError,
    export_step_primitive,
    probe_step_capability,
)
from kala_agent.studio.geometry.stl import (
    STL_HEADER_FILL,
    STL_HEADER_SIZE,
    write_binary_stl,
)

__all__ = (
    "ANALYTIC_WARNING",
    "DEFAULT_BOX_EDGE_SUBDIVISIONS",
    "DEFAULT_BOX_SIZE_MM",
    "DEFAULT_CYLINDER_HEIGHT_MM",
    "DEFAULT_CYLINDER_HEIGHT_SEGMENTS",
    "DEFAULT_CYLINDER_RADIAL_SEGMENTS",
    "DEFAULT_CYLINDER_RADIUS_MM",
    "DEFAULT_SPHERE_RADIUS_MM",
    "DEFAULT_SPHERE_RINGS",
    "DEFAULT_SPHERE_SECTORS",
    "MAX_BOX_EDGE_SUBDIVISIONS",
    "MAX_CYLINDER_HEIGHT_SEGMENTS",
    "MAX_CYLINDER_RADIAL_SEGMENTS",
    "MAX_DIMENSION_MM",
    "MAX_MESH_TRIANGLES",
    "MAX_MESH_VERTICES",
    "MAX_SPHERE_RINGS",
    "MAX_SPHERE_SECTORS",
    "MESH_ENGINE",
    "MIN_CYLINDER_HEIGHT_SEGMENTS",
    "MIN_CYLINDER_RADIAL_SEGMENTS",
    "MIN_DIMENSION_MM",
    "MIN_SPHERE_RINGS",
    "MIN_SPHERE_SECTORS",
    "OCP_MODULE_NAMES",
    "STEP_OUTPUT_MODE",
    "STEP_UNAVAILABLE_MESSAGE",
    "STL_HEADER_FILL",
    "STL_HEADER_SIZE",
    "Bounds",
    "Mesh",
    "StepCapability",
    "StepExportError",
    "StepUnavailableError",
    "build_box",
    "build_cylinder",
    "build_primitive",
    "build_sphere",
    "compute_bounds",
    "dimension_keys",
    "export_step_primitive",
    "probe_step_capability",
    "resolve_dimension_parameters",
    "signed_volume",
    "validate_finite_positive",
    "validate_int_in_range",
    "welded_edges",
    "welded_edges_by_position",
    "welded_edges_by_position_pairs",
    "write_binary_stl",
)
