"""Accept a construction script only when it meets the engineering duties.

The model still writes the script. This module returns the duties that are
missing. It does not replace the script with a stored solid.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Final

from kala_agent.contracts import ComponentScript, ComponentStep, ConstructionScript

_AXIS_MM: Final[float] = 2
_MIN_DIAMETER_MM: Final[float] = 12
_MAX_DIAMETER_MM: Final[float] = 80
_HOLE: Final[re.Pattern[str]] = re.compile(r"\b(holes?|bores?)\b", re.IGNORECASE)
_THREAD: Final[re.Pattern[str]] = re.compile(r"\b(screws?|threads?|threaded)\b", re.IGNORECASE)
_JACK: Final[re.Pattern[str]] = re.compile(r"\bscrew\s*jack\b|\bjack\s*screw\b", re.IGNORECASE)
_NAMED_DIAMETER: Final[re.Pattern[str]] = re.compile(
    r"(?:(?:screw|thread|diameter|dia|[øØ])\D{0,20}(\d+(?:\.\d+)?)"
    r"|(\d+(?:\.\d+)?)\s*mm\D{0,20}(?:screw|thread|diameter|dia))",
    re.IGNORECASE,
)
_ANY_SIZE: Final[re.Pattern[str]] = re.compile(r"\d+(?:\.\d+)?\s*mm\b", re.IGNORECASE)
_CHANGE: Final[str] = "\n\nChange:\n"
_BASE: Final[str] = "The base must be a plate wider than the screw, so the jack can stand."
_SCREW: Final[str] = (
    "The screw must be a shaft with a helix. "
    "The pitch must be coarser than a bolt thread and smaller than the diameter."
)
_CUP: Final[str] = "The cup must be a short part on the screw axis, large enough to sit on the screw."
_TOMMY: Final[str] = "The tommy bar must be longer than the screw diameter and must cross the screw."
_AXIS: Final[str] = "The base, the screw, and the cup must share one axis."
_HOLE_DUTY: Final[str] = "A hole must be a pocket in an existing part, not a separate body."
_THREAD_DUTY: Final[str] = "A screw or a thread must include a helix."


@dataclass(frozen=True, slots=True)
class _Solid:
    profile: str
    width: float
    height: float
    length: float
    x: float
    y: float


@dataclass(frozen=True, slots=True)
class _Part:
    name: str
    solids: tuple[_Solid, ...]
    helix: bool
    pitch: float | None
    pocket: bool
    x: float
    y: float


def accept_design(prompt: str, script: ConstructionScript) -> list[str]:
    """Return the missing duties. An empty list accepts the script."""
    asked = _asked(prompt)
    duties: list[str] = []
    if _HOLE.search(asked) and not _any(script, "pocket"):
        duties.append(_HOLE_DUTY)
    jack = _JACK.search(prompt) is not None
    if not jack and _THREAD.search(asked) and not _any(script, "helix"):
        duties.append(_THREAD_DUTY)
    if jack:
        duties.extend(_jack(asked, script))
    return duties


def _asked(prompt: str) -> str:
    """The user's words, without a previous script pasted in for a revision."""
    if _CHANGE in prompt:
        return prompt.rsplit(_CHANGE, 1)[-1]
    return prompt


def _any(script: ConstructionScript, op: str) -> bool:
    return any(step.op == op for component in script.components for step in component.steps)


def _jack(prompt: str, script: ConstructionScript) -> list[str]:
    parts = _parts(script)
    screw = _screw(parts)
    duties: list[str] = []
    diameter = _diameter(screw)
    if screw is None or diameter is None or not _pitch_ok(screw, diameter):
        duties.append(_SCREW)
    elif not _diameter_ok(prompt, diameter):
        duties.append(_diameter_duty(prompt))
    base = None if diameter is None else _base(parts, screw, diameter)
    tommy = None if diameter is None else _tommy(parts, screw, base, diameter)
    cup = None if screw is None or diameter is None else _cup(parts, screw, base, tommy, diameter)
    if base is None:
        duties.append(_BASE)
    if cup is None:
        duties.append(_CUP)
    if tommy is None:
        duties.append(_TOMMY)
    if base is not None and screw is not None and cup is not None and not _aligned(base, screw, cup):
        duties.append(_AXIS)
    return duties


def _diameter_duty(prompt: str) -> str:
    named = _named_diameter(prompt)
    if named is not None:
        return f"The screw diameter must be {named:g} mm, the size given in the request."
    return (
        "The screw diameter must stay between "
        f"{_MIN_DIAMETER_MM:g} and {_MAX_DIAMETER_MM:g} mm when the request gives no size."
    )


def _diameter_ok(prompt: str, diameter: float) -> bool:
    named = _named_diameter(prompt)
    if named is not None:
        return abs(diameter - named) <= 0.5
    if _ANY_SIZE.search(prompt):
        return True
    return _MIN_DIAMETER_MM <= diameter <= _MAX_DIAMETER_MM


def _named_diameter(prompt: str) -> float | None:
    match = _NAMED_DIAMETER.search(prompt)
    if match is None:
        return None
    raw = match.group(1) or match.group(2)
    return float(raw)


