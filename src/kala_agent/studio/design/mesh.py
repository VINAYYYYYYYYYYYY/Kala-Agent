"""Bounded OpenCascade tessellation for Studio design results.

The design mesh is a real triangulation of the compiled B-Rep, produced by
``BRepMesh_IncrementalMesh`` and read back face by face. Positions are in
millimetres in the assembly coordinate frame; normals are the kernel's own
per-vertex normals, flipped and re-wound for faces whose orientation is
reversed; indices are zero-based triangle corners.

The mesh is bounded twice, by vertex count and by triangle count, from
:mod:`kala_agent.studio.geometry.limits`. When a shape exceeds either bound
the module refuses with :class:`MeshError`; it never truncates, because a
truncated mesh would silently show a different part.

The module needs OCP at import time, like the compiler and drawing modules.
The design package ``__init__`` deliberately does not re-export it.
"""

from __future__ import annotations

import math
import struct
from dataclasses import dataclass
from typing import Final

from OCP.BRep import BRep_Tool  # type: ignore[import-untyped]
from OCP.BRepMesh import BRepMesh_IncrementalMesh  # type: ignore[import-untyped]
from OCP.TopAbs import (  # type: ignore[import-untyped]
    TopAbs_FACE,
    TopAbs_REVERSED,
)
from OCP.TopExp import TopExp_Explorer  # type: ignore[import-untyped]
from OCP.TopLoc import TopLoc_Location  # type: ignore[import-untyped]
from OCP.TopoDS import TopoDS, TopoDS_Shape  # type: ignore[import-untyped]

from kala_agent.studio.geometry.limits import (
    MAX_MESH_TRIANGLES,
    MAX_MESH_VERTICES,
)

DEFAULT_MESH_DEFLECTION_MM: Final[float] = 0.1
DEFAULT_ANGULAR_DEFLECTION_RAD: Final[float] = 0.5
MESH_ENGINE: Final[str] = "ocp"
STL_HEADER_FILL: Final[bytes] = b"kala-studio design mesh, binary STL export"
STL_HEADER_SIZE: Final[int] = 80
_STL_TRIANGLE_FORMAT: Final[str] = "<12fH"

__all__ = (
    "DEFAULT_ANGULAR_DEFLECTION_RAD",
    "DEFAULT_MESH_DEFLECTION_MM",
    "MESH_ENGINE",
    "MeshData",
    "MeshError",
    "stl_bytes",
    "tessellate",
)


class MeshError(ValueError):
    """The shape could not be tessellated inside the mesh budget."""


@dataclass(frozen=True, slots=True)
class MeshData:
    """A triangle mesh stored as three parallel flat arrays."""

    positions: tuple[float, ...]
    normals: tuple[float, ...]
    indices: tuple[int, ...]
    deflection_mm: float
    angular_deflection_rad: float

    @property
    def vertex_count(self) -> int:
        """Return the number of vertices."""
        return len(self.positions) // 3

    @property
    def triangle_count(self) -> int:
        """Return the number of triangles."""
        return len(self.indices) // 3


