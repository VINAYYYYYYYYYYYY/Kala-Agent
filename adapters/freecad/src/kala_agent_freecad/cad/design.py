"""Apply one ordered list of CAD tool calls inside a single FreeCAD document.

The worker holds the document for the whole list. Each call adds the object
the matching toolbar command would add. After a valid solid, the FEM calls in
the list run as the analysis pass. A redesign changes the solid only when
that pass produced real stress numbers.
"""

from __future__ import annotations

import json
import math
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Final, cast

from kala_agent.contracts import ArtifactRef
from kala_agent.tools.protocol import JSONValue
from kala_agent_freecad.cad.arguments import text, unique_members
from kala_agent_freecad.cad.artifacts import (
    PublishedArtifact,
    bounded_label,
    discard,
    finalize,
    prepare_target,
    resolve_target,
)
from kala_agent_freecad.cad.fem import result_summary
from kala_agent_freecad.cad.geometry import _measure
from kala_agent_freecad.cad.procedure import DesignCall, check_procedure, parse_calls
from kala_agent_freecad.cad.runtime import (
    FreeCADModules,
    FreeCADObject,
    freecad_session,
    load_sketcher,
)
from kala_agent_freecad.contracts import (
    ARTIFACT_DIRECTORY,
    DEFAULT_ARTIFACT_STEM,
    DEFAULT_DOCUMENT_NAME,
    EXPORT_SUFFIXES,
    GEOMETRY_EXPORT_FORMATS,
)
from kala_agent_freecad.errors import (
    FreeCADAdapterError,
    FreeCADInputError,
    FreeCADOperationError,
)

MAX_DIMENSION_MM: Final[float] = 5_000.0
MAX_COORDINATE_MM: Final[float] = 5_000.0
MAX_TRACE: Final[int] = 160
MAX_WARNINGS: Final[int] = 16
MAX_NAME_LENGTH: Final[int] = 64
MAX_VERSION_LENGTH: Final[int] = 64
MAX_PATH_LENGTH: Final[int] = 1_024
MAX_DETAIL_LENGTH: Final[int] = 512
STIFFEN_STRESS_MPA: Final[float] = 250.0
RELIEVE_STRESS_MPA: Final[float] = 50.0
_PLANES: Final[Mapping[str, str]] = {"XY": "XY_Plane", "XZ": "XZ_Plane", "YZ": "YZ_Plane"}


@dataclass(frozen=True, slots=True)
class DesignRequest:
    calls: tuple[DesignCall, ...]
    export_formats: tuple[str, ...]
    document_name: str
    artifact_stem: str


@dataclass(frozen=True, slots=True)
class DesignOutcome:
    result: dict[str, JSONValue]
    artifacts: tuple[ArtifactRef, ...]


def parse_request(arguments: Mapping[str, object]) -> DesignRequest:
    """Turn validated arguments into a design request."""
    if not isinstance(arguments, Mapping):
        raise FreeCADInputError("arguments must be a JSON object")
    raw = arguments.get("calls_json")
    if not isinstance(raw, str):
        raise FreeCADInputError("calls_json must be a string")
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError as error:
        raise FreeCADInputError("calls_json must be a JSON list of tool calls") from error
    if "export_formats" in arguments:
        export_formats = unique_members(arguments, "export_formats", GEOMETRY_EXPORT_FORMATS)
    else:
        export_formats = ("step", "fcstd")
    return DesignRequest(
        calls=parse_calls(parsed),
        export_formats=export_formats,
        document_name=text(arguments, "document_name", DEFAULT_DOCUMENT_NAME),
        artifact_stem=text(arguments, "artifact_stem", DEFAULT_ARTIFACT_STEM),
    )


def run_open(
    request: DesignRequest,
    modules: FreeCADModules,
    *,
    on_step: Callable[[str, str], None] | None = None,
    calculix_path: str = "",
    automatic_redesign: bool = True,
) -> dict[str, object]:
    """Apply the call list in an already open FreeCAD document and leave it open."""
    check_procedure(request.calls)
    document = _new_document(modules.freecad, request.document_name)
    activate = getattr(modules.freecad, "setActiveDocument", None)
    if callable(activate):
        activate(document.Name)
    session = _Session(modules, load_sketcher(), document, calculix_path)
    session.apply_all(request.calls, on_step=on_step)
    if automatic_redesign:
        session.improve()
    return {
        "analysis_detail": session.analysis_detail,
        "analysis_status": session.analysis_status,
        "peak_stress_mpa": session.peak_stress,
    }


