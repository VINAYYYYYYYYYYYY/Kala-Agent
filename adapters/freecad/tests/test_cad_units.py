"""The adapter's own helpers, exercised without a FreeCAD installation."""

from __future__ import annotations

import dataclasses
import os
import stat
from pathlib import Path
from types import SimpleNamespace
from typing import Any, Final, cast

import pytest

from kala_agent.contracts import ArtifactClassification
from kala_agent.tools.protocol import ToolWorkspaceError, sha256_hex
from kala_agent_freecad.cad import arguments, fem, geometry
from kala_agent_freecad.cad.artifacts import (
    PublishedArtifact,
    bounded_label,
    digest_artifact,
    discard,
    finalize,
    optional_message,
    prepare_target,
    resolve_target,
)
from kala_agent_freecad.errors import (
    FreeCADArtifactError,
    FreeCADInputError,
    FreeCADOutputTooLargeError,
    FreeCADWorkspaceError,
)

PAYLOAD: Final[bytes] = b"ISO-10303-21;\n"
RESULT_TYPE_ID: Final[str] = "Fem::FemResultObjectPython"
MESH_TYPE_ID: Final[str] = "Fem::FemMeshShapeBaseObjectPython"


def _write(target: Path, payload: bytes = PAYLOAD) -> Path:
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(payload)
    return target


def test_member_accepts_only_a_closed_set() -> None:
    assert arguments.member({"shape": "box"}, "shape", ("box", "cylinder")) == "box"
    with pytest.raises(FreeCADInputError):
        arguments.member({"shape": "sphere"}, "shape", ("box",))
    with pytest.raises(FreeCADInputError):
        arguments.member({}, "shape", ("box",))
    with pytest.raises(FreeCADInputError):
        arguments.member({"shape": 1}, "shape", ("box",))


def test_text_falls_back_to_a_bounded_default() -> None:
    assert arguments.text({}, "document_name", "kala") == "kala"
    assert arguments.text({"document_name": "run"}, "document_name", "kala") == "run"
    with pytest.raises(FreeCADInputError):
        arguments.text({"document_name": ""}, "document_name", "kala")
    with pytest.raises(FreeCADInputError):
        arguments.text({"document_name": 4}, "document_name", "kala")
    with pytest.raises(FreeCADInputError):
        arguments.text({"document_name": "x" * 65}, "document_name", "kala")


@pytest.mark.parametrize("name", ("length_mm", "width_mm", "height_mm", "radius_mm"))
def test_dimension_bounds_every_extent(name: str) -> None:
    assert arguments.dimension({}, name, 5_000.0) == 0.0
    assert arguments.dimension({name: 1}, name, 5_000.0) == 1.0
    assert arguments.dimension({name: 5_000}, name, 5_000.0) == 5_000.0
    for bad in (0.0, -1.0, 5_000.1, float("inf"), float("nan"), "10", True):
        with pytest.raises(FreeCADInputError):
            arguments.dimension({name: bad}, name, 5_000.0)


def test_positive_bounds_a_material_property() -> None:
    assert arguments.positive({}, "density_kg_m3", 7_850.0, 22_000.0) == 7_850.0
    assert arguments.positive({"density_kg_m3": 1.0}, "density_kg_m3", 7_850.0, 22_000.0) == 1.0
    for bad in (0.0, -2.7e3, 22_000.1, "7800", True, float("nan")):
        with pytest.raises(FreeCADInputError):
            arguments.positive({"density_kg_m3": bad}, "density_kg_m3", 7_850.0, 22_000.0)


def test_ratio_bounds_a_poisson_ratio() -> None:
    assert arguments.ratio({}, "poisson_ratio", 0.3) == 0.3
    assert arguments.ratio({"poisson_ratio": 0.49}, "poisson_ratio", 0.3) == 0.49
    for bad in (0.0, 0.5, 1.0, -0.1, "0.3", True):
        with pytest.raises(FreeCADInputError):
            arguments.ratio({"poisson_ratio": bad}, "poisson_ratio", 0.3)


def test_integer_bounds_a_face_index() -> None:
    assert arguments.integer({}, "fixed_face", 1, 6) == 1
    assert arguments.integer({"fixed_face": 6}, "fixed_face", 1, 6) == 6
    for bad in (0, 7, 1.5, "1", True):
        with pytest.raises(FreeCADInputError):
            arguments.integer({"fixed_face": bad}, "fixed_face", 1, 6)


def test_flag_requires_a_boolean() -> None:
    assert arguments.flag({}, "run_solver", default=False) is False
    assert arguments.flag({"run_solver": True}, "run_solver", default=False) is True
    with pytest.raises(FreeCADInputError):
        arguments.flag({"run_solver": "true"}, "run_solver", default=False)


def test_unique_members_rejects_repeats_and_unknowns() -> None:
    assert arguments.unique_members(
        {"export_formats": ["step", "iges"]},
        "export_formats",
        ("step", "iges"),
    ) == ("step", "iges")
    for bad in ([], "step", ["step", "step"], ["stp"], [1]):
        with pytest.raises(FreeCADInputError):
            arguments.unique_members({"export_formats": bad}, "export_formats", ("step", "iges"))


