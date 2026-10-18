"""Finite bounds and resource caps for studio geometry and preview requests.

These limits are the only place the studio package expresses "how big is too
big". Every public entry point that accepts user-controlled numbers normalises
through :func:`validate_finite_positive` so a non-finite, non-positive, or
out-of-range value is rejected with a bounded, sanitized message rather than
let into the math. Nothing here reaches the network or reads a credential; the
constants are intentionally small enough to keep preview work bounded in time
and memory on a single operator's machine.
"""

from __future__ import annotations

import math
from typing import Final

# Dimensions are millimetres. One metre is already beyond anything a single
# operator studio preview should ever need, so two metres is a hard ceiling.
MAX_DIMENSION_MM: Final[float] = 2000.0
MIN_DIMENSION_MM: Final[float] = 0.1

# Box edge count limit and tessellation counts are independent of dimension.
# They bound vertex, triangle, and resource use regardless of how small a
# primitive is requested.
MAX_BOX_EDGE_SUBDIVISIONS: Final[int] = 64
DEFAULT_BOX_EDGE_SUBDIVISIONS: Final[int] = 1

MAX_CYLINDER_RADIAL_SEGMENTS: Final[int] = 256
MIN_CYLINDER_RADIAL_SEGMENTS: Final[int] = 6
DEFAULT_CYLINDER_RADIAL_SEGMENTS: Final[int] = 48

MAX_CYLINDER_HEIGHT_SEGMENTS: Final[int] = 64
MIN_CYLINDER_HEIGHT_SEGMENTS: Final[int] = 1
DEFAULT_CYLINDER_HEIGHT_SEGMENTS: Final[int] = 1

MAX_SPHERE_RINGS: Final[int] = 96
MIN_SPHERE_RINGS: Final[int] = 3
DEFAULT_SPHERE_RINGS: Final[int] = 24

MAX_SPHERE_SECTORS: Final[int] = 128
MIN_SPHERE_SECTORS: Final[int] = 4
DEFAULT_SPHERE_SECTORS: Final[int] = 32

# Hard resource ceiling for a single mesh, expressed in vertices. The numeric
# ceiling is small enough that no legitimate primitive request would approach
# it. A pathological request is refused at the validator rather than allowed
# to grow a vertex buffer without bound.
MAX_MESH_VERTICES: Final[int] = 200_000
MAX_MESH_TRIANGLES: Final[int] = 400_000


def validate_finite_positive(name: str, value: object) -> float:
    """Return a finite, positive numeric value within millimetre bounds.

    The function rejects booleans because Python treats them as a subclass of
    int and a stray True/False should never silently enter a size calculation.
    Non-finite, non-positive, or out-of-range numbers raise ``ValueError`` with
    a message that does not echo the submitted value.
    """
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a finite number")
    candidate = float(value)
    if not math.isfinite(candidate):
        raise ValueError(f"{name} must be a finite number")
    if candidate < MIN_DIMENSION_MM or candidate > MAX_DIMENSION_MM:
        raise ValueError(
            f"{name} must be between {MIN_DIMENSION_MM} and {MAX_DIMENSION_MM} millimetres"
        )
    return candidate


def validate_int_in_range(name: str, value: object, *, minimum: int, maximum: int) -> int:
    """Return an integer count bounded by an inclusive range.

    Booleans are rejected, exactly as in :func:`validate_finite_positive`, and
    the message never echoes the submitted value.
    """
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    if value < minimum or value > maximum:
        raise ValueError(f"{name} must be between {minimum} and {maximum}")
    return value


__all__ = (
    "DEFAULT_BOX_EDGE_SUBDIVISIONS",
    "DEFAULT_CYLINDER_HEIGHT_SEGMENTS",
    "DEFAULT_CYLINDER_RADIAL_SEGMENTS",
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
    "MIN_CYLINDER_HEIGHT_SEGMENTS",
    "MIN_CYLINDER_RADIAL_SEGMENTS",
    "MIN_DIMENSION_MM",
    "MIN_SPHERE_RINGS",
    "MIN_SPHERE_SECTORS",
    "validate_finite_positive",
    "validate_int_in_range",
)
