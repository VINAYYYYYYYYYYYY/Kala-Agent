"""FEM work that runs inside a FreeCAD worker.

The analysis is always built: a solid, a solid material, a fixed constraint, a
Gmsh mesh object, and a CalculiX solver object are added to one Fem analysis
container. The solver only ever runs when a safely discovered ``ccx``
executable exists; without one the call reports ``solver_unavailable``, the
result statistics stay empty, and nothing is invented.
"""

from __future__ import annotations

import math
import shutil
from collections.abc import Mapping, Sequence, Sized
from dataclasses import dataclass
from pathlib import Path
from typing import Final

from kala_agent.contracts import ArtifactRef
from kala_agent.tools.protocol import JSONValue
from kala_agent_freecad.availability import is_safe_calculix
from kala_agent_freecad.cad.arguments import (
    MAX_DENSITY_KG_M3,
    MAX_FACE_INDEX,
    MAX_POISSON_RATIO,
    MAX_RADIUS_MM,
    MAX_YOUNGS_MODULUS_MPA,
    dimension,
    flag,
    integer,
    member,
    positive,
    ratio,
    text,
)
from kala_agent_freecad.cad.artifacts import (
    PublishedArtifact,
    bounded_label,
    discard,
    finalize,
    optional_message,
    prepare_target,
    resolve_target,
)
from kala_agent_freecad.cad.runtime import (
    FreeCADModules,
    FreeCADObject,
    freecad_session,
)
from kala_agent_freecad.contracts import (
    ARTIFACT_DIRECTORY,
    DEFAULT_ANALYSIS_NAME,
    DEFAULT_ARTIFACT_STEM,
    DEFAULT_DOCUMENT_NAME,
    DEFAULT_MATERIAL_NAME,
    EXPORT_SUFFIXES,
    SHAPE_KINDS,
    SOLVER_ENGINE,
)
from kala_agent_freecad.errors import (
    FreeCADAdapterError,
    FreeCADArtifactError,
    FreeCADInputError,
    FreeCADOperationError,
    FreeCADWorkspaceError,
)

MAX_DIMENSION_MM: Final[float] = 5_000.0
DEFAULT_YOUNGS_MODULUS_MPA: Final[float] = 210_000.0
DEFAULT_POISSON_RATIO: Final[float] = 0.3
DEFAULT_DENSITY_KG_M3: Final[float] = 7_850.0
DEFAULT_OBJECT_NAME_LENGTH: Final[int] = 64
MAX_VERSION_LENGTH: Final[int] = 64
MAX_PATH_LENGTH: Final[int] = 1_024
MAX_MESSAGE_LENGTH: Final[int] = 512
SHAPE_OBJECT_NAME: Final[str] = "Shape"
MATERIAL_OBJECT_NAME: Final[str] = "Material"
CONSTRAINT_OBJECT_NAME: Final[str] = "Fixed"
MESH_OBJECT_NAME: Final[str] = "Mesh"
SOLVER_OBJECT_NAME: Final[str] = "Solver"
RESULT_TYPE_ID: Final[str] = "Fem::FemResultObjectPython"
ELEMENT_COUNT_ATTRIBUTES: Final[tuple[str, ...]] = (
    "TetraCount",
    "TriangleCount",
    "QuadrangleCount",
    "PyramidCount",
    "PrismCount",
    "HexaCount",
)


@dataclass(frozen=True, slots=True)
class FemRequest:
    shape: str
    document_name: str
    analysis_name: str
    artifact_stem: str
    material_name: str
    length_mm: float
    width_mm: float
    height_mm: float
    radius_mm: float
    youngs_modulus_mpa: float
    poisson_ratio: float
    density_kg_m3: float
    fixed_face: int
    run_solver: bool
    export_solid: bool


@dataclass(frozen=True, slots=True)
class FemOutcome:
    result: dict[str, JSONValue]
    artifacts: tuple[ArtifactRef, ...]


@dataclass(frozen=True, slots=True)
class _AnalysisObjects:
    analysis_name: str
    material_name: str
    constraint_name: str
    mesh_name: str
    solver_name: str
    analysis: FreeCADObject
    mesh: FreeCADObject
    solver: FreeCADObject
    feature: FreeCADObject