def test_geometry_request_defaults() -> None:
    request = geometry.parse_request({"shape": "box", "export_formats": ["step"]})
    assert request.shape == "box"
    assert request.document_name == "kala-freecad"
    assert request.artifact_stem == "kala-model"
    assert request.measure is True
    assert request.radius_mm == 0.0


def test_geometry_request_rejects_a_foreign_mapping() -> None:
    with pytest.raises(FreeCADInputError):
        geometry.parse_request(cast("Any", ["box"]))


def test_fem_request_defaults() -> None:
    request = fem.parse_request({"shape": "box"})
    assert request.analysis_name == "KalaAnalysis"
    assert request.material_name == "StructuralSteel"
    assert request.youngs_modulus_mpa == 210_000.0
    assert request.poisson_ratio == 0.3
    assert request.density_kg_m3 == 7_850.0
    assert request.fixed_face == 1
    assert request.run_solver is False
    assert request.export_solid is True
    assert request.length_mm == 0.0


def test_mesh_counts_report_zero_without_a_mesh() -> None:
    assert fem.mesh_counts(SimpleNamespace()) == (0, 0)
    assert fem.mesh_counts(SimpleNamespace(FemMesh=None)) == (0, 0)
    mesh = SimpleNamespace(
        FemMesh=SimpleNamespace(NodeCount=120, TetraCount=48, TriangleCount=2, HexaCount=0)
    )
    assert fem.mesh_counts(mesh) == (120, 50)
    assert fem.mesh_summary(mesh) == {
        "element_count": 50,
        "generated": True,
        "node_count": 120,
        "object_name": "unknown",
    }


def test_mesh_summary_is_not_generated_without_a_mesh() -> None:
    summary = fem.mesh_summary(SimpleNamespace(Name="Mesh"))
    assert summary == {
        "element_count": 0,
        "generated": False,
        "node_count": 0,
        "object_name": "Mesh",
    }


def test_result_summary_without_a_result_object() -> None:
    document = SimpleNamespace(Objects=[])
    assert fem.result_summary(document) == {
        "available": False,
        "element_count": 0,
        "max_displacement_mm": None,
        "max_stress_mpa": None,
        "message": "no Fem result object exists in the analysis",
        "node_count": 0,
    }


def test_result_summary_reads_a_real_result_object() -> None:
    result = SimpleNamespace(
        TypeId=RESULT_TYPE_ID,
        DisplacementLengths=[0.0, 0.25, 1.5],
        vonMises=[10.0, 220.0],
        Mesh=SimpleNamespace(FemMesh=SimpleNamespace(NodeCount=8, TetraCount=3, TriangleCount=0)),
    )
    document = SimpleNamespace(Objects=[result, SimpleNamespace(TypeId=MESH_TYPE_ID)])
    summary = fem.result_summary(document)
    assert summary["available"] is True
    assert summary["max_displacement_mm"] == 1.5
    assert summary["max_stress_mpa"] == 220.0
    assert summary["node_count"] == 8
    assert summary["element_count"] == 3
    assert summary["message"] is None


def test_result_summary_without_values() -> None:
    result = SimpleNamespace(TypeId=RESULT_TYPE_ID, DisplacementLengths=[], vonMises=())
    document = SimpleNamespace(Objects=[result])
    summary = fem.result_summary(document)
    assert summary["available"] is False
    assert "no displacement or stress values" in str(summary["message"])


def test_result_summary_survives_a_broken_result_object() -> None:
    document = SimpleNamespace(Objects="not-a-list")
    assert fem.result_summary(document)["available"] is False


def test_resolve_target_stays_inside_the_workspace(tmp_path: Path) -> None:
    target = resolve_target(tmp_path, "artifacts/model.step")
    assert target == tmp_path / "artifacts" / "model.step"
    for unsafe in ("../escape", "/etc/passwd", "a/../../b", "", "  "):
        with pytest.raises(FreeCADWorkspaceError):
            resolve_target(tmp_path, unsafe)


def test_resolve_target_wraps_the_kernel_error(tmp_path: Path) -> None:
    with pytest.raises(FreeCADWorkspaceError) as wrapped:
        resolve_target(tmp_path, "../escape")
    assert isinstance(wrapped.value.__cause__, ToolWorkspaceError)


def test_prepare_target_creates_a_real_directory(tmp_path: Path) -> None:
    target = tmp_path / "artifacts" / "model.step"
    prepare_target(target)
    assert target.parent.is_dir()
    assert stat.S_IMODE(target.parent.stat().st_mode) == 0o700


def test_prepare_target_refuses_a_symlinked_directory(tmp_path: Path) -> None:
    outside = tmp_path / "outside"
    outside.mkdir()
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    os.symlink(outside, workspace / "artifacts")
    with pytest.raises(FreeCADWorkspaceError):
        prepare_target(workspace / "artifacts" / "model.step")


