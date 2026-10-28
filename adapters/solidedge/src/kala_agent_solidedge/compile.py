"""Lower a Kala construction script into bridge protocol v1 operations.

This is the only place where the shared construction language meets the Solid
Edge wire contract. It validates the script against the supported subset and
refuses everything else *before* a socket is opened: helix/threads, fuse,
compound, analysis, fillets, and a rectangle revolved on ``XZ`` never reach the
bridge.

Semantics follow ``docs/construction-semantics.md``:

* planes are ``XY`` ``(u, v, 0)`` with normal ``+Z``, ``XZ`` ``(u, 0, v)`` with
  normal ``-Y``, and ``YZ`` ``(0, u, v)`` with normal ``+X``;
* an extrusion keeps ``reversed``;
* a pocket becomes a cut, with ``None`` meaning through-all;
* a rectangle revolve becomes ``solid.revolve`` around the component-local
  ``Y`` axis, while a circle revolve is lowered to an extrusion whose length is
  the supplied length or, when absent, the diameter;
* several additive features may extend one part;
* every part is created, measured, saved, and STEP-exported;
* an assembly is emitted when there is more than one component or any nonzero
  translation, and every component is placed exactly once.
"""

from __future__ import annotations

import uuid
from collections.abc import Mapping, Sequence
from typing import Final

from pydantic import ValidationError

from kala_agent.contracts.construction import (
    AnalysisScript,
    AssemblyStep,
    ComponentScript,
    ComponentStep,
    ConstructionScript,
)
from kala_solidedge_bridge.protocol import (
    MAX_OPERATIONS,
    MAX_PARTS,
    AssemblyExportStep,
    AssemblyNew,
    AssemblyPlace,
    AssemblySave,
    BridgeOp,
    BuildRequest,
    PartExportStep,
    PartMeasure,
    PartNew,
    PartSave,
    Plane,
    SketchCircle,
    SketchRectangle,
    SolidCut,
    SolidExtrude,
    SolidRevolve,
)

from .errors import CompileError

ASSEMBLY_NAME: Final[str] = "Assembly"
DEFAULT_TIMEOUT_SECONDS: Final[float] = 120.0
MAX_SUMMARY_CHARS: Final[int] = 500
_ZERO_MM: Final[float] = 1e-9

_PLANE_VALUES: Final[dict[str, Plane]] = {
    "XY": Plane.XY,
    "XZ": Plane.XZ,
    "YZ": Plane.YZ,
}


def new_request_id() -> str:
    """Return a fresh safe request id for one semantic submission."""
    return uuid.uuid4().hex


def compile_operations(
    script: ConstructionScript,
    *,
    analysis: AnalysisScript | None = None,
    fillets: Mapping[str, float] | None = None,
) -> tuple[BridgeOp, ...]:
    """Compile a validated script, refusing anything outside the subset.

    ``analysis`` and ``fillets`` are accepted only so a caller can prove they
    are refused rather than silently dropped. Any non-empty value raises
    :class:`CompileError` before a request exists.
    """
    if analysis is not None:
        raise CompileError(
            "analysis operations (material, fix, force, pressure, mesh, solve) are not "
            "supported by the Solid Edge integration"
        )
    if fillets:
        names = ", ".join(sorted(fillets))
        raise CompileError(f"fillets are not supported by the Solid Edge integration: {names}")

    operations: list[BridgeOp] = []
    placements = _placements(script.assembly)
    for component in script.components:
        operations.extend(_component(component))

    if _needs_assembly(script, placements):
        operations.append(AssemblyNew(name=_assembly_name(script)))
        for component in script.components:
            place = placements.get(component.name)
            operations.append(
                AssemblyPlace(
                    component=component.name,
                    x=0.0 if place is None else place.x,
                    y=0.0 if place is None else place.y,
                    z=0.0 if place is None else place.z,
                )
            )
        operations.append(AssemblySave())
        operations.append(AssemblyExportStep())

    if not operations:
        raise CompileError("the construction script did not compile")
    if len(script.components) > MAX_PARTS:
        raise CompileError(
            f"the script has {len(script.components)} components; the wire cap is {MAX_PARTS} parts"
        )
    if len(operations) > MAX_OPERATIONS:
        raise CompileError(
            f"the compiled procedure has {len(operations)} operations; the wire cap is "
            f"{MAX_OPERATIONS}"
        )
    return tuple(operations)


def build_request(
    script: ConstructionScript,
    *,
    request_id: str,
    calibration_id: str | None = None,
    timeout_seconds: float = DEFAULT_TIMEOUT_SECONDS,
    density_kg_m3: float | None = None,
    analysis: AnalysisScript | None = None,
    fillets: Mapping[str, float] | None = None,
) -> BuildRequest:
    """Compile the script and wrap it in one deterministic :class:`BuildRequest`.

    ``request_id`` is transport correlation only and is retained by the caller
    across retries, so an ambiguous submission is never turned into a second
    job. ``calibration_id`` is the one the bridge reported for this host; a
    bridge still refuses an uncalibrated host at admission.
    """
    procedure = compile_operations(script, analysis=analysis, fillets=fillets)
    try:
        return BuildRequest(
            request_id=request_id,
            calibration_id=calibration_id,
            summary=_summary(script.summary),
            procedure=procedure,
            timeout_seconds=timeout_seconds,
            density_kg_m3=density_kg_m3,
        )
    except ValidationError as error:
        raise CompileError(_validation_message(error)) from None