@dataclass(frozen=True, slots=True)
class _SolverReport:
    status: str
    message: str
    executed: bool
    executable: str


def parse_request(arguments: Mapping[str, object]) -> FemRequest:
    """Turn validated arguments into a request, refusing anything ambiguous."""
    if not isinstance(arguments, Mapping):
        raise FreeCADInputError("arguments must be a JSON object")
    shape = member(arguments, "shape", SHAPE_KINDS)
    if shape == "box" and "radius_mm" in arguments:
        raise FreeCADInputError("radius_mm only applies to a cylinder")
    if shape == "cylinder" and ("length_mm" in arguments or "width_mm" in arguments):
        raise FreeCADInputError("length_mm and width_mm only apply to a box")
    return FemRequest(
        shape=shape,
        document_name=text(arguments, "document_name", DEFAULT_DOCUMENT_NAME),
        analysis_name=text(arguments, "analysis_name", DEFAULT_ANALYSIS_NAME),
        artifact_stem=text(arguments, "artifact_stem", DEFAULT_ARTIFACT_STEM),
        material_name=text(arguments, "material_name", DEFAULT_MATERIAL_NAME),
        length_mm=dimension(arguments, "length_mm", MAX_DIMENSION_MM),
        width_mm=dimension(arguments, "width_mm", MAX_DIMENSION_MM),
        height_mm=dimension(arguments, "height_mm", MAX_DIMENSION_MM),
        radius_mm=dimension(arguments, "radius_mm", MAX_RADIUS_MM),
        youngs_modulus_mpa=positive(
            arguments,
            "youngs_modulus_mpa",
            DEFAULT_YOUNGS_MODULUS_MPA,
            MAX_YOUNGS_MODULUS_MPA,
        ),
        poisson_ratio=ratio(arguments, "poisson_ratio", DEFAULT_POISSON_RATIO),
        density_kg_m3=positive(
            arguments,
            "density_kg_m3",
            DEFAULT_DENSITY_KG_M3,
            MAX_DENSITY_KG_M3,
        ),
        fixed_face=integer(arguments, "fixed_face", 1, MAX_FACE_INDEX),
        run_solver=flag(arguments, "run_solver", default=False),
        export_solid=flag(arguments, "export_solid", default=True),
    )


def run(
    request: FemRequest,
    *,
    workspace: Path,
    import_roots: Sequence[Path],
    calculix_path: str,
    max_artifact_bytes: int,
    call_id: str,
) -> FemOutcome:
    """Build the analysis, publish the document, and report the solver status."""
    safe_solver = calculix_path if is_safe_calculix(calculix_path) else ""
    with freecad_session(import_roots) as modules:
        document = _new_document(modules.freecad, request.document_name)
        try:
            built = _build_analysis(modules, document, request)
            _recompute(document)
            report = _run_solver(modules, built, request, safe_solver, workspace)
            published = _export_all(
                modules,
                document,
                built,
                request,
                workspace=workspace,
                max_artifact_bytes=max_artifact_bytes,
                call_id=call_id,
            )
            if report.status == "completed":
                published += _publish_solver_result(
                    request,
                    workspace=workspace,
                    max_artifact_bytes=max_artifact_bytes,
                    call_id=call_id,
                )
            result: dict[str, JSONValue] = {
                "analysis_name": bounded_label(
                    request.analysis_name,
                    DEFAULT_OBJECT_NAME_LENGTH,
                ),
                "artifacts": [item.entry() for item in published],
                "document_name": bounded_label(request.document_name, MAX_PATH_LENGTH),
                "freecad_version": bounded_label(modules.version, MAX_VERSION_LENGTH),
                "import_root": bounded_label(modules.import_root, MAX_PATH_LENGTH),
                "material": {
                    "density_kg_m3": request.density_kg_m3,
                    "name": bounded_label(request.material_name, DEFAULT_OBJECT_NAME_LENGTH),
                    "object_name": built.material_name,
                    "poisson_ratio": request.poisson_ratio,
                    "youngs_modulus_mpa": request.youngs_modulus_mpa,
                },
                "mesh": mesh_summary(built.mesh),
                "objects": {
                    "analysis": built.analysis_name,
                    "constraint": built.constraint_name,
                    "material": built.material_name,
                    "mesh": built.mesh_name,
                    "solver": built.solver_name,
                },
                "results": result_summary(document),
                "shape": request.shape,
                "solver": {
                    "available": bool(report.executable),
                    "engine": SOLVER_ENGINE,
                    "executable": report.executable or None,
                    "executed": report.executed,
                    "message": optional_message(report.message, MAX_MESSAGE_LENGTH),
                    "object_name": built.solver_name,
                    "status": report.status,
                },
                "warnings": [],
            }
        finally:
            _close(modules.freecad, document)
    return FemOutcome(
        result=result,
        artifacts=tuple(item.artifact for item in published),
    )