def _pitch_ok(part: _Part, diameter: float) -> bool:
    if not part.helix or part.pitch is None or diameter <= 0:
        return False
    return diameter / 8 < part.pitch < diameter


def _screw(parts: tuple[_Part, ...]) -> _Part | None:
    for part in parts:
        solid = part.solids[0]
        if part.helix and solid.profile == "circle":
            return part
    return None


def _diameter(part: _Part | None) -> float | None:
    if part is None:
        return None
    return part.solids[0].width


def _base(parts: tuple[_Part, ...], screw: _Part | None, diameter: float) -> _Part | None:
    plates = [
        part
        for part in parts
        if part is not screw and _is_plate(part.solids[0], diameter)
    ]
    if not plates:
        return None
    return max(plates, key=lambda part: _area(part.solids[0]))


def _tommy(
    parts: tuple[_Part, ...],
    screw: _Part | None,
    base: _Part | None,
    diameter: float,
) -> _Part | None:
    if screw is not None and any(_is_bar(solid, diameter) for solid in screw.solids[1:]):
        return screw
    for part in parts:
        if part is screw or part is base:
            continue
        if _is_bar(part.solids[0], diameter) and screw is not None and _crosses(part, screw):
            return part
    return None


def _cup(
    parts: tuple[_Part, ...],
    screw: _Part,
    base: _Part | None,
    tommy: _Part | None,
    diameter: float,
) -> _Part | None:
    skip = {part.name for part in (screw, base, tommy) if part is not None}
    screw_length = screw.solids[0].length
    for part in parts:
        if part.name in skip:
            continue
        solid = part.solids[0]
        sits = part.pocket or _span(solid) >= diameter
        if solid.length < screw_length and sits:
            return part
    return None


def _aligned(base: _Part, screw: _Part, cup: _Part) -> bool:
    return _near(base, screw) and _near(cup, screw)


def _near(left: _Part, right: _Part) -> bool:
    return abs(left.x - right.x) <= _AXIS_MM and abs(left.y - right.y) <= _AXIS_MM


def _crosses(bar: _Part, screw: _Part) -> bool:
    solid = bar.solids[0]
    long = max(solid.width, solid.height, solid.length)
    dx = abs(bar.x - screw.x)
    dy = abs(bar.y - screw.y)
    near, far = sorted((dx, dy))
    return far <= long / 2 + _AXIS_MM and near <= _thickness(solid) + _AXIS_MM


def _is_plate(solid: _Solid, diameter: float) -> bool:
    if _is_bar(solid, diameter):
        return False
    if solid.profile == "circle":
        return solid.width > diameter and solid.length < solid.width
    footprint = min(solid.width, solid.height)
    return footprint > diameter and solid.length <= footprint


def _is_bar(solid: _Solid, diameter: float) -> bool:
    dims = (solid.width, solid.height, solid.length)
    long = max(dims)
    small, middle = sorted(dims)[:2]
    return long > diameter and long >= 3 * small and long >= 3 * middle


def _area(solid: _Solid) -> float:
    if solid.profile == "circle":
        return solid.width * solid.width
    return solid.width * solid.height


def _span(solid: _Solid) -> float:
    if solid.profile == "circle":
        return solid.width
    return min(solid.width, solid.height)


def _thickness(solid: _Solid) -> float:
    return min(solid.width, solid.height, solid.length)


def _parts(script: ConstructionScript) -> tuple[_Part, ...]:
    found: list[_Part] = []
    for component in script.components:
        part = _part(script, component)
        if part is not None:
            found.append(part)
    return tuple(found)


def _part(script: ConstructionScript, component: ComponentScript) -> _Part | None:
    solids: list[_Solid] = []
    pending: ComponentStep | None = None
    helix = False
    pitch: float | None = None
    pocket = False
    for step in component.steps:
        if step.op == "sketch":
            pending = step
        elif step.op in {"extrude", "revolve"} and pending is not None:
            solids.append(_solid(pending, step))
            pending = None
        elif step.op == "pocket":
            pocket = True
            pending = None
        elif step.op == "helix":
            helix = True
            pitch = step.pitch
    if not solids:
        return None
    place_x, place_y = _place(script, component.name)
    first = solids[0]
    return _Part(
        name=component.name,
        solids=tuple(solids),
        helix=helix,
        pitch=pitch,
        pocket=pocket,
        x=place_x + first.x,
        y=place_y + first.y,
    )


def _solid(sketch: ComponentStep, step: ComponentStep) -> _Solid:
    if sketch.profile == "circle":
        diameter = float(sketch.diameter or 0)
        width = height = diameter
    else:
        width = float(sketch.width or 0)
        height = float(sketch.height or 0)
    return _Solid(
        profile=sketch.profile or "rectangle",
        width=width,
        height=height,
        length=float(step.length or 0),
        x=sketch.x,
        y=sketch.y,
    )


def _place(script: ConstructionScript, name: str) -> tuple[float, float]:
    for step in script.assembly:
        if step.op == "place" and step.component == name:
            return (step.x, step.y)
    return (0.0, 0.0)
