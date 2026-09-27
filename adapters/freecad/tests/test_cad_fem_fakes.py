"""Fem analysis work driven by fake FreeCAD modules.

The solver is the part of this adapter that could do real damage, so every
status it can report is proved here against a bridge that misbehaves in one
named way: a bridge that will not set up, unmet prerequisites, a solve that
exits nonzero, and a result file that is not a file at all.
"""

from __future__ import annotations

from pathlib import Path
from types import SimpleNamespace
from typing import Any, Final, cast

import pytest

from freecad_fakes import (
    RESULT_PAYLOAD,
    STEP_PAYLOAD,
    ExportOnlyPart,
    FakeCcxTools,
    FakeFemMesh,
    FakeFemToolsCcx,
    FakeFreeCAD,
    FakeObjectsFem,
    FakePart,
    FakeResultObject,
    calculator,
    modules_for,
    raising_shape,
    session_factory,
)
from kala_agent.tools.protocol import sha256_hex
from kala_agent_freecad.cad import fem
from kala_agent_freecad.cad.fem import FemOutcome, FemRequest, mesh_counts, result_summary
from kala_agent_freecad.cad.runtime import FreeCADModules
from kala_agent_freecad.errors import (
    FreeCADArtifactError,
    FreeCADInputError,
    FreeCADOperationError,
    FreeCADWorkspaceError,
)

BUDGET: Final[int] = 8_388_608
CALL_ID: Final[str] = "kala-fem-call-1"
UNSAFE_SOLVER: Final[str] = "/usr/local/bin/ccx; rm -rf /"


def _request(**overrides: object) -> FemRequest:
    values: dict[str, Any] = {
        "shape": "box",
        "document_name": "kala-freecad",
        "analysis_name": "KalaAnalysis",
        "artifact_stem": "kala-model",
        "material_name": "StructuralSteel",
        "length_mm": 120.0,
        "width_mm": 40.0,
        "height_mm": 20.0,
        "radius_mm": 0.0,
        "youngs_modulus_mpa": 210_000.0,
        "poisson_ratio": 0.3,
        "density_kg_m3": 7_850.0,
        "fixed_face": 1,
        "run_solver": False,
        "export_solid": True,
    }
    values.update(overrides)
    return FemRequest(**values)


def _modules(
    *,
    tools: FakeFemToolsCcx | None = None,
    part: object | None = None,
    freecad: FakeFreeCAD | None = None,
    factories: tuple[str, ...] | None = None,
) -> FreeCADModules:
    objects_fem = FakeObjectsFem() if factories is None else FakeObjectsFem(factories)
    return modules_for(
        freecad=freecad,
        part=part,
        objects_fem=objects_fem,
        ccx_tools=None if tools is None else FakeCcxTools(tools),
    )


def _run(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    request: FemRequest,
    modules: FreeCADModules,
    *,
    calculix_path: str = "",
    noted: list[list[str]] | None = None,
) -> FemOutcome:
    monkeypatch.setattr(fem, "freecad_session", session_factory(modules, noted))
    return fem.run(
        request,
        workspace=tmp_path,
        import_roots=(tmp_path,),
        calculix_path=calculix_path,
        max_artifact_bytes=BUDGET,
        call_id=CALL_ID,
    )


def _solver(outcome: FemOutcome) -> dict[str, object]:
    return cast("dict[str, object]", outcome.result["solver"])


def _results(outcome: FemOutcome) -> dict[str, object]:
    return cast("dict[str, object]", outcome.result["results"])


def test_parse_request_refuses_anything_that_is_not_an_object() -> None:
    with pytest.raises(FreeCADInputError, match="must be a JSON object"):
        fem.parse_request(cast("Any", ["box"]))


def test_parse_request_refuses_a_radius_on_a_box() -> None:
    with pytest.raises(FreeCADInputError, match="radius_mm only applies to a cylinder"):
        fem.parse_request({"shape": "box", "radius_mm": 4.0})


