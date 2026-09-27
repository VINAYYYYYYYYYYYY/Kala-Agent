"""Stand-ins for the native FreeCAD modules.

The adapter only ever reaches FreeCAD through
:func:`~kala_agent_freecad.cad.runtime.freecad_session`, so a test can drive
every build, export, and solver path by replacing that one context manager
with these fakes. Nothing here imports FreeCAD, writes outside ``tmp_path``,
or runs a process, which keeps the failure paths that a real installation
cannot produce (a refused document, a silent shape, a broken bridge) honest.
"""

from __future__ import annotations

from collections.abc import Callable, Iterator, Mapping, Sequence
from contextlib import AbstractContextManager, contextmanager
from pathlib import Path
from typing import Final

from kala_agent_freecad.cad.runtime import FreeCADModules

STEP_PAYLOAD: Final[bytes] = b"ISO-10303-21;\n"
DOCUMENT_PAYLOAD: Final[bytes] = b"PK\x03\x04 freecad document\n"
RESULT_PAYLOAD: Final[bytes] = b"1C,2ELASTIC\n"
RESULT_TYPE_ID: Final[str] = "Fem::FemResultObjectPython"
ALL_FEM_FACTORIES: Final[tuple[str, ...]] = (
    "makeAnalysis",
    "makeMaterialSolid",
    "makeConstraintFixed",
    "makeMeshGmsh",
    "makeSolverCalculiXCcxTools",
)
FEM_TYPE_IDS: Final[Mapping[str, str]] = {
    "makeAnalysis": "Fem::Fem::Analysis",
    "makeMaterialSolid": "Fem::FemMaterialCommon",
    "makeConstraintFixed": "Fem::FemConstraintFixed",
    "makeMeshGmsh": "Fem::FemMeshShapeNetgenObjectPython",
    "makeSolverCalculiXCcxTools": "Fem::FemSolverCalculiXCcxToolsPython",
}
ShapeBuilder = Callable[[str, float, float, float], object]


class FakeShape:
    """A shape that answers the measurement attributes as plain values."""

    def __init__(
        self,
        *,
        length: float = 120.0,
        width: float = 40.0,
        height: float = 20.0,
    ) -> None:
        self.BoundBox = FakeBoundBox(length, width, height)
        self.Area = 2.0 * (length * width + length * height + width * height)
        self.Volume = length * width * height
        self.ShapeType = "Solid"
        self.isValid = True
        self.Edges: list[object] = [object() for _ in range(12)]
        self.Faces: list[object] = [object() for _ in range(6)]
        self.Vertexes: list[object] = [object() for _ in range(8)]


class FakeBoundBox:
    def __init__(self, x_length: float, y_length: float, z_length: float) -> None:
        self.XLength = x_length
        self.YLength = y_length
        self.ZLength = z_length


class FakeCylinderShape(FakeShape):
    def __init__(self, *, radius: float, height: float) -> None:
        super().__init__(length=radius * 2.0, width=radius * 2.0, height=height)
        self.Area = 2.0 * 3.141592653589793 * radius * (radius + height)
        self.Volume = 3.141592653589793 * radius * radius * height
        self.Edges = [object() for _ in range(3)]
        self.Faces = [object() for _ in range(3)]
        self.Vertexes = [object() for _ in range(2)]


class FakeFeature:
    def __init__(self, name: str, shape: object | None = None) -> None:
        self.Name = name
        if shape is not None:
            self.Shape = shape


class FakeDocument:
    """A document that records what the adapter asked it to do."""

    def __init__(
        self,
        name: object,
        *,
        recompute_failure_at: int = 0,
        save_fails: bool = False,
        add_error: Exception | None = None,
    ) -> None:
        self.Name: object = name
        self.Objects: list[object] = []
        self.type_ids: list[str] = []
        self.recomputes = 0
        self.saved: list[str] = []
        self._recompute_failure_at = recompute_failure_at
        self._save_fails = save_fails
        self._add_error = add_error

    def addObject(self, type_id: str, name: str) -> FakeFeature:
        if self._add_error is not None:
            raise self._add_error
        feature = FakeFeature(name)
        self.Objects.append(feature)
        self.type_ids.append(type_id)
        return feature

    def recompute(self) -> None:
        self.recomputes += 1
        if self._recompute_failure_at == self.recomputes:
            raise RuntimeError("the document could not be recomputed")

    def saveAs(self, target: str) -> None:
        path = Path(target)
        path.write_bytes(DOCUMENT_PAYLOAD[:4])
        if self._save_fails:
            raise RuntimeError("the document could not be written")
        self.saved.append(target)
        path.write_bytes(DOCUMENT_PAYLOAD)


