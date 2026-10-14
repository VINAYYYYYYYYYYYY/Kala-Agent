"""Choose FreeCAD toolbar calls for a short design request.

The result is a tool-call list, not a Python script. Every list is checked
against the feature procedure before it is returned, so a caller cannot
receive a line before a sketch or a pad before a closed profile.
"""

from __future__ import annotations

import math
import re
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Final

from kala_agent_freecad.cad.procedure import DesignCall, parse_calls
from kala_agent_freecad.errors import FreeCADInputError

MAX_PROMPT_CHARS: Final[int] = 4_000
MAX_DIMENSION_MM: Final[float] = 5_000.0
MAX_RADIUS_MM: Final[float] = 2_500.0
_NUMBER: Final[re.Pattern[str]] = re.compile(
    r"(\d+(?:\.\d+)?)(?:\s*(mm|cm|m)\b)?",
    re.IGNORECASE,
)
_ANALYSIS: Final[tuple[dict[str, object], ...]] = (
    {"arguments": {}, "tool": "fem.analysis"},
    {"arguments": {"name": "StructuralSteel"}, "tool": "fem.material"},
    {"arguments": {"face": 1}, "tool": "fem.fixed"},
    {"arguments": {"face": 2, "force_n": 100}, "tool": "fem.force"},
    {"arguments": {}, "tool": "fem.mesh"},
    {"arguments": {}, "tool": "fem.solve"},
    {"arguments": {}, "tool": "fem.result"},
)


@dataclass(frozen=True, slots=True)
class DesignPlan:
    """The tool calls a request became, plus a one-line account of them."""

    calls: tuple[DesignCall, ...]
    summary: str
    kind: str
    log: tuple[str, ...] = ()
    source: object | None = None
    analysis_source: object | None = None
    script: Mapping[str, object] | None = None

    def as_json(self) -> list[dict[str, object]]:
        """Return the calls in the shape the worker and the design box accept."""
        return [{"arguments": dict(call.arguments), "tool": call.tool} for call in self.calls]


def decide(prompt: str) -> DesignPlan:
    """Read a design request and emit the toolbar calls that build it."""
    if not isinstance(prompt, str):
        raise FreeCADInputError("prompt must be a string")
    text = prompt.strip()
    if not text:
        raise FreeCADInputError("prompt must not be empty")
    if len(text) > MAX_PROMPT_CHARS:
        raise FreeCADInputError("prompt exceeds the bounded length")
    folded = text.casefold()
    kind = _kind(folded)
    numbers = _numbers(text)
    if kind == "revolution":
        raw, summary = _revolution(folded, numbers)
    elif kind == "cylinder":
        raw, summary = _cylinder(folded, numbers)
    elif kind == "bracket":
        raw, summary = _bracket(numbers)
    else:
        raw, summary = _plate(numbers, hole=_word(folded, "hole"))
        if kind != "plate":
            kind = "box"
            summary = summary.replace("Plate ", "Box ", 1)
    calls = parse_calls(raw)
    return DesignPlan(calls=calls, summary=summary, kind=kind)


def _kind(text: str) -> str:
    if _word(text, "revolve") or _word(text, "turned"):
        return "revolution"
    if any(_word(text, word) for word in ("cylinder", "rod", "pin", "shaft")):
        return "cylinder"
    if _word(text, "bracket") or _word(text, "angle"):
        return "bracket"
    if _word(text, "plate") or _word(text, "hole"):
        return "plate"
    return "box"


def _word(text: str, word: str) -> bool:
    return re.search(rf"\b{re.escape(word)}\b", text) is not None


def _numbers(text: str) -> tuple[float, ...]:
    found: list[float] = []
    for match in _NUMBER.finditer(text):
        value = float(match.group(1))
        unit = (match.group(2) or "mm").casefold()
        scale = {"mm": 1.0, "cm": 10.0, "m": 1000.0}[unit]
        found.append(value * scale)
        if len(found) == 6:
            break
    return tuple(found)


def _take(numbers: Sequence[float], index: int, default: float) -> float:
    if index < len(numbers):
        return numbers[index]
    return default