def test_prepare_target_refuses_a_symlinked_destination(tmp_path: Path) -> None:
    outside = tmp_path / "outside.step"
    outside.write_bytes(PAYLOAD)
    target = tmp_path / "artifacts" / "model.step"
    target.parent.mkdir()
    os.symlink(outside, target)
    with pytest.raises(FreeCADWorkspaceError):
        prepare_target(target)


def test_digest_artifact_matches_the_bytes_on_disk(tmp_path: Path) -> None:
    target = _write(tmp_path / "artifacts" / "model.step")
    digest, size = digest_artifact(target, max_artifact_bytes=4_096)
    assert digest == sha256_hex(PAYLOAD)
    assert size == len(PAYLOAD)


def test_digest_artifact_refuses_an_empty_file(tmp_path: Path) -> None:
    target = _write(tmp_path / "empty.step", b"")
    with pytest.raises(FreeCADArtifactError):
        digest_artifact(target, max_artifact_bytes=4_096)


def test_digest_artifact_refuses_a_missing_file(tmp_path: Path) -> None:
    with pytest.raises(FreeCADArtifactError):
        digest_artifact(tmp_path / "missing.step", max_artifact_bytes=4_096)


def test_digest_artifact_enforces_the_budget(tmp_path: Path) -> None:
    target = _write(tmp_path / "artifacts" / "model.step", b"x" * 4_096)
    with pytest.raises(FreeCADOutputTooLargeError):
        digest_artifact(target, max_artifact_bytes=1_024)


def test_finalize_describes_a_published_file(tmp_path: Path) -> None:
    target = _write(tmp_path / "artifacts" / "model.step")
    published = finalize(
        target,
        relative_path="artifacts/model.step",
        export_format="step",
        artifact_id="kala-call-1-step",
        max_artifact_bytes=4_096,
    )
    assert published.digest == sha256_hex(PAYLOAD)
    assert published.size_bytes == len(PAYLOAD)
    assert published.media_type == "model/step"
    assert published.uri == f"file://{target.as_posix()}"
    assert published.entry() == {
        "digest": sha256_hex(PAYLOAD),
        "format": "step",
        "media_type": "model/step",
        "relative_path": "artifacts/model.step",
        "size_bytes": len(PAYLOAD),
    }
    artifact = published.artifact
    assert artifact.classification is ArtifactClassification.DESIGN_BINARY
    assert artifact.artifact_id == "kala-call-1-step"


def test_finalize_uses_the_media_type_of_every_format(tmp_path: Path) -> None:
    for export_format, media_type in (
        ("step", "model/step"),
        ("iges", "model/iges"),
        ("fcstd", "application/x-free-cad"),
        ("frd", "text/plain"),
    ):
        target = _write(tmp_path / f"artifacts/model.{export_format}")
        published = finalize(
            target,
            relative_path=f"artifacts/model.{export_format}",
            export_format=export_format,
            artifact_id=f"kala-call-1-{export_format}",
            max_artifact_bytes=4_096,
        )
        assert published.media_type == media_type
        assert published.artifact.classification is ArtifactClassification.DESIGN_BINARY


def test_finalize_refuses_an_empty_export(tmp_path: Path) -> None:
    target = _write(tmp_path / "artifacts" / "model.step", b"")
    with pytest.raises(FreeCADArtifactError):
        finalize(
            target,
            relative_path="artifacts/model.step",
            export_format="step",
            artifact_id="kala-call-1-step",
            max_artifact_bytes=4_096,
        )


def test_finalize_refuses_an_export_above_the_budget(tmp_path: Path) -> None:
    target = _write(tmp_path / "artifacts" / "model.step", b"x" * 4_096)
    with pytest.raises(FreeCADOutputTooLargeError):
        finalize(
            target,
            relative_path="artifacts/model.step",
            export_format="step",
            artifact_id="kala-call-1-step",
            max_artifact_bytes=1_024,
        )


def test_finalize_refuses_a_file_that_disappeared(tmp_path: Path) -> None:
    with pytest.raises(FreeCADArtifactError):
        finalize(
            tmp_path / "artifacts" / "model.step",
            relative_path="artifacts/model.step",
            export_format="step",
            artifact_id="kala-call-1-step",
            max_artifact_bytes=4_096,
        )


def test_discard_removes_a_rejected_export(tmp_path: Path) -> None:
    target = _write(tmp_path / "artifacts" / "model.step")
    discard(target)
    assert not target.exists()
    discard(target)


def test_labels_and_messages_are_bounded() -> None:
    assert bounded_label("  a   b  ", 8) == "a b"
    assert bounded_label("", 8) == "unknown"
    assert bounded_label("x" * 32, 4) == "xxxx"
    assert optional_message("", 8) is None
    assert optional_message(" a\n b ", 8) == "a b"
    assert optional_message("x" * 32, 4) == "xxxx"


def test_published_artifact_is_immutable() -> None:
    published = PublishedArtifact(
        artifact_id="kala-call-1-step",
        relative_path="artifacts/model.step",
        export_format="step",
        media_type="model/step",
        digest="0" * 64,
        size_bytes=1,
        uri="file:///tmp/model.step",
    )
    with pytest.raises(dataclasses.FrozenInstanceError):
        published.size_bytes = 2  # type: ignore[misc]
