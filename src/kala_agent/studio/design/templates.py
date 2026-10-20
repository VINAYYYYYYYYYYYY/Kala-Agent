"""Deterministic part templates for Studio design turns.

A template is a pure function: a bounded set of declared millimetre values in,
one :class:`~kala_agent.contracts.construction.ConstructionScript` plus the
declared envelope, declared tolerance strings, and a label out. Templates
never declare a volume or a surface area; those come from the kernel after the
script compiles (ADR-0009 section 2).

The constructions follow the v1 compiler's same-plane pocket rule. The
L-bracket upright is therefore an ``XZ`` pad that is extruded to
``y in [-T, 0]``, bored through along ``Y``, and then placed at ``y = +T``
before it is fused with the ``XY`` base plate. The housing is built from a
floor and four walls rather than a single box with a pocket, because a v1
pocket cuts along the canonical plane normal (+Z for ``XY``) and a single-pad
box would open downward; the five prisms fuse into exactly the same
floor-and-wall envelope with the opening on +Z.

Every template validates its own parameter ranges and geometric feasibility
before it builds a script, and raises :class:`TemplateError` with a readable
message when the declared values cannot produce the part.
"""

from __future__ import annotations

from typing import Final, NamedTuple

from kala_agent.contracts.construction import (
    MAX_COMPONENTS,
    AssemblyStep,
    ComponentScript,
    ComponentStep,
    ConstructionScript,
)
from kala_agent.studio.design.intent import (
    DEFAULT_BRACKET_DEPTH_MM,
    DEFAULT_BRACKET_WIDTH_MM,
)
from kala_agent.studio.design.measure import Bounds
from kala_agent.studio.geometry.limits import MAX_DIMENSION_MM, validate_finite_positive

DEFAULT_HOLE_EDGE_MM: Final[float] = 6.0
MAX_SEGMENTS: Final[int] = MAX_COMPONENTS

__all__ = (
    "DEFAULT_BRACKET_DEPTH_MM",
    "DEFAULT_BRACKET_WIDTH_MM",
    "DEFAULT_HOLE_EDGE_MM",
    "MAX_SEGMENTS",
    "ShaftSegment",
    "TemplateError",
    "TemplateResult",
    "housing_envelope",
    "l_bracket",
    "stepped_shaft",
)


class TemplateError(ValueError):
    """The declared values cannot produce a feasible template part."""


class TemplateResult(NamedTuple):
    """What a template declares: a script, an envelope, tolerances, a label."""

    script: ConstructionScript
    envelope: Bounds
    tolerances: tuple[str, ...]
    label: str


class ShaftSegment(NamedTuple):
    """One diameter/length pair of a stepped shaft, in millimetres."""

    diameter_mm: float
    length_mm: float


def _dimension(name: str, value: object) -> float:
    """Validate one declared dimension and return it as a float."""
    try:
        return validate_finite_positive(name, value)
    except ValueError as error:
        raise TemplateError(str(error)) from None


def _require_name(name: str, value: object) -> float:
    return _dimension(name, value)


