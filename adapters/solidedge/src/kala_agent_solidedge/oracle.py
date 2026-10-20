"""Closed-form expectations for the measured Solid Edge parity corpus.

The oracle is deliberately narrow. It can predict a single rectangle or circle
pad on any canonical plane, with an optional same-plane contained circular hole,
and nothing else. Everything outside that returns ``checked=False`` with a
reason instead of a guessed number.

Plane conventions (``docs/construction-semantics.md``):

* ``XY``: a point ``(u, v)`` is ``(u, v, 0)`` and the normal is ``+Z``;
* ``XZ``: ``(u, v)`` is ``(u, 0, v)`` and the normal is ``-Y``;
* ``YZ``: ``(u, v)`` is ``(0, u, v)`` and the normal is ``+X``.

An extrusion runs along the normal, or against it when reversed. A cut runs
along the canonical normal only, so a cut drawn on a reversed pad removes
nothing. Assembly bounds are the union of the translated part boxes; the oracle
never sums volumes across components.
"""

from __future__ import annotations

import math
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Final

from kala_agent.contracts.construction import (
    ComponentScript,
    ComponentStep,
    ConstructionScript,
)
from kala_solidedge_bridge.protocol import (
    BuildResult,
    PartResult,
    RangeBox,
)

VOLUME_RELATIVE_TOLERANCE: Final[float] = 0.001
BOUNDS_ABSOLUTE_TOLERANCE_MM: Final[float] = 0.001
_ZERO_MM: Final[float] = 1e-9
_ZERO_VOLUME_MM3: Final[float] = 1e-9

_PLANE_NORMALS: Final[dict[str, tuple[float, float, float]]] = {
    "XY": (0.0, 0.0, 1.0),
    "XZ": (0.0, -1.0, 0.0),
    "YZ": (1.0, 0.0, 0.0),
}


@dataclass(frozen=True)
class PartExpectation:
    """One component's predicted local geometry, or why it is unchecked."""

    name: str
    checked: bool
    reason: str
    volume_mm3: float | None = None
    min_mm: tuple[float, float, float] | None = None
    max_mm: tuple[float, float, float] | None = None


@dataclass(frozen=True)
class OracleExpectation:
    """The whole script's expectation, including assembly placement."""

    parts: tuple[PartExpectation, ...]
    assembly_expected: bool
    assembly_bounds: RangeBox | None
    reason: str


@dataclass(frozen=True)
class OracleReport:
    """What the oracle could check and what it found wrong."""

    checked: bool
    failures: tuple[str, ...]
    notes: tuple[str, ...]

    @property
    def passed(self) -> bool:
        return self.checked and not self.failures


def expectation(script: ConstructionScript) -> OracleExpectation:
    """Predict the corpus subset, or explain why this script is unchecked."""
    placements = _placements(script)
    parts = tuple(_part_expectation(component) for component in script.components)
    assembly_expected = len(script.components) > 1 or any(
        abs(step[0]) > _ZERO_MM or abs(step[1]) > _ZERO_MM or abs(step[2]) > _ZERO_MM
        for step in placements.values()
    )
    bounds = None
    reason = ""
    if assembly_expected:
        if not all(part.checked for part in parts):
            reason = "assembly bounds need every component inside the checked corpus"
        else:
            bounds = _union_bounds(parts, placements)
    if not all(part.checked for part in parts) and not reason:
        reason = next(part.reason for part in parts if not part.checked)
    return OracleExpectation(
        parts=parts,
        assembly_expected=assembly_expected,
        assembly_bounds=bounds,
        reason=reason,
    )


def compare(
    script: ConstructionScript,
    result: BuildResult,
) -> OracleReport:
    """Compare a completed build against the closed-form expectation."""
    expected = expectation(script)
    failures: list[str] = []
    notes: list[str] = []

    if not expected.parts or not all(part.checked for part in expected.parts):
        return OracleReport(
            checked=False,
            failures=(),
            notes=(
                expected.reason or "this script is outside the closed-form corpus",
                "the result was not compared against a predicted volume or bounding box",
            ),
        )

    by_name: dict[str, PartResult] = {part.name: part for part in result.parts}
    for part in expected.parts:
        actual = by_name.get(part.name)
        if actual is None:
            failures.append(f"{part.name}: the result has no measured part")
            continue
        failures.extend(_compare_part(part, actual))

    if expected.assembly_expected:
        if result.assembly_bounds is None:
            failures.append("assembly bounds are missing from the result")
        elif expected.assembly_bounds is not None:
            failures.extend(
                _compare_box("assembly", expected.assembly_bounds, result.assembly_bounds)
            )
        else:
            notes.append("assembly bounds were not predicted")

    return OracleReport(checked=True, failures=tuple(failures), notes=tuple(notes))


