"""Binary STL export for studio preview meshes.

The writer emits the binary STL format defined by the de facto standard used
by CAD tooling: an 80-byte header (filled with ASCII, never credentials), a
little-endian uint32 triangle count, and per triangle a 12-byte normal, a
36-byte position triple, and a 2-byte attribute count of zero. The output is
deterministic for a given mesh; nothing here reads a credential, runs a
solver, or opens the network.

ASCII STL is intentionally not supported: the only callers are the studio
export route and tests, both of which want a byte-stable file. ASCII STL is
notable for using ``solid name`` headers that look like a credential to the
transport boundary, so emitting binary STL keeps the output boring.
"""

from __future__ import annotations

import struct
from typing import Final

from kala_agent.studio.geometry.meshes import Mesh

STL_HEADER_SIZE: Final[int] = 80
STL_TRIANGLE_SIZE: Final[int] = 50
TRIANGLES_PER_FACE: Final[int] = 1  # one triangle per STL record, by design
# One STL triangle record is laid out as: 12 bytes normal (3 little-endian
# floats), 12 bytes vertex A (3 floats), 12 bytes vertex B (3 floats), 12
# bytes vertex C (3 floats), and 2 bytes attribute byte count (uint16). The
# string is 50 bytes total when packed: three floats for the normal plus
# nine floats for the three vertices equals twelve floats (48 bytes), plus
# a 16-bit attribute count (2 bytes).
STL_BINARY_FORMAT: Final[str] = "<12fH"
STL_HEADER_FILL: Final[bytes] = b"kala-studio analytic tessellation, binary STL export"

assert struct.calcsize(STL_BINARY_FORMAT) == STL_TRIANGLE_SIZE, (
    "the binary STL record layout must be exactly 50 bytes"
)


def write_binary_stl(mesh: Mesh) -> bytes:
    """Serialise a mesh as a binary STL byte string.

    Each triangle is emitted with a per-triangle normal that is computed from
    the submitted vertex positions rather than read from the mesh's smooth
    normal array. STL consumers (and most CAM tools) expect the triangle
    normal to match the triangle's geometric surface, and the analytic preview
    keeps its smooth per-vertex normal array for the real-time renderer.
    """
    positions = mesh.positions
    indices = mesh.indices
    triangle_count = len(indices) // 3
    header = (STL_HEADER_FILL + b" " * STL_HEADER_SIZE)[:STL_HEADER_SIZE]
    if triangle_count == 0:
        return header + struct.pack("<I", 0)

    pieces: list[bytes] = [header, struct.pack("<I", triangle_count)]
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
        # Edges B - A and C - A
        ux, uy, uz = bx - ax, by - ay, bz - az
        vx, vy, vz = cx - ax, cy - ay, cz - az
        # Cross product U x V
        nx = uy * vz - uz * vy
        ny = uz * vx - ux * vz
        nz = ux * vy - uy * vx
        # Normalise defensively. For closed analytic meshes the cross product
        # is already non-zero, but a pathological mesh with co-linear edges
        # would otherwise emit a zero normal that some STL readers reject.
        magnitude_sq = nx * nx + ny * ny + nz * nz
        if magnitude_sq > 0.0:
            inv = 1.0 / (magnitude_sq**0.5)
            nx *= inv
            ny *= inv
            nz *= inv
        record = struct.pack(
            STL_BINARY_FORMAT,
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
        pieces.append(record)
    return b"".join(pieces)


__all__ = (
    "STL_BINARY_FORMAT",
    "STL_HEADER_FILL",
    "STL_HEADER_SIZE",
    "STL_TRIANGLE_SIZE",
    "TRIANGLES_PER_FACE",
    "write_binary_stl",
)
