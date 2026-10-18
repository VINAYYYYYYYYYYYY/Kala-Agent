"""Canonical primitive dimensions shared by the mesh and STEP paths.

The studio expresses "no parameters supplied" in exactly one place. The
mesh builder and the exact B-rep STEP writer both call
:func:`resolve_dimension_parameters`, so a default preview and a default
export can never diverge, and an out-of-range value is refused with the
same :class:`ValueError` before either path runs.
"""

from __future__ import annotations

from collections.abc import Mapping
from typing import Final

from kala_agent.studio.geometry.limits import validate_finite_positive

DEFAULT_BOX_SIZE_MM: Final[float] = 10.0
DEFAULT_CYLINDER_RADIUS_MM: Final[float] = 5.0
DEFAULT_CYLINDER_HEIGHT_MM: Final[float] = 10.0
DEFAULT_SPHERE_RADIUS_MM: Final[float] = 5.0

_DIMENSION_DEFAULTS: Final[dict[str, dict[str, float]]] = {
    "box": {
        "size_x": DEFAULT_BOX_SIZE_MM,
        "size_y": DEFAULT_BOX_SIZE_MM,
        "size_z": DEFAULT_BOX_SIZE_MM,
    },
    "cylinder": {
        "radius": DEFAULT_CYLINDER_RADIUS_MM,
        "height": DEFAULT_CYLINDER_HEIGHT_MM,
    },
    "sphere": {"radius": DEFAULT_SPHERE_RADIUS_MM},
}


def dimension_keys(kind: str) -> tuple[str, ...]:
    """Return the dimension parameter names for ``kind``."""
    defaults = _DIMENSION_DEFAULTS.get(kind)
    if defaults is None:
        raise ValueError(f"unsupported kind: {kind}")
    return tuple(defaults)


def resolve_dimension_parameters(
    kind: str,
    parameters: Mapping[str, object] | None,
) -> dict[str, float]:
    """Return canonical, validated dimension parameters for ``kind``.

    Missing values take the same default the preview mesh uses; every
    value is checked through the shared finite/positive guard so the STEP
    writer and the mesh builder agree on both defaults and bounds.
    """
    defaults = _DIMENSION_DEFAULTS.get(kind)
    if defaults is None:
        raise ValueError(f"unsupported kind: {kind}")
    supplied = parameters or {}
    resolved: dict[str, float] = {}
    for name, default in defaults.items():
        resolved[name] = validate_finite_positive(name, supplied.get(name, default))
    return resolved


__all__ = (
    "DEFAULT_BOX_SIZE_MM",
    "DEFAULT_CYLINDER_HEIGHT_MM",
    "DEFAULT_CYLINDER_RADIUS_MM",
    "DEFAULT_SPHERE_RADIUS_MM",
    "dimension_keys",
    "resolve_dimension_parameters",
)
