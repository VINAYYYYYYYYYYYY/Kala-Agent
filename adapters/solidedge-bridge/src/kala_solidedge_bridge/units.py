"""Millimetre, metre, radian, and area conversions with finite validation.

Every function accepts a plain JSON number (an ``int`` or ``float``) and
rejects booleans, numeric strings, ``NaN``, and infinities. Negative lengths
are mathematically valid conversions and are not rejected here; bounds belong
to the contract models.
"""

from __future__ import annotations

import math
from typing import Final

_MM_PER_M: Final[float] = 1_000.0
_MM3_PER_M3: Final[float] = 1_000_000_000.0
_MM2_PER_M2: Final[float] = 1_000_000.0


def _finite(value: object, label: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{label} must be a finite number, not a boolean or string")
    number = float(value)
    if not math.isfinite(number):
        raise ValueError(f"{label} must be finite")
    return number


def mm_to_m(value: float) -> float:
    """Convert millimetres to metres."""
    return _finite(value, "millimetres") / _MM_PER_M


def m_to_mm(value: float) -> float:
    """Convert metres to millimetres."""
    return _finite(value, "metres") * _MM_PER_M


def deg_to_rad(value: float) -> float:
    """Convert degrees to radians."""
    return math.radians(_finite(value, "degrees"))


def m3_to_mm3(value: float) -> float:
    """Convert cubic metres to cubic millimetres."""
    return _finite(value, "cubic metres") * _MM3_PER_M3


def m2_to_mm2(value: float) -> float:
    """Convert square metres to square millimetres."""
    return _finite(value, "square metres") * _MM2_PER_M2


__all__ = (
    "deg_to_rad",
    "m2_to_mm2",
    "m3_to_mm3",
    "m_to_mm",
    "mm_to_m",
)
