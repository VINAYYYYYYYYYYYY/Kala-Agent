"""Compile a Kala ConstructionScript into OpenCascade B-Rep geometry.

This is Studio's v1 construction compiler. It accepts the neutral script
language in :mod:`kala_agent.contracts.construction` and lowers the supported
subset to exact ``TopoDS_Shape`` solids through OCP. The semantics are pinned
by ``docs/construction-semantics.md``, not by intuition:

* ``XY`` maps ``(u, v)`` to ``(u, v, 0)`` with normal ``+Z``;
* ``XZ`` maps ``(u, v)`` to ``(u, 0, v)`` with normal ``-Y``;
* ``YZ`` maps ``(u, v)`` to ``(0, u, v)`` with normal ``+X``;
* an extrusion follows the normal, or the opposite direction when
  ``reversed`` is set;
* a pocket always cuts along the canonical normal, independent of whether the
  preceding extrusion was reversed.

Everything outside the subset is refused before any geometry is built, with a
:class:`~kala_agent.contracts.construction.ConstructionError` that names the
operation. The compiler never writes an artifact and never approximates an
unsupported feature.

The L-bracket shape used by the ADR is assembled from two components because
the bore runs along ``Y`` while the base plate is an ``XY`` pad. The upright is
an ``XZ`` pad with a through pocket; the whole upright is then placed at
``y = T`` and fused with the base. That is the v1-compatible construction.
"""

from __future__ import annotations

import math
import time
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Final

from OCP.Bnd import Bnd_Box  # type: ignore[import-untyped]
from OCP.BRep import BRep_Builder  # type: ignore[import-untyped]
from OCP.BRepAlgoAPI import (  # type: ignore[import-untyped]
    BRepAlgoAPI_Cut,
    BRepAlgoAPI_Fuse,
)
from OCP.BRepBndLib import BRepBndLib  # type: ignore[import-untyped]
from OCP.BRepBuilderAPI import (  # type: ignore[import-untyped]
    BRepBuilderAPI_MakeEdge,
    BRepBuilderAPI_MakeFace,
    BRepBuilderAPI_MakePolygon,
    BRepBuilderAPI_MakeWire,
    BRepBuilderAPI_Transform,
)
from OCP.BRepCheck import BRepCheck_Analyzer  # type: ignore[import-untyped]
from OCP.BRepGProp import BRepGProp  # type: ignore[import-untyped]
from OCP.BRepPrimAPI import (  # type: ignore[import-untyped]
    BRepPrimAPI_MakePrism,
    BRepPrimAPI_MakeRevol,
)
from OCP.gp import (  # type: ignore[import-untyped]
    gp_Ax1,
    gp_Ax2,
    gp_Circ,
    gp_Dir,
    gp_Pnt,
    gp_Trsf,
    gp_Vec,
)
from OCP.GProp import GProp_GProps  # type: ignore[import-untyped]
from OCP.TopAbs import TopAbs_SOLID  # type: ignore[import-untyped]
from OCP.TopExp import TopExp_Explorer  # type: ignore[import-untyped]
from OCP.TopoDS import (  # type: ignore[import-untyped]
    TopoDS_Compound,
    TopoDS_Shape,
)

from kala_agent.contracts.construction import (
    AnalysisScript,
    AssemblyStep,
    ComponentScript,
    ComponentStep,
    ConstructionError,
    ConstructionScript,
    EditStep,
)

DEFAULT_COMPILE_TIMEOUT_SECONDS: Final[float] = 5.0
SUPPORTED_COMPONENT_OPS: Final[frozenset[str]] = frozenset(
    {"sketch", "extrude", "revolve", "pocket"}
)
SUPPORTED_ASSEMBLY_OPS: Final[frozenset[str]] = frozenset({"place", "fuse"})
UNSUPPORTED_PREFIX: Final[str] = "unsupported by the Studio OCP compiler v1: "
THROUGH_ALL_MARGIN_MM: Final[float] = 1.0
_ZERO_MM: Final[float] = 1e-9
_ZERO_VOLUME_MM3: Final[float] = 1e-9

_PLANE_NORMALS: Final[dict[str, tuple[float, float, float]]] = {
    "XY": (0.0, 0.0, 1.0),
    "XZ": (0.0, -1.0, 0.0),
    "YZ": (1.0, 0.0, 0.0),
}
_REVOLVE_AXIS: Final[gp_Ax1] = gp_Ax1(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 1.0, 0.0))


