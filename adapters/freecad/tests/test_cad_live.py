"""Tests that drive a real FreeCAD installation.

Every test here is marked ``freecad`` and skips cleanly when discovery cannot
find an installation, so the suite is still meaningful on a host without it.
Nothing here writes outside ``tmp_path`` and nothing here reaches the network.
"""

from __future__ import annotations

import json
import math
import subprocess
import sys
from collections.abc import Mapping
from pathlib import Path
from typing import Any, Final, cast

import pytest

from conftest import (
    BOX_ARGUMENTS,
    CYLINDER_ARGUMENTS,
    FEM_BOX_ARGUMENTS,
    make_fem_request,
    make_request,
)
from kala_agent.contracts import ArtifactClassification, ToolRunStatus
from kala_agent.contracts.construction import ConstructionScript
from kala_agent.studio.design.templates import l_bracket
from kala_agent.tools.protocol import (
    JSONValue,
    ToolErrorCode,
    ToolResponseEnvelope,
    sha256_hex,
    validate_against_schema,
)
from kala_agent_freecad.availability import FreeCADAvailability
from kala_agent_freecad.cad.compile import compile_construction
from kala_agent_freecad.cad.runtime import (
    FREECAD_IMPORT_ORDER,
    LIB_ROOT_ENVIRONMENT_KEY,
    freecad_session,
)
from kala_agent_freecad.contracts import (
    DESIGN_OUTPUT_SCHEMA,
    DESIGN_TOOL_ID,
    DESIGN_TOOL_VERSION,
    FEM_OUTPUT_SCHEMA,
    GEOMETRY_OUTPUT_SCHEMA,
)
from kala_agent_freecad.worker import invoke_design, invoke_fem, invoke_geometry

pytestmark = pytest.mark.freecad

ADAPTER_PACKAGE_ROOT: Final[Path] = Path(__file__).resolve().parent.parent / "src"
LOADER_TIMEOUT_SECONDS: Final[float] = 300.0


def _result(response: ToolResponseEnvelope) -> Mapping[str, JSONValue]:
    assert response.status is ToolRunStatus.SUCCEEDED, response.error_message
    assert response.result is not None
    return response.result


def _measurement(response: ToolResponseEnvelope) -> Mapping[str, JSONValue]:
    measurement = _result(response)["measurement"]
    assert isinstance(measurement, Mapping)
    return measurement


def test_geometry_builds_measures_and_exports(
    tmp_path: Path, import_roots: tuple[Path, ...]
) -> None:
    response = invoke_geometry(make_request(dict(BOX_ARGUMENTS), workspace=tmp_path))
    result = _result(response)
    validate_against_schema(result, GEOMETRY_OUTPUT_SCHEMA, path="$")
    measurement = _measurement(response)
    assert measurement["volume_mm3"] == pytest.approx(120.0 * 40.0 * 20.0, rel=1e-6)
    assert measurement["face_count"] == 6
    assert measurement["edge_count"] == 12
    assert measurement["vertex_count"] == 8
    assert measurement["solid"] is True
    assert measurement["bounding_box_mm"] == [120.0, 40.0, 20.0]
    assert result["valid"] is True
    assert result["shape"] == "box"
    assert str(result["freecad_version"]).count(".") == 2
    assert len(response.artifacts) == 3
    assert {artifact.artifact_id for artifact in response.artifacts} == {
        "kala-call-1-step",
        "kala-call-1-iges",
        "kala-call-1-fcstd",
    }
    for artifact in response.artifacts:
        target = tmp_path / "artifacts" / Path(artifact.uri.removeprefix("file://")).name
        assert target.is_file()
        assert target.resolve().is_relative_to(tmp_path.resolve())
        assert target.stat().st_size == artifact.size_bytes
        assert sha256_hex(target.read_bytes()) == artifact.digest
        assert artifact.classification is ArtifactClassification.DESIGN_BINARY
    assert (tmp_path / "artifacts" / "kala-model.step").read_bytes().startswith(b"ISO-10303")


def test_geometry_cylinder_volume_is_analytic(
    tmp_path: Path,
    import_roots: tuple[Path, ...],
) -> None:
    response = invoke_geometry(make_request(dict(CYLINDER_ARGUMENTS), workspace=tmp_path))
    measurement = _measurement(response)
    expected = math.pi * 12.5**2 * 60.0
    assert measurement["volume_mm3"] == pytest.approx(expected, rel=1e-4)
    assert measurement["face_count"] == 3
    assert measurement["solid"] is True
    assert len(response.artifacts) == 1


def test_geometry_can_skip_measurement(tmp_path: Path, import_roots: tuple[Path, ...]) -> None:
    arguments: dict[str, Any] = dict(BOX_ARGUMENTS)
    arguments["measure"] = False
    response = invoke_geometry(make_request(cast("Any", arguments), workspace=tmp_path))
    assert _measurement(response)["volume_mm3"] == 0.0
    assert len(response.artifacts) == 3