def run(
    request: DesignRequest,
    *,
    workspace: Path,
    import_roots: Sequence[Path],
    max_artifact_bytes: int,
    call_id: str,
    calculix_path: str = "",
) -> DesignOutcome:
    """Apply the call list, measure the solid, and publish the exports."""
    check_procedure(request.calls)
    with freecad_session(import_roots) as modules:
        document = _new_document(modules.freecad, request.document_name)
        published: tuple[PublishedArtifact, ...] = ()
        try:
            session = _Session(modules, load_sketcher(), document, calculix_path)
            session.apply_all(request.calls)
            session.improve()
            shape = _solid_shape(session.feature)
            published = _export_all(
                modules,
                document,
                session.feature,
                request,
                workspace=workspace,
                max_artifact_bytes=max_artifact_bytes,
                call_id=call_id,
            )
            names = cast("list[JSONValue]", session.names[:32])
            result: dict[str, JSONValue] = {
                "analysis_detail": session.analysis_detail[:MAX_DETAIL_LENGTH],
                "analysis_status": session.analysis_status,
                "artifacts": [item.entry() for item in published],
                "degrees_of_freedom": session.degrees_of_freedom,
                "document_name": bounded_label(request.document_name, MAX_NAME_LENGTH),
                "freecad_version": bounded_label(modules.version, MAX_VERSION_LENGTH),
                "import_root": bounded_label(modules.import_root, MAX_PATH_LENGTH),
                "measurement": _measure(shape),
                "object_count": max(len(names), 1),
                "object_names": names,
                "redesign_trace": cast("list[JSONValue]", session.redesign[:MAX_TRACE]),
                "stage": "solid",
                "tool_trace": cast("list[JSONValue]", session.trace[:MAX_TRACE]),
                "valid": bool(getattr(shape, "isValid", False)),
                "warnings": cast("list[JSONValue]", session.warnings[:MAX_WARNINGS]),
            }
        finally:
            _close(modules.freecad, document)
    return DesignOutcome(result=result, artifacts=tuple(item.artifact for item in published))


