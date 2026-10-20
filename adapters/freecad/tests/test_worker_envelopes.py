"""Worker envelopes over a fake FreeCAD call.

The kernel only ever sees one envelope per call, so these tests drive the
dispatched entry points with a fake module and check the whole contract: the
workspace the envelope names, the error code a refusal carries, the schema
the result has to satisfy, and the artifacts the kernel will re-verify.
"""

from __future__ import annotations

import json
from pathlib import Path
from types import ModuleType
from typing import Any, Final, cast

import pytest

from conftest import BOX_ARGUMENTS, FEM_BOX_ARGUMENTS, make_fem_request, make_request
from freecad_fakes import (
    RESULT_PAYLOAD,
    STEP_PAYLOAD,
    FakeCcxTools,
    FakeFemToolsCcx,
    FakeFreeCAD,
    FakePart,
    calculator,
    modules_for,
    session_factory,
)
from kala_agent.contracts import ToolRunStatus
from kala_agent.tools.protocol import (
    JSONValue,
    ToolErrorCode,
    ToolRequestEnvelope,
    ToolResponseEnvelope,
    validate_against_schema,
)
from kala_agent_freecad import worker
from kala_agent_freecad.cad import design, fem, geometry
from kala_agent_freecad.cad.runtime import FreeCADModules
from kala_agent_freecad.contracts import (
    DESIGN_OUTPUT_SCHEMA,
    DESIGN_TOOL_ID,
    DESIGN_TOOL_VERSION,
    FEM_OUTPUT_SCHEMA,
    GEOMETRY_OUTPUT_SCHEMA,
)
from kala_agent_freecad.errors import FreeCADDependencyError

SMALL_BUDGET: Final[int] = 4
CALL_ID: Final[str] = "kala-call-1"


def _tampered(request: ToolRequestEnvelope, workspace: str) -> ToolRequestEnvelope:
    object.__setattr__(request, "workspace", workspace)
    return request


def _use(
    monkeypatch: pytest.MonkeyPatch,
    target: ModuleType,
    modules: FreeCADModules,
) -> None:
    monkeypatch.setattr(target, "freecad_session", session_factory(modules))


def _result(response: ToolResponseEnvelope) -> dict[str, Any]:
    assert response.status is ToolRunStatus.SUCCEEDED, response.error_message
    assert response.result is not None
    return cast("dict[str, Any]", response.result)


def test_a_workspace_that_is_not_absolute_is_refused(tmp_path: Path) -> None:
    request = _tampered(make_request(dict(BOX_ARGUMENTS), workspace=tmp_path), "artifacts")
    response = worker.invoke_geometry(request)
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.WORKSPACE_VIOLATION.value
    assert "not an absolute bounded path" in str(response.error_message)
    assert response.artifacts == ()
    assert response.result is None


def test_a_fem_workspace_that_is_not_absolute_is_refused(tmp_path: Path) -> None:
    request = _tampered(
        make_fem_request(dict(FEM_BOX_ARGUMENTS), workspace=tmp_path),
        "artifacts",
    )
    response = worker.invoke_fem(request)
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.WORKSPACE_VIOLATION.value
    assert response.artifacts == ()


def test_a_workspace_beyond_the_bound_is_refused(tmp_path: Path) -> None:
    request = _tampered(
        make_request(dict(BOX_ARGUMENTS), workspace=tmp_path),
        "/" + "w" * 2_000,
    )
    response = worker.invoke_geometry(request)
    assert response.error_code == ToolErrorCode.WORKSPACE_VIOLATION.value
    assert list(tmp_path.iterdir()) == []


def test_a_geometry_call_publishes_verifiable_artifacts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _use(monkeypatch, geometry, modules_for())
    response = worker.invoke_geometry(make_request(dict(BOX_ARGUMENTS), workspace=tmp_path))
    result = _result(response)
    validate_against_schema(result, GEOMETRY_OUTPUT_SCHEMA, path="$")
    assert result["shape"] == "box"
    assert response.error_code is None
    assert response.result_json is not None
    assert [artifact.artifact_id for artifact in response.artifacts] == [
        "kala-call-1-step",
        "kala-call-1-iges",
        "kala-call-1-fcstd",
    ]
    for artifact in response.artifacts:
        target = Path(artifact.uri.removeprefix("file://"))
        assert target.is_file()
        assert target.is_relative_to(tmp_path)
        assert target.stat().st_size == artifact.size_bytes
    assert (tmp_path / "artifacts" / "kala-model.step").read_bytes() == STEP_PAYLOAD
    assert sorted(entry.name for entry in (tmp_path / "artifacts").iterdir()) == [
        "kala-model.fcstd",
        "kala-model.iges",
        "kala-model.step",
    ]