def assembly_bounds_for(
    parts: Sequence[PartResult],
    placements: Mapping[str, tuple[float, float, float]],
) -> RangeBox:
    """Union the translated local part boxes. No volume is claimed."""
    minimum = [math.inf, math.inf, math.inf]
    maximum = [-math.inf, -math.inf, -math.inf]
    for part in parts:
        offset = placements.get(part.name, (0.0, 0.0, 0.0))
        for axis in range(3):
            minimum[axis] = min(minimum[axis], part.range_box.min_mm[axis] + offset[axis])
            maximum[axis] = max(maximum[axis], part.range_box.max_mm[axis] + offset[axis])
    if any(math.isinf(value) for value in minimum + maximum):
        raise ValueError("an assembly needs at least one placed part")
    return RangeBox(
        min_mm=(minimum[0], minimum[1], minimum[2]),
        max_mm=(maximum[0], maximum[1], maximum[2]),
    )


# --------------------------------------------------------------------- prediction


def _placements(script: ConstructionScript) -> dict[str, tuple[float, float, float]]:
    return {
        step.component: (step.x, step.y, step.z)
        for step in script.assembly
        if step.op == "place" and step.component is not None
    }


def _part_expectation(component: ComponentScript) -> PartExpectation:
    steps = component.steps
    if len(steps) == 2 and steps[0].op == "sketch":
        return _pad_expectation(component.name, steps[0], steps[1])
    if (
        len(steps) == 4
        and steps[0].op == "sketch"
        and steps[1].op in {"extrude", "revolve"}
        and steps[2].op == "sketch"
        and steps[3].op == "pocket"
    ):
        return _hole_expectation(component.name, steps)
    return PartExpectation(
        name=component.name,
        checked=False,
        reason=(
            f"component {component.name!r} is outside the closed-form corpus: only a single "
            "pad and a pad with one same-plane circular hole are predicted"
        ),
    )


def _pad_expectation(
    name: str,
    sketch: ComponentStep,
    solid: ComponentStep,
) -> PartExpectation:
    if solid.op == "revolve":
        if sketch.profile != "circle":
            return PartExpectation(
                name=name,
                checked=False,
                reason=(
                    f"component {name!r} revolves a profile; revolve volumes are not part of "
                    "the closed-form corpus"
                ),
            )
        # A circle revolve is lowered to a pad whose length is the supplied
        # length or the diameter.
        length = solid.length if solid.length is not None else sketch.diameter
    elif solid.op == "extrude":
        length = solid.length
    else:  # pragma: no cover - the construction validator refuses other ops
        length = None
    if length is None:
        return PartExpectation(
            name=name, checked=False, reason=f"component {name!r} has no extrusion length"
        )
    profile = _profile_bounds(sketch)
    if profile is None:
        return PartExpectation(
            name=name, checked=False, reason=f"component {name!r} has no supported profile"
        )
    u0, u1, v0, v1, area = profile
    plane = sketch.plane
    normal = _PLANE_NORMALS.get(plane)
    if normal is None:  # pragma: no cover - the construction validator refuses other planes
        return PartExpectation(
            name=name, checked=False, reason=f"component {name!r} has an unknown plane"
        )
    start, end = (0.0, -length) if solid.reversed else (0.0, length)
    return PartExpectation(
        name=name,
        checked=True,
        reason="",
        volume_mm3=area * length,
        min_mm=_world_min(plane, u0, u1, v0, v1, start, end),
        max_mm=_world_max(plane, u0, u1, v0, v1, start, end),
    )