class FakeFreeCAD:
    """The application module: documents, versions, and their lifecycle."""

    def __init__(
        self,
        version: object = ("1", "1", "3"),
        *,
        new_document_fails: bool = False,
        close_fails: bool = False,
        unnamed: bool = False,
        recompute_failure_at: int = 0,
        save_fails: bool = False,
        add_error: Exception | None = None,
    ) -> None:
        self._version = version
        self._new_document_fails = new_document_fails
        self._close_fails = close_fails
        self._unnamed = unnamed
        self._recompute_failure_at = recompute_failure_at
        self._save_fails = save_fails
        self._add_error = add_error
        self.documents: dict[str, FakeDocument] = {}
        self.opened: list[FakeDocument] = []
        self.closed: list[str] = []

    def Version(self) -> object:
        return self._version

    def newDocument(self, name: str) -> FakeDocument:
        if self._new_document_fails:
            raise RuntimeError("the document service is not initialized")
        document = FakeDocument(
            None if self._unnamed else name,
            recompute_failure_at=self._recompute_failure_at,
            save_fails=self._save_fails,
            add_error=self._add_error,
        )
        self.documents[name] = document
        self.opened.append(document)
        return document

    def listDocuments(self) -> list[str]:
        return list(self.documents)

    def closeDocument(self, name: str) -> None:
        if self._close_fails:
            raise RuntimeError("the document is still busy")
        self.closed.append(name)
        self.documents.pop(name, None)


class FakePart:
    """The Part module: primitive factories and the STEP/IGES exporter."""

    def __init__(
        self,
        *,
        shape: ShapeBuilder | None = None,
        export_fails: bool = False,
    ) -> None:
        self.dimensions: list[tuple[str, tuple[float, ...]]] = []
        self.exports: list[str] = []
        self._shape: ShapeBuilder = shape if shape is not None else _default_shape
        self._export_fails = export_fails

    def makeBox(self, length: float, width: float, height: float) -> object:
        self.dimensions.append(("box", (length, width, height)))
        return self._shape("box", length, width, height)

    def makeCylinder(self, radius: float, height: float) -> object:
        self.dimensions.append(("cylinder", (radius, height)))
        return self._shape("cylinder", radius, height, 0.0)

    def export(self, features: Sequence[object], target: str) -> None:
        path = Path(target)
        path.write_bytes(STEP_PAYLOAD[:3])
        if self._export_fails:
            raise RuntimeError("the exporter could not write the file")
        self.exports.append(target)
        path.write_bytes(STEP_PAYLOAD)


class ExportOnlyPart:
    """A Part module that can export but offers no primitive factory."""

    def __init__(self) -> None:
        self.exports: list[str] = []

    def export(self, features: Sequence[object], target: str) -> None:
        self.exports.append(target)
        Path(target).write_bytes(STEP_PAYLOAD)


class FakeAnalysis:
    def __init__(self, document: FakeDocument, name: str) -> None:
        self.Name = name
        self.document = document
        self.members: list[object] = []

    def addObject(self, member: object) -> None:
        self.members.append(member)


class FakeMaterial:
    def __init__(self, name: str) -> None:
        self.Name = name
        self.Material: dict[str, str] = {}


class FakeConstraint:
    def __init__(self, name: str) -> None:
        self.Name = name
        self.References: list[object] = []


class FakeFemMesh:
    def __init__(self, node_count: object = 120, tetra_count: object = 48) -> None:
        self.NodeCount = node_count
        self.TetraCount = tetra_count
        self.TriangleCount = 0


class FakeMesh:
    def __init__(self, name: str, fem_mesh: object | None = None) -> None:
        self.Name = name
        if fem_mesh is not None:
            self.FemMesh = fem_mesh


class FakeSolver:
    def __init__(self, name: str) -> None:
        self.Name = name


class FakeResultObject:
    def __init__(
        self,
        *,
        displacement: object = (0.0, 0.25, 1.5),
        stress: object = (10.0, 220.0),
        node_count: object = 8,
        tetra_count: object = 3,
    ) -> None:
        self.TypeId = RESULT_TYPE_ID
        self.DisplacementLengths = displacement
        self.vonMises = stress
        self.Mesh = FakeMesh("ResultMesh", FakeFemMesh(node_count, tetra_count))


class FakeObjectsFem:
    """The Fem workbench module, with every factory it should offer."""

    def __init__(self, factories: Sequence[str] = ALL_FEM_FACTORIES) -> None:
        for factory in factories:
            setattr(self, factory, self._builder(factory))

    def _builder(self, factory: str) -> Callable[[FakeDocument, str], object]:
        def build(document: FakeDocument, name: str) -> object:
            if factory == "makeAnalysis":
                created: object = FakeAnalysis(document, name)
            elif factory == "makeMaterialSolid":
                created = FakeMaterial(name)
            elif factory == "makeConstraintFixed":
                created = FakeConstraint(name)
            elif factory == "makeMeshGmsh":
                created = FakeMesh(name, FakeFemMesh())
            else:
                created = FakeSolver(name)
            document.Objects.append(created)
            document.type_ids.append(FEM_TYPE_IDS[factory])
            return created

        return build


