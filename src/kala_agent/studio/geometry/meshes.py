"""Analytic parametric triangle meshes for studio preview primitives.

Every public function in this module emits a watertight, outward-wound
triangle mesh in millimetres. Triangle winding is enforced by the
per-triangle test ``dot(cross(B-A, C-A), centroid) > 0``: for an
origin-centred convex primitive the outward normal must agree with the
centroid direction, and any triangle that fails the test is emitted in the
opposite winding. This is exactly the same property the brief asks us to
verify, and using it on construction means downstream callers can rely on
``positions`` to drive both a real-time renderer and an STL writer without
flipping winding on either side.

The studio identifies its engine as ``analytic`` in every preview response
so a caller never mistakes the preview for a CAD-grade NURBS or B-rep
surface. The mesh is a tessellation, not a kernel output, and the
:func:`write_binary_stl` writer and the B-rep STEP writer handle it
appropriately.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Final

from kala_agent.studio.geometry.limits import (
    DEFAULT_BOX_EDGE_SUBDIVISIONS,
    DEFAULT_CYLINDER_HEIGHT_SEGMENTS,
    DEFAULT_CYLINDER_RADIAL_SEGMENTS,
    DEFAULT_SPHERE_RINGS,
    DEFAULT_SPHERE_SECTORS,
    MAX_MESH_TRIANGLES,
    MAX_MESH_VERTICES,
    validate_finite_positive,
    validate_int_in_range,
)
from kala_agent.studio.geometry.primitives import resolve_dimension_parameters

MESH_ENGINE: Final[str] = "analytic"
ANALYTIC_WARNING: Final[str] = (
    "analytic tessellation is a preview representation; not CAD-grade NURBS or B-rep"
)


@dataclass(frozen=True, slots=True)
class Mesh:
    """A triangle mesh stored as three parallel flat arrays."""

    positions: tuple[float, ...]
    normals: tuple[float, ...]
    indices: tuple[int, ...]

    @property
    def vertex_count(self) -> int:
        """Return the number of vertices."""
        return len(self.positions) // 3

    @property
    def triangle_count(self) -> int:
        """Return the number of triangles."""
        return len(self.indices) // 3


@dataclass(frozen=True, slots=True)
class Bounds:
    """Axis-aligned bounding box in the mesh's native coordinate frame."""

    min_x: float
    min_y: float
    min_z: float
    max_x: float
    max_y: float
    max_z: float


def _check_resource_bounds(vertex_count: int, triangle_count: int) -> None:
    if vertex_count > MAX_MESH_VERTICES:
        raise ValueError(
            f"the requested primitive would produce {vertex_count} vertices; "
            f"the limit is {MAX_MESH_VERTICES}"
        )
    if triangle_count > MAX_MESH_TRIANGLES:
        raise ValueError(
            f"the requested primitive would produce {triangle_count} triangles; "
            f"the limit is {MAX_MESH_TRIANGLES}"
        )