def test_geometry_confines_every_write_to_the_workspace(
    tmp_path: Path,
    import_roots: tuple[Path, ...],
) -> None:
    workspace = tmp_path / "call"
    workspace.mkdir()
    sibling = tmp_path / "escape.step"
    response = invoke_geometry(make_request(dict(BOX_ARGUMENTS), workspace=workspace))
    _result(response)
    assert not sibling.exists()
    written = sorted(path.relative_to(workspace).as_posix() for path in workspace.rglob("*"))
    assert all(not entry.startswith("..") for entry in written)
    assert {entry for entry in written if entry.endswith((".step", ".iges", ".fcstd"))} == {
        "artifacts/kala-model.step",
        "artifacts/kala-model.iges",
        "artifacts/kala-model.fcstd",
    }


def test_geometry_stays_inside_the_artifact_budget(
    tmp_path: Path,
    import_roots: tuple[Path, ...],
) -> None:
    request = make_request(dict(BOX_ARGUMENTS), workspace=tmp_path, max_artifact_bytes=1_024)
    response = invoke_geometry(request)
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code in {
        ToolErrorCode.OUTPUT_TOO_LARGE.value,
        ToolErrorCode.ARTIFACT_REJECTED.value,
    }
    assert response.artifacts == ()


def test_fem_builds_an_analysis_without_a_solver(
    tmp_path: Path,
    import_roots: tuple[Path, ...],
) -> None:
    response = invoke_fem(make_fem_request(dict(FEM_BOX_ARGUMENTS), workspace=tmp_path))
    result = _result(response)
    validate_against_schema(result, FEM_OUTPUT_SCHEMA, path="$")
    objects = cast("Mapping[str, JSONValue]", result["objects"])
    assert set(objects) == {"analysis", "constraint", "material", "mesh", "solver"}
    assert all(str(name) for name in objects.values())
    assert result["analysis_name"] == "KalaAnalysis"
    material = cast("Mapping[str, JSONValue]", result["material"])
    assert material["youngs_modulus_mpa"] == 210_000.0
    assert material["poisson_ratio"] == 0.3
    assert material["density_kg_m3"] == 7_850.0
    assert {artifact.artifact_id for artifact in response.artifacts} == {
        "kala-fem-call-1-fcstd",
        "kala-fem-call-1-step",
    }
    for artifact in response.artifacts:
        target = Path(artifact.uri.removeprefix("file://"))
        assert target.is_file()
        assert sha256_hex(target.read_bytes()) == artifact.digest
        assert target.resolve().is_relative_to(tmp_path.resolve())


def test_fem_reports_a_missing_solver_without_inventing_results(
    tmp_path: Path,
    availability: FreeCADAvailability,
    import_roots: tuple[Path, ...],
) -> None:
    if availability.solver_available:
        pytest.skip("this host has a CalculiX executable, so nothing is missing")
    response = invoke_fem(make_fem_request(dict(FEM_BOX_ARGUMENTS), workspace=tmp_path))
    result = _result(response)
    solver = cast("Mapping[str, JSONValue]", result["solver"])
    assert solver["status"] == "solver_unavailable"
    assert solver["executed"] is False
    assert solver["available"] is False
    assert solver["executable"] is None
    assert "no safe CalculiX executable" in str(solver["message"])
    results = cast("Mapping[str, JSONValue]", result["results"])
    assert results["available"] is False
    assert results["max_displacement_mm"] is None
    assert results["max_stress_mpa"] is None
    assert results["node_count"] == 0
    mesh = cast("Mapping[str, JSONValue]", result["mesh"])
    assert mesh["generated"] is False


def test_fem_without_a_solve_reports_a_clean_skip(
    tmp_path: Path,
    import_roots: tuple[Path, ...],
) -> None:
    arguments: dict[str, Any] = dict(FEM_BOX_ARGUMENTS)
    arguments["run_solver"] = False
    response = invoke_fem(make_fem_request(cast("Any", arguments), workspace=tmp_path))
    solver = cast("Mapping[str, JSONValue]", _result(response)["solver"])
    assert solver["status"] == "not_requested"
    assert solver["executed"] is False
    assert solver["message"] == "the caller did not ask for a solve"


def test_fem_exports_only_the_document_when_asked(
    tmp_path: Path,
    import_roots: tuple[Path, ...],
) -> None:
    arguments: dict[str, Any] = dict(FEM_BOX_ARGUMENTS)
    arguments["export_solid"] = False
    response = invoke_fem(make_fem_request(cast("Any", arguments), workspace=tmp_path))
    assert [artifact.artifact_id for artifact in response.artifacts] == ["kala-fem-call-1-fcstd"]