@dataclass(frozen=True, slots=True)
class CompiledComponent:
    """One component's local and placed B-Rep shape."""

    name: str
    local_shape: TopoDS_Shape
    placed_shape: TopoDS_Shape
    placement_mm: tuple[float, float, float]


@dataclass(frozen=True, slots=True)
class CompilationResult:
    """The components and the single placed-or-fused assembly shape."""

    components: tuple[CompiledComponent, ...]
    assembly_shape: TopoDS_Shape
    fused: bool
    warnings: tuple[str, ...] = ()

    @property
    def shape(self) -> TopoDS_Shape:
        """Return the assembly shape (the shape a caller measures or exports)."""
        return self.assembly_shape

    def component(self, name: str) -> CompiledComponent:
        """Return a compiled component by name."""
        for component in self.components:
            if component.name == name:
                return component
        raise KeyError(name)


def compile_script(
    script: ConstructionScript,
    *,
    analysis: AnalysisScript | None = None,
    edits: Sequence[EditStep] = (),
    fillets: Mapping[str, float] | None = None,
    timeout_seconds: float = DEFAULT_COMPILE_TIMEOUT_SECONDS,
) -> CompilationResult:
    """Compile ``script`` into B-Rep solids within a wall-clock budget.

    ``analysis``, ``edits``, and ``fillets`` exist so a caller can prove those
    features are refused rather than silently dropped. Any non-empty value
    raises :class:`ConstructionError` naming the first offending operation.
    """
    if not math.isfinite(timeout_seconds) or timeout_seconds < 0:
        raise ConstructionError("the compile budget must be a non-negative finite number")
    started = time.monotonic()
    _check_budget(started, timeout_seconds)
    _refuse_unsupported(script, analysis=analysis, edits=edits, fillets=fillets)

    placements = _placements(script.assembly)
    compiled: list[CompiledComponent] = []
    warnings: list[str] = []
    try:
        for component in script.components:
            _check_budget(started, timeout_seconds)
            local_shape, component_warnings = _build_component(component, started, timeout_seconds)
            _require_publishable(local_shape, f"component {component.name!r}")
            placement = placements.get(component.name, (0.0, 0.0, 0.0))
            placed_shape = _translated(local_shape, placement)
            if placed_shape is not local_shape:
                _require_publishable(placed_shape, f"placed component {component.name!r}")
            compiled.append(
                CompiledComponent(
                    name=component.name,
                    local_shape=local_shape,
                    placed_shape=placed_shape,
                    placement_mm=placement,
                )
            )
            warnings.extend(component_warnings)

        _check_budget(started, timeout_seconds)
        assembly_shape, fused = _assemble(script, compiled)
        _require_publishable(assembly_shape, "assembly")
    except ConstructionError:
        raise
    except Exception as error:  # pragma: no cover - OCP raises bare exceptions
        raise ConstructionError(
            f"the geometry kernel could not complete the script: {type(error).__name__}"
        ) from None

    _check_budget(started, timeout_seconds)
    return CompilationResult(
        components=tuple(compiled),
        assembly_shape=assembly_shape,
        fused=fused,
        warnings=tuple(warnings),
    )


# --------------------------------------------------------------------- refusals


def _refuse_unsupported(
    script: ConstructionScript,
    *,
    analysis: AnalysisScript | None,
    edits: Sequence[EditStep],
    fillets: Mapping[str, float] | None,
) -> None:
    if analysis is not None and analysis.steps:
        raise ConstructionError(f"{UNSUPPORTED_PREFIX}{analysis.steps[0].op}")
    for edit in edits:
        raise ConstructionError(f"{UNSUPPORTED_PREFIX}{edit.op}")
    if fillets:
        raise ConstructionError(f"{UNSUPPORTED_PREFIX}fillet")
    for component in script.components:
        for step in component.steps:
            if step.op not in SUPPORTED_COMPONENT_OPS:
                raise ConstructionError(f"{UNSUPPORTED_PREFIX}{step.op}")
    for assembly_step in script.assembly:
        if assembly_step.op not in SUPPORTED_ASSEMBLY_OPS:
            raise ConstructionError(f"{UNSUPPORTED_PREFIX}{assembly_step.op}")


def _placements(assembly: Sequence[AssemblyStep]) -> dict[str, tuple[float, float, float]]:
    placements: dict[str, tuple[float, float, float]] = {}
    for step in assembly:
        if step.op != "place":
            continue
        if step.component is None:
            raise ConstructionError("a place operation must name a component")
        name = step.component
        if name in placements:
            raise ConstructionError(f"component {name!r} is placed more than once")
        placements[name] = (step.x, step.y, step.z)
    return placements