def compute_bounds(positions: tuple[float, ...]) -> Bounds:
    """Return the axis-aligned bounding box of a flat positions array."""
    if not positions:
        return Bounds(0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
    xs = positions[0::3]
    ys = positions[1::3]
    zs = positions[2::3]
    return Bounds(min(xs), min(ys), min(zs), max(xs), max(ys), max(zs))


def _signed_face_normal(
    ax: float,
    ay: float,
    az: float,
    bx: float,
    by: float,
    bz: float,
    cx: float,
    cy: float,
    cz: float,
) -> tuple[float, float, float]:
    """Return the unnormalised face normal of triangle (A, B, C)."""
    ux, uy, uz = bx - ax, by - ay, bz - az
    vx, vy, vz = cx - ax, cy - ay, cz - az
    return (
        uy * vz - uz * vy,
        uz * vx - ux * vz,
        ux * vy - uy * vx,
    )


def _enforce_outward(
    positions: list[float],
    indices: list[int],
    *,
    i0: int,
    i1: int,
    i2: int,
    auto_correct: bool = True,
) -> None:
    """Append triangle (i0, i1, i2), flipping winding if it points inward.

    The test uses the per-triangle cross product dotted with the triangle
    centroid: for an origin-centred convex primitive that must be positive.
    Any triangle that does not satisfy the test is reordered rather than
    discarded, so the watertight count of the mesh never changes.
    """
    if i0 == i1 or i1 == i2 or i0 == i2:
        return
    ax = positions[i0 * 3]
    ay = positions[i0 * 3 + 1]
    az = positions[i0 * 3 + 2]
    bx = positions[i1 * 3]
    by = positions[i1 * 3 + 1]
    bz = positions[i1 * 3 + 2]
    cx = positions[i2 * 3]
    cy = positions[i2 * 3 + 1]
    cz = positions[i2 * 3 + 2]
    nx, ny, nz = _signed_face_normal(ax, ay, az, bx, by, bz, cx, cy, cz)
    magnitude_sq = nx * nx + ny * ny + nz * nz
    if magnitude_sq <= 0.0:
        return
    centroid_x = (ax + bx + cx) / 3.0
    centroid_y = (ay + by + cy) / 3.0
    centroid_z = (az + bz + cz) / 3.0
    length = math.sqrt(centroid_x * centroid_x + centroid_y * centroid_y + centroid_z * centroid_z)
    if length == 0.0:
        return
    out_dot = (nx * centroid_x + ny * centroid_y + nz * centroid_z) / length
    if out_dot < 0.0:
        if not auto_correct:
            raise ValueError("the requested primitive produced an inward face")
        i1, i2 = i2, i1
    indices.extend((i0, i1, i2))


def _max_box_subs() -> int:
    from kala_agent.studio.geometry.limits import MAX_BOX_EDGE_SUBDIVISIONS

    return MAX_BOX_EDGE_SUBDIVISIONS


def build_box(
    size_x: object,
    size_y: object,
    size_z: object,
    *,
    subdivisions: object = DEFAULT_BOX_EDGE_SUBDIVISIONS,
) -> Mesh:
    """Build a parametric axis-aligned box centred on the origin.

    Each face uses a regular grid of quads. Subdivisions above one tile
    each face with ``n x n`` quads (useful for preview-only faceted
    shading); the default of one matches the analytic intent of flat
    rectangular faces.
    """
    sx = validate_finite_positive("size_x", size_x)
    sy = validate_finite_positive("size_y", size_y)
    sz = validate_finite_positive("size_z", size_z)
    n = validate_int_in_range("subdivisions", subdivisions, minimum=1, maximum=_max_box_subs())

    positions: list[float] = []
    indices: list[int] = []
    half_x = sx / 2.0
    half_y = sy / 2.0
    half_z = sz / 2.0

    faces = (
        # (u_axis, v_axis, constant axis, constant value, outward direction)
        (0, 1, 2, half_z, (0.0, 0.0, 1.0)),  # +Z face
        (0, 1, 2, -half_z, (0.0, 0.0, -1.0)),  # -Z face
        (0, 2, 1, half_y, (0.0, 1.0, 0.0)),  # +Y face
        (0, 2, 1, -half_y, (0.0, -1.0, 0.0)),  # -Y face
        (1, 2, 0, half_x, (1.0, 0.0, 0.0)),  # +X face
        (1, 2, 0, -half_x, (-1.0, 0.0, 0.0)),  # -X face
    )

    for u_axis, v_axis, c_axis, c_value, _outward in faces:
        half_u = (sx if u_axis == 0 else sy if u_axis == 1 else sz) / 2.0
        half_v = (sx if v_axis == 0 else sy if v_axis == 1 else sz) / 2.0
        base_index = len(positions) // 3
        for row in range(n + 1):
            v = (row / n) * 2.0 - 1.0
            for column in range(n + 1):
                u = (column / n) * 2.0 - 1.0
                coords = [0.0, 0.0, 0.0]
                coords[u_axis] = u * half_u
                coords[v_axis] = v * half_v
                coords[c_axis] = c_value
                positions.extend(coords)
        for row in range(n):
            for column in range(n):
                v0 = base_index + row * (n + 1) + column
                v1 = v0 + 1
                v2 = v0 + (n + 1)
                v3 = v2 + 1
                _enforce_outward(positions, indices, i0=v0, i1=v1, i2=v2)
                _enforce_outward(positions, indices, i0=v1, i1=v3, i2=v2)

    return _finalise(positions, indices)


def _finalise(positions: list[float], indices: list[int]) -> Mesh:
    if len(positions) % 3 != 0:
        raise ValueError("the mesh positions array is not a multiple of 3")
    normals = _vertex_normals(positions, indices)
    positions_tuple = tuple(positions)
    normals_tuple = tuple(normals)
    indices_tuple = tuple(indices)
    _check_resource_bounds(len(positions_tuple) // 3, len(indices_tuple) // 3)
    return Mesh(positions=positions_tuple, normals=normals_tuple, indices=indices_tuple)


def _vertex_normals(positions: list[float], indices: list[int]) -> list[float]:
    """Return per-vertex normals that match the face cross product.

    The face cross product is reused so the analytic tessellation and any
    B-rep export share the same orientation. A vertex shared by multiple
    triangles receives the area-weighted average of the face normals, then
    is normalised. This matches what a real-time renderer expects for
    previewing.
    """
    normals = [0.0] * len(positions)
    for offset in range(0, len(indices), 3):
        i0, i1, i2 = indices[offset], indices[offset + 1], indices[offset + 2]
        ax = positions[i0 * 3]
        ay = positions[i0 * 3 + 1]
        az = positions[i0 * 3 + 2]
        bx = positions[i1 * 3]
        by = positions[i1 * 3 + 1]
        bz = positions[i1 * 3 + 2]
        cx = positions[i2 * 3]
        cy = positions[i2 * 3 + 1]
        cz = positions[i2 * 3 + 2]
        nx, ny, nz = _signed_face_normal(ax, ay, az, bx, by, bz, cx, cy, cz)
        for index in (i0, i1, i2):
            normals[index * 3] += nx
            normals[index * 3 + 1] += ny
            normals[index * 3 + 2] += nz
    for vertex in range(len(positions) // 3):
        nx = normals[vertex * 3]
        ny = normals[vertex * 3 + 1]
        nz = normals[vertex * 3 + 2]
        length = math.sqrt(nx * nx + ny * ny + nz * nz)
        if length > 0.0:
            normals[vertex * 3] = nx / length
            normals[vertex * 3 + 1] = ny / length
            normals[vertex * 3 + 2] = nz / length
    return normals


def _min_radial() -> int:
    from kala_agent.studio.geometry.limits import MIN_CYLINDER_RADIAL_SEGMENTS

    return MIN_CYLINDER_RADIAL_SEGMENTS


def _max_radial() -> int:
    from kala_agent.studio.geometry.limits import MAX_CYLINDER_RADIAL_SEGMENTS

    return MAX_CYLINDER_RADIAL_SEGMENTS


def _min_height() -> int:
    from kala_agent.studio.geometry.limits import MIN_CYLINDER_HEIGHT_SEGMENTS

    return MIN_CYLINDER_HEIGHT_SEGMENTS


def _max_height() -> int:
    from kala_agent.studio.geometry.limits import MAX_CYLINDER_HEIGHT_SEGMENTS

    return MAX_CYLINDER_HEIGHT_SEGMENTS


def build_cylinder(
    radius: object,
    height: object,
    *,
    radial_segments: object = DEFAULT_CYLINDER_RADIAL_SEGMENTS,
    height_segments: object = DEFAULT_CYLINDER_HEIGHT_SEGMENTS,
) -> Mesh:
    """Build a closed Z-axis cylinder centred on the origin.

    The function builds the side wall first, then closes the two end caps
    by stitching additional triangles that share the cap centre and the
    first/last ring vertices. The height-segment count drives vertical
    tessellation; the radial-segment count is the number of wedges around
    the axis.
    """
    r = validate_finite_positive("radius", radius)
    h = validate_finite_positive("height", height)
    radial = validate_int_in_range(
        "radial_segments", radial_segments, minimum=_min_radial(), maximum=_max_radial()
    )
    rings = validate_int_in_range(
        "height_segments", height_segments, minimum=_min_height(), maximum=_max_height()
    )
    ring_count = rings + 1

    positions: list[float] = []
    indices: list[int] = []
    half_h = h / 2.0

    ring_first = [0] * ring_count
    for ring_index in range(ring_count):
        ring_first[ring_index] = len(positions) // 3
        z = -half_h + (ring_index / rings) * h
        for segment in range(radial + 1):
            theta = (segment / radial) * 2.0 * math.pi
            cos_t = math.cos(theta)
            sin_t = math.sin(theta)
            positions.extend((r * cos_t, r * sin_t, z))

    for ring_index in range(rings):
        for segment in range(radial):
            i0 = ring_first[ring_index] + segment
            i1 = i0 + 1
            i2 = ring_first[ring_index + 1] + segment
            i3 = i2 + 1
            _enforce_outward(positions, indices, i0=i0, i1=i1, i2=i2)
            _enforce_outward(positions, indices, i0=i1, i1=i3, i2=i2)

    # Top cap: a centre vertex plus a duplicate of the top ring so the cap
    # has its own normal direction. The cap ring matches the top wall ring
    # positionally; only the normal differs.
    top_center_index = len(positions) // 3
    positions.extend((0.0, 0.0, half_h))
    top_ring_first = len(positions) // 3
    for segment in range(radial + 1):
        theta = (segment / radial) * 2.0 * math.pi
        cos_t = math.cos(theta)
        sin_t = math.sin(theta)
        positions.extend((r * cos_t, r * sin_t, half_h))
    for segment in range(radial):
        i0 = top_ring_first + segment
        i1 = i0 + 1
        _enforce_outward(positions, indices, i0=top_center_index, i1=i0, i2=i1)

    # Bottom cap.
    bottom_center_index = len(positions) // 3
    positions.extend((0.0, 0.0, -half_h))
    bottom_ring_first = len(positions) // 3
    for segment in range(radial + 1):
        theta = (segment / radial) * 2.0 * math.pi
        cos_t = math.cos(theta)
        sin_t = math.sin(theta)
        positions.extend((r * cos_t, r * sin_t, -half_h))
    for segment in range(radial):
        i0 = bottom_ring_first + segment
        i1 = i0 + 1
        _enforce_outward(positions, indices, i0=bottom_center_index, i1=i1, i2=i0)

    return _finalise(positions, indices)


def _min_rings() -> int:
    from kala_agent.studio.geometry.limits import MIN_SPHERE_RINGS

    return MIN_SPHERE_RINGS


def _max_rings() -> int:
    from kala_agent.studio.geometry.limits import MAX_SPHERE_RINGS

    return MAX_SPHERE_RINGS


def _min_sectors() -> int:
    from kala_agent.studio.geometry.limits import MIN_SPHERE_SECTORS

    return MIN_SPHERE_SECTORS


def _max_sectors() -> int:
    from kala_agent.studio.geometry.limits import MAX_SPHERE_SECTORS

    return MAX_SPHERE_SECTORS


def build_sphere(
    radius: object,
    *,
    rings: object = DEFAULT_SPHERE_RINGS,
    sectors: object = DEFAULT_SPHERE_SECTORS,
) -> Mesh:
    """Build a UV sphere centred on the origin.

    Each of the two polar vertices is a single shared position; the
    intermediate latitude rings carry a complete ``sector_count + 1`` grid
    of vertices (the extra vertex closes the U seam). Triangles are wound
    outward by the same per-triangle test the box and cylinder use.
    """
    r = validate_finite_positive("radius", radius)
    ring_count = validate_int_in_range("rings", rings, minimum=_min_rings(), maximum=_max_rings())
    sector_count = validate_int_in_range(
        "sectors", sectors, minimum=_min_sectors(), maximum=_max_sectors()
    )

    positions: list[float] = []
    indices: list[int] = []

    # South pole first so the indices used during emission remain stable
    # as the array grows; the north pole comes after the latitude rings.
    south_pole = len(positions) // 3
    positions.extend((0.0, 0.0, -r))

    ring_first: list[int] = [-1] * (ring_count + 1)
    ring_first[ring_count] = south_pole

    for ring_index in range(1, ring_count):
        ring_first[ring_index] = len(positions) // 3
        phi = math.pi * ring_index / ring_count
        sin_phi = math.sin(phi)
        cos_phi = math.cos(phi)
        for sector_index in range(sector_count + 1):
            theta = 2.0 * math.pi * sector_index / sector_count
            cos_t = math.cos(theta)
            sin_t = math.sin(theta)
            positions.extend(
                (
                    r * sin_phi * cos_t,
                    r * sin_phi * sin_t,
                    r * cos_phi,
                )
            )

    north_pole = len(positions) // 3
    positions.extend((0.0, 0.0, r))
    ring_first[0] = north_pole

    for ring_index in range(1, ring_count - 1):
        ring_first_top = ring_first[ring_index]
        ring_first_next = ring_first[ring_index + 1]
        for sector_index in range(sector_count):
            v0 = ring_first_top + sector_index
            v1 = v0 + 1
            v2 = ring_first_next + sector_index
            v3 = v2 + 1
            _enforce_outward(positions, indices, i0=v0, i1=v1, i2=v2)
            _enforce_outward(positions, indices, i0=v1, i1=v3, i2=v2)

    first_ring_first = ring_first[1]
    for sector_index in range(sector_count):
        i0 = first_ring_first + sector_index
        i1 = i0 + 1
        _enforce_outward(positions, indices, i0=north_pole, i1=i0, i2=i1)

    last_ring_first = ring_first[ring_count - 1]
    for sector_index in range(sector_count):
        i0 = last_ring_first + sector_index
        i1 = i0 + 1
        _enforce_outward(positions, indices, i0=south_pole, i1=i1, i2=i0)

    return _finalise(positions, indices)


def build_primitive(
    kind: str,
    parameters: dict[str, object] | None,
) -> Mesh:
    """Build one primitive from its kind and parameter mapping.

    The dispatch refuses unknown kinds and unknown parameter names. Unknown
    parameters are treated as a validation error rather than ignored, which
    keeps the studio from silently changing behaviour when the UI or the
    operator writes a typo.
    """
    if not isinstance(kind, str) or not kind:
        raise ValueError("kind must be a non-empty string")
    params = dict(parameters or {})
    if kind == "box":
        if set(params) - {"size_x", "size_y", "size_z", "subdivisions"}:
            raise ValueError(
                "box parameters must be a subset of size_x, size_y, size_z, subdivisions"
            )
        dims = resolve_dimension_parameters("box", params)
        return build_box(
            dims["size_x"],
            dims["size_y"],
            dims["size_z"],
            subdivisions=params.get("subdivisions", DEFAULT_BOX_EDGE_SUBDIVISIONS),
        )
    if kind == "cylinder":
        if set(params) - {"radius", "height", "radial_segments", "height_segments"}:
            raise ValueError(
                "cylinder parameters must be a subset of "
                "radius, height, radial_segments, height_segments"
            )
        dims = resolve_dimension_parameters("cylinder", params)
        return build_cylinder(
            dims["radius"],
            dims["height"],
            radial_segments=params.get("radial_segments", DEFAULT_CYLINDER_RADIAL_SEGMENTS),
            height_segments=params.get("height_segments", DEFAULT_CYLINDER_HEIGHT_SEGMENTS),
        )
    if kind == "sphere":
        if set(params) - {"radius", "rings", "sectors"}:
            raise ValueError("sphere parameters must be a subset of radius, rings, sectors")
        dims = resolve_dimension_parameters("sphere", params)
        return build_sphere(
            dims["radius"],
            rings=params.get("rings", DEFAULT_SPHERE_RINGS),
            sectors=params.get("sectors", DEFAULT_SPHERE_SECTORS),
        )
    raise ValueError(f"unsupported kind: {kind}")


def welded_edges(mesh: Mesh) -> dict[tuple[int, int], int]:
    """Return a welding map keyed on the ordered pair of vertex indices.

    Each entry counts how many triangles referenced that edge in the
    given order. The function does not collapse spatially identical
    vertices; it answers the structural question "is each edge used
    exactly twice with opposite orientation", which is the manifold
    invariant for a watertight mesh.
    """
    counts: dict[tuple[int, int], int] = {}
    for offset in range(0, len(mesh.indices), 3):
        i0 = int(mesh.indices[offset])
        i1 = int(mesh.indices[offset + 1])
        i2 = int(mesh.indices[offset + 2])
        for first, second in ((i0, i1), (i1, i2), (i2, i0)):
            counts[(first, second)] = counts.get((first, second), 0) + 1
    return counts


def welded_edges_by_position(
    mesh: Mesh,
) -> dict[tuple[tuple[int, int, int], tuple[int, int, int]], int]:
    """Return a position-keyed map of directed edge uses.

    The function uses the spatial endpoint of each edge (millimetre-rounded
    to the nearest integer), so spatially identical seam vertices appear
    as one logical endpoint. A watertight manifold has every directed
    edge appearing exactly twice with opposite orientation, so the counts
    split evenly. The function returns the directed counts without
    pairing; ``welded_edges_by_position_pairs`` returns a paired view that
    collapses the two orientations into one symmetric key.
    """
    positions = mesh.positions
    counts: dict[tuple[tuple[int, int, int], tuple[int, int, int]], int] = {}
    for offset in range(0, len(mesh.indices), 3):
        i0 = int(mesh.indices[offset])
        i1 = int(mesh.indices[offset + 1])
        i2 = int(mesh.indices[offset + 2])
        for first, second in ((i0, i1), (i1, i2), (i2, i0)):
            key = (
                _position_key(positions, first),
                _position_key(positions, second),
            )
            counts[key] = counts.get(key, 0) + 1
    return counts


def welded_edges_by_position_pairs(
    mesh: Mesh,
) -> dict[tuple[tuple[int, int, int], tuple[int, int, int]], int]:
    """Return a symmetric position-keyed welding map.

    Each entry counts the total number of triangle uses for the undirected
    edge. A watertight manifold has every edge used exactly twice; a
    boundary edge is used once. The function uses millimetre-rounded
    position keys so seam vertices collapse to one logical endpoint.
    """
    directed = welded_edges_by_position(mesh)
    paired: dict[tuple[tuple[int, int, int], tuple[int, int, int]], int] = {}
    for (a, b), count in directed.items():
        key = (a, b) if a <= b else (b, a)
        paired[key] = paired.get(key, 0) + count
    return paired


def _position_key(positions: tuple[float, ...] | list[float], index: int) -> tuple[int, int, int]:
    """Return a millimetre-rounded spatial key for a vertex index."""
    base = index * 3
    # round() returns int without a second argument on Python 3.
    key_x = round(positions[base] * 1000)
    key_y = round(positions[base + 1] * 1000)
    key_z = round(positions[base + 2] * 1000)
    return (key_x, key_y, key_z)


def signed_volume(mesh: Mesh) -> float:
    """Return the signed tetrahedral volume of a mesh from the origin."""
    total = 0.0
    positions = mesh.positions
    for offset in range(0, len(mesh.indices), 3):
        i0 = int(mesh.indices[offset])
        i1 = int(mesh.indices[offset + 1])
        i2 = int(mesh.indices[offset + 2])
        ax = positions[i0 * 3]
        ay = positions[i0 * 3 + 1]
        az = positions[i0 * 3 + 2]
        bx = positions[i1 * 3]
        by = positions[i1 * 3 + 1]
        bz = positions[i1 * 3 + 2]
        cx = positions[i2 * 3]
        cy = positions[i2 * 3 + 1]
        cz = positions[i2 * 3 + 2]
        total += (
            ax * (by * cz - bz * cy) + ay * (bz * cx - bx * cz) + az * (bx * cy - by * cx)
        ) / 6.0
    return total


__all__ = (
    "ANALYTIC_WARNING",
    "MESH_ENGINE",
    "Bounds",
    "Mesh",
    "build_box",
    "build_cylinder",
    "build_primitive",
    "build_sphere",
    "compute_bounds",
    "signed_volume",
    "welded_edges",
    "welded_edges_by_position",
    "welded_edges_by_position_pairs",
)