def _hole_expectation(name: str, steps: Sequence[ComponentStep]) -> PartExpectation:
    base = _pad_expectation(name, steps[0], steps[1])
    if not base.checked or base.volume_mm3 is None:
        return base
    hole = steps[2]
    if hole.profile != "circle" or hole.diameter is None:
        return PartExpectation(
            name=name,
            checked=False,
            reason=f"component {name!r} has a non-circular or unsized hole",
        )
    if hole.plane != steps[0].plane:
        return PartExpectation(
            name=name,
            checked=False,
            reason=(
                f"component {name!r} cuts on {hole.plane} while the pad is on "
                f"{steps[0].plane}; only a same-plane hole is predicted"
            ),
        )
    if not _contained(steps[0], hole):
        return PartExpectation(
            name=name,
            checked=False,
            reason=f"component {name!r} has a hole that is not contained in the pad profile",
        )
    if steps[1].reversed:
        return PartExpectation(
            name=name,
            checked=False,
            reason=(
                f"component {name!r} cuts a reversed pad; the canonical cut removes nothing "
                "there, and an unchecked no-op cut is not a validated result"
            ),
        )
    if base.min_mm is None or base.max_mm is None:  # pragma: no cover - checked above
        return base
    cut = steps[3]
    radius = hole.diameter / 2.0
    depth = _cut_depth(steps[0], steps[1], cut)
    removed = math.pi * radius * radius * depth
    volume = base.volume_mm3 - removed
    if volume <= _ZERO_VOLUME_MM3:
        return PartExpectation(
            name=name,
            checked=False,
            reason=f"component {name!r} would cut away its whole solid",
        )
    # A contained hole changes no extreme, so the pad box is the part box.
    return PartExpectation(
        name=name,
        checked=True,
        reason="",
        volume_mm3=volume,
        min_mm=base.min_mm,
        max_mm=base.max_mm,
    )


def _cut_depth(sketch: ComponentStep, solid: ComponentStep, cut: ComponentStep) -> float:
    """How much material the canonical cut can remove from a forward pad.

    A cut runs along the canonical normal, so on a forward pad it can only
    remove up to the pad's own length. A reversed pad puts material on the other
    side of the plane and the canonical cut removes nothing; that case is
    outside the closed-form corpus rather than silently predicted as zero.
    """
    pad_length = solid.length if solid.length is not None else sketch.diameter
    if pad_length is None:  # pragma: no cover - the pad was checked already
        return 0.0
    if solid.reversed:
        return 0.0
    if cut.length is None:
        return pad_length
    return min(cut.length, pad_length)


def _profile_bounds(sketch: ComponentStep) -> tuple[float, float, float, float, float] | None:
    if sketch.profile == "rectangle":
        if sketch.width is None or sketch.height is None:
            return None
        u0, u1 = sketch.x, sketch.x + sketch.width
        v0, v1 = sketch.y, sketch.y + sketch.height
        return u0, u1, v0, v1, sketch.width * sketch.height
    if sketch.profile == "circle":
        if sketch.diameter is None:
            return None
        radius = sketch.diameter / 2.0
        return (
            sketch.x - radius,
            sketch.x + radius,
            sketch.y - radius,
            sketch.y + radius,
            math.pi * radius * radius,
        )
    return None


def _contained(pad: ComponentStep, hole: ComponentStep) -> bool:
    """A circle is contained when its centre is far enough inside the pad."""
    if pad.profile == "rectangle":
        if pad.width is None or pad.height is None or hole.diameter is None:
            return False
        radius = hole.diameter / 2.0
        return (
            hole.x - radius >= pad.x - _ZERO_MM
            and hole.x + radius <= pad.x + pad.width + _ZERO_MM
            and hole.y - radius >= pad.y - _ZERO_MM
            and hole.y + radius <= pad.y + pad.height + _ZERO_MM
        )
    if pad.profile == "circle":
        if pad.diameter is None or hole.diameter is None:
            return False
        distance = math.hypot(hole.x - pad.x, hole.y - pad.y)
        return distance + hole.diameter / 2.0 <= pad.diameter / 2.0 + _ZERO_MM
    return False


def _world_min(
    plane: str,
    u0: float,
    u1: float,
    v0: float,
    v1: float,
    n0: float,
    n1: float,
) -> tuple[float, float, float]:
    return _world_corners(plane, u0, u1, v0, v1, n0, n1)[0]