# --------------------------------------------------------------------- components


def _build_component(
    component: ComponentScript,
    started: float,
    timeout_seconds: float,
) -> tuple[TopoDS_Shape, tuple[str, ...]]:
    shape: TopoDS_Shape | None = None
    pending: ComponentStep | None = None
    last_solid_plane: str | None = None
    warnings: list[str] = []

    for step in component.steps:
        _check_budget(started, timeout_seconds)
        if step.op == "sketch":
            pending = step
            continue
        if step.op in {"extrude", "revolve"}:
            if pending is None:
                raise ConstructionError(f"{step.op} in component {component.name!r} has no sketch")
            solid = _additive_solid(step, pending, component.name)
            if shape is None:
                shape = solid
            else:
                shape = _fuse(shape, solid, component.name)
            last_solid_plane = pending.plane
            pending = None
            continue
        if step.op == "pocket":
            if pending is None or shape is None:
                raise ConstructionError(
                    f"pocket in component {component.name!r} has no preceding solid and sketch"
                )
            if pending.plane != last_solid_plane:
                raise ConstructionError(
                    f"{UNSUPPORTED_PREFIX}pocket on {pending.plane} over a "
                    f"{last_solid_plane or 'missing'} solid"
                )
            shape, removed = _cut(shape, pending, step, component.name)
            if not removed:
                warnings.append(
                    f"component {component.name!r}: pocket on {pending.plane} removed no material"
                )
            pending = None
            continue
        raise ConstructionError(f"{UNSUPPORTED_PREFIX}{step.op}")

    if shape is None:
        raise ConstructionError(f"component {component.name!r} produced no solid")
    if pending is not None:
        raise ConstructionError(f"component {component.name!r} ends with an unused sketch")
    return shape, tuple(warnings)


def _additive_solid(
    step: ComponentStep,
    sketch: ComponentStep,
    component_name: str,
) -> TopoDS_Shape:
    if sketch.profile == "circle" and step.op == "revolve":
        # The measured FreeCAD lowering: a circle revolve is an extrusion whose
        # length is the supplied length or the circle diameter.
        length = step.length if step.length is not None else sketch.diameter
        if length is None:
            raise ConstructionError(
                f"component {component_name!r} has a circle revolve without a size"
            )
        face = _profile_face(sketch)
        return _prism(face, _plane_normal(sketch.plane), length)

    face = _profile_face(sketch)
    if step.op == "extrude":
        if step.length is None:
            raise ConstructionError(f"component {component_name!r} extrudes without a length")
        direction = _plane_normal(sketch.plane)
        if step.reversed:
            direction = _negated(direction)
        return _prism(face, direction, step.length)

    if sketch.plane == "XZ":
        raise ConstructionError(
            f"{UNSUPPORTED_PREFIX}revolve of a rectangle on XZ "
            "(the component-local Y axis is not the profile axis)"
        )
    angle = 360.0 if step.angle is None else step.angle
    builder = BRepPrimAPI_MakeRevol(face, _REVOLVE_AXIS, math.radians(angle), True)
    if not builder.IsDone():
        raise ConstructionError(
            f"the geometry kernel could not revolve component {component_name!r}"
        )
    shape: TopoDS_Shape = builder.Shape()
    return shape


def _cut(
    shape: TopoDS_Shape,
    sketch: ComponentStep,
    cut: ComponentStep,
    component_name: str,
) -> tuple[TopoDS_Shape, bool]:
    depth = _cut_depth(shape, sketch, cut)
    cutter = _prism(_profile_face(sketch), _plane_normal(sketch.plane), depth)
    before = _volume(shape)
    operation = BRepAlgoAPI_Cut(shape, cutter)
    if not operation.IsDone():
        raise ConstructionError(f"the geometry kernel could not cut component {component_name!r}")
    result: TopoDS_Shape = operation.Shape()
    if result.IsNull():
        raise ConstructionError(
            f"the geometry kernel produced an empty cut in component {component_name!r}"
        )
    after = _volume(result)
    if after <= _ZERO_VOLUME_MM3:
        raise ConstructionError(f"the cut in component {component_name!r} removed the whole solid")
    removed = before - after > max(_ZERO_VOLUME_MM3, before * 1e-12)
    _require_publishable(result, f"cut component {component_name!r}")
    return result, removed