class _Session:
    """The open document the call list builds."""

    def __init__(
        self,
        modules: FreeCADModules,
        sketcher: FreeCADObject,
        document: FreeCADObject,
        calculix_path: str,
    ) -> None:
        self.modules = modules
        self.sketcher = sketcher
        self.document = document
        self.calculix_path = calculix_path
        self.body: FreeCADObject | None = None
        self.sketch: FreeCADObject | None = None
        self.profile: FreeCADObject | None = None
        self.feature: FreeCADObject | None = None
        self.pad_feature: FreeCADObject | None = None
        self.analysis: FreeCADObject | None = None
        self.solver: FreeCADObject | None = None
        self.trace: list[str] = []
        self.redesign: list[str] = []
        self.names: list[str] = []
        self.warnings: list[str] = []
        self.degrees_of_freedom = 0
        self.analysis_status = "not_run"
        self.analysis_detail = "analysis was not requested"
        self.peak_stress: float | None = None
        self._analysis_failed = False
        self._bodies: dict[str, FreeCADObject] = {}
        self._features: dict[str, FreeCADObject] = {}
        self._component = ""

    def apply_all(
        self,
        calls: Sequence[DesignCall],
        on_step: Callable[[str, str], None] | None = None,
    ) -> None:
        for call in calls:
            if call.tool.startswith("fem.") and self._analysis_failed:
                continue
            try:
                self._apply(call)
            except FreeCADAdapterError:
                if not call.tool.startswith("fem."):
                    raise
                self._note_analysis_failure(call.tool)
            except Exception as error:
                if not call.tool.startswith("fem."):
                    detail = " ".join(str(error).split())
                    raise FreeCADOperationError(
                        f"{call.tool} failed: {type(error).__name__}: {detail}"[:MAX_DETAIL_LENGTH]
                    ) from error
                self._note_analysis_failure(call.tool)
            else:
                self.trace.append(call.tool)
                if on_step is not None:
                    sketch_name = str(getattr(self.sketch, "Name", "") or "")
                    on_step(call.tool, sketch_name)

    def improve(self) -> None:
        """Change the pad only when a completed solve reported a real stress."""
        if (
            self.analysis_status != "completed"
            or self.peak_stress is None
            or self.pad_feature is None
        ):
            return
        current = _as_float(getattr(self.pad_feature, "Length", None))
        if current is None:
            return
        if self.peak_stress > STIFFEN_STRESS_MPA:
            updated = min(current * 1.2, MAX_DIMENSION_MM)
            reason = "partdesign.pad"
        elif self.peak_stress < RELIEVE_STRESS_MPA and current > 2.0:
            updated = max(current * 0.9, 1.0)
            reason = "partdesign.pad"
        else:
            return
        self.pad_feature.Length = updated
        _recompute(self.document)
        self.redesign.append(reason)
        if self.solver is not None and self.analysis is not None:
            self._attempt_solve()
            self._read_result()

    def _apply(self, call: DesignCall) -> None:
        handler = _HANDLERS.get(call.tool)
        if handler is None:
            raise FreeCADOperationError(f"no FreeCAD binding for {call.tool}")
        handler(self, call.arguments)

    def _note_analysis_failure(self, tool: str) -> None:
        self._analysis_failed = True
        self.analysis_status = "failed"
        self.analysis_detail = f"{tool} could not be applied to the solid"
        self._warn(self.analysis_detail)
        self.trace.append(tool)

    def _warn(self, message: str) -> None:
        if len(self.warnings) < MAX_WARNINGS:
            self.warnings.append(message[:MAX_DETAIL_LENGTH])

    def _remember(self, feature: FreeCADObject) -> None:
        name = getattr(feature, "Name", None)
        if isinstance(name, str) and name and len(self.names) < 32:
            self.names.append(name[:MAX_NAME_LENGTH])

    def _vector(self, x_mm: float, y_mm: float) -> FreeCADObject:
        return self.modules.freecad.Vector(x_mm, y_mm, 0)

    def _constraint(self, *parts: object) -> None:
        if self.sketch is None:
            raise FreeCADInputError("a constraint needs an open sketch")
        self.sketch.addConstraint(self.sketcher.Constraint(*parts))

    def _require_sketch(self) -> FreeCADObject:
        if self.sketch is None:
            raise FreeCADInputError("sketch geometry needs an open sketch")
        return self.sketch

    def _require_body(self) -> FreeCADObject:
        if self.body is None:
            raise FreeCADInputError("this feature needs a PartDesign body")
        return self.body

    def body_new(self, arguments: Mapping[str, object]) -> None:
        label = arguments.get("name")
        body = self.document.addObject("PartDesign::Body", "Body")
        if isinstance(label, str) and label.strip():
            self._component = label.strip()
            body.Label = self._component
            self._bodies[self._component] = body
        _recompute(self.document)
        self.body = body
        self._remember(body)

    def sketch_new(self, arguments: Mapping[str, object]) -> None:
        plane_name = arguments.get("plane", "XY")
        if not isinstance(plane_name, str) or plane_name not in _PLANES:
            raise FreeCADInputError("plane must be XY, XZ, or YZ")
        body = self._require_body()
        plane = self._origin_feature(_PLANES[plane_name])
        sketch = body.newObject("Sketcher::SketchObject", "Sketch")
        sketch.AttachmentSupport = [(plane, "")]
        sketch.MapMode = "FlatFace"
        _recompute(self.document)
        self.sketch = sketch
        self._remember(sketch)

    def sketch_rectangle(self, arguments: Mapping[str, object]) -> None:
        x_mm = _coordinate(arguments, "x", default=0.0)
        y_mm = _coordinate(arguments, "y", default=0.0)
        width = _length(arguments, "width")
        height = _length(arguments, "height")
        points = (
            (x_mm, y_mm),
            (x_mm + width, y_mm),
            (x_mm + width, y_mm + height),
            (x_mm, y_mm + height),
        )
        self._polyline(points, closed=True)

    def sketch_line(self, arguments: Mapping[str, object]) -> None:
        self._polyline(
            (
                (_coordinate(arguments, "x1"), _coordinate(arguments, "y1")),
                (_coordinate(arguments, "x2"), _coordinate(arguments, "y2")),
            ),
            closed=False,
        )

    def sketch_polyline(self, arguments: Mapping[str, object]) -> None:
        self._polyline(_points(arguments), closed=bool(arguments.get("closed", False)))

    def sketch_circle(self, arguments: Mapping[str, object]) -> None:
        sketch = self._require_sketch()
        radius = _length(arguments, "radius")
        center = self._vector(
            _coordinate(arguments, "x", default=0.0),
            _coordinate(arguments, "y", default=0.0),
        )
        normal = self.modules.freecad.Vector(0, 0, 1)
        sketch.addGeometry(self.modules.part.Circle(center, normal, radius), False)

    def sketch_point(self, arguments: Mapping[str, object]) -> None:
        sketch = self._require_sketch()
        point = self.modules.part.Point(
            self._vector(_coordinate(arguments, "x"), _coordinate(arguments, "y"))
        )
        sketch.addGeometry(point, False)

    def _polyline(self, points: Sequence[tuple[float, float]], *, closed: bool) -> None:
        sketch = self._require_sketch()
        count = len(points)
        last = count if closed else count - 1
        if last < 1:
            raise FreeCADInputError("a profile needs at least one segment")
        for index in range(last):
            start = points[index]
            end = points[(index + 1) % count]
            sketch.addGeometry(
                self.modules.part.LineSegment(self._vector(*start), self._vector(*end)),
                False,
            )

    def sketch_coincident(self, arguments: Mapping[str, object]) -> None:
        self._constraint(
            "Coincident",
            _index(arguments, "geo"),
            _index(arguments, "pos"),
            _index(arguments, "geo2"),
            _index(arguments, "pos2"),
        )

    def sketch_horizontal(self, arguments: Mapping[str, object]) -> None:
        self._constraint("Horizontal", _index(arguments, "geo"))

    def sketch_vertical(self, arguments: Mapping[str, object]) -> None:
        self._constraint("Vertical", _index(arguments, "geo"))

    def sketch_parallel(self, arguments: Mapping[str, object]) -> None:
        self._constraint("Parallel", _index(arguments, "geo"), _index(arguments, "geo2"))

    def sketch_perpendicular(self, arguments: Mapping[str, object]) -> None:
        self._constraint("Perpendicular", _index(arguments, "geo"), _index(arguments, "geo2"))

    def sketch_equal(self, arguments: Mapping[str, object]) -> None:
        self._constraint("Equal", _index(arguments, "geo"), _index(arguments, "geo2"))

    def sketch_tangent(self, arguments: Mapping[str, object]) -> None:
        self._constraint("Tangent", _index(arguments, "geo"), _index(arguments, "geo2"))

    def sketch_distance(self, arguments: Mapping[str, object]) -> None:
        self._constraint("Distance", _index(arguments, "geo"), _length(arguments, "length"))

    def sketch_distance_x(self, arguments: Mapping[str, object]) -> None:
        self._dimensional("DistanceX", arguments)

    def sketch_distance_y(self, arguments: Mapping[str, object]) -> None:
        self._dimensional("DistanceY", arguments)

    def sketch_radius(self, arguments: Mapping[str, object]) -> None:
        self._constraint("Radius", _index(arguments, "geo"), _length(arguments, "length"))

    def sketch_diameter(self, arguments: Mapping[str, object]) -> None:
        self._constraint("Diameter", _index(arguments, "geo"), _length(arguments, "length"))

    def _dimensional(self, kind: str, arguments: Mapping[str, object]) -> None:
        geo = _index(arguments, "geo")
        # A point coordinate is signed. A span between two points stays positive.
        minimum = 0.0 if "geo2" in arguments else -MAX_DIMENSION_MM
        length = _length(arguments, "length", minimum=minimum, maximum=MAX_DIMENSION_MM)
        if "geo2" in arguments:
            self._constraint(
                kind,
                geo,
                _index(arguments, "pos"),
                _index(arguments, "geo2"),
                _index(arguments, "pos2"),
                length,
            )
            return
        if "pos" in arguments:
            self._constraint(kind, geo, _index(arguments, "pos"), length)
            return
        self._constraint(kind, geo, length)

    def sketch_validate(self, arguments: Mapping[str, object]) -> None:
        del arguments
        self._solve(require_closed=True, require_defined=False)

    def sketch_leave(self, arguments: Mapping[str, object]) -> None:
        del arguments
        self._solve(require_closed=True, require_defined=True)
        self.profile = self.sketch
        self.sketch = None

    def _solve(self, *, require_closed: bool, require_defined: bool) -> None:
        sketch = self._require_sketch()
        code = int(sketch.solve())
        freedom = int(sketch.DoF)
        self.degrees_of_freedom = max(freedom, 0)
        if code != 0:
            raise FreeCADInputError("the sketch constraints are conflicting or redundant")
        if require_defined and freedom != 0:
            raise FreeCADInputError(f"the sketch still has {freedom} degrees of freedom")
        _recompute(self.document)
        if require_closed and not _closed(getattr(sketch, "Shape", None)):
            raise FreeCADInputError("the sketch profile is not a closed wire")

    def place(self, arguments: Mapping[str, object]) -> None:
        body = self._named(self._bodies, arguments) or self._require_body()
        placement = body.Placement
        placement.Base = self.modules.freecad.Vector(
            _coordinate(arguments, "x", default=0.0),
            _coordinate(arguments, "y", default=0.0),
            _coordinate(arguments, "z", default=0.0),
        )
        body.Placement = placement

    def helix(self, arguments: Mapping[str, object]) -> None:
        feature = self._require_body().newObject("PartDesign::AdditiveHelix", "Helix")
        feature.Profile = self._profile()
        feature.ReferenceAxis = (self._origin_axis("Z_Axis"), [""])
        feature.Mode = "pitch-height-angle"
        feature.Pitch = _length(arguments, "pitch")
        feature.Height = _length(arguments, "height")
        _recompute(self.document)
        self.feature = feature
        self._keep_feature(feature)
        self._remember(feature)

    def add_pad(self, arguments: Mapping[str, object]) -> None:
        pad = self._require_body().newObject("PartDesign::Pad", "Pad")
        pad.Profile = self._profile()
        pad.Length = _length(arguments, "length")
        if arguments.get("reversed") is True:
            pad.Reversed = True
        _recompute(self.document)
        self.pad_feature = pad
        self.feature = pad
        self._keep_feature(pad)
        self._remember(pad)

    def pocket(self, arguments: Mapping[str, object]) -> None:
        pocket = self._require_body().newObject("PartDesign::Pocket", "Pocket")
        pocket.Profile = self._profile()
        if "length" in arguments:
            pocket.Type = "Length"
            pocket.Length = _length(arguments, "length")
        else:
            pocket.Type = "ThroughAll"
        pocket.Reversed = True
        _recompute(self.document)
        self.feature = pocket
        self._keep_feature(pocket)
        self._remember(pocket)

    def revolution(self, arguments: Mapping[str, object]) -> None:
        feature = self._require_body().newObject("PartDesign::Revolution", "Revolution")
        feature.Profile = self._profile()
        feature.ReferenceAxis = (self._origin_axis("Y_Axis"), [""])
        feature.Angle = _length(arguments, "angle", default=360.0, maximum=360.0)
        _recompute(self.document)
        self.feature = feature
        self._keep_feature(feature)
        self._remember(feature)

    def fillet(self, arguments: Mapping[str, object]) -> None:
        named = self._named(self._features, arguments)
        if named is not None:
            self.feature = named
            body = self._named(self._bodies, arguments)
            if body is not None:
                self.body = body
        feature = self._require_feature()
        shape = getattr(feature, "Shape", None)
        edges = getattr(shape, "Edges", ())
        chosen = [f"Edge{index}" for index in range(1, min(len(edges), 4) + 1)]
        if not chosen:
            raise FreeCADOperationError("the solid has no edges to fillet")
        fillet = self._require_body().newObject("PartDesign::Fillet", "Fillet")
        fillet.Base = (feature, chosen)
        fillet.Radius = _length(arguments, "radius")
        _recompute(self.document)
        self.feature = fillet
        self._remember(fillet)

    def check(self, arguments: Mapping[str, object]) -> None:
        del arguments
        shape = _solid_shape(self.feature)
        if not bool(getattr(shape, "isValid", False)):
            raise FreeCADOperationError("the feature is not a valid solid")

    def part_fuse(self, arguments: Mapping[str, object]) -> None:
        shapes = self._member_shapes(arguments)
        fused = shapes[0]
        for other in shapes[1:]:
            fuse = getattr(fused, "fuse", None)
            if not callable(fuse):
                raise FreeCADOperationError("the shape cannot be fused")
            fused = fuse(other)
        self._primitive("Assembly", fused)

    def part_compound(self, arguments: Mapping[str, object]) -> None:
        shapes = self._member_shapes(arguments)
        make = getattr(self.modules.part, "makeCompound", None)
        if not callable(make):
            raise FreeCADOperationError("the Part module cannot make a compound")
        self._primitive("Compound", make(shapes))

    def fem_pressure(self, arguments: Mapping[str, object]) -> None:
        maker = getattr(self.modules.objects_fem, "makeConstraintPressure", None)
        if not callable(maker):
            raise FreeCADOperationError("no FreeCAD binding for fem.pressure")
        constraint = maker(self.document, "Pressure")
        face = _index(arguments, "face", default=1)
        constraint.References = [(self._require_feature(), f"Face{face}")]
        constraint.Pressure = _length(
            arguments,
            "pressure_mpa",
            default=1.0,
            maximum=1_000.0,
        )
        self._adopt(constraint)

    def part_box(self, arguments: Mapping[str, object]) -> None:
        self._primitive(
            "Box",
            self.modules.part.makeBox(
                _length(arguments, "length"),
                _length(arguments, "width"),
                _length(arguments, "height"),
            ),
        )

    def part_cylinder(self, arguments: Mapping[str, object]) -> None:
        self._primitive(
            "Cylinder",
            self.modules.part.makeCylinder(
                _length(arguments, "radius"),
                _length(arguments, "height"),
            ),
        )

    def _primitive(self, name: str, shape: FreeCADObject) -> None:
        feature = self.document.addObject("Part::Feature", name)
        feature.Shape = shape
        _recompute(self.document)
        self.feature = feature
        self._remember(feature)

    def _profile(self) -> FreeCADObject:
        if self.profile is None:
            raise FreeCADInputError("a feature needs a sketch profile")
        return self.profile

    def _named(
        self,
        found: Mapping[str, FreeCADObject],
        arguments: Mapping[str, object],
    ) -> FreeCADObject | None:
        name = arguments.get("name")
        if isinstance(name, str) and name in found:
            return found[name]
        return None

    def _keep_feature(self, feature: FreeCADObject) -> None:
        if self._component:
            self._features[self._component] = feature

    def _member_shapes(self, arguments: Mapping[str, object]) -> list[FreeCADObject]:
        names = arguments.get("components")
        if isinstance(names, (str, bytes)) or not isinstance(names, Sequence) or len(names) < 2:
            raise FreeCADInputError("fuse needs at least two components")
        shapes: list[FreeCADObject] = []
        for name in names:
            if not isinstance(name, str) or name not in self._bodies:
                raise FreeCADInputError(f"unknown component {name}")
            shape = getattr(self._bodies[name], "Shape", None)
            if shape is None:
                raise FreeCADOperationError(f"{name} has no shape")
            shapes.append(shape)
        return shapes

    def _require_feature(self) -> FreeCADObject:
        if self.feature is None:
            raise FreeCADInputError("this command needs a solid")
        return self.feature

    def _origin_axis(self, name: str) -> FreeCADObject:
        return self._origin_feature(name)

    def _origin_feature(self, role: str) -> FreeCADObject:
        """Return the origin plane or axis for this body.

        The first body names them XY_Plane and Z_Axis. Every later body keeps
        that role and renames the object XY_Plane001, Z_Axis001, and so on.
        """
        origin = getattr(self._require_body(), "Origin", None)
        features = getattr(origin, "OriginFeatures", ())
        if isinstance(features, (str, bytes)) or not isinstance(features, Sequence):
            features = ()
        by_name: FreeCADObject | None = None
        by_suffix: FreeCADObject | None = None
        for item in features:
            if getattr(item, "Role", "") == role:
                return item
            name = getattr(item, "Name", "")
            if name == role:
                by_name = item
            elif isinstance(name, str) and _numbered(name, role):
                by_suffix = item
        chosen = by_name if by_name is not None else by_suffix
        if chosen is None:
            raise FreeCADOperationError(f"the body origin has no {role}")
        return chosen

    def fem_analysis(self, arguments: Mapping[str, object]) -> None:
        del arguments
        self._require_feature()
        analysis = self.modules.objects_fem.makeAnalysis(self.document, "Analysis")
        self.analysis = analysis
        self.analysis_status = "not_run"
        self.analysis_detail = "the analysis is open and has not been solved"
        self._remember(analysis)

    def fem_material(self, arguments: Mapping[str, object]) -> None:
        material = self.modules.objects_fem.makeMaterialSolid(self.document, "Material")
        name = arguments.get("name", "StructuralSteel")
        material.Material = {
            "Density": "7850 kg/m^3",
            "Name": name if isinstance(name, str) else "StructuralSteel",
            "PoissonRatio": "0.3",
            "YoungsModulus": "210000 MPa",
        }
        self._adopt(material)

    def fem_fixed(self, arguments: Mapping[str, object]) -> None:
        constraint = self.modules.objects_fem.makeConstraintFixed(self.document, "Fixed")
        face = _index(arguments, "face", default=1)
        constraint.References = [(self._require_feature(), f"Face{face}")]
        self._adopt(constraint)

    def fem_force(self, arguments: Mapping[str, object]) -> None:
        constraint = self.modules.objects_fem.makeConstraintForce(self.document, "Force")
        face = _index(arguments, "face", default=2)
        constraint.References = [(self._require_feature(), f"Face{face}")]
        constraint.Force = _length(arguments, "force_n", default=100.0, maximum=1_000_000.0)
        self._adopt(constraint)

    def fem_mesh(self, arguments: Mapping[str, object]) -> None:
        del arguments
        mesh = self.modules.objects_fem.makeMeshGmsh(self.document, "Mesh")
        mesh.Shape = self._require_feature()
        self._adopt(mesh)

    def fem_solve(self, arguments: Mapping[str, object]) -> None:
        del arguments
        solver = self.modules.objects_fem.makeSolverCalculiXCcxTools(self.document, "Solver")
        self._adopt(solver)
        self.solver = solver
        _recompute(self.document)
        self._attempt_solve()

    def fem_result(self, arguments: Mapping[str, object]) -> None:
        del arguments
        self._read_result()

    def _adopt(self, member: FreeCADObject) -> None:
        if self.analysis is None:
            raise FreeCADInputError("an analysis container is required first")
        self.analysis.addObject(member)
        self._remember(member)

    def _attempt_solve(self) -> None:
        if not self.calculix_path or self.modules.ccx_tools is None or self.solver is None:
            self.analysis_status = "solver_unavailable"
            self.analysis_detail = (
                "no safe CalculiX executable was discovered, so no solve was attempted"
            )
            return
        if self.analysis is None:
            self.analysis_status = "failed"
            self.analysis_detail = "the analysis container is missing"
            return
        try:
            tools = self.modules.ccx_tools.FemToolsCcx(self.analysis, self.solver)
            tools.update_objects()
            problems = tools.check_prerequisites()
        except Exception as error:
            self.analysis_status = "failed"
            self.analysis_detail = (
                f"the CalculiX bridge refused the analysis: {type(error).__name__}"
            )
            return
        if isinstance(problems, str) and problems.strip():
            self.analysis_status = "prerequisites_unmet"
            self.analysis_detail = " ".join(problems.split())[:MAX_DETAIL_LENGTH]
            return
        try:
            tools.run()
        except Exception as error:
            self.analysis_status = "failed"
            self.analysis_detail = f"the CalculiX solve did not complete: {type(error).__name__}"
            return
        self.analysis_status = "completed"
        self.analysis_detail = "the CalculiX solve completed"

    def _read_result(self) -> None:
        summary = result_summary(self.document)
        stress = summary.get("max_stress_mpa")
        if isinstance(stress, (int, float)) and not isinstance(stress, bool):
            self.peak_stress = float(stress)
        if self.analysis_status == "completed" and summary.get("available") is not True:
            message = summary.get("message")
            if isinstance(message, str) and message:
                self.analysis_detail = message[:MAX_DETAIL_LENGTH]