def _new_document(freecad: FreeCADObject, name: str) -> FreeCADObject:
    try:
        return freecad.newDocument(name)
    except Exception as error:
        raise FreeCADOperationError(
            f"FreeCAD could not open a document: {type(error).__name__}: {error}"
        ) from error


def _build_analysis(
    modules: FreeCADModules,
    document: FreeCADObject,
    request: FemRequest,
) -> _AnalysisObjects:
    fem = modules.objects_fem
    try:
        feature = document.addObject("Part::Feature", SHAPE_OBJECT_NAME)
        feature.Shape = _build_shape(modules, request)
        document.recompute()
        analysis = _make(fem, "makeAnalysis", document, request.analysis_name)
        material = _make(fem, "makeMaterialSolid", document, MATERIAL_OBJECT_NAME)
        material.Material = _material_card(request)
        constraint = _make(fem, "makeConstraintFixed", document, CONSTRAINT_OBJECT_NAME)
        constraint.References = [(feature, f"Face{request.fixed_face}")]
        mesh = _make(fem, "makeMeshGmsh", document, MESH_OBJECT_NAME)
        mesh.Shape = feature
        solver = _make(fem, "makeSolverCalculiXCcxTools", document, SOLVER_OBJECT_NAME)
        for member_object in (material, constraint, mesh, solver):
            analysis.addObject(member_object)
        document.recompute()
    except FreeCADAdapterError:
        raise
    except Exception as error:
        raise FreeCADOperationError(
            f"FreeCAD could not build the Fem analysis: {type(error).__name__}: {error}"
        ) from error
    return _AnalysisObjects(
        analysis_name=_object_name(analysis),
        material_name=_object_name(material),
        constraint_name=_object_name(constraint),
        mesh_name=_object_name(mesh),
        solver_name=_object_name(solver),
        analysis=analysis,
        mesh=mesh,
        solver=solver,
        feature=feature,
    )


def _material_card(request: FemRequest) -> dict[str, str]:
    return {
        "Density": f"{request.density_kg_m3} kg/m^3",
        "Name": request.material_name,
        "PoissonRatio": str(request.poisson_ratio),
        "YoungsModulus": f"{request.youngs_modulus_mpa} MPa",
    }


def _make(fem: FreeCADObject, factory: str, document: FreeCADObject, name: str) -> FreeCADObject:
    builder = getattr(fem, factory, None)
    if not callable(builder):
        raise FreeCADOperationError(f"the FreeCAD Fem module does not offer {factory}")
    return builder(document, name)


def _build_shape(modules: FreeCADModules, request: FemRequest) -> FreeCADObject:
    factory_name = "makeBox" if request.shape == "box" else "makeCylinder"
    factory = getattr(modules.part, factory_name, None)
    if not callable(factory):
        raise FreeCADOperationError(f"the FreeCAD Part module does not offer {factory_name}")
    try:
        if request.shape == "box":
            return factory(request.length_mm, request.width_mm, request.height_mm)
        return factory(request.radius_mm, request.height_mm)
    except Exception as error:
        raise FreeCADOperationError(
            f"FreeCAD refused the {request.shape} dimensions: {type(error).__name__}: {error}"
        ) from error