def _cut_depth(shape: TopoDS_Shape, sketch: ComponentStep, cut: ComponentStep) -> float:
    if cut.length is not None:
        return cut.length
    normal = _plane_normal(sketch.plane)
    box = _optimal_box(shape)
    if box is None:
        raise ConstructionError("the pocket cannot measure the current solid")
    minimum, maximum = box
    corners = (
        (minimum[0], minimum[1], minimum[2]),
        (minimum[0], minimum[1], maximum[2]),
        (minimum[0], maximum[1], minimum[2]),
        (minimum[0], maximum[1], maximum[2]),
        (maximum[0], minimum[1], minimum[2]),
        (maximum[0], minimum[1], maximum[2]),
        (maximum[0], maximum[1], minimum[2]),
        (maximum[0], maximum[1], maximum[2]),
    )
    highest = max(x * normal[0] + y * normal[1] + z * normal[2] for x, y, z in corners)
    # The sketch plane passes through the component-local origin. Cutting a
    # little past the body is safe; cutting short leaves a membrane.
    return max(0.0, highest) + THROUGH_ALL_MARGIN_MM


# --------------------------------------------------------------------- assembly


def _assemble(
    script: ConstructionScript,
    compiled: Sequence[CompiledComponent],
) -> tuple[TopoDS_Shape, bool]:
    placed = {component.name: component.placed_shape for component in compiled}
    ordered = [component.name for component in compiled]
    fuse_names: set[str] = set()
    for step in script.assembly:
        if step.op == "fuse":
            fuse_names.update(step.components)
    if not fuse_names:
        shapes = [placed[name] for name in ordered]
        return _compound(shapes), False

    fused_names = [name for name in ordered if name in fuse_names]
    fused_shape = placed[fused_names[0]]
    for name in fused_names[1:]:
        fused_shape = _fuse(fused_shape, placed[name], "assembly")
    remaining = [placed[name] for name in ordered if name not in fuse_names]
    if remaining:
        return _compound([fused_shape, *remaining]), True
    return fused_shape, True


def _fuse(left: TopoDS_Shape, right: TopoDS_Shape, label: str) -> TopoDS_Shape:
    operation = BRepAlgoAPI_Fuse(left, right)
    if not operation.IsDone():
        raise ConstructionError(f"the geometry kernel could not fuse {label}")
    result: TopoDS_Shape = operation.Shape()
    if result.IsNull():
        raise ConstructionError(f"the geometry kernel produced an empty fuse for {label}")
    return result


def _compound(shapes: Sequence[TopoDS_Shape]) -> TopoDS_Shape:
    if len(shapes) == 1:
        return shapes[0]
    compound = TopoDS_Compound()
    builder = BRep_Builder()
    builder.MakeCompound(compound)
    for shape in shapes:
        builder.Add(compound, shape)
    result: TopoDS_Shape = compound
    return result


# --------------------------------------------------------------------- geometry


def _profile_face(sketch: ComponentStep) -> TopoDS_Shape:
    if sketch.profile == "rectangle":
        if sketch.width is None or sketch.height is None:
            raise ConstructionError("a rectangle sketch requires width and height")
        u0, v0 = sketch.x, sketch.y
        u1, v1 = u0 + sketch.width, v0 + sketch.height
        points = (
            _to_world(sketch.plane, u0, v0),
            _to_world(sketch.plane, u1, v0),
            _to_world(sketch.plane, u1, v1),
            _to_world(sketch.plane, u0, v1),
        )
        polygon = BRepBuilderAPI_MakePolygon()
        for point in points:
            polygon.Add(gp_Pnt(point[0], point[1], point[2]))
        polygon.Close()
        if not polygon.IsDone():
            raise ConstructionError("the geometry kernel could not close the rectangle")
        face_builder = BRepBuilderAPI_MakeFace(polygon.Wire())
        if not face_builder.IsDone():
            raise ConstructionError("the geometry kernel could not build the rectangle face")
        face: TopoDS_Shape = face_builder.Face()
        return face

    if sketch.profile == "circle":
        if sketch.diameter is None:
            raise ConstructionError("a circle sketch requires a diameter")
        centre = _to_world(sketch.plane, sketch.x, sketch.y)
        axis = gp_Ax2(
            gp_Pnt(centre[0], centre[1], centre[2]),
            gp_Dir(*_plane_normal(sketch.plane)),
        )
        circle = gp_Circ(axis, sketch.diameter / 2.0)
        edge_builder = BRepBuilderAPI_MakeEdge(circle)
        if not edge_builder.IsDone():
            raise ConstructionError("the geometry kernel could not build the circle edge")
        wire_builder = BRepBuilderAPI_MakeWire(edge_builder.Edge())
        if not wire_builder.IsDone():
            raise ConstructionError("the geometry kernel could not build the circle wire")
        face_builder = BRepBuilderAPI_MakeFace(wire_builder.Wire())
        if not face_builder.IsDone():
            raise ConstructionError("the geometry kernel could not build the circle face")
        face = face_builder.Face()
        return face

    raise ConstructionError(f"{UNSUPPORTED_PREFIX}sketch profile {sketch.profile!r}")


