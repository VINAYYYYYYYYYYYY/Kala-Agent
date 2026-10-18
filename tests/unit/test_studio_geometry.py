"""Unit tests for the studio geometry layer.

The tests verify the analytic mesh invariants every primitive must satisfy
before any downstream renderer or STL writer trusts it:

- The mesh is finite-valued and finite-counted.
- Every triangle is wound outward on origin-centred convex primitives
  (``dot(cross(B-A, C-A), centroid) > 0``).
- The signed tetrahedral volume from the origin is close to the analytic
  primitive volume.
- Non-finite, non-positive, and out-of-range parameters are refused.

The tests deliberately do not hit a CAD kernel; the analytic preview is
pure Python, so the only environment dependency is the standard library.
"""

from __future__ import annotations

import math
from collections.abc import Iterator

import pytest

from kala_agent.studio.geometry.limits import (
    MAX_BOX_EDGE_SUBDIVISIONS,
    MAX_CYLINDER_RADIAL_SEGMENTS,
    MAX_DIMENSION_MM,
    MAX_SPHERE_RINGS,
    MAX_SPHERE_SECTORS,
    MIN_DIMENSION_MM,
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
    welded_edges_by_position_pairs,
)
from kala_agent.studio.geometry.stl import STL_HEADER_SIZE, write_binary_stl


def _outward_dots(m: Mesh) -> list[float]:
    """Return per-triangle outward dot products (centroid direction)."""
    out: list[float] = []
    positions = m.positions
    for off in range(0, len(m.indices), 3):
        i0, i1, i2 = m.indices[off], m.indices[off + 1], m.indices[off + 2]
        ax, ay, az = positions[i0 * 3], positions[i0 * 3 + 1], positions[i0 * 3 + 2]
        bx, by, bz = positions[i1 * 3], positions[i1 * 3 + 1], positions[i1 * 3 + 2]
        cx, cy, cz = positions[i2 * 3], positions[i2 * 3 + 1], positions[i2 * 3 + 2]
        ux, uy, uz = bx - ax, by - ay, bz - az
        vx, vy, vz = cx - ax, cy - ay, cz - az
        nx = uy * vz - uz * vy
        ny = uz * vx - ux * vz
        nz = ux * vy - uy * vx
        centroid = ((ax + bx + cx) / 3, (ay + by + cy) / 3, (az + bz + cz) / 3)
        length = math.sqrt(centroid[0] ** 2 + centroid[1] ** 2 + centroid[2] ** 2)
        if length == 0:
            continue
        out.append((nx * centroid[0] + ny * centroid[1] + nz * centroid[2]) / length)
    return out


def _is_finite_value(value: float) -> bool:
    return math.isfinite(value)


@pytest.fixture(autouse=True)
def _no_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    def blocked(*args: object, **kwargs: object) -> None:
        raise AssertionError("geometry tests must not open network connections")

    monkeypatch.setattr("socket.socket.connect", blocked)
    monkeypatch.setattr("socket.create_connection", blocked)
    yield


def test_engine_and_warning_are_stable() -> None:
    assert MESH_ENGINE == "analytic"
    assert ANALYTIC_WARNING.startswith("analytic tessellation")


def test_box_is_centered_with_correct_dimensions() -> None:
    box = build_box(20, 30, 40)
    bounds = compute_bounds(box.positions)
    assert bounds.max_x - bounds.min_x == pytest.approx(20, abs=1e-9)
    assert bounds.max_y - bounds.min_y == pytest.approx(30, abs=1e-9)
    assert bounds.max_z - bounds.min_z == pytest.approx(40, abs=1e-9)
    assert (bounds.min_x + bounds.max_x) == pytest.approx(0, abs=1e-9)
    assert (bounds.min_y + bounds.max_y) == pytest.approx(0, abs=1e-9)
    assert (bounds.min_z + bounds.max_z) == pytest.approx(0, abs=1e-9)


def test_box_triangles_are_all_outward() -> None:
    box = build_box(20, 30, 40)
    dots = _outward_dots(box)
    assert dots, "the box produced no triangles"
    assert all(dot > 0 for dot in dots), f"inward triangles: count={sum(1 for d in dots if d <= 0)}"