def test_documents_are_closed_after_every_call(
    tmp_path: Path,
    import_roots: tuple[Path, ...],
) -> None:
    invoke_geometry(make_request(dict(BOX_ARGUMENTS), workspace=tmp_path, call_id="close-1"))
    invoke_fem(
        make_fem_request(
            {"shape": "cylinder", "radius_mm": 5.0, "height_mm": 10.0},
            workspace=tmp_path,
            call_id="close-2",
        )
    )
    freecad = sys.modules.get("FreeCAD")
    if freecad is None:  # pragma: no cover - the calls above load it
        pytest.fail("the calls did not load FreeCAD")
    assert list(freecad.listDocuments()) == []


def test_freecad_is_imported_before_its_workbenches(
    tmp_path: Path,
    require_freecad: FreeCADAvailability,
) -> None:
    root = require_freecad.import_root_paths[0]
    report = tmp_path / "import-order.json"
    source = "\n".join(
        (
            "import json",
            "import sys",
            f"sys.path.insert(0, {str(root)!r})",
            f"sys.path.insert(0, {str(ADAPTER_PACKAGE_ROOT)!r})",
            "from kala_agent_freecad.cad.runtime import freecad_session",
            "with freecad_session() as modules:",
            "    payload = {",
            "        'order': list(modules.import_order),",
            "        'version': modules.version,",
            "        'root': modules.import_root,",
            "    }",
            f"open({str(report)!r}, 'w', encoding='utf-8').write(json.dumps(payload))",
        )
    )
    completed = subprocess.run(
        [sys.executable, "-I", "-B", "-c", source],
        capture_output=True,
        check=False,
        cwd=tmp_path,
        env={
            "HOME": str(tmp_path),
            LIB_ROOT_ENVIRONMENT_KEY: str(root),
            "LANG": "C.UTF-8",
            "PATH": "/usr/local/bin:/usr/bin:/bin",
            "PYTHONDONTWRITEBYTECODE": "1",
        },
        timeout=LOADER_TIMEOUT_SECONDS,
    )
    assert completed.returncode == 0, completed.stderr.decode("utf-8", errors="replace")
    payload = json.loads(report.read_text(encoding="utf-8"))
    assert tuple(payload["order"][:3]) == FREECAD_IMPORT_ORDER[:3]
    assert payload["order"][0] == "FreeCAD"
    assert payload["version"].count(".") == 2
    assert payload["root"] == root.as_posix()


def test_the_session_reports_the_fem_modules_it_loaded(
    import_roots: tuple[Path, ...],
) -> None:
    with freecad_session(import_roots) as modules:
        assert modules.freecad.Version()[0]
        assert modules.part is not None
        assert modules.objects_fem is not None
        assert modules.import_order[:3] == FREECAD_IMPORT_ORDER[:3]
        assert modules.solver_bridge_available == (modules.ccx_tools is not None)
        if modules.ccx_tools is None:
            assert modules.solver_bridge_detail
        document = modules.freecad.newDocument("session-probe")
        assert modules.freecad.listDocuments()
        modules.freecad.closeDocument(document.Name)


ADR_LBRACKET_VOLUME_MM3: Final[float] = 35_571.416047


def test_design_entry_point_builds_the_long_l_bracket_call_list(
    tmp_path: Path,
    import_roots: tuple[Path, ...],
) -> None:
    """The ADR-0009 L-bracket has 85 calls, more than the old 64-entry cap."""
    script: ConstructionScript = l_bracket(
        width=60,
        depth=45,
        height=40,
        wall=8,
        bore_d=10,
        hole_d=5.5,
        hole_count=4,
    ).script
    calls = compile_construction(script)
    assert len(calls) > 64

    request = make_request(
        {"calls_json": json.dumps(calls), "export_formats": ["step", "fcstd"]},
        workspace=tmp_path,
        tool_id=DESIGN_TOOL_ID,
        tool_version=DESIGN_TOOL_VERSION,
    )
    response = invoke_design(request)
    result = _result(response)
    validate_against_schema(result, DESIGN_OUTPUT_SCHEMA, path="$")

    trace = cast("list[JSONValue]", result["tool_trace"])
    assert len(trace) == len(calls)
    measurement = cast("Mapping[str, JSONValue]", result["measurement"])
    assert measurement["volume_mm3"] == pytest.approx(ADR_LBRACKET_VOLUME_MM3, rel=1e-6)
    assert measurement["solid"] is True
    assert result["valid"] is True
    assert result["stage"] == "solid"
    step = tmp_path / "artifacts" / "kala-model.step"
    assert step.read_bytes().startswith(b"ISO-10303-21;")
