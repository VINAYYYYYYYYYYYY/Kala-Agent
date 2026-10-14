"""Lower a construction script to FreeCAD toolbar calls.

The script is the shared protocol. Each component becomes one body, assembly
becomes placement and boolean calls, and a fillet stays a toolbar call rather
than a change to the script.
"""

from __future__ import annotations

from collections.abc import Mapping, Sequence

from kala_agent.contracts.construction import (
    AnalysisScript,
    AnalysisStep,
    AssemblyStep,
    ComponentScript,
    ComponentStep,
    ConstructionScript,
    EditStep,
    RevisionAction,
)
from kala_agent_freecad.cad.decide import _call, _profile_constraints, _rectangle_constraints
from kala_agent_freecad.errors import FreeCADInputError

_ORIGIN = 1e-9


def compile_construction(
    script: ConstructionScript,
    *,
    analysis: AnalysisScript | None = None,
    fillets: Mapping[str, float] | None = None,
) -> list[dict[str, object]]:
    """Compile components, then assembly, then an optional analysis script."""
    calls: list[dict[str, object]] = []
    radii = fillets or {}
    for component in script.components:
        calls.extend(_component(component, _place(script, component.name)))
        radius = radii.get(component.name)
        if radius is not None:
            calls.append(_call("partdesign.fillet", name=component.name, radius=radius))
    calls.extend(_assembly(script))
    if analysis is not None:
        calls.extend(_analysis(analysis))
    if not calls:
        raise FreeCADInputError("the construction script did not compile")
    return calls


def apply_edits(script: ConstructionScript, edits: Sequence[EditStep]) -> ConstructionScript:
    """Return a script with moves and resizes applied. Fillets stay out of the script."""
    by_name = {component.name: component for component in script.components}
    assembly = list(script.assembly)
    for edit in edits:
        if edit.component not in by_name:
            raise FreeCADInputError(f"{edit.op} names an unknown component {edit.component}")
        if edit.op == "move":
            assembly = _move(assembly, edit)
        elif edit.op == "resize":
            by_name[edit.component] = _resize(by_name[edit.component], edit)
        elif edit.op != "fillet":
            raise FreeCADInputError(f"unknown operation {edit.op}")
    return ConstructionScript(
        summary=script.summary,
        components=tuple(by_name[component.name] for component in script.components),
        assembly=tuple(assembly),
    )


def rewrite_component(
    script: ConstructionScript,
    replacement: ComponentScript,
) -> ConstructionScript:
    """Replace one named component. The name must already exist."""
    if replacement.name not in {component.name for component in script.components}:
        raise FreeCADInputError(f"script rewrite names an unknown component {replacement.name}")
    components = tuple(
        replacement if component.name == replacement.name else component
        for component in script.components
    )
    return ConstructionScript(
        summary=script.summary,
        components=components,
        assembly=script.assembly,
    )


def fillets_from_edits(edits: Sequence[EditStep]) -> dict[str, float]:
    """Collect fillet radii so compile_construction can emit them."""
    radii: dict[str, float] = {}
    for edit in edits:
        if edit.op == "fillet" and edit.radius is not None:
            radii[edit.component] = edit.radius
    return radii


def compile_revision(
    script: ConstructionScript,
    edits: Sequence[EditStep],
    rewrite: ComponentScript | None,
    action: RevisionAction,
    *,
    analysis: AnalysisScript | None = None,
) -> tuple[ConstructionScript, list[dict[str, object]]] | None:
    """Apply the decision model's choice and compile the design again.

    An accept, or a choice whose change was not proposed, leaves the design.
    """
    if action is RevisionAction.ACCEPT:
        return None
    if action is RevisionAction.TOOL_EDIT:
        if not edits:
            return None
        revised = apply_edits(script, edits)
        return revised, compile_construction(
            revised,
            analysis=analysis,
            fillets=fillets_from_edits(edits),
        )
    if rewrite is None:
        return None
    revised = rewrite_component(script, rewrite)
    return revised, compile_construction(revised, analysis=analysis)


