"""A place-only assembly exports its placed occurrences, not its tip feature.

These tests drive ``design.run`` with the same fake FreeCAD modules the rest of
this suite uses. They pin the export target: the finished body of every placed
component, with its placement, is what STEP, FCStd, and the measurement
describe. A ``fuse`` step still exports its own joined result, and a script
with no placement still exports its tip feature.
"""

from __future__ import annotations

import json
from collections.abc import Iterator, Sequence
from contextlib import contextmanager
from pathlib import Path
from typing import Any, Final, cast

import pytest

from kala_agent.contracts.construction import ConstructionScript, load_construction
from kala_agent_freecad.cad import design
from kala_agent_freecad.cad.compile import compile_construction
from kala_agent_freecad.cad.runtime import FREECAD_IMPORT_ORDER, FreeCADModules

STEP_PAYLOAD: Final[bytes] = b"ISO-10303-21;\n"
DOCUMENT_PAYLOAD: Final[bytes] = b"PK\x03\x04 fcstd\n"
Vector3 = tuple[float, float, float]


class _Box:
    def __init__(self, minimum: Vector3, maximum: Vector3) -> None:
        self.XMin, self.YMin, self.ZMin = minimum
        self.XMax, self.YMax, self.ZMax = maximum

    @property
    def XLength(self) -> float:
        return self.XMax - self.XMin

    @property
    def YLength(self) -> float:
        return self.YMax - self.YMin

    @property
    def ZLength(self) -> float:
        return self.ZMax - self.ZMin


class _Wire:
    def isClosed(self) -> bool:
        return True


class _Shape:
    """A solid or compound that answers the measurement attributes."""

    def __init__(
        self,
        *,
        kind: str = "Solid",
        minimum: Vector3 = (0.0, 0.0, 0.0),
        maximum: Vector3 = (10.0, 10.0, 5.0),
        volume: float = 500.0,
        solids: int = 0,
    ) -> None:
        self.ShapeType = kind
        self.isValid = True
        self.BoundBox = _Box(minimum, maximum)
        self.Volume = volume
        self.Area = 2.0 * (
            self.BoundBox.XLength * self.BoundBox.YLength
            + self.BoundBox.XLength * self.BoundBox.ZLength
            + self.BoundBox.YLength * self.BoundBox.ZLength
        )
        self.Edges = [object() for _ in range(12)]
        self.Faces = [object() for _ in range(6)]
        self.Vertexes = [object() for _ in range(8)]
        self.Solids = [object() for _ in range(solids)]
        self.Wires: list[object] = []

    def fuse(self, other: _Shape) -> _Shape:
        minimum = tuple(min(a, b) for a, b in zip(self._minimum, other._minimum, strict=True))
        maximum = tuple(max(a, b) for a, b in zip(self._maximum, other._maximum, strict=True))
        return _Shape(
            minimum=cast("Vector3", minimum),
            maximum=cast("Vector3", maximum),
            volume=self.Volume + other.Volume,
        )

    @property
    def _minimum(self) -> Vector3:
        return (self.BoundBox.XMin, self.BoundBox.YMin, self.BoundBox.ZMin)

    @property
    def _maximum(self) -> Vector3:
        return (self.BoundBox.XMax, self.BoundBox.YMax, self.BoundBox.ZMax)


class _Placement:
    def __init__(self) -> None:
        self.Base: object = (0.0, 0.0, 0.0)


class _Sketch:
    def __init__(self, name: str) -> None:
        self.Name = name
        self.Shape = _Shape(kind="Wire", volume=0.0, solids=0)
        self.Shape.Wires = [_Wire()]
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
        self.Reversed = False
        self.Shape = _Shape()


class _Body:
    """A body whose shape is its local part moved by its placement."""

    def __init__(self, name: str) -> None:
        self.Name = name
        self.Label = ""
        self.Placement = _Placement()
        self.local = _Shape()
        self.Origin = _Origin()
        self.made: list[object] = []

    @property
    def Shape(self) -> _Shape:
        base = cast("Vector3", self.Placement.Base)
        local = self.local
        minimum = tuple(value + offset for value, offset in zip(local._minimum, base, strict=True))
        maximum = tuple(value + offset for value, offset in zip(local._maximum, base, strict=True))
        return _Shape(
            minimum=cast("Vector3", minimum),
            maximum=cast("Vector3", maximum),
            volume=local.Volume,
        )

    def newObject(self, type_id: str, name: str) -> object:
        del type_id
        created: object = _Sketch(name) if name == "Sketch" else _Pad(name)
        self.made.append(created)
        return created