def l_bracket(
    width: float,
    depth: float,
    height: float,
    wall: float,
    bore_d: float | None = None,
    hole_d: float | None = None,
    hole_count: int = 0,
    edge: float | None = None,
) -> TemplateResult:
    """Build the ADR-0009 L-bracket.

    The base plate spans ``x in [-W/2, W/2]``, ``y in [0, D]``,
    ``z in [0, T]``. The upright spans ``x in [-W/2, W/2]``,
    ``y in [0, T]``, ``z in [0, H]``. The optional bore runs along ``Y``
    through the upright at ``(x=0, z=T+(H-T)/2)``. Base holes run along
    ``Z`` at ``x = +/- (W/2 - edge)`` and ``y in {T+edge, D-edge}`` for four
    holes or ``y = (T+D)/2`` for two.
    """
    w = _dimension("width", width)
    d = _dimension("depth", depth)
    h = _dimension("height", height)
    t = _dimension("wall", wall)
    if h <= t:
        raise TemplateError("height must be greater than the wall thickness")
    if d <= t:
        raise TemplateError("depth must be greater than the wall thickness")

    if hole_count not in (0, 2, 4):
        raise TemplateError("hole_count must be 0, 2, or 4")

    bore: float | None = None
    if bore_d is not None:
        bore = _dimension("bore_d", bore_d)
        if bore >= w:
            raise TemplateError("the bore must be narrower than the bracket width")
        if bore >= h - t:
            raise TemplateError("the bore must fit inside the upright leg (height - wall)")

    holes: float | None = None
    hole_edge = 0.0
    if hole_count:
        if hole_d is None:
            raise TemplateError("hole_d is required when hole_count is not 0")
        holes = _dimension("hole_d", hole_d)
        if holes >= t:
            raise TemplateError("the base holes must be smaller than the wall thickness")
        hole_edge = (
            max(1.5 * holes, DEFAULT_HOLE_EDGE_MM) if edge is None else _dimension("edge", edge)
        )
        if hole_edge <= holes / 2.0:
            raise TemplateError("the hole edge distance must exceed the hole radius")
        if 2.0 * hole_edge >= w:
            raise TemplateError("two edge distances do not fit across the bracket width")
        if hole_edge + holes / 2.0 >= w / 2.0:
            raise TemplateError("the hole edge distance leaves too little material at the edge")
        if hole_count == 4:
            if d - t <= 2.0 * hole_edge:
                raise TemplateError(
                    "the base plate is too shallow for four holes at this edge distance"
                )
        elif d - t <= holes:
            raise TemplateError("the base plate is too shallow for two centred holes")

    base_steps: list[ComponentStep] = [
        ComponentStep(
            op="sketch",
            plane="XY",
            profile="rectangle",
            width=w,
            height=d,
            x=-w / 2.0,
            y=0.0,
        ),
        ComponentStep(op="extrude", length=t),
    ]
    if holes is not None:
        hole_rows: tuple[float, ...]
        if hole_count == 4:
            hole_rows = (t + hole_edge, d - hole_edge)
        else:
            hole_rows = ((t + d) / 2.0,)
        for row in hole_rows:
            for column in (-(w / 2.0 - hole_edge), w / 2.0 - hole_edge):
                base_steps.extend(
                    (
                        ComponentStep(
                            op="sketch",
                            plane="XY",
                            profile="circle",
                            diameter=holes,
                            x=column,
                            y=row,
                        ),
                        ComponentStep(op="pocket"),
                    )
                )

    upright_steps: list[ComponentStep] = [
        ComponentStep(
            op="sketch",
            plane="XZ",
            profile="rectangle",
            width=w,
            height=h,
            x=-w / 2.0,
            y=0.0,
        ),
        ComponentStep(op="extrude", length=t),
    ]
    if bore is not None:
        upright_steps.extend(
            (
                ComponentStep(
                    op="sketch",
                    plane="XZ",
                    profile="circle",
                    diameter=bore,
                    x=0.0,
                    y=(h + t) / 2.0,
                ),
                ComponentStep(op="pocket"),
            )
        )

    script = ConstructionScript(
        summary=f"L-bracket {w:g} x {d:g} x {h:g} mm",
        components=(
            ComponentScript(name="base", steps=tuple(base_steps)),
            ComponentScript(name="upright", steps=tuple(upright_steps)),
        ),
        assembly=(
            AssemblyStep(op="place", component="upright", y=t),
            AssemblyStep(op="fuse", components=("base", "upright")),
        ),
    )

    tolerances: list[str] = []
    if bore is not None:
        tolerances.append(f"bore diameter {bore:g} mm (declared)")
    if holes is not None:
        tolerances.append(f"{hole_count} x hole diameter {holes:g} mm (declared)")
    tolerances.append("ISO 2768-m (declared)")
    envelope = Bounds(
        min_mm=(-w / 2.0, 0.0, 0.0),
        max_mm=(w / 2.0, d, h),
    )
    return TemplateResult(
        script=script,
        envelope=envelope,
        tolerances=tuple(tolerances),
        label=f"L-bracket {w:g} x {d:g} x {h:g}",
    )