def _prism(
    face: TopoDS_Shape,
    direction: tuple[float, float, float],
    length: float,
) -> TopoDS_Shape:
    vector = gp_Vec(
        direction[0] * length,
        direction[1] * length,
        direction[2] * length,
    )
    builder = BRepPrimAPI_MakePrism(face, vector)
    if not builder.IsDone():
        raise ConstructionError("the geometry kernel could not extrude the profile")
    shape: TopoDS_Shape = builder.Shape()
    if shape.IsNull():
        raise ConstructionError("the geometry kernel returned an empty extrusion")
    return shape


def _translated(
    shape: TopoDS_Shape,
    translation: tuple[float, float, float],
) -> TopoDS_Shape:
    if all(abs(value) <= _ZERO_MM for value in translation):
        return shape
    transform = gp_Trsf()
    transform.SetTranslation(gp_Vec(translation[0], translation[1], translation[2]))
    builder = BRepBuilderAPI_Transform(shape, transform, True)
    if not builder.IsDone():
        raise ConstructionError("the geometry kernel could not place a component")
    result: TopoDS_Shape = builder.Shape()
    return result


def _to_world(plane: str, u: float, v: float) -> tuple[float, float, float]:
    if plane == "XY":
        return (u, v, 0.0)
    if plane == "XZ":
        return (u, 0.0, v)
    return (0.0, u, v)


def _plane_normal(plane: str) -> tuple[float, float, float]:
    try:
        return _PLANE_NORMALS[plane]
    except KeyError:
        raise ConstructionError(f"unknown sketch plane {plane!r}") from None


def _negated(vector: tuple[float, float, float]) -> tuple[float, float, float]:
    return (-vector[0], -vector[1], -vector[2])


# --------------------------------------------------------------------- checks


def _optimal_box(
    shape: TopoDS_Shape,
) -> tuple[tuple[float, float, float], tuple[float, float, float]] | None:
    box = Bnd_Box()
    box.SetGap(0.0)
    BRepBndLib.AddOptimal_s(shape, box, True, False)
    if box.IsVoid():
        return None
    minimum = box.CornerMin()
    maximum = box.CornerMax()
    return (
        (minimum.X(), minimum.Y(), minimum.Z()),
        (maximum.X(), maximum.Y(), maximum.Z()),
    )


def _volume(shape: TopoDS_Shape) -> float:
    properties = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, properties)
    return float(properties.Mass())


def _solid_count(shape: TopoDS_Shape) -> int:
    explorer = TopExp_Explorer(shape, TopAbs_SOLID)
    count = 0
    while explorer.More():
        count += 1
        explorer.Next()
    return count


def _require_publishable(shape: TopoDS_Shape, label: str) -> None:
    if shape.IsNull():
        raise ConstructionError(f"{label} is a null shape")
    if _solid_count(shape) < 1:
        raise ConstructionError(f"{label} contains no solid")
    if _volume(shape) <= _ZERO_VOLUME_MM3:
        raise ConstructionError(f"{label} has zero volume")
    if not BRepCheck_Analyzer(shape).IsValid():
        raise ConstructionError(f"{label} failed the B-Rep validity check")


def _check_budget(started: float, timeout_seconds: float) -> None:
    if time.monotonic() - started > timeout_seconds:
        raise ConstructionError(
            f"the construction script exceeded the {timeout_seconds:g} s compile budget"
        )


__all__ = (
    "DEFAULT_COMPILE_TIMEOUT_SECONDS",
    "SUPPORTED_ASSEMBLY_OPS",
    "SUPPORTED_COMPONENT_OPS",
    "UNSUPPORTED_PREFIX",
    "CompilationResult",
    "CompiledComponent",
    "compile_script",
)