def _recompute(document: FreeCADObject) -> None:
    try:
        document.recompute()
    except Exception as error:
        raise FreeCADOperationError(
            f"FreeCAD could not recompute the document: {type(error).__name__}: {error}"
        ) from error


def _run_solver(
    modules: FreeCADModules,
    built: _AnalysisObjects,
    request: FemRequest,
    safe_solver: str,
    workspace: Path,
) -> _SolverReport:
    """Run the solver only behind a safe executable, and report what happened."""
    if not request.run_solver:
        return _SolverReport("not_requested", "the caller did not ask for a solve", False, "")
    if not safe_solver:
        return _SolverReport(
            "solver_unavailable",
            "no safe CalculiX executable was discovered, so no solve was attempted",
            False,
            "",
        )
    if modules.ccx_tools is None:
        return _SolverReport(
            "solver_unavailable",
            modules.solver_bridge_detail or "the CalculiX bridge is unavailable",
            False,
            safe_solver,
        )
    working_directory = _solver_working_directory(workspace)
    try:
        tools = modules.ccx_tools.FemToolsCcx(built.analysis, built.solver)
        tools.setup_working_dir(str(working_directory))
        tools.update_objects()
        problems = tools.check_prerequisites()
    except Exception as error:
        return _SolverReport(
            "failed",
            f"the CalculiX bridge refused the analysis: {type(error).__name__}: {error}",
            False,
            safe_solver,
        )
    if isinstance(problems, str) and problems.strip():
        return _SolverReport(
            "prerequisites_unmet",
            problems.strip(),
            False,
            safe_solver,
        )
    try:
        tools.run()
    except Exception as error:
        return _SolverReport(
            "failed",
            f"the CalculiX solve did not complete: {type(error).__name__}: {error}",
            False,
            safe_solver,
        )
    return _SolverReport("completed", "the CalculiX solve completed", True, safe_solver)


def _solver_working_directory(workspace: Path) -> Path:
    try:
        return resolve_target(workspace, ARTIFACT_DIRECTORY)
    except FreeCADWorkspaceError:  # pragma: no cover - the resolver is authoritative
        return workspace


def mesh_summary(mesh: FreeCADObject) -> dict[str, JSONValue]:
    """Describe the mesh object as the result payload reports it."""
    nodes, elements = mesh_counts(mesh)
    return {
        "element_count": elements,
        "generated": elements > 0 and nodes > 0,
        "node_count": nodes,
        "object_name": _object_name(mesh),
    }


def mesh_counts(mesh: FreeCADObject) -> tuple[int, int]:
    """Return the node and element counts a mesh object really holds."""
    fem_mesh = getattr(mesh, "FemMesh", None)
    if fem_mesh is None:
        return 0, 0
    nodes = _count(getattr(fem_mesh, "NodeCount", 0))
    elements = 0
    for attribute in ELEMENT_COUNT_ATTRIBUTES:
        elements += _count(getattr(fem_mesh, attribute, 0))
    return nodes, elements


def result_summary(document: FreeCADObject) -> dict[str, JSONValue]:
    """Read a real result object, or report that no result exists."""
    result_object = _first_object(document, RESULT_TYPE_ID)
    if result_object is None:
        return {
            "available": False,
            "element_count": 0,
            "max_displacement_mm": None,
            "max_stress_mpa": None,
            "message": "no Fem result object exists in the analysis",
            "node_count": 0,
        }
    nodes, elements = mesh_counts(getattr(result_object, "Mesh", None))
    displacement = _extreme(getattr(result_object, "DisplacementLengths", None))
    stress = _extreme(getattr(result_object, "vonMises", None))
    return {
        "available": displacement is not None or stress is not None,
        "element_count": elements,
        "max_displacement_mm": displacement,
        "max_stress_mpa": stress,
        "message": None
        if displacement is not None or stress is not None
        else ("the Fem result object carries no displacement or stress values"),
        "node_count": nodes,
    }


def _extreme(values: FreeCADObject) -> float | None:
    if not isinstance(values, (list, tuple)) or not values:
        return None
    numbers = [float(value) for value in values if _is_number(value)]
    if not numbers:
        return None
    return round(max(numbers), 6)