def test_an_export_failure_is_reported_as_an_internal_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _use(monkeypatch, geometry, modules_for(part=FakePart(export_fails=True)))
    response = worker.invoke_geometry(make_request(dict(BOX_ARGUMENTS), workspace=tmp_path))
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.INTERNAL_ERROR.value
    assert "could not write the step export" in str(response.error_message)
    assert response.artifacts == ()
    assert list((tmp_path / "artifacts").iterdir()) == []


def test_an_export_above_the_budget_is_reported_as_too_large(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _use(monkeypatch, geometry, modules_for())
    request = make_request(
        dict(BOX_ARGUMENTS),
        workspace=tmp_path,
        max_artifact_bytes=SMALL_BUDGET,
    )
    response = worker.invoke_geometry(request)
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.OUTPUT_TOO_LARGE.value
    assert response.artifacts == ()


def test_a_dependency_failure_is_reported_as_sandbox_unavailable(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def unavailable(*_roots: object, **_options: object) -> object:
        raise FreeCADDependencyError("no FreeCAD import root is on the loader path")

    monkeypatch.setattr(geometry, "freecad_session", unavailable)
    response = worker.invoke_geometry(make_request(dict(BOX_ARGUMENTS), workspace=tmp_path))
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.SANDBOX_UNAVAILABLE.value
    assert "no FreeCAD import root" in str(response.error_message)
    assert list(tmp_path.iterdir()) == []


def test_a_fem_call_without_a_solver_invents_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(worker, "resolve_calculix", lambda: "")
    _use(monkeypatch, fem, modules_for())
    response = worker.invoke_fem(make_fem_request(dict(FEM_BOX_ARGUMENTS), workspace=tmp_path))
    result = _result(response)
    validate_against_schema(result, FEM_OUTPUT_SCHEMA, path="$")
    solver = cast("dict[str, Any]", result["solver"])
    assert solver["status"] == "solver_unavailable"
    assert solver["executed"] is False
    assert solver["available"] is False
    assert solver["executable"] is None
    results = cast("dict[str, Any]", result["results"])
    assert results["available"] is False
    assert results["max_displacement_mm"] is None
    assert results["max_stress_mpa"] is None
    assert [artifact.artifact_id for artifact in response.artifacts] == [
        "kala-fem-call-1-fcstd",
        "kala-fem-call-1-step",
    ]


def test_a_fem_call_reports_a_completed_solve_with_its_result(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    solver_path = calculator(tmp_path)
    monkeypatch.setattr(worker, "resolve_calculix", lambda: solver_path.as_posix())
    _use(monkeypatch, fem, modules_for(ccx_tools=FakeCcxTools(FakeFemToolsCcx())))
    response = worker.invoke_fem(make_fem_request(dict(FEM_BOX_ARGUMENTS), workspace=tmp_path))
    result = _result(response)
    validate_against_schema(result, FEM_OUTPUT_SCHEMA, path="$")
    solver = cast("dict[str, Any]", result["solver"])
    assert solver["status"] == "completed"
    assert solver["executed"] is True
    assert solver["executable"] == solver_path.as_posix()
    results = cast("dict[str, Any]", result["results"])
    assert results["available"] is True
    assert results["max_displacement_mm"] == 1.5
    assert results["max_stress_mpa"] == 220.0
    assert [artifact.artifact_id for artifact in response.artifacts] == [
        "kala-fem-call-1-fcstd",
        "kala-fem-call-1-step",
        "kala-fem-call-1-frd",
    ]
    assert (tmp_path / "artifacts" / "kala-model.frd").read_bytes() == RESULT_PAYLOAD
    assert response.artifacts[-1].uri.endswith("artifacts/kala-model.frd")


def test_a_fem_call_with_a_broken_bridge_still_reports_the_analysis(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    solver_path = calculator(tmp_path)
    monkeypatch.setattr(worker, "resolve_calculix", lambda: solver_path.as_posix())
    _use(
        monkeypatch,
        fem,
        modules_for(solver_bridge_detail="femtools.ccxtools is unavailable"),
    )
    response = worker.invoke_fem(make_fem_request(dict(FEM_BOX_ARGUMENTS), workspace=tmp_path))
    result = _result(response)
    solver = cast("dict[str, Any]", result["solver"])
    assert solver["status"] == "solver_unavailable"
    assert solver["executed"] is False
    assert solver["message"] == "femtools.ccxtools is unavailable"
    assert cast("dict[str, Any]", result["mesh"])["generated"] is True


def test_a_native_failure_never_reaches_the_caller(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _use(monkeypatch, geometry, modules_for(freecad=FakeFreeCAD(new_document_fails=True)))
    response = worker.invoke_geometry(make_request(dict(BOX_ARGUMENTS), workspace=tmp_path))
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.INTERNAL_ERROR.value
    assert "RuntimeError" in str(response.error_message)
    assert response.result_json is None
    assert list(tmp_path.iterdir()) == []


def test_the_envelope_names_the_call_it_answers(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _use(monkeypatch, geometry, modules_for())
    request = make_request(dict(BOX_ARGUMENTS), workspace=tmp_path, call_id=CALL_ID)
    response = worker.invoke_geometry(request)
    assert response.call_id == CALL_ID
    assert response.tool_id == request.tool_id
    assert response.tool_version == request.tool_version
    assert response.kind.value == "response"
    entries = cast("list[JSONValue]", _result(response)["artifacts"])
    assert len(entries) == len(response.artifacts) == 3


DESIGN_TRACE_LENGTH: Final[int] = 85
DESIGN_RESULT: Final[dict[str, JSONValue]] = {
    "analysis_detail": "analysis was not requested",
    "analysis_status": "not_run",
    "artifacts": [
        {
            "digest": "0" * 64,
            "format": "step",
            "media_type": "model/step",
            "relative_path": "artifacts/kala-model.step",
            "size_bytes": 2_048,
        }
    ],
    "degrees_of_freedom": 0,
    "document_name": "kala-freecad",
    "freecad_version": "1.1.4",
    "import_root": "/usr/lib/freecad/lib",
    "measurement": {
        "area_mm2": 11_889.101731,
        "bounding_box_mm": [60.0, 45.0, 40.0],
        "edge_count": 36,
        "face_count": 24,
        "solid": True,
        "vertex_count": 72,
        "volume_mm3": 35_571.416047,
    },
    "object_count": 1,
    "object_names": ["Pad"],
    "redesign_trace": [],
    "stage": "solid",
    "tool_trace": [f"call-{index}" for index in range(DESIGN_TRACE_LENGTH)],
    "valid": True,
    "warnings": [],
}


def test_the_design_schema_accepts_the_builder_trace_cap() -> None:
    properties = cast("dict[str, Any]", DESIGN_OUTPUT_SCHEMA["properties"])
    trace = cast("dict[str, Any]", properties["tool_trace"])
    assert trace["maxItems"] == design.MAX_TRACE
    assert design.MAX_TRACE > 64


def test_invoke_design_accepts_a_trace_longer_than_sixty_four(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def fake_run(*_args: object, **_kwargs: object) -> design.DesignOutcome:
        return design.DesignOutcome(result=dict(DESIGN_RESULT), artifacts=())

    monkeypatch.setattr(design, "run", fake_run)
    request = make_request(
        {
            "calls_json": json.dumps(
                [
                    {"tool": "part.cylinder", "arguments": {"radius": 5, "height": 10}},
                    {"tool": "partdesign.check"},
                ]
            )
        },
        workspace=tmp_path,
        tool_id=DESIGN_TOOL_ID,
        tool_version=DESIGN_TOOL_VERSION,
    )
    response = worker.invoke_design(request)
    result = _result(response)
    validate_against_schema(result, DESIGN_OUTPUT_SCHEMA, path="$")
    assert len(cast("list[JSONValue]", result["tool_trace"])) == DESIGN_TRACE_LENGTH
    assert response.error_code is None