def _place(script: ConstructionScript, name: str) -> AssemblyStep | None:
    for step in script.assembly:
        if step.op == "place" and step.component == name:
            return step
    return None


def _component(
    component: ComponentScript,
    place: AssemblyStep | None,
) -> list[dict[str, object]]:
    calls: list[dict[str, object]] = [_call("partdesign.body", name=component.name)]
    if place is not None:
        calls.append(
            _call(
                "partdesign.placement",
                name=component.name,
                x=place.x,
                y=place.y,
                z=place.z,
            )
        )
    pending: ComponentStep | None = None
    for step in component.steps:
        if step.op == "sketch":
            if pending is not None:
                raise FreeCADInputError("a sketch must be followed by extrude, revolve, or pocket")
            pending = step
            continue
        if step.op == "helix":
            if pending is not None:
                raise FreeCADInputError("a sketch must be followed by extrude, revolve, or pocket")
            calls.extend(_helix(step))
            continue
        if step.op not in {"extrude", "revolve", "pocket"}:
            raise FreeCADInputError(f"unknown operation {step.op}")
        if pending is None:
            raise FreeCADInputError(f"{step.op} needs a sketch")
        calls.extend(_profile(pending))
        calls.append(_solid(step, pending))
        calls.append(_call("partdesign.check"))
        pending = None
    if pending is not None:
        raise FreeCADInputError("a sketch must be followed by extrude, revolve, or pocket")
    return calls


def _profile(sketch: ComponentStep) -> list[dict[str, object]]:
    calls: list[dict[str, object]] = [_call("sketch.new", plane=sketch.plane)]
    if sketch.profile == "circle":
        diameter = _required(sketch.diameter, "diameter")
        calls.append(_call("sketch.circle", radius=diameter / 2.0, x=sketch.x, y=sketch.y))
        calls.append(_call("sketch.diameter", geo=0, length=diameter))
        if abs(sketch.x) <= _ORIGIN and abs(sketch.y) <= _ORIGIN:
            calls.append(_call("sketch.coincident", geo=0, geo2=-1, pos=3, pos2=1))
        else:
            calls.append(_call("sketch.distance_x", geo=0, length=sketch.x, pos=3))
            calls.append(_call("sketch.distance_y", geo=0, length=sketch.y, pos=3))
    elif sketch.profile == "rectangle":
        width = _required(sketch.width, "width")
        height = _required(sketch.height, "height")
        if abs(sketch.x) <= _ORIGIN and abs(sketch.y) <= _ORIGIN:
            calls.append(_call("sketch.rectangle", height=height, width=width, x=0, y=0))
            calls.extend(_rectangle_constraints(width, height))
        else:
            points = (
                (sketch.x, sketch.y),
                (sketch.x + width, sketch.y),
                (sketch.x + width, sketch.y + height),
                (sketch.x, sketch.y + height),
            )
            calls.append(
                _call(
                    "sketch.polyline",
                    closed=True,
                    points=[[x_mm, y_mm] for x_mm, y_mm in points],
                )
            )
            calls.extend(_profile_constraints(points))
    else:
        raise FreeCADInputError("a sketch profile must be a rectangle or a circle")
    calls.append(_call("sketch.validate"))
    calls.append(_call("sketch.leave"))
    return calls


def _solid(step: ComponentStep, sketch: ComponentStep) -> dict[str, object]:
    if step.op == "pocket":
        if step.length is None:
            return _call("partdesign.pocket")
        return _call("partdesign.pocket", length=step.length)
    # A circle is not revolved. Compose lowers it to a pad of the given length.
    if step.op == "revolve" and sketch.profile != "circle":
        angle = 360.0 if step.angle is None else step.angle
        return _call("partdesign.revolution", angle=angle)
    length = step.length if step.length is not None else _required(sketch.diameter, "length")
    if step.reversed:
        return _call("partdesign.pad", length=length, reversed=True)
    return _call("partdesign.pad", length=length)