def _world_max(
    plane: str,
    u0: float,
    u1: float,
    v0: float,
    v1: float,
    n0: float,
    n1: float,
) -> tuple[float, float, float]:
    return _world_corners(plane, u0, u1, v0, v1, n0, n1)[1]


def _world_corners(
    plane: str,
    u0: float,
    u1: float,
    v0: float,
    v1: float,
    n0: float,
    n1: float,
) -> tuple[tuple[float, float, float], tuple[float, float, float]]:
    lo_u, hi_u = min(u0, u1), max(u0, u1)
    lo_v, hi_v = min(v0, v1), max(v0, v1)
    lo_n, hi_n = min(n0, n1), max(n0, n1)
    if plane == "XY":
        return (lo_u, lo_v, lo_n), (hi_u, hi_v, hi_n)
    if plane == "XZ":
        return (lo_u, -hi_n, lo_v), (hi_u, -lo_n, hi_v)
    return (lo_n, lo_u, lo_v), (hi_n, hi_u, hi_v)


def _union_bounds(
    parts: Sequence[PartExpectation],
    placements: Mapping[str, tuple[float, float, float]],
) -> RangeBox:
    minimum = [math.inf, math.inf, math.inf]
    maximum = [-math.inf, -math.inf, -math.inf]
    for part in parts:
        if part.min_mm is None or part.max_mm is None:  # pragma: no cover - checked by caller
            continue
        offset = placements.get(part.name, (0.0, 0.0, 0.0))
        for axis in range(3):
            minimum[axis] = min(minimum[axis], part.min_mm[axis] + offset[axis])
            maximum[axis] = max(maximum[axis], part.max_mm[axis] + offset[axis])
    return RangeBox(
        min_mm=(minimum[0], minimum[1], minimum[2]),
        max_mm=(maximum[0], maximum[1], maximum[2]),
    )


# --------------------------------------------------------------------- comparison


def _compare_part(expected: PartExpectation, actual: PartResult) -> list[str]:
    failures: list[str] = []
    if not actual.valid:
        failures.append(f"{expected.name}: the bridge reported an invalid solid")
        return failures
    if actual.volume_mm3 <= _ZERO_VOLUME_MM3:
        failures.append(f"{expected.name}: the reported volume is zero")
        return failures
    if expected.volume_mm3 is None or expected.min_mm is None or expected.max_mm is None:
        return failures
    difference = abs(actual.volume_mm3 - expected.volume_mm3)
    limit = abs(expected.volume_mm3) * VOLUME_RELATIVE_TOLERANCE
    if difference > limit:
        failures.append(
            f"{expected.name}: volume {actual.volume_mm3:.6f} mm3 differs from the predicted "
            f"{expected.volume_mm3:.6f} mm3 by more than 0.1%"
        )
    failures.extend(
        _compare_box(expected.name, _box(expected.min_mm, expected.max_mm), actual.range_box)
    )
    return failures


def _box(minimum: tuple[float, float, float], maximum: tuple[float, float, float]) -> RangeBox:
    return RangeBox(min_mm=minimum, max_mm=maximum)


def _compare_box(label: str, expected: RangeBox, actual: RangeBox) -> list[str]:
    failures: list[str] = []
    for axis, name in enumerate(("x", "y", "z")):
        if abs(expected.min_mm[axis] - actual.min_mm[axis]) > BOUNDS_ABSOLUTE_TOLERANCE_MM:
            failures.append(
                f"{label}: min {name} is {actual.min_mm[axis]:.6f} mm, expected "
                f"{expected.min_mm[axis]:.6f} mm"
            )
        if abs(expected.max_mm[axis] - actual.max_mm[axis]) > BOUNDS_ABSOLUTE_TOLERANCE_MM:
            failures.append(
                f"{label}: max {name} is {actual.max_mm[axis]:.6f} mm, expected "
                f"{expected.max_mm[axis]:.6f} mm"
            )
    return failures


__all__ = (
    "BOUNDS_ABSOLUTE_TOLERANCE_MM",
    "VOLUME_RELATIVE_TOLERANCE",
    "OracleExpectation",
    "OracleReport",
    "PartExpectation",
    "assembly_bounds_for",
    "compare",
    "expectation",
)