@pytest.mark.parametrize("mismatched", ("length_mm", "width_mm"))
def test_parse_request_refuses_box_extents_on_a_cylinder(mismatched: str) -> None:
    with pytest.raises(FreeCADInputError, match="only apply to a box"):
        fem.parse_request({"shape": "cylinder", mismatched: 4.0})


def test_an_analysis_is_built_and_reported_without_a_solve(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    freecad = FakeFreeCAD()
    outcome = _run(monkeypatch, tmp_path, _request(), _modules(freecad=freecad))
    assert _solver(outcome) == {
        "available": False,
        "engine": "calculix_ccx_tools",
        "executable": None,
        "executed": False,
        "message": "the caller did not ask for a solve",
        "object_name": "Solver",
        "status": "not_requested",
    }
    assert _results(outcome) == {
        "available": False,
        "element_count": 0,
        "max_displacement_mm": None,
        "max_stress_mpa": None,
        "message": "no Fem result object exists in the analysis",
        "node_count": 0,
    }
    assert outcome.result["objects"] == {
        "analysis": "KalaAnalysis",
        "constraint": "Fixed",
        "material": "Material",
        "mesh": "Mesh",
        "solver": "Solver",
    }
    assert outcome.result["mesh"] == {
        "element_count": 48,
        "generated": True,
        "node_count": 120,
        "object_name": "Mesh",
    }
    material = cast("dict[str, object]", outcome.result["material"])
    assert material["name"] == "StructuralSteel"
    assert material["youngs_modulus_mpa"] == 210_000.0
    assert material["poisson_ratio"] == 0.3
    assert material["density_kg_m3"] == 7_850.0
    assert [artifact.artifact_id for artifact in outcome.artifacts] == [
        "kala-fem-call-1-fcstd",
        "kala-fem-call-1-step",
    ]
    document = freecad.opened[0]
    assert document.type_ids == [
        "Part::Feature",
        "Fem::Fem::Analysis",
        "Fem::FemMaterialCommon",
        "Fem::FemConstraintFixed",
        "Fem::FemMeshShapeNetgenObjectPython",
        "Fem::FemSolverCalculiXCcxToolsPython",
    ]
    assert document.recomputes == 3
    assert freecad.closed == ["kala-freecad"]


def test_only_the_document_is_exported_when_the_solid_is_not_wanted(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    outcome = _run(
        monkeypatch,
        tmp_path,
        _request(export_solid=False),
        _modules(),
    )
    assert [artifact.artifact_id for artifact in outcome.artifacts] == ["kala-fem-call-1-fcstd"]


def test_an_unsafe_solver_path_is_never_trusted(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    tools = FakeFemToolsCcx()
    outcome = _run(
        monkeypatch,
        tmp_path,
        _request(run_solver=True),
        _modules(tools=tools),
        calculix_path=UNSAFE_SOLVER,
    )
    assert _solver(outcome)["status"] == "solver_unavailable"
    assert _solver(outcome)["executable"] is None
    assert "no safe CalculiX executable" in str(_solver(outcome)["message"])
    assert tools.built == 0
    assert tools.ran == 0


def test_a_missing_bridge_is_reported_with_the_reason_it_gave(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    modules = modules_for(solver_bridge_detail="the CalculiX bridge is unavailable")
    outcome = _run(
        monkeypatch,
        tmp_path,
        _request(run_solver=True),
        modules,
        calculix_path=calculator(tmp_path).as_posix(),
    )
    assert _solver(outcome)["status"] == "solver_unavailable"
    assert _solver(outcome)["available"] is True
    assert _solver(outcome)["executed"] is False
    assert _solver(outcome)["message"] == "the CalculiX bridge is unavailable"
    assert _solver(outcome)["executable"] is not None


def test_a_bridge_that_will_not_set_up_is_reported_as_a_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    tools = FakeFemToolsCcx(
        setup_fails=True,
    )
    outcome = _run(
        monkeypatch,
        tmp_path,
        _request(run_solver=True),
        _modules(tools=tools),
        calculix_path=calculator(tmp_path).as_posix(),
    )
    assert _solver(outcome)["status"] == "failed"
    assert _solver(outcome)["executed"] is False
    assert "refused the analysis" in str(_solver(outcome)["message"])
    assert tools.ran == 0


def test_unmet_prerequisites_stop_the_solve_before_it_starts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    tools = FakeFemToolsCcx(
        prerequisites="  the mesh has no elements  ",
    )
    outcome = _run(
        monkeypatch,
        tmp_path,
        _request(run_solver=True),
        _modules(tools=tools),
        calculix_path=calculator(tmp_path).as_posix(),
    )
    assert _solver(outcome)["status"] == "prerequisites_unmet"
    assert _solver(outcome)["message"] == "the mesh has no elements"
    assert tools.working_directory == (tmp_path / "artifacts").as_posix()
    assert tools.ran == 0


def test_a_solve_that_exits_nonzero_is_reported_as_a_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    tools = FakeFemToolsCcx(
        run_fails=True,
    )
    outcome = _run(
        monkeypatch,
        tmp_path,
        _request(run_solver=True),
        _modules(tools=tools),
        calculix_path=calculator(tmp_path).as_posix(),
    )
    assert _solver(outcome)["status"] == "failed"
    assert _solver(outcome)["executed"] is False
    assert "did not complete" in str(_solver(outcome)["message"])
    assert tools.ran == 0


def test_a_completed_solve_publishes_the_result_file_it_left(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    tools = FakeFemToolsCcx()
    outcome = _run(
        monkeypatch,
        tmp_path,
        _request(run_solver=True),
        _modules(tools=tools),
        calculix_path=calculator(tmp_path).as_posix(),
    )
    assert _solver(outcome)["status"] == "completed"
    assert _solver(outcome)["executed"] is True
    assert _solver(outcome)["message"] == "the CalculiX solve completed"
    assert tools.ran == 1
    assert [artifact.artifact_id for artifact in outcome.artifacts] == [
        "kala-fem-call-1-fcstd",
        "kala-fem-call-1-step",
        "kala-fem-call-1-frd",
    ]
    published = tmp_path / "artifacts" / "kala-model.frd"
    assert published.read_bytes() == RESULT_PAYLOAD
    assert outcome.artifacts[-1].digest == sha256_hex(RESULT_PAYLOAD)
    assert outcome.artifacts[-1].media_type == "text/plain"
    results = _results(outcome)
    assert results["available"] is True
    assert results["max_displacement_mm"] == 1.5
    assert results["max_stress_mpa"] == 220.0
    assert results["node_count"] == 8
    assert results["element_count"] == 3
    assert results["message"] is None


def test_a_result_already_at_the_published_path_is_not_copied_again(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    tools = FakeFemToolsCcx(result_name="kala-model.frd")
    outcome = _run(
        monkeypatch,
        tmp_path,
        _request(run_solver=True),
        _modules(tools=tools),
        calculix_path=calculator(tmp_path).as_posix(),
    )
    assert _solver(outcome)["status"] == "completed"
    assert (tmp_path / "artifacts" / "kala-model.frd").read_bytes() == RESULT_PAYLOAD
    assert outcome.artifacts[-1].size_bytes == len(RESULT_PAYLOAD)


def test_a_solve_that_writes_no_result_publishes_nothing_extra(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    tools = FakeFemToolsCcx(result_name=None)
    outcome = _run(
        monkeypatch,
        tmp_path,
        _request(run_solver=True),
        _modules(tools=tools),
        calculix_path=calculator(tmp_path).as_posix(),
    )
    assert _solver(outcome)["status"] == "completed"
    assert [artifact.artifact_id for artifact in outcome.artifacts] == [
        "kala-fem-call-1-fcstd",
        "kala-fem-call-1-step",
    ]
    assert not (tmp_path / "artifacts" / "kala-model.frd").exists()
    assert _results(outcome)["available"] is False


def test_a_directory_or_link_in_the_artifact_directory_is_never_published(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    artifacts = tmp_path / "artifacts"
    artifacts.mkdir()
    (artifacts / "a-directory.frd").mkdir()
    (artifacts / "b-link.frd").symlink_to(artifacts / "a-directory.frd")
    tools = FakeFemToolsCcx()
    outcome = _run(
        monkeypatch,
        tmp_path,
        _request(run_solver=True),
        _modules(tools=tools),
        calculix_path=calculator(tmp_path).as_posix(),
    )
    published = outcome.artifacts[-1]
    assert published.artifact_id == "kala-fem-call-1-frd"
    assert published.digest == sha256_hex(RESULT_PAYLOAD)
    assert (artifacts / "a-directory.frd").is_dir()
    assert (artifacts / "b-link.frd").is_symlink()


def test_a_result_the_publisher_cannot_copy_is_reported(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (tmp_path / "artifacts" / "kala-model.frd").mkdir(parents=True)
    tools = FakeFemToolsCcx()
    with pytest.raises(FreeCADArtifactError, match="could not be published") as failure:
        _run(
            monkeypatch,
            tmp_path,
            _request(run_solver=True),
            _modules(tools=tools),
            calculix_path=calculator(tmp_path).as_posix(),
        )
    assert isinstance(failure.value.__cause__, OSError)
    assert failure.value.__cause__.strerror


def test_a_document_that_will_not_open_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="could not open a document") as failure:
        _run(
            monkeypatch,
            tmp_path,
            _request(),
            _modules(freecad=FakeFreeCAD(new_document_fails=True)),
        )
    assert isinstance(failure.value.__cause__, RuntimeError)
    assert list(tmp_path.iterdir()) == []


def test_a_native_failure_while_building_is_reported_as_an_operation_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="could not build the Fem analysis") as failure:
        _run(
            monkeypatch,
            tmp_path,
            _request(),
            _modules(freecad=FakeFreeCAD(add_error=RuntimeError("the shape is invalid"))),
        )
    assert isinstance(failure.value.__cause__, RuntimeError)
    assert "the shape is invalid" in str(failure.value)


def test_an_adapter_failure_while_building_is_not_wrapped_twice(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    original = FreeCADWorkspaceError("the analysis left the document")
    with pytest.raises(FreeCADWorkspaceError) as raised:
        _run(
            monkeypatch,
            tmp_path,
            _request(),
            _modules(freecad=FakeFreeCAD(add_error=original)),
        )
    assert raised.value is original
    assert raised.value.__cause__ is None


def test_an_analysis_the_fem_module_cannot_build_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    factories = (
        "makeAnalysis",
        "makeMaterialSolid",
        "makeConstraintFixed",
        "makeMeshGmsh",
    )
    with pytest.raises(FreeCADOperationError, match="does not offer makeSolverCalculiXCcxTools"):
        _run(monkeypatch, tmp_path, _request(), _modules(factories=factories))


def test_a_part_module_without_the_shape_factory_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="does not offer makeBox"):
        _run(
            monkeypatch,
            tmp_path,
            _request(),
            _modules(part=ExportOnlyPart()),
        )


def test_dimensions_the_module_refuses_are_reported_unchanged(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="refused the box dimensions") as failure:
        _run(
            monkeypatch,
            tmp_path,
            _request(),
            _modules(part=FakePart(shape=raising_shape("negative shape volume"))),
        )
    assert isinstance(failure.value.__cause__, ValueError)


def test_a_failure_while_recomputing_the_analysis_is_reported(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="could not build the Fem analysis"):
        _run(
            monkeypatch,
            tmp_path,
            _request(),
            _modules(freecad=FakeFreeCAD(recompute_failure_at=1)),
        )


def test_a_failure_while_recomputing_the_built_analysis_is_reported(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="could not recompute the document") as failure:
        _run(
            monkeypatch,
            tmp_path,
            _request(),
            _modules(freecad=FakeFreeCAD(recompute_failure_at=3)),
        )
    assert isinstance(failure.value.__cause__, RuntimeError)
    assert list(tmp_path.iterdir()) == []


def test_an_exporter_that_fails_leaves_nothing_behind(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="could not write the step export"):
        _run(
            monkeypatch,
            tmp_path,
            _request(),
            _modules(part=FakePart(export_fails=True)),
        )
    assert not (tmp_path / "artifacts" / "kala-model.step").exists()
    assert (tmp_path / "artifacts" / "kala-model.fcstd").is_file()


def test_a_document_that_will_not_save_leaves_nothing_behind(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="could not write the fcstd export"):
        _run(
            monkeypatch,
            tmp_path,
            _request(),
            _modules(freecad=FakeFreeCAD(save_fails=True)),
        )
    assert list((tmp_path / "artifacts").iterdir()) == []


def test_a_document_the_module_cannot_name_is_left_to_the_session(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    noted: list[list[str]] = []
    outcome = _run(
        monkeypatch,
        tmp_path,
        _request(),
        _modules(freecad=FakeFreeCAD(unnamed=True)),
        noted=noted,
    )
    assert noted == [["kala-freecad"]]
    assert len(outcome.artifacts) == 2


def test_a_named_document_is_closed_before_the_session_reclaims_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    noted: list[list[str]] = []
    freecad = FakeFreeCAD()
    _run(monkeypatch, tmp_path, _request(), _modules(freecad=freecad), noted=noted)
    assert noted == [[]]
    assert freecad.listDocuments() == []


def test_mesh_counts_ignore_anything_that_is_not_a_count() -> None:
    assert mesh_counts(
        SimpleNamespace(FemMesh=SimpleNamespace(NodeCount=True, TetraCount=[1, 2, 3]))
    ) == (0, 3)
    assert mesh_counts(
        SimpleNamespace(FemMesh=SimpleNamespace(NodeCount=float("nan"), TetraCount=-4))
    ) == (0, 0)
    assert mesh_counts(
        SimpleNamespace(FemMesh=SimpleNamespace(NodeCount="12", TetraCount=object()))
    ) == (0, 0)
    assert mesh_counts(SimpleNamespace(FemMesh=SimpleNamespace(NodeCount=1.9, TetraCount=2.9))) == (
        1,
        2,
    )
    assert mesh_counts(SimpleNamespace(FemMesh=FakeFemMesh())) == (120, 48)
    assert mesh_counts(SimpleNamespace(FemMesh=SimpleNamespace())) == (0, 0)


def test_a_result_object_without_numbers_reports_nothing() -> None:
    document = SimpleNamespace(
        Objects=[FakeResultObject(displacement=(None, "x", True, float("inf")), stress=())]
    )
    summary = result_summary(document)
    assert summary["available"] is False
    assert summary["max_displacement_mm"] is None
    assert summary["max_stress_mpa"] is None
    assert "no displacement or stress values" in str(summary["message"])
    assert summary["node_count"] == 8


def test_a_result_object_with_only_stress_is_still_available() -> None:
    document = SimpleNamespace(Objects=[FakeResultObject(displacement=(), stress=(1.0, 2.5, 2.25))])
    summary = result_summary(document)
    assert summary["available"] is True
    assert summary["max_stress_mpa"] == 2.5
    assert summary["max_displacement_mm"] is None
    assert summary["node_count"] == 8
    assert summary["element_count"] == 3


def test_the_exported_solid_is_the_analysed_feature(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    part = FakePart()
    _run(monkeypatch, tmp_path, _request(), _modules(part=part))
    assert part.exports == [(tmp_path / "artifacts" / "kala-model.step").as_posix()]
    assert (tmp_path / "artifacts" / "kala-model.step").read_bytes() == STEP_PAYLOAD