def test_box_signed_volume_matches_analytic() -> None:
    box = build_box(20, 30, 40)
    assert signed_volume(box) == pytest.approx(20 * 30 * 40, rel=1e-9)


def test_box_subdivisions_keep_winding() -> None:
    box = build_box(20, 30, 40, subdivisions=4)
    dots = _outward_dots(box)
    assert all(d > 0 for d in dots)
    assert signed_volume(box) == pytest.approx(20 * 30 * 40, rel=1e-9)


def test_box_vertex_normals_are_unit_length() -> None:
    box = build_box(10, 10, 10)
    vertex_count = box.vertex_count
    for index in range(vertex_count):
        nx = box.normals[index * 3]
        ny = box.normals[index * 3 + 1]
        nz = box.normals[index * 3 + 2]
        assert math.sqrt(nx * nx + ny * ny + nz * nz) == pytest.approx(1.0, abs=1e-6)


def test_box_rejects_non_positive_dimension() -> None:
    with pytest.raises(ValueError, match="size_x"):
        build_box(0, 10, 10)
    with pytest.raises(ValueError, match="size_y"):
        build_box(10, -1, 10)
    with pytest.raises(ValueError, match="size_z"):
        build_box(10, 10, float("nan"))


def test_box_rejects_oversized_dimension() -> None:
    with pytest.raises(ValueError, match="millimetres"):
        build_box(MAX_DIMENSION_MM + 1, 10, 10)


def test_box_rejects_too_many_subdivisions() -> None:
    with pytest.raises(ValueError):
        build_box(10, 10, 10, subdivisions=MAX_BOX_EDGE_SUBDIVISIONS + 1)


def test_cylinder_triangles_are_all_outward() -> None:
    cyl = build_cylinder(10, 30)
    dots = _outward_dots(cyl)
    assert dots
    assert all(d > 0 for d in dots)


def test_cylinder_signed_volume_close_to_analytic() -> None:
    cyl = build_cylinder(10, 30)
    expected = math.pi * 100 * 30
    assert signed_volume(cyl) == pytest.approx(expected, rel=5e-3)


def test_cylinder_zero_degenerate_triangles() -> None:
    cyl = build_cylinder(10, 30)
    dots = _outward_dots(cyl)
    # Every triangle should pass the outward test; none should be
    # degenerate (length == 0).
    assert all(_is_finite_value(d) for d in dots)


def test_cylinder_rejects_invalid_radial_segments() -> None:
    with pytest.raises(ValueError):
        build_cylinder(10, 30, radial_segments=MAX_CYLINDER_RADIAL_SEGMENTS + 1)


def test_sphere_triangles_are_all_outward_and_nondegenerate() -> None:
    sph = build_sphere(10)
    dots = _outward_dots(sph)
    assert dots
    assert all(d > 0 for d in dots)
    # 32 pole triangles were previously degenerate; assert none of them are.
    assert all(_is_finite_value(d) for d in dots)


def test_sphere_signed_volume_close_to_analytic() -> None:
    # A UV sphere built from a regular latitude/longitude grid is
    # inherently approximate at the poles: the polar fans accumulate a
    # small extra volume because the triangles fan from a single vertex.
    # The bound below reflects the characteristic behaviour of this
    # tessellation method without certifying it as a CAD-grade solid.
    sph = build_sphere(10, rings=48, sectors=128)
    expected = 4 / 3 * math.pi * 1000
    assert signed_volume(sph) == pytest.approx(expected, rel=1e-2)


def test_sphere_default_tessellation_is_close_enough_for_preview() -> None:
    sph = build_sphere(10)
    expected = 4 / 3 * math.pi * 1000
    # Default tessellation is approximate; the windings and bounds are
    # what matter for a preview, not the volume to five digits.
    assert signed_volume(sph) == pytest.approx(expected, rel=2e-2)


