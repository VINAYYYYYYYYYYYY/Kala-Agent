"""One FreeCAD document applies a whole design call list."""

from __future__ import annotations

import json
from collections.abc import Iterator, Sequence
from contextlib import contextmanager
from pathlib import Path

import pytest

from kala_agent_freecad.availability import FreeCADAvailability
from kala_agent_freecad.cad import design
from kala_agent_freecad.cad.decide import decide
from kala_agent_freecad.cad.runtime import FREECAD_IMPORT_ORDER, FreeCADModules
from kala_agent_freecad.errors import FreeCADInputError


class _Wire:
    def isClosed(self) -> bool:
        return True


class _Shape:
    def __init__(self, *, solid: bool) -> None:
        self.Wires = [_Wire()]
        self.ShapeType = "Solid" if solid else "Wire"
        self.isValid = True
        self.Volume = 25600.0
        self.Area = 8000.0
        self.BoundBox = _Box()
        self.Edges = [object() for _ in range(12)]
        self.Faces = [object() for _ in range(6)]
        self.Vertexes = [object() for _ in range(8)]
        self.Solids: list[object] = []


class _Box:
    XLength = 80.0
    YLength = 40.0
    ZLength = 8.0


class _Sketch:
    def __init__(self, name: str) -> None:
        self.Name = name
        self.Shape = _Shape(solid=False)
        self.DoF = 0
        self.AttachmentSupport: object = None
        self.MapMode = ""
        self.constraints: list[object] = []

    def addGeometry(self, geometry: object, construction: bool) -> int:
        del geometry, construction
        return 0

    def addConstraint(self, constraint: object) -> int:
        self.constraints.append(constraint)
        return 0

    def solve(self) -> int:
        return 0


class _Pad:
    def __init__(self, name: str) -> None:
        self.Name = name
        self.Profile = None
        self.Length = 0.0
        self.Shape = _Shape(solid=True)


class _Origin:
    def __init__(self) -> None:
        self.OriginFeatures = [
            type("Plane", (), {"Name": name})()
            for name in ("XY_Plane", "XZ_Plane", "YZ_Plane", "Y_Axis")
        ]


class _Body:
    def __init__(self) -> None:
        self.Name = "Body"
        self.Origin = _Origin()

    def newObject(self, type_id: str, name: str) -> object:
        del type_id
        if name == "Sketch":
            return _Sketch(name)
        return _Pad(name)


class _Member:
    def __init__(self, name: str) -> None:
        self.Name = name
        self.Material: dict[str, str] = {}
        self.References = None
        self.Force = 0.0
        self.Shape = None


class _Analysis(_Member):
    def __init__(self) -> None:
        super().__init__("Analysis")
        self.members: list[object] = []

    def addObject(self, member: object) -> None:
        self.members.append(member)


class _Fem:
    def makeAnalysis(self, document: object, name: str) -> _Analysis:
        del document, name
        return _Analysis()

    def makeMaterialSolid(self, document: object, name: str) -> _Member:
        del document
        return _Member(name)

    def makeConstraintFixed(self, document: object, name: str) -> _Member:
        del document
        return _Member(name)

    def makeConstraintForce(self, document: object, name: str) -> _Member:
        del document
        return _Member(name)

    def makeMeshGmsh(self, document: object, name: str) -> _Member:
        del document
        return _Member(name)

    def makeSolverCalculiXCcxTools(self, document: object, name: str) -> _Member:
        del document
        return _Member(name)


class _Document:
    def __init__(self, name: str) -> None:
        self.Name = name
        self.Objects: list[object] = []

    def addObject(self, type_id: str, name: str) -> object:
        del type_id, name
        body = _Body()
        self.Objects.append(body)
        return body

    def recompute(self) -> None:
        return None

    def saveAs(self, path: str) -> None:
        Path(path).write_bytes(b"PK\x03\x04")


class _FreeCAD:
    def __init__(self, document: _Document) -> None:
        self.document = document

    def newDocument(self, name: str) -> _Document:
        self.document.Name = name
        return self.document

    def closeDocument(self, name: str) -> None:
        del name

    def Vector(self, x_mm: float, y_mm: float, z_mm: float) -> tuple[float, float, float]:
        return (x_mm, y_mm, z_mm)