def stepped_shaft(
    segments: tuple[tuple[float, float], ...] | list[tuple[float, float]],
) -> TemplateResult:
    """Build a coaxial stepped shaft from ``(diameter, length)`` segments.

    Each segment is a circle extruded along +Z, placed at the cumulative
    length of the segments before it, and fused into one solid.
    """
    if not segments:
        raise TemplateError("a stepped shaft requires at least one segment")
    if len(segments) > MAX_SEGMENTS:
        raise TemplateError(f"a stepped shaft supports at most {MAX_SEGMENTS} segments")

    measured: list[ShaftSegment] = []
    total_length = 0.0
    for index, segment in enumerate(segments, start=1):
        diameter, length = segment
        d = _require_name(f"segment {index} diameter", diameter)
        segment_length = _require_name(f"segment {index} length", length)
        measured.append(ShaftSegment(diameter_mm=d, length_mm=segment_length))
        total_length += segment_length
        if total_length > MAX_DIMENSION_MM:
            raise TemplateError("the stepped shaft length exceeds the studio dimension limit")

    components: list[ComponentScript] = []
    assembly: list[AssemblyStep] = []
    offset = 0.0
    for index, segment in enumerate(measured, start=1):
        name = f"segment_{index}"
        components.append(
            ComponentScript(
                name=name,
                steps=(
                    ComponentStep(
                        op="sketch",
                        plane="XY",
                        profile="circle",
                        diameter=segment.diameter_mm,
                        x=0.0,
                        y=0.0,
                    ),
                    ComponentStep(op="extrude", length=segment.length_mm),
                ),
            )
        )
        assembly.append(AssemblyStep(op="place", component=name, z=offset))
        offset += segment.length_mm

    names = tuple(component.name for component in components)
    if len(names) > 1:
        assembly.append(AssemblyStep(op="fuse", components=names))

    max_diameter = max(segment.diameter_mm for segment in measured)
    script = ConstructionScript(
        summary=f"Stepped shaft {len(measured)} segments, {total_length:g} mm long",
        components=tuple(components),
        assembly=tuple(assembly),
    )
    tolerances = (
        *(
            f"segment {index} diameter {segment.diameter_mm:g} mm (declared)"
            for index, segment in enumerate(measured, start=1)
        ),
        "ISO 2768-m (declared)",
    )
    envelope = Bounds(
        min_mm=(-max_diameter / 2.0, -max_diameter / 2.0, 0.0),
        max_mm=(max_diameter / 2.0, max_diameter / 2.0, total_length),
    )
    return TemplateResult(
        script=script,
        envelope=envelope,
        tolerances=tolerances,
        label=f"Stepped shaft {len(measured)} segments",
    )


def housing_envelope(
    width: float,
    depth: float,
    height: float,
    wall: float,
) -> TemplateResult:
    """Build a rectangular housing open on +Z with floor and walls ``wall`` thick."""
    w = _dimension("width", width)
    d = _dimension("depth", depth)
    h = _dimension("height", height)
    t = _dimension("wall", wall)
    if 2.0 * t >= w:
        raise TemplateError("the wall thickness leaves no interior across the width")
    if 2.0 * t >= d:
        raise TemplateError("the wall thickness leaves no interior across the depth")
    if t >= h:
        raise TemplateError("the wall thickness must be less than the housing height")

    inner_width = w - 2.0 * t
    components = (
        ComponentScript(
            name="floor",
            steps=(
                ComponentStep(
                    op="sketch",
                    plane="XY",
                    profile="rectangle",
                    width=w,
                    height=d,
                    x=-w / 2.0,
                    y=-d / 2.0,
                ),
                ComponentStep(op="extrude", length=t),
            ),
        ),
        ComponentScript(
            name="wall_left",
            steps=(
                ComponentStep(
                    op="sketch",
                    plane="XY",
                    profile="rectangle",
                    width=t,
                    height=d,
                    x=-w / 2.0,
                    y=-d / 2.0,
                ),
                ComponentStep(op="extrude", length=h),
            ),
        ),
        ComponentScript(
            name="wall_right",
            steps=(
                ComponentStep(
                    op="sketch",
                    plane="XY",
                    profile="rectangle",
                    width=t,
                    height=d,
                    x=w / 2.0 - t,
                    y=-d / 2.0,
                ),
                ComponentStep(op="extrude", length=h),
            ),
        ),
        ComponentScript(
            name="wall_front",
            steps=(
                ComponentStep(
                    op="sketch",
                    plane="XY",
                    profile="rectangle",
                    width=inner_width,
                    height=t,
                    x=-w / 2.0 + t,
                    y=-d / 2.0,
                ),
                ComponentStep(op="extrude", length=h),
            ),
        ),
        ComponentScript(
            name="wall_back",
            steps=(
                ComponentStep(
                    op="sketch",
                    plane="XY",
                    profile="rectangle",
                    width=inner_width,
                    height=t,
                    x=-w / 2.0 + t,
                    y=d / 2.0 - t,
                ),
                ComponentStep(op="extrude", length=h),
            ),
        ),
    )
    names = tuple(component.name for component in components)
    script = ConstructionScript(
        summary=f"Housing envelope {w:g} x {d:g} x {h:g} mm",
        components=components,
        assembly=(AssemblyStep(op="fuse", components=names),),
    )
    envelope = Bounds(
        min_mm=(-w / 2.0, -d / 2.0, 0.0),
        max_mm=(w / 2.0, d / 2.0, h),
    )
    return TemplateResult(
        script=script,
        envelope=envelope,
        tolerances=(f"walls and floor {t:g} mm (declared)", "ISO 2768-m (declared)"),
        label=f"Housing {w:g} x {d:g} x {h:g}",
    )