class _OriginFeature:
    def __init__(self, name: str, role: str) -> None:
        self.Name = name
        self.Role = role


class _Origin:
    def __init__(self) -> None:
        self.OriginFeatures = [
            _OriginFeature(name, name) for name in ("XY_Plane", "XZ_Plane", "YZ_Plane", "Y_Axis")
        ]


class _Feature:
    def __init__(self, name: str) -> None:
        self.Name = name
        self.Shape: object = None


class _Document:
    def __init__(self, name: str) -> None:
        self.Name = name
        self.Objects: list[object] = []
        self.saved: list[str] = []

    def addObject(self, type_id: str, name: str) -> object:
        if type_id == "PartDesign::Body":
            created: object = _Body(name)
        else:
            created = _Feature(name)
        self.Objects.append(created)
        return created

    def recompute(self) -> None:
        return None

    def saveAs(self, path: str) -> None:
        Path(path).write_bytes(DOCUMENT_PAYLOAD)
        self.saved.append(path)


class _FreeCAD:
    def __init__(self, document: _Document) -> None:
        self.document = document

    def newDocument(self, name: str) -> _Document:
        self.document.Name = name
        return self.document

    def closeDocument(self, name: str) -> None:
        del name

    def Vector(self, x_mm: float, y_mm: float, z_mm: float) -> Vector3:
        return (x_mm, y_mm, z_mm)


class _Part:
    def __init__(self) -> None:
        self.exported: list[_Shape] = []

    def LineSegment(self, start: object, end: object) -> tuple[object, object]:
        return (start, end)

    def Circle(self, center: object, normal: object, radius: float) -> tuple[object, ...]:
        return (center, normal, radius)

    def makeCompound(self, shapes: Sequence[_Shape]) -> _Shape:
        minimum = tuple(min(shape._minimum[axis] for shape in shapes) for axis in range(3))
        maximum = tuple(max(shape._maximum[axis] for shape in shapes) for axis in range(3))
        return _Shape(
            kind="Compound",
            minimum=cast("Vector3", minimum),
            maximum=cast("Vector3", maximum),
            volume=sum(shape.Volume for shape in shapes),
            solids=len(shapes),
        )

    def export(self, features: Sequence[object], path: str) -> None:
        for feature in features:
            shape = getattr(feature, "Shape", None)
            assert isinstance(shape, _Shape)
            self.exported.append(shape)
        Path(path).write_bytes(STEP_PAYLOAD)


class _Sketcher:
    def Constraint(self, *parts: object) -> tuple[object, ...]:
        return parts


class _Modules:
    def __init__(self, part: _Part, document: _Document) -> None:
        self.part = part
        self.document = document

    @contextmanager
    def session(self, _roots: Sequence[Path]) -> Iterator[FreeCADModules]:
        yield FreeCADModules(
            freecad=_FreeCAD(self.document),
            part=self.part,
            objects_fem=cast("Any", None),
            ccx_tools=None,
            version="fake",
            import_root="/opt/freecad/lib",
            import_order=FREECAD_IMPORT_ORDER,
            solver_bridge_detail="calculix bridge absent",
        )


def _run(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    script: ConstructionScript,
) -> tuple[dict[str, object], _Part, _Document]:
    part = _Part()
    document = _Document("Kala")
    modules = _Modules(part, document)
    monkeypatch.setattr(design, "freecad_session", modules.session)
    monkeypatch.setattr(design, "load_sketcher", lambda: _Sketcher())
    calls = compile_construction(script)
    outcome = design.run(
        design.parse_request(
            {"calls_json": json.dumps(calls), "export_formats": ["step", "fcstd"]}
        ),
        workspace=tmp_path,
        import_roots=(),
        max_artifact_bytes=1_000_000,
        call_id="assembly-1",
    )
    return dict(outcome.result), part, document