def tessellate(
    shape: TopoDS_Shape,
    *,
    deflection_mm: float = DEFAULT_MESH_DEFLECTION_MM,
    angular_deflection_rad: float = DEFAULT_ANGULAR_DEFLECTION_RAD,
    max_vertices: int = MAX_MESH_VERTICES,
    max_triangles: int = MAX_MESH_TRIANGLES,
) -> MeshData:
    """Triangulate ``shape`` into bounded positions, normals, and indices.

    Raises:
        MeshError: the shape is null, the kernel produced no triangles, a face
            has no triangulation, or the result exceeds the mesh budget.
    """
    if not math.isfinite(deflection_mm) or deflection_mm <= 0.0:
        raise MeshError("the mesh deflection must be a positive finite number")
    if not math.isfinite(angular_deflection_rad) or angular_deflection_rad <= 0.0:
        raise MeshError("the angular deflection must be a positive finite number")
    if shape is None or shape.IsNull():
        raise MeshError("the mesh needs a non-null shape")

    mesher = BRepMesh_IncrementalMesh(shape, deflection_mm, False, angular_deflection_rad, True)
    if not mesher.IsDone():
        raise MeshError("the geometry kernel could not tessellate the shape")

    positions: list[float] = []
    normals: list[float] = []
    indices: list[int] = []
    location = TopLoc_Location()
    explorer = TopExp_Explorer(shape, TopAbs_FACE)
    while explorer.More():
        face = TopoDS.Face(explorer.Current())
        triangulation = BRep_Tool.Triangulation_s(face, location)
        if triangulation is None:
            raise MeshError("a face of the shape has no triangulation")
        try:
            if not triangulation.HasNormals():
                triangulation.ComputeNormals()
        except Exception as error:  # pragma: no cover - OCP raises bare exceptions
            raise MeshError(
                f"the geometry kernel could not compute mesh normals: {type(error).__name__}"
            ) from None

        node_count = int(triangulation.NbNodes())
        triangle_count = int(triangulation.NbTriangles())
        if len(positions) // 3 + node_count > max_vertices:
            raise MeshError(
                f"the mesh would exceed the {max_vertices} vertex limit; refusing to truncate it"
            )
        if len(indices) // 3 + triangle_count > max_triangles:
            raise MeshError(
                f"the mesh would exceed the {max_triangles} triangle limit; refusing to truncate it"
            )

        transform = location.Transformation()
        reversed_face = face.Orientation() == TopAbs_REVERSED
        offset = len(positions) // 3
        for node in range(1, node_count + 1):
            point = triangulation.Node(node).Transformed(transform)
            positions.extend((float(point.X()), float(point.Y()), float(point.Z())))
            normal = triangulation.Normal(node)
            nx, ny, nz = float(normal.X()), float(normal.Y()), float(normal.Z())
            if reversed_face:
                nx, ny, nz = -nx, -ny, -nz
            magnitude = math.sqrt(nx * nx + ny * ny + nz * nz)
            if magnitude > 0.0:
                nx, ny, nz = nx / magnitude, ny / magnitude, nz / magnitude
            normals.extend((nx, ny, nz))

        for triangle in range(1, triangle_count + 1):
            first, second, third = triangulation.Triangle(triangle).Get()
            if reversed_face:
                second, third = third, second
            indices.extend((offset + first - 1, offset + second - 1, offset + third - 1))
        explorer.Next()

    if not indices:
        raise MeshError("the geometry kernel produced no mesh triangles")
    return MeshData(
        positions=tuple(positions),
        normals=tuple(normals),
        indices=tuple(indices),
        deflection_mm=deflection_mm,
        angular_deflection_rad=angular_deflection_rad,
    )


def stl_bytes(mesh: MeshData, *, header: bytes = STL_HEADER_FILL) -> bytes:
    """Serialise ``mesh`` as a deterministic binary STL byte string."""
    fill = (header + b" " * STL_HEADER_SIZE)[:STL_HEADER_SIZE]
    triangle_count = mesh.triangle_count
    if triangle_count == 0:
        return fill + struct.pack("<I", 0)
    pieces: list[bytes] = [fill, struct.pack("<I", triangle_count)]
    positions = mesh.positions
    indices = mesh.indices
    for offset in range(0, len(indices), 3):
        i0, i1, i2 = indices[offset], indices[offset + 1], indices[offset + 2]
        ax, ay, az = positions[i0 * 3], positions[i0 * 3 + 1], positions[i0 * 3 + 2]
        bx, by, bz = positions[i1 * 3], positions[i1 * 3 + 1], positions[i1 * 3 + 2]
        cx, cy, cz = positions[i2 * 3], positions[i2 * 3 + 1], positions[i2 * 3 + 2]
        ux, uy, uz = bx - ax, by - ay, bz - az
        vx, vy, vz = cx - ax, cy - ay, cz - az
        nx = uy * vz - uz * vy
        ny = uz * vx - ux * vz
        nz = ux * vy - uy * vx
        magnitude_sq = nx * nx + ny * ny + nz * nz
        if magnitude_sq > 0.0:
            inv = 1.0 / math.sqrt(magnitude_sq)
            nx, ny, nz = nx * inv, ny * inv, nz * inv
        pieces.append(
            struct.pack(
                _STL_TRIANGLE_FORMAT,
                nx,
                ny,
                nz,
                ax,
                ay,
                az,
                bx,
                by,
                bz,
                cx,
                cy,
                cz,
                0,
            )
        )
    return b"".join(pieces)