_HANDLERS: Final[dict[str, Callable[[_Session, Mapping[str, object]], None]]] = {
    "fem.analysis": _Session.fem_analysis,
    "fem.fixed": _Session.fem_fixed,
    "fem.force": _Session.fem_force,
    "fem.material": _Session.fem_material,
    "fem.mesh": _Session.fem_mesh,
    "fem.pressure": _Session.fem_pressure,
    "fem.result": _Session.fem_result,
    "fem.solve": _Session.fem_solve,
    "part.box": _Session.part_box,
    "part.compound": _Session.part_compound,
    "part.fuse": _Session.part_fuse,
    "part.cylinder": _Session.part_cylinder,
    "partdesign.body": _Session.body_new,
    "partdesign.check": _Session.check,
    "partdesign.fillet": _Session.fillet,
    "partdesign.helix": _Session.helix,
    "partdesign.pad": _Session.add_pad,
    "partdesign.placement": _Session.place,
    "partdesign.pocket": _Session.pocket,
    "partdesign.revolution": _Session.revolution,
    "sketch.circle": _Session.sketch_circle,
    "sketch.coincident": _Session.sketch_coincident,
    "sketch.diameter": _Session.sketch_diameter,
    "sketch.distance": _Session.sketch_distance,
    "sketch.distance_x": _Session.sketch_distance_x,
    "sketch.distance_y": _Session.sketch_distance_y,
    "sketch.equal": _Session.sketch_equal,
    "sketch.horizontal": _Session.sketch_horizontal,
    "sketch.leave": _Session.sketch_leave,
    "sketch.line": _Session.sketch_line,
    "sketch.new": _Session.sketch_new,
    "sketch.parallel": _Session.sketch_parallel,
    "sketch.perpendicular": _Session.sketch_perpendicular,
    "sketch.point": _Session.sketch_point,
    "sketch.polyline": _Session.sketch_polyline,
    "sketch.radius": _Session.sketch_radius,
    "sketch.rectangle": _Session.sketch_rectangle,
    "sketch.tangent": _Session.sketch_tangent,
    "sketch.validate": _Session.sketch_validate,
    "sketch.vertical": _Session.sketch_vertical,
}