def _fit(value: float, label: str, *, maximum: float = MAX_DIMENSION_MM) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise FreeCADInputError(f"{label} must be a number")
    number = float(value)
    if not math.isfinite(number) or not 0.001 <= number <= maximum:
        raise FreeCADInputError(f"{label} must be between 0.001 and {maximum:g} mm")
    return number


def _shown(value: float) -> str:
    rounded = round(value, 3)
    if abs(rounded - round(rounded)) < 1e-9:
        return str(round(rounded))
    return f"{rounded:.3f}".rstrip("0").rstrip(".")


def _call(tool: str, **arguments: object) -> dict[str, object]:
    return {"arguments": arguments, "tool": tool}


def _analysis() -> list[dict[str, object]]:
    return [dict(call) for call in _ANALYSIS]


def _rectangle_constraints(width: float, height: float) -> list[dict[str, object]]:
    """The constraint set that drives a four-line rectangle to zero degrees of freedom."""
    calls = [
        _call("sketch.coincident", geo=index, geo2=(index + 1) % 4, pos=2, pos2=1)
        for index in range(4)
    ]
    calls.extend(
        (
            _call("sketch.horizontal", geo=0),
            _call("sketch.horizontal", geo=2),
            _call("sketch.vertical", geo=1),
            _call("sketch.vertical", geo=3),
            _call("sketch.distance_x", geo=0, geo2=0, length=width, pos=1, pos2=2),
            _call("sketch.distance_y", geo=1, geo2=1, length=height, pos=1, pos2=2),
            _call("sketch.coincident", geo=0, geo2=-1, pos=1, pos2=1),
        )
    )
    return calls


def _profile_constraints(points: Sequence[tuple[float, float]]) -> list[dict[str, object]]:
    """Constrain a closed axis-aligned polyline, leaving the closing lengths driven."""
    count = len(points)
    calls: list[dict[str, object]] = []
    horizontal: list[tuple[int, float]] = []
    vertical: list[tuple[int, float]] = []
    for index in range(count):
        calls.append(
            _call(
                "sketch.coincident",
                geo=index,
                geo2=(index + 1) % count,
                pos=2,
                pos2=1,
            )
        )
        x_start, y_start = points[index]
        x_end, y_end = points[(index + 1) % count]
        dx = x_end - x_start
        dy = y_end - y_start
        if abs(dy) <= 1e-9:
            calls.append(_call("sketch.horizontal", geo=index))
            horizontal.append((index, abs(dx)))
        elif abs(dx) <= 1e-9:
            calls.append(_call("sketch.vertical", geo=index))
            vertical.append((index, abs(dy)))
        else:
            calls.append(_call("sketch.distance", geo=index, length=math.hypot(dx, dy)))
    sized_horizontal = horizontal[:-1] if len(horizontal) > 1 else horizontal
    sized_vertical = vertical[:-1] if len(vertical) > 1 else vertical
    calls.extend(
        _call("sketch.distance", geo=index, length=length)
        for index, length in (*sized_horizontal, *sized_vertical)
    )
    x_origin, y_origin = points[0]
    if abs(x_origin) <= 1e-9 and abs(y_origin) <= 1e-9:
        calls.append(_call("sketch.coincident", geo=0, geo2=-1, pos=1, pos2=1))
    else:
        calls.append(_call("sketch.distance_x", geo=0, length=x_origin, pos=1))
        calls.append(_call("sketch.distance_y", geo=0, length=y_origin, pos=1))
    return calls


def _feature(
    profile: list[dict[str, object]],
    feature: dict[str, object],
) -> list[dict[str, object]]:
    calls = [
        _call("partdesign.body"),
        _call("sketch.new", plane="XY"),
        *profile,
        _call("sketch.validate"),
        _call("sketch.leave"),
        feature,
        _call("partdesign.check"),
    ]
    calls.extend(_analysis())
    return calls