class _Part:
    def LineSegment(self, start: object, end: object) -> tuple[object, object]:
        return (start, end)

    def Circle(self, center: object, normal: object, radius: float) -> tuple[object, object, float]:
        return (center, normal, radius)

    def export(self, features: list[object], path: str) -> None:
        del features
        Path(path).write_bytes(b"ISO-10303-21;\n")


class _Sketcher:
    def Constraint(self, *parts: object) -> tuple[object, ...]:
        return parts


@contextmanager
def _open(_roots: Sequence[Path]) -> Iterator[FreeCADModules]:
    document = _Document("Kala")
    yield FreeCADModules(
        freecad=_FreeCAD(document),
        part=_Part(),
        objects_fem=_Fem(),
        ccx_tools=None,
        version="fake",
        import_root="/opt/freecad/lib",
        import_order=FREECAD_IMPORT_ORDER,
        solver_bridge_detail="calculix bridge absent",
    )


class _Placement:
    def __init__(self) -> None:
        self.Base: object = (0.0, 0.0, 0.0)


class _NamedFeature:
    def __init__(self, name: str, role: str) -> None:
        self.Name = name
        self.Role = role


class _NumberedOrigin:
    def __init__(self, index: int) -> None:
        suffix = "" if index == 0 else f"{index:03d}"
        roles = ("X_Axis", "Y_Axis", "Z_Axis", "XY_Plane", "XZ_Plane", "YZ_Plane")
        self.by_role = {role: _NamedFeature(f"{role}{suffix}", role) for role in roles}
        self.OriginFeatures = list(self.by_role.values())


class _NumberedBody:
    def __init__(self, index: int) -> None:
        self.Name = "Body" if index == 0 else f"Body{index:03d}"
        self.Origin = _NumberedOrigin(index)
        self.Placement = _Placement()
        self.made: list[object] = []

    def newObject(self, type_id: str, name: str) -> object:
        del type_id
        created: object = _Sketch(name) if name == "Sketch" else _Pad(name)
        self.made.append(created)
        return created


class _NumberedDocument(_Document):
    def __init__(self) -> None:
        super().__init__("Kala")
        self.bodies: list[_NumberedBody] = []

    def addObject(self, type_id: str, name: str) -> _NumberedBody:
        del type_id, name
        body = _NumberedBody(len(self.bodies))
        self.bodies.append(body)
        self.Objects.append(body)
        return body


def test_a_later_part_uses_its_own_origin(monkeypatch: pytest.MonkeyPatch) -> None:
    from kala_agent_freecad.cad.compose import realize

    document = _NumberedDocument()
    monkeypatch.setattr(design, "load_sketcher", lambda: _Sketcher())
    plan = realize(
        {
            "analyze": False,
            "parts": (
                {
                    "features": (
                        {
                            "height": 40,
                            "kind": "pad",
                            "length": 12,
                            "plane": "XY",
                            "profile": "rectangle",
                            "width": 80,
                        },
                    ),
                    "name": "base",
                },
                {
                    "features": (
                        {
                            "diameter": 16,
                            "kind": "pad",
                            "length": 40,
                            "plane": "XY",
                            "profile": "circle",
                        },
                        {"diameter": 16, "kind": "helix", "length": 24, "pitch": 4},
                    ),
                    "name": "screw",
                    "z": 12,
                },
            ),
            "summary": "Two parts.",
        },
        lambda _state, options: next(key for key in options if key != "stop"),
    )
    design.run_open(
        design.parse_request({"calls_json": json.dumps(plan.as_json()), "document_name": "Kala"}),
        FreeCADModules(
            freecad=_FreeCAD(document),
            part=_Part(),
            objects_fem=_Fem(),
            ccx_tools=None,
            version="fake",
            import_root="/opt/freecad/lib",
            import_order=FREECAD_IMPORT_ORDER,
            solver_bridge_detail="",
        ),
    )
    assert [body.Name for body in document.bodies] == ["Body", "Body001"]
    second = document.bodies[1]
    sketches = [item for item in second.made if isinstance(item, _Sketch)]
    assert sketches
    attached = sketches[0].AttachmentSupport
    assert isinstance(attached, list)
    assert attached[0][0] is second.Origin.by_role["XY_Plane"]
    assert second.Origin.by_role["XY_Plane"].Name == "XY_Plane001"
    helix = next(item for item in second.made if getattr(item, "Name", "") == "Helix")
    axis = getattr(helix, "ReferenceAxis", None)
    assert isinstance(axis, tuple)
    assert axis[0] is second.Origin.by_role["Z_Axis"]
    assert second.Placement.Base == (0.0, 0.0, 12.0)