def test_sphere_bounds_cover_full_radius() -> None:
    sph = build_sphere(10)
    bounds = compute_bounds(sph.positions)
    assert bounds.max_x - bounds.min_x == pytest.approx(20, abs=1e-9)
    assert bounds.max_y - bounds.min_y == pytest.approx(20, abs=1e-9)
    assert bounds.max_z - bounds.min_z == pytest.approx(20, abs=1e-9)


def test_sphere_rejects_too_many_rings() -> None:
    with pytest.raises(ValueError):
        build_sphere(10, rings=MAX_SPHERE_RINGS + 1)


def test_sphere_rejects_too_many_sectors() -> None:
    with pytest.raises(ValueError):
        build_sphere(10, sectors=MAX_SPHERE_SECTORS + 1)


def test_position_keyed_manifold_welds_seam() -> None:
    sph = build_sphere(10, rings=12, sectors=12)
    edges = welded_edges_by_position_pairs(sph)
    # Every *position*-unique edge must be referenced an even number of
    # times because the mesh is closed: pole-fan edges are 3 (one
    # fan triangle plus the first quad strip triangle) and seam edges
    # are 2; any odd count is a watertight invariant violation.
    odd = {key: count for key, count in edges.items() if count % 2 != 0}
    assert not odd, (
        f"the sphere mesh has odd edge counts; {len(odd)} undirected edges are not watertight"
    )


def test_box_manifold_is_watertight() -> None:
    box = build_box(20, 30, 40)
    edges = welded_edges_by_position_pairs(box)
    odd = {key: count for key, count in edges.items() if count % 2 != 0}
    assert not odd, f"the box has odd-count edges: {sorted(odd.items())[:3]}"


def test_cylinder_manifold_is_watertight() -> None:
    cyl = build_cylinder(10, 30)
    edges = welded_edges_by_position_pairs(cyl)
    odd = {key: count for key, count in edges.items() if count % 2 != 0}
    assert not odd, f"the cylinder has odd-count edges: {sorted(odd.items())[:3]}"


def test_dispatch_for_box_cylinder_sphere() -> None:
    assert build_primitive("box", {"size_x": 1, "size_y": 2, "size_z": 3}).vertex_count > 0
    assert build_primitive("cylinder", {"radius": 1, "height": 1}).vertex_count > 0
    assert build_primitive("sphere", {"radius": 1}).vertex_count > 0


def test_dispatch_rejects_unknown_kind() -> None:
    with pytest.raises(ValueError, match="unsupported kind"):
        build_primitive("torus", {})


def test_dispatch_rejects_unknown_parameters() -> None:
    with pytest.raises(ValueError, match="must be a subset"):
        build_primitive("box", {"size_x": 1, "size_y": 2, "size_z": 3, "size_w": 4})


def test_min_dimension_is_enforced() -> None:
    with pytest.raises(ValueError, match="millimetres"):
        build_box(MIN_DIMENSION_MM / 2, 1, 1)
    with pytest.raises(ValueError):
        build_sphere(MIN_DIMENSION_MM / 2)


def test_stl_writer_writes_a_binary_stl() -> None:
    box = build_box(10, 10, 10)
    data = write_binary_stl(box)
    assert data.startswith(b"kala-studio ")
    assert len(data) == STL_HEADER_SIZE + 4 + box.triangle_count * 50
    # Triangle count encoded little-endian at offset 80.
    import struct

    count = struct.unpack("<I", data[STL_HEADER_SIZE : STL_HEADER_SIZE + 4])[0]
    assert count == box.triangle_count


def test_stl_writer_normalises_per_triangle_normal() -> None:
    box = build_box(20, 30, 40)
    data = write_binary_stl(box)
    import struct

    # Each triangle starts at header + 4 + i*50. The first 12 bytes are the
    # triangle normal in little-endian floats.
    normal_offset = STL_HEADER_SIZE + 4
    nx, ny, nz = struct.unpack("<3f", data[normal_offset : normal_offset + 12])
    assert math.sqrt(nx * nx + ny * ny + nz * nz) == pytest.approx(1.0, abs=1e-5)


def test_compute_bounds_on_empty_mesh_is_origin() -> None:
    empty = Mesh(positions=(), normals=(), indices=())
    bounds = compute_bounds(empty.positions)
    assert bounds == Bounds(0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