def _new_document(freecad: FreeCADObject, name: str) -> FreeCADObject:
    try:
        return freecad.newDocument(name)
    except Exception as error:
        raise FreeCADOperationError(
            f"FreeCAD could not open a document: {type(error).__name__}: {error}"
        ) from error


def _numbered(name: str, role: str) -> bool:
    suffix = name[len(role) :]
    return name.startswith(role) and suffix.isdigit() and suffix != ""


def _recompute(document: FreeCADObject) -> None:
    try:
        document.recompute()
    except Exception as error:
        raise FreeCADOperationError(
            f"FreeCAD could not recompute the document: {type(error).__name__}: {error}"
        ) from error


def _close(freecad: FreeCADObject, document: FreeCADObject) -> None:
    name = getattr(document, "Name", None)
    if not isinstance(name, str) or not name:
        return
    try:
        freecad.closeDocument(name)
    except Exception:
        return


def _solid_shape(feature: FreeCADObject | None) -> FreeCADObject:
    if feature is None:
        raise FreeCADOperationError("the design did not produce a solid")
    shape = getattr(feature, "Shape", None)
    if shape is None:
        raise FreeCADOperationError("the feature has no shape")
    if getattr(shape, "ShapeType", "") == "Solid":
        return shape
    solids = getattr(shape, "Solids", None)
    if isinstance(solids, Sequence) and not isinstance(solids, (str, bytes)) and len(solids) == 1:
        inner = solids[0]
        if getattr(inner, "ShapeType", "") == "Solid":
            return inner
    raise FreeCADOperationError("the feature did not produce one valid solid")