def test_a_circle_may_sit_on_the_negative_side() -> None:
    sketch = _Sketch("Sketch")
    session = design._Session(
        FreeCADModules(
            freecad=_FreeCAD(_Document("Kala")),
            part=_Part(),
            objects_fem=_Fem(),
            ccx_tools=None,
            version="fake",
            import_root="/opt/freecad/lib",
            import_order=FREECAD_IMPORT_ORDER,
            solver_bridge_detail="",
        ),
        _Sketcher(),
        _Document("Kala"),
        "",
    )
    session.sketch = sketch
    session.sketch_distance_x({"geo": 0, "length": -8.75, "pos": 3})
    session.sketch_distance_y({"geo": 0, "length": -15.155, "pos": 3})
    assert sketch.constraints == [
        ("DistanceX", 0, 3, -8.75),
        ("DistanceY", 0, 3, -15.155),
    ]
    with pytest.raises(FreeCADInputError, match="between 0 and 5000"):
        session.sketch_distance_x({"geo": 0, "geo2": 1, "length": -8.75, "pos": 1, "pos2": 2})


def test_an_illegal_list_is_refused_before_freecad_opens() -> None:
    with pytest.raises(FreeCADInputError, match="not available"):
        design.parse_request({"calls_json": '[{"tool": "sketch.line", "arguments": {}}]'})


def test_a_plate_plan_exports_the_solid_without_inventing_stress(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(design, "freecad_session", _open)
    monkeypatch.setattr(design, "load_sketcher", lambda: _Sketcher())
    plan = decide("80 x 40 x 8 mm plate")
    outcome = design.run(
        design.parse_request(
            {
                "calls_json": json.dumps(plan.as_json()),
                "export_formats": ["step", "fcstd"],
            }
        ),
        workspace=tmp_path,
        import_roots=(),
        max_artifact_bytes=1_000_000,
        call_id="plate-1",
        calculix_path="",
    )
    assert outcome.result["valid"] is True
    assert outcome.result["degrees_of_freedom"] == 0
    assert outcome.result["analysis_status"] == "solver_unavailable"
    assert outcome.result["redesign_trace"] == []
    measurement = outcome.result["measurement"]
    assert isinstance(measurement, dict)
    assert measurement["volume_mm3"] == 25600.0
    paths = {artifact.uri for artifact in outcome.artifacts}
    assert any(path.endswith(".step") for path in paths)
    assert any(path.endswith(".fcstd") for path in paths)


@pytest.mark.freecad
def test_freecad_pads_a_fully_constrained_plate(
    tmp_path: Path,
    require_freecad: FreeCADAvailability,
) -> None:
    plan = decide("80 x 40 x 8 mm plate")
    outcome = design.run(
        design.parse_request(
            {
                "calls_json": json.dumps(plan.as_json()),
                "export_formats": ["step", "fcstd"],
            }
        ),
        workspace=tmp_path,
        import_roots=require_freecad.import_root_paths,
        max_artifact_bytes=8_000_000,
        call_id="live-plate",
        calculix_path=require_freecad.calculix_path,
    )
    measurement = outcome.result["measurement"]
    assert isinstance(measurement, dict)
    assert measurement["volume_mm3"] == 25600.0
    assert measurement["solid"] is True
    assert outcome.result["degrees_of_freedom"] == 0
    assert outcome.result["valid"] is True
    assert outcome.result["analysis_status"] in {
        "solver_unavailable",
        "completed",
        "failed",
        "prerequisites_unmet",
    }
    step = next(artifact for artifact in outcome.artifacts if artifact.uri.endswith(".step"))
    assert Path(step.uri.removeprefix("file://")).read_bytes().startswith(b"ISO-10303-21;")