def _summary(text: str) -> str:
    stripped = " ".join(text.split())
    if len(stripped) <= MAX_SUMMARY_CHARS:
        return stripped
    return stripped[:MAX_SUMMARY_CHARS]


def _validation_message(error: ValidationError) -> str:
    first = error.errors()[0]
    location = ".".join(str(part) for part in first.get("loc", ())) or "request"
    message = str(first.get("msg", "invalid request"))
    return f"the build request is invalid at {location}: {message}"


def _placements(assembly: Sequence[AssemblyStep]) -> dict[str, AssemblyStep]:
    """Return one placement per component, refusing duplicates and booleans.

    ``fuse`` and ``compound`` are refused here, not reinterpreted as an
    assembly: an assembly is a set of placed occurrences, and silently turning
    a boolean union into one would change the requested geometry.
    """
    placements: dict[str, AssemblyStep] = {}
    for step in assembly:
        if step.op != "place":
            raise CompileError(
                f"assembly operation {step.op!r} is not supported; version 1 supports only "
                "positioned parts, not fuse or compound"
            )
        if step.component is None:
            raise CompileError("an assembly place operation must name a component")
        if step.component in placements:
            raise CompileError(f"component {step.component!r} is placed more than once")
        placements[step.component] = step
    return placements


def _needs_assembly(script: ConstructionScript, placements: Mapping[str, AssemblyStep]) -> bool:
    """An assembly is needed for several components or any nonzero translation."""
    if len(script.components) > 1:
        return True
    return any(_translated(step) for step in placements.values())


def _assembly_name(script: ConstructionScript) -> str:
    """Return an assembly name that cannot collide with a component name.

    The wire contract keeps parts and the assembly in separate namespaces, so a
    collision is legal there; a native executor may not be so forgiving. The
    derived name keeps the document unambiguous without inventing a random one.
    """
    taken = {component.name for component in script.components}
    if ASSEMBLY_NAME not in taken:
        return ASSEMBLY_NAME
    for index in range(1, 100):
        candidate = f"{ASSEMBLY_NAME}{index}"
        if candidate not in taken:
            return candidate
    raise CompileError("a unique assembly name could not be derived")  # pragma: no cover


def _translated(step: AssemblyStep) -> bool:
    return abs(step.x) > _ZERO_MM or abs(step.y) > _ZERO_MM or abs(step.z) > _ZERO_MM


def _component(component: ComponentScript) -> list[BridgeOp]:
    operations: list[BridgeOp] = [PartNew(name=component.name)]
    pending: ComponentStep | None = None
    for step in component.steps:
        if step.op == "sketch":
            if pending is not None:
                raise CompileError("a sketch must be consumed before another sketch")
            pending = step
            continue
        if step.op == "helix":
            raise CompileError(
                "helix and thread features are not supported by the Solid Edge integration"
            )
        if step.op not in {"extrude", "revolve", "pocket"}:
            raise CompileError(f"unknown operation {step.op!r} in component {component.name!r}")
        if pending is None:
            raise CompileError(f"{step.op} needs a sketch in component {component.name!r}")
        operations.extend(_profile(pending, component.name))
        operations.append(_solid(step, pending, component.name))
        pending = None
    if pending is not None:
        raise CompileError(f"component {component.name!r} ends with an unconsumed sketch")
    operations.append(PartMeasure())
    operations.append(PartSave())
    operations.append(PartExportStep())
    return operations


def _profile(sketch: ComponentStep, component: str) -> list[BridgeOp]:
    plane = _PLANE_VALUES.get(sketch.plane)
    if plane is None:
        raise CompileError(f"component {component!r} uses an unknown plane {sketch.plane!r}")
    if sketch.profile == "rectangle":
        if sketch.width is None or sketch.height is None:
            raise CompileError(f"component {component!r} has a rectangle without width and height")
        return [
            SketchRectangle(
                plane=plane,
                width=sketch.width,
                height=sketch.height,
                x=sketch.x,
                y=sketch.y,
            )
        ]
    if sketch.profile == "circle":
        if sketch.diameter is None:
            raise CompileError(f"component {component!r} has a circle without a diameter")
        return [SketchCircle(plane=plane, diameter=sketch.diameter, x=sketch.x, y=sketch.y)]
    raise CompileError(
        f"component {component!r} has profile {sketch.profile!r}; only rectangle and circle "
        "are supported"
    )


def _solid(step: ComponentStep, sketch: ComponentStep, component: str) -> BridgeOp:
    if step.op == "pocket":
        return SolidCut(length=step.length)
    if step.op == "extrude":
        if step.length is None:
            raise CompileError(f"component {component!r} has an extrusion without a length")
        return SolidExtrude(length=step.length, reversed=step.reversed)
    if step.op == "revolve":
        if sketch.profile == "circle":
            length = step.length if step.length is not None else sketch.diameter
            if length is None:  # pragma: no cover - the sketch validator requires a diameter
                raise CompileError(f"component {component!r} has a circle revolve without a size")
            return SolidExtrude(length=length, reversed=step.reversed)
        if sketch.plane == "XZ":
            raise CompileError(
                f"component {component!r} revolves a rectangle on XZ; the component-local Y "
                "axis is not the profile axis, so this case is refused"
            )
        return SolidRevolve(angle=360.0 if step.angle is None else step.angle, axis="Y")
    raise CompileError(f"unknown operation {step.op!r} in component {component!r}")


__all__ = (
    "ASSEMBLY_NAME",
    "DEFAULT_TIMEOUT_SECONDS",
    "build_request",
    "compile_operations",
    "new_request_id",
)