def _closed(shape: FreeCADObject) -> bool:
    wires = getattr(shape, "Wires", None)
    if isinstance(wires, (str, bytes)) or not isinstance(wires, Sequence) or not wires:
        return False
    return all(bool(wire.isClosed()) for wire in wires)


def _points(arguments: Mapping[str, object]) -> tuple[tuple[float, float], ...]:
    raw = arguments.get("points")
    if isinstance(raw, (str, bytes)) or not isinstance(raw, Sequence) or len(raw) < 2:
        raise FreeCADInputError("points must list at least two positions")
    points: list[tuple[float, float]] = []
    for item in raw:
        if isinstance(item, (str, bytes)) or not isinstance(item, Sequence) or len(item) < 2:
            raise FreeCADInputError("each point must be a pair of coordinates")
        points.append((_coordinate_value(item[0], "x"), _coordinate_value(item[1], "y")))
        if len(points) > 32:
            raise FreeCADInputError("a profile is limited to 32 points")
    return tuple(points)


def _index(arguments: Mapping[str, object], key: str, *, default: int | None = None) -> int:
    if key not in arguments:
        if default is None:
            raise FreeCADInputError(f"{key} is required")
        return default
    value = arguments[key]
    if isinstance(value, bool) or not isinstance(value, int) or not -8 <= value <= 256:
        raise FreeCADInputError(f"{key} must be an integer geometry index")
    return value