def _place_only_two() -> ConstructionScript:
    return load_construction(
        {
            "summary": "two placed plates",
            "components": [
                {
                    "name": "base",
                    "steps": [
                        {
                            "op": "sketch",
                            "profile": "rectangle",
                            "plane": "XY",
                            "width": 10,
                            "height": 10,
                        },
                        {"op": "extrude", "length": 5},
                    ],
                },
                {
                    "name": "riser",
                    "steps": [
                        {
                            "op": "sketch",
                            "profile": "rectangle",
                            "plane": "XY",
                            "width": 10,
                            "height": 10,
                        },
                        {"op": "extrude", "length": 5},
                    ],
                },
            ],
            "assembly": [
                {"op": "place", "component": "base", "x": 0, "y": 0, "z": 0},
                {"op": "place", "component": "riser", "x": 20, "y": 0, "z": 0},
            ],
        }
    )


def _place_only_one() -> ConstructionScript:
    return load_construction(
        {
            "summary": "one placed plate",
            "components": [
                {
                    "name": "base",
                    "steps": [
                        {
                            "op": "sketch",
                            "profile": "rectangle",
                            "plane": "XY",
                            "width": 10,
                            "height": 10,
                        },
                        {"op": "extrude", "length": 5},
                    ],
                }
            ],
            "assembly": [{"op": "place", "component": "base", "x": 5, "y": 6, "z": 7}],
        }
    )


def test_a_place_only_assembly_exports_every_placed_body(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    result, part, document = _run(monkeypatch, tmp_path, _place_only_two())

    assert len(part.exported) == 1
    exported = part.exported[0]
    assert exported.ShapeType == "Compound"
    assert len(exported.Solids) == 2
    assert exported.Volume == pytest.approx(1_000.0)
    assert exported.BoundBox.XMin == pytest.approx(0.0)
    assert exported.BoundBox.XMax == pytest.approx(30.0)
    measurement = cast("dict[str, object]", result["measurement"])
    assert measurement["volume_mm3"] == pytest.approx(1_000.0)
    assert measurement["bounding_box_mm"] == pytest.approx([30.0, 10.0, 5.0])
    assert result["valid"] is True
    assembly = document.Objects[-1]
    assert isinstance(assembly, _Feature)
    assert assembly.Shape is exported


def test_a_single_placed_body_exports_its_placement(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    result, part, _document = _run(monkeypatch, tmp_path, _place_only_one())

    assert len(part.exported) == 1
    exported = part.exported[0]
    assert exported.BoundBox.XMin == pytest.approx(5.0)
    assert exported.BoundBox.YMin == pytest.approx(6.0)
    assert exported.BoundBox.ZMin == pytest.approx(7.0)
    assert exported.BoundBox.XMax == pytest.approx(15.0)
    assert exported.BoundBox.YMax == pytest.approx(16.0)
    assert exported.BoundBox.ZMax == pytest.approx(12.0)
    measurement = cast("dict[str, object]", result["measurement"])
    assert measurement["volume_mm3"] == pytest.approx(500.0)


def test_a_fused_script_still_exports_the_joined_result(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    script = load_construction(
        {
            "summary": "two fused plates",
            "components": [
                {
                    "name": "base",
                    "steps": [
                        {
                            "op": "sketch",
                            "profile": "rectangle",
                            "plane": "XY",
                            "width": 10,
                            "height": 10,
                        },
                        {"op": "extrude", "length": 5},
                    ],
                },
                {
                    "name": "riser",
                    "steps": [
                        {
                            "op": "sketch",
                            "profile": "rectangle",
                            "plane": "XY",
                            "width": 10,
                            "height": 10,
                        },
                        {"op": "extrude", "length": 5},
                    ],
                },
            ],
            "assembly": [
                {"op": "place", "component": "riser", "x": 0, "y": 0, "z": 5},
                {"op": "fuse", "components": ["base", "riser"]},
            ],
        }
    )
    result, part, document = _run(monkeypatch, tmp_path, script)

    assert len(part.exported) == 1
    exported = part.exported[0]
    assert exported.ShapeType == "Solid"
    assert exported.Volume == pytest.approx(1_000.0)
    # Only the fuse created the "Assembly" feature; no occurrence compound was
    # added on top of the joined result.
    assert [item.Name for item in document.Objects if isinstance(item, _Feature)] == ["Assembly"]
    measurement = cast("dict[str, object]", result["measurement"])
    assert measurement["volume_mm3"] == pytest.approx(1_000.0)