def _is_number(value: FreeCADObject) -> bool:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return False
    return math.isfinite(float(value))


def _count(value: FreeCADObject) -> int:
    if isinstance(value, bool):
        return 0
    if isinstance(value, (int, float)):
        if not math.isfinite(float(value)) or value < 0:
            return 0
        return int(float(value))
    if isinstance(value, Sized) and not isinstance(value, (str, bytes, bytearray)):
        return max(len(value), 0)
    return 0


def _export_all(
    modules: FreeCADModules,
    document: FreeCADObject,
    built: _AnalysisObjects,
    request: FemRequest,
    *,
    workspace: Path,
    max_artifact_bytes: int,
    call_id: str,
) -> tuple[PublishedArtifact, ...]:
    formats: list[str] = ["fcstd"]
    if request.export_solid:
        formats.append("step")
    published: list[PublishedArtifact] = []
    for export_format in formats:
        relative = f"{ARTIFACT_DIRECTORY}/{request.artifact_stem}{EXPORT_SUFFIXES[export_format]}"
        target = resolve_target(workspace, relative)
        prepare_target(target)
        try:
            if export_format == "fcstd":
                document.saveAs(str(target))
            else:
                modules.part.export([built.feature], str(target))
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


def _publish_solver_result(
    request: FemRequest,
    *,
    workspace: Path,
    max_artifact_bytes: int,
    call_id: str,
) -> tuple[PublishedArtifact, ...]:
    """Publish the CalculiX result file a completed solve left behind.

    The solver writes into the artifact directory, so the result is already
    inside the workspace; it is republished under the call's own stem so the
    reported path, media type, and digest describe exactly one file.
    """
    directory = _solver_working_directory(workspace)
    relative = f"{ARTIFACT_DIRECTORY}/{request.artifact_stem}{EXPORT_SUFFIXES['frd']}"
    target = resolve_target(workspace, relative)
    for candidate in sorted(directory.glob("*.frd")):
        if not candidate.is_file() or candidate.is_symlink():
            continue
        try:
            if candidate.resolve() != target.resolve():
                shutil.copyfile(candidate, target)
        except OSError as error:
            raise FreeCADArtifactError(
                f"the solver result could not be published: {error.strerror}"
            ) from error
        return (
            finalize(
                target,
                relative_path=relative,
                export_format="frd",
                artifact_id=f"{call_id}-frd",
                max_artifact_bytes=max_artifact_bytes,
            ),
        )
    return ()


def _first_object(document: FreeCADObject, type_id: str) -> FreeCADObject | None:
    try:
        candidates = list(document.Objects)
    except Exception:  # pragma: no cover - defensive
        return None
    for candidate in candidates:
        if getattr(candidate, "TypeId", "") == type_id:
            return candidate
    return None


def _object_name(candidate: FreeCADObject) -> str:
    return bounded_label(_raw_name(candidate), DEFAULT_OBJECT_NAME_LENGTH)


def _raw_name(candidate: FreeCADObject) -> str:
    name = getattr(candidate, "Name", "")
    return name if isinstance(name, str) else ""


def _close(freecad: FreeCADObject, document: FreeCADObject) -> None:
    name = getattr(document, "Name", None)
    if not isinstance(name, str) or not name:
        return
    try:
        freecad.closeDocument(name)
    except Exception:  # pragma: no cover - teardown noise
        return


__all__ = (
    "DEFAULT_ANALYSIS_NAME",
    "DEFAULT_DENSITY_KG_M3",
    "DEFAULT_MATERIAL_NAME",
    "DEFAULT_POISSON_RATIO",
    "DEFAULT_YOUNGS_MODULUS_MPA",
    "ELEMENT_COUNT_ATTRIBUTES",
    "MAX_DENSITY_KG_M3",
    "MAX_FACE_INDEX",
    "MAX_POISSON_RATIO",
    "MAX_YOUNGS_MODULUS_MPA",
    "RESULT_TYPE_ID",
    "SHAPE_KINDS",
    "FemOutcome",
    "FemRequest",
    "mesh_counts",
    "mesh_summary",
    "parse_request",
    "result_summary",
    "run",
)