def _plate(numbers: Sequence[float], *, hole: bool) -> tuple[list[dict[str, object]], str]:
    length = _fit(_take(numbers, 0, 80.0), "length")
    width = _fit(_take(numbers, 1, 40.0), "width")
    thickness = _fit(_take(numbers, 2, 8.0), "thickness")
    profile = [
        _call("sketch.rectangle", height=width, width=length, x=0, y=0),
        *_rectangle_constraints(length, width),
    ]
    calls = _feature(profile, _call("partdesign.pad", length=thickness))
    summary = f"Plate {_shown(length)} x {_shown(width)} x {_shown(thickness)} mm"
    if hole:
        diameter = _fit(_take(numbers, 3, 8.0), "hole diameter")
        if diameter >= length or diameter >= width:
            raise FreeCADInputError("the hole must be smaller than the plate")
        calls = calls[: -len(_ANALYSIS)]
        calls.extend(_hole(length / 2.0, width / 2.0, diameter))
        calls.extend(_analysis())
        summary = f"{summary} with a {_shown(diameter)} mm hole"
    return calls, f"{summary}."


def _hole(x_mm: float, y_mm: float, diameter: float) -> list[dict[str, object]]:
    radius = diameter / 2.0
    return [
        _call("sketch.new", plane="XY"),
        _call("sketch.circle", radius=radius, x=x_mm, y=y_mm),
        _call("sketch.diameter", geo=0, length=diameter),
        _call("sketch.distance_x", geo=0, length=x_mm, pos=3),
        _call("sketch.distance_y", geo=0, length=y_mm, pos=3),
        _call("sketch.validate"),
        _call("sketch.leave"),
        _call("partdesign.pocket"),
        _call("partdesign.check"),
    ]


def _cylinder(text: str, numbers: Sequence[float]) -> tuple[list[dict[str, object]], str]:
    diameter = _word(text, "diameter") or _word(text, "dia")
    first = _take(numbers, 0, 20.0 if diameter else 10.0)
    radius = _fit(first / 2.0 if diameter else first, "radius", maximum=MAX_RADIUS_MM)
    height = _fit(_take(numbers, 1, 40.0), "height")
    profile = [
        _call("sketch.circle", radius=radius, x=0, y=0),
        _call("sketch.diameter", geo=0, length=radius * 2.0),
        _call("sketch.coincident", geo=0, geo2=-1, pos=3, pos2=1),
    ]
    calls = _feature(profile, _call("partdesign.pad", length=height))
    summary = f"Cylinder radius {_shown(radius)} mm, height {_shown(height)} mm."
    return calls, summary


def _revolution(text: str, numbers: Sequence[float]) -> tuple[list[dict[str, object]], str]:
    diameter = _word(text, "diameter") or _word(text, "dia")
    first = _take(numbers, 0, 20.0 if diameter else 10.0)
    radius = _fit(first / 2.0 if diameter else first, "radius", maximum=MAX_RADIUS_MM)
    height = _fit(_take(numbers, 1, 40.0), "height")
    profile = [
        _call("sketch.rectangle", height=height, width=radius, x=0, y=0),
        *_rectangle_constraints(radius, height),
    ]
    calls = _feature(profile, _call("partdesign.revolution", angle=360))
    summary = f"Turned part radius {_shown(radius)} mm, height {_shown(height)} mm."
    return calls, summary


def _bracket(numbers: Sequence[float]) -> tuple[list[dict[str, object]], str]:
    length = _fit(_take(numbers, 0, 80.0), "length")
    width = _fit(_take(numbers, 1, 40.0), "width")
    thickness = _fit(_take(numbers, 2, 8.0), "thickness")
    height = _fit(_take(numbers, 3, 40.0), "height")
    if thickness >= length or thickness >= height:
        raise FreeCADInputError("the bracket thickness must be smaller than its length and height")
    points = (
        (0.0, 0.0),
        (length, 0.0),
        (length, thickness),
        (thickness, thickness),
        (thickness, height),
        (0.0, height),
    )
    profile = [
        _call(
            "sketch.polyline",
            closed=True,
            points=[[x_mm, y_mm] for x_mm, y_mm in points],
        ),
        *_profile_constraints(points),
    ]
    calls = _feature(profile, _call("partdesign.pad", length=width))
    summary = (
        f"Angle bracket {_shown(length)} x {_shown(width)} mm, "
        f"{_shown(thickness)} mm thick, {_shown(height)} mm tall."
    )
    return calls, summary


__all__ = (
    "MAX_DIMENSION_MM",
    "MAX_PROMPT_CHARS",
    "MAX_RADIUS_MM",
    "DesignPlan",
    "decide",
)