def _length(
    arguments: Mapping[str, object],
    key: str,
    *,
    default: float | None = None,
    minimum: float = 0.001,
    maximum: float = MAX_DIMENSION_MM,
) -> float:
    if key not in arguments:
        if default is None:
            raise FreeCADInputError(f"{key} is required")
        return default
    number = _number(arguments[key], key)
    if not minimum <= number <= maximum:
        raise FreeCADInputError(f"{key} must be between {minimum:g} and {maximum:g}")
    return number


def _coordinate(
    arguments: Mapping[str, object], key: str, *, default: float | None = None
) -> float:
    if key not in arguments:
        if default is None:
            raise FreeCADInputError(f"{key} is required")
        return default
    number = _number(arguments[key], key)
    if abs(number) > MAX_COORDINATE_MM:
        raise FreeCADInputError(f"{key} is outside the model bounds")
    return number


def _coordinate_value(value: object, key: str) -> float:
    number = _number(value, key)
    if abs(number) > MAX_COORDINATE_MM:
        raise FreeCADInputError(f"{key} is outside the model bounds")
    return number


def _number(value: object, key: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise FreeCADInputError(f"{key} must be a number")
    number = float(value)
    if not math.isfinite(number):
        raise FreeCADInputError(f"{key} must be finite")
    return number


def _as_float(value: object) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    number = float(value)
    if not math.isfinite(number):
        return None
    return number


def _export_all(
    modules: FreeCADModules,
    document: FreeCADObject,
    feature: FreeCADObject | None,
    request: DesignRequest,
    *,
    workspace: Path,
    max_artifact_bytes: int,
    call_id: str,
) -> tuple[PublishedArtifact, ...]:
    if feature is None:
        raise FreeCADOperationError("the design did not produce a solid")
    published: list[PublishedArtifact] = []
    for export_format in request.export_formats:
        relative = f"{ARTIFACT_DIRECTORY}/{request.artifact_stem}{EXPORT_SUFFIXES[export_format]}"
        target = resolve_target(workspace, relative)
        prepare_target(target)
        try:
            if export_format == "fcstd":
                document.saveAs(str(target))
            else:
                modules.part.export([feature], str(target))
        except Exception as error:
            discard(target)
            raise FreeCADOperationError(
                f"FreeCAD could not write the {export_format} export: "
                f"{type(error).__name__}: {error}"
            ) from error
        published.append(
            finalize(
                target,
                relative_path=relative,
                export_format=export_format,
                artifact_id=f"{call_id}-{export_format}",
                max_artifact_bytes=max_artifact_bytes,
            )
        )
    return tuple(published)


__all__ = (
    "MAX_DIMENSION_MM",
    "DesignOutcome",
    "DesignRequest",
    "parse_request",
    "run",
    "run_open",
)
