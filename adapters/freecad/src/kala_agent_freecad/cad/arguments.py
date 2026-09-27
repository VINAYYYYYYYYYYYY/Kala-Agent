"""Strict scalar readers for adapter arguments.

The shipped JSON schemas already bound every value; these readers re-check the
same bounds so a caller that reaches a tool without the registry, or a future
schema that loosens a bound, still cannot reach FreeCAD with an unbounded or
mismatched argument. Each reader raises
:class:`~kala_agent_freecad.errors.FreeCADInputError`, which the worker
reports as ``invalid_arguments``.
"""

from __future__ import annotations

import math
from collections.abc import Mapping, Sequence
from typing import Final

from kala_agent_freecad.errors import FreeCADInputError

MIN_DIMENSION_MM: Final[float] = 0.001
MAX_DIMENSION_MM: Final[float] = 5_000.0
MAX_RADIUS_MM: Final[float] = 2_500.0
MAX_DENSITY_KG_M3: Final[float] = 22_000.0
MAX_YOUNGS_MODULUS_MPA: Final[float] = 1_000_000.0
MAX_POISSON_RATIO: Final[float] = 0.5
MAX_FACE_INDEX: Final[int] = 6
MAX_TEXT_LENGTH: Final[int] = 64


def member(arguments: Mapping[str, object], name: str, permitted: Sequence[str]) -> str:
    value = arguments.get(name)
    if not isinstance(value, str) or value not in permitted:
        raise FreeCADInputError(f"{name} must be one of {', '.join(permitted)}")
    return value


def text(
    arguments: Mapping[str, object],
    name: str,
    default: str,
    maximum: int = MAX_TEXT_LENGTH,
) -> str:
    if name not in arguments:
        return default
    value = arguments[name]
    if not isinstance(value, str) or not value or len(value) > maximum:
        raise FreeCADInputError(f"{name} must be a bounded nonempty string")
    return value


def dimension(arguments: Mapping[str, object], name: str, maximum: float) -> float:
    if name not in arguments:
        return 0.0
    value = arguments[name]
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise FreeCADInputError(f"{name} must be a number")
    number = float(value)
    if not math.isfinite(number) or not MIN_DIMENSION_MM <= number <= maximum:
        raise FreeCADInputError(f"{name} must be between {MIN_DIMENSION_MM} and {maximum}")
    return number


def positive(arguments: Mapping[str, object], name: str, default: float, maximum: float) -> float:
    if name not in arguments:
        return default
    value = arguments[name]
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise FreeCADInputError(f"{name} must be a number")
    number = float(value)
    if not math.isfinite(number) or not 0.0 < number <= maximum:
        raise FreeCADInputError(f"{name} must be above zero and at most {maximum}")
    return number


def ratio(arguments: Mapping[str, object], name: str, default: float) -> float:
    if name not in arguments:
        return default
    value = arguments[name]
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise FreeCADInputError(f"{name} must be a number")
    number = float(value)
    if not math.isfinite(number) or not 0.0 < number < MAX_POISSON_RATIO:
        raise FreeCADInputError(f"{name} must be between zero and {MAX_POISSON_RATIO}")
    return number


def integer(arguments: Mapping[str, object], name: str, default: int, maximum: int) -> int:
    if name not in arguments:
        return default
    value = arguments[name]
    if isinstance(value, bool) or not isinstance(value, int):
        raise FreeCADInputError(f"{name} must be an integer")
    if not 1 <= value <= maximum:
        raise FreeCADInputError(f"{name} must be between 1 and {maximum}")
    return value


def flag(arguments: Mapping[str, object], name: str, *, default: bool) -> bool:
    if name not in arguments:
        return default
    value = arguments[name]
    if not isinstance(value, bool):
        raise FreeCADInputError(f"{name} must be a boolean")
    return value


def unique_members(
    arguments: Mapping[str, object],
    name: str,
    permitted: Sequence[str],
) -> tuple[str, ...]:
    value = arguments.get(name)
    if not isinstance(value, (list, tuple)) or not value:
        raise FreeCADInputError(f"{name} must be a nonempty array")
    selected: list[str] = []
    for entry in value:
        if not isinstance(entry, str) or entry not in permitted:
            raise FreeCADInputError(f"{name} entries must be one of {', '.join(permitted)}")
        if entry in selected:
            raise FreeCADInputError(f"{name} entries must be unique")
        selected.append(entry)
    return tuple(selected)


__all__ = (
    "MAX_DENSITY_KG_M3",
    "MAX_DIMENSION_MM",
    "MAX_FACE_INDEX",
    "MAX_POISSON_RATIO",
    "MAX_RADIUS_MM",
    "MAX_TEXT_LENGTH",
    "MAX_YOUNGS_MODULUS_MPA",
    "MIN_DIMENSION_MM",
    "dimension",
    "flag",
    "integer",
    "member",
    "positive",
    "ratio",
    "text",
    "unique_members",
)