class FakeFemToolsCcx:
    """The CalculiX bridge, and the result file a real solve would leave."""

    def __init__(
        self,
        analysis: FakeAnalysis | None = None,
        solver: FakeSolver | None = None,
        *,
        prerequisites: str = "",
        setup_fails: bool = False,
        run_fails: bool = False,
        result_name: str | None = "solver.frd",
        result_payload: bytes = RESULT_PAYLOAD,
    ) -> None:
        self.analysis = analysis
        self.solver = solver
        self.working_directory = ""
        self.built = 0
        self.ran = 0
        self._prerequisites = prerequisites
        self._setup_fails = setup_fails
        self._run_fails = run_fails
        self._result_name = result_name
        self._result_payload = result_payload

    def bind(self, analysis: FakeAnalysis, solver: FakeSolver) -> None:
        self.built += 1
        self.analysis = analysis
        self.solver = solver

    def setup_working_dir(self, directory: str) -> None:
        if self._setup_fails:
            raise RuntimeError("the CalculiX working directory could not be created")
        self.working_directory = directory

    def update_objects(self) -> None:
        return

    def check_prerequisites(self) -> str:
        return self._prerequisites

    def run(self) -> None:
        if self._run_fails:
            raise RuntimeError("ccx exited with status 1")
        self.ran += 1
        if self._result_name is None:
            return
        directory = Path(self.working_directory)
        directory.mkdir(parents=True, exist_ok=True)
        (directory / self._result_name).write_bytes(self._result_payload)
        if self.analysis is not None:
            self.analysis.document.Objects.append(FakeResultObject(node_count=8, tetra_count=3))


class FakeCcxTools:
    """The ``femtools.ccxtools`` module, handing out one bridge object."""

    def __init__(self, tools: FakeFemToolsCcx) -> None:
        self.tools = tools
        self.built = 0

    def FemToolsCcx(self, analysis: FakeAnalysis, solver: FakeSolver) -> FakeFemToolsCcx:
        self.tools.bind(analysis, solver)
        return self.tools


def _default_shape(kind: str, first: float, second: float, _third: float) -> object:
    if kind == "cylinder":
        return FakeCylinderShape(radius=first, height=second)
    return FakeShape(length=first, width=second, height=_third)


def raising_shape(message: str) -> ShapeBuilder:
    """A Part factory that refuses the extents it was handed."""

    def refuse(_kind: str, *_extents: float) -> object:
        raise ValueError(message)

    return refuse


def silent_shape() -> ShapeBuilder:
    """A Part factory that builds nothing at all."""

    def build_nothing(_kind: str, *_extents: float) -> object:
        return None

    return build_nothing


def modules_for(
    *,
    freecad: FakeFreeCAD | None = None,
    part: object | None = None,
    objects_fem: object | None = None,
    ccx_tools: object | None = None,
    version: str = "1.1.3",
    import_root: str = "/usr/lib/freecad/lib",
    solver_bridge_detail: str = "",
    import_order: Sequence[str] = ("FreeCAD", "Part", "ObjectsFem", "femtools.ccxtools"),
) -> FreeCADModules:
    return FreeCADModules(
        freecad=freecad if freecad is not None else FakeFreeCAD(),
        part=part if part is not None else FakePart(),
        objects_fem=objects_fem if objects_fem is not None else FakeObjectsFem(),
        ccx_tools=ccx_tools,
        version=version,
        import_root=import_root,
        import_order=tuple(import_order),
        solver_bridge_detail=solver_bridge_detail,
    )


@contextmanager
def session_for(
    modules: FreeCADModules,
    noted: list[list[str]] | None = None,
) -> Iterator[FreeCADModules]:
    """A stand-in for the FreeCAD session, with the real teardown shape."""
    try:
        yield modules
    finally:
        open_documents = list(getattr(modules.freecad, "documents", {}))
        if noted is not None:
            noted.append(open_documents)
        for name in open_documents:
            modules.freecad.closeDocument(name)


def session_factory(
    modules: FreeCADModules,
    noted: list[list[str]] | None = None,
) -> Callable[..., AbstractContextManager[FreeCADModules]]:
    """A ``freecad_session`` stand-in that accepts the real call signature."""

    def open_session(*_roots: object, **_options: object) -> AbstractContextManager[FreeCADModules]:
        return session_for(modules, noted)

    return open_session


def calculator(workspace: Path) -> Path:
    """Write a small executable that a safe-solver check accepts."""
    program = workspace / "ccx"
    program.write_bytes(b"#!/bin/sh\nexit 0\n")
    program.chmod(0o755)
    return program


def geometry_arguments(**overrides: object) -> Mapping[str, object]:
    values: dict[str, object] = {
        "export_formats": ["step"],
        "height_mm": 20.0,
        "length_mm": 120.0,
        "shape": "box",
        "width_mm": 40.0,
    }
    values.update(overrides)
    return values


def fem_arguments(**overrides: object) -> Mapping[str, object]:
    values: dict[str, object] = {
        "height_mm": 20.0,
        "length_mm": 120.0,
        "shape": "box",
        "width_mm": 40.0,
    }
    values.update(overrides)
    return values