def _helix(step: ComponentStep) -> list[dict[str, object]]:
    diameter = _required(step.diameter, "diameter")
    pitch = _required(step.pitch, "pitch")
    length = _required(step.length, "length")
    radius = diameter / 2.0
    depth = max(1.0, diameter * 0.08)
    tooth = max(0.5, min(pitch * 0.45, length * 0.25))
    points = (
        (radius, 0.5),
        (radius + depth, 0.5),
        (radius + depth, 0.5 + tooth),
        (radius, 0.5 + tooth),
    )
    return [
        _call("sketch.new", plane="XZ"),
        _call(
            "sketch.polyline",
            closed=True,
            points=[[x_mm, y_mm] for x_mm, y_mm in points],
        ),
        *_profile_constraints(points),
        _call("sketch.validate"),
        _call("sketch.leave"),
        _call("partdesign.helix", height=length, pitch=pitch),
        _call("partdesign.check"),
    ]


def _assembly(script: ConstructionScript) -> list[dict[str, object]]:
    calls: list[dict[str, object]] = []
    for step in script.assembly:
        if step.op == "place":
            continue
        if step.op == "fuse":
            tool = "part.fuse"
        elif step.op == "compound":
            tool = "part.compound"
        else:
            raise FreeCADInputError(f"unknown operation {step.op}")
        calls.append(_call(tool, components=list(step.components)))
    return calls


def _analysis(script: AnalysisScript) -> list[dict[str, object]]:
    calls = [_call("fem.analysis")]
    for step in script.steps:
        calls.extend(_analysis_step(step))
    return calls


def _analysis_step(step: AnalysisStep) -> list[dict[str, object]]:
    if step.op == "material":
        return [_call("fem.material", name=step.name or "StructuralSteel")]
    if step.op == "fix":
        return [_call("fem.fixed", face=step.face or 1)]
    if step.op == "force":
        force_n = 100.0 if step.force_n is None else step.force_n
        return [_call("fem.force", face=step.face or 2, force_n=force_n)]
    if step.op == "pressure":
        return [
            _call(
                "fem.force",
                face=step.face or 2,
                force_n=_required(step.pressure_mpa, "pressure_mpa"),
            )
        ]
    if step.op == "mesh":
        return [_call("fem.mesh")]
    if step.op == "solve":
        return [_call("fem.solve"), _call("fem.result")]
    raise FreeCADInputError(f"unknown operation {step.op}")


def _move(assembly: Sequence[AssemblyStep], edit: EditStep) -> list[AssemblyStep]:
    updated: list[AssemblyStep] = []
    found = False
    for step in assembly:
        if step.op == "place" and step.component == edit.component:
            if not found:
                updated.append(
                    AssemblyStep(
                        op="place",
                        component=edit.component,
                        x=edit.x,
                        y=edit.y,
                        z=edit.z,
                    )
                )
                found = True
            continue
        updated.append(step)
    if not found:
        updated.append(
            AssemblyStep(
                op="place",
                component=edit.component,
                x=edit.x,
                y=edit.y,
                z=edit.z,
            )
        )
    return updated


def _resize(component: ComponentScript, edit: EditStep) -> ComponentScript:
    steps: list[ComponentStep] = []
    for step in component.steps:
        if edit.length is not None and step.op == "extrude":
            steps.append(step.model_copy(update={"length": edit.length}))
            continue
        if edit.diameter is not None and step.op == "sketch" and step.profile == "circle":
            steps.append(step.model_copy(update={"diameter": edit.diameter}))
            continue
        steps.append(step)
    return component.model_copy(update={"steps": tuple(steps)})


def _required(value: float | None, label: str) -> float:
    if value is None:
        raise FreeCADInputError(f"{label} must be a number")
    return value


__all__ = (
    "apply_edits",
    "compile_construction",
    "compile_revision",
    "fillets_from_edits",
    "rewrite_component",
)
