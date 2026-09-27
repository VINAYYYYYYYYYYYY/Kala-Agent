"""Geometry work driven by fake FreeCAD modules.

Every refusal the adapter owes its caller is a property of how it treats a
broken module rather than of the schema, so they are proved here against a
module that misbehaves in a specific, named way: a document that will not
open, a factory that is absent, a shape that measures as a word, an exporter
that leaves a partial file behind.
"""

from __future__ import annotations

import math
from pathlib import Path
from typing import Any, Final, cast

import pytest

from freecad_fakes import (
    DOCUMENT_PAYLOAD,
    STEP_PAYLOAD,
    ExportOnlyPart,
    FakeBoundBox,
    FakeFreeCAD,
    FakePart,
    FakeShape,
    ShapeBuilder,
    modules_for,
    raising_shape,
    session_factory,
    silent_shape,
)
from kala_agent.tools.protocol import sha256_hex
from kala_agent_freecad.cad import geometry
from kala_agent_freecad.cad.geometry import GeometryOutcome, GeometryRequest
from kala_agent_freecad.errors import (
    FreeCADOperationError,
    FreeCADWorkspaceError,
)

BUDGET: Final[int] = 8_388_608
CALL_ID: Final[str] = "kala-call-1"
EXPECTED_VOLUME: Final[float] = 120.0 * 40.0 * 20.0


def _request(**overrides: object) -> GeometryRequest:
    values: dict[str, Any] = {
        "shape": "box",
        "export_formats": ("step", "fcstd"),
        "document_name": "kala-freecad",
        "artifact_stem": "kala-model",
        "length_mm": 120.0,
        "width_mm": 40.0,
        "height_mm": 20.0,
        "radius_mm": 0.0,
        "measure": True,
    }
    values.update(overrides)
    return GeometryRequest(**values)


def _run(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    request: GeometryRequest,
    *,
    part: object | None = None,
    freecad: FakeFreeCAD | None = None,
    noted: list[list[str]] | None = None,
) -> GeometryOutcome:
    monkeypatch.setattr(
        geometry,
        "freecad_session",
        session_factory(modules_for(freecad=freecad, part=part), noted),
    )
    return geometry.run(
        request,
        workspace=tmp_path,
        import_roots=(tmp_path,),
        max_artifact_bytes=BUDGET,
        call_id=CALL_ID,
    )


def _shape_with(**overrides: object) -> ShapeBuilder:
    def build(_kind: str, *_extents: float) -> object:
        shape = FakeShape()
        for name, value in overrides.items():
            setattr(shape, name, value)
        return shape

    return build


def test_a_box_is_measured_and_every_export_published(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    part = FakePart()
    freecad = FakeFreeCAD()
    noted: list[list[str]] = []
    outcome = _run(monkeypatch, tmp_path, _request(), part=part, freecad=freecad, noted=noted)

    assert outcome.result["shape"] == "box"
    assert outcome.result["valid"] is True
    assert outcome.result["shape_name"] == "Shape"
    assert outcome.result["document_name"] == "kala-freecad"
    assert outcome.result["freecad_version"] == "1.1.3"
    assert outcome.result["import_root"] == "/usr/lib/freecad/lib"
    measurement = cast("dict[str, object]", outcome.result["measurement"])
    assert measurement["volume_mm3"] == pytest.approx(EXPECTED_VOLUME)
    assert measurement["bounding_box_mm"] == [120.0, 40.0, 20.0]
    assert measurement["face_count"] == 6
    assert measurement["edge_count"] == 12
    assert measurement["vertex_count"] == 8
    assert measurement["solid"] is True
    assert measurement["area_mm2"] == pytest.approx(2.0 * (4800.0 + 2400.0 + 800.0))
    assert part.dimensions == [("box", (120.0, 40.0, 20.0))]

    entries = cast("list[dict[str, object]]", outcome.result["artifacts"])
    assert [entry["format"] for entry in entries] == ["step", "fcstd"]
    assert [entry["relative_path"] for entry in entries] == [
        "artifacts/kala-model.step",
        "artifacts/kala-model.fcstd",
    ]
    assert [artifact.artifact_id for artifact in outcome.artifacts] == [
        "kala-call-1-step",
        "kala-call-1-fcstd",
    ]
    step = tmp_path / "artifacts" / "kala-model.step"
    document = tmp_path / "artifacts" / "kala-model.fcstd"
    assert step.read_bytes() == STEP_PAYLOAD
    assert document.read_bytes() == DOCUMENT_PAYLOAD
    assert [artifact.digest for artifact in outcome.artifacts] == [
        sha256_hex(STEP_PAYLOAD),
        sha256_hex(DOCUMENT_PAYLOAD),
    ]
    assert [artifact.size_bytes for artifact in outcome.artifacts] == [
        len(STEP_PAYLOAD),
        len(DOCUMENT_PAYLOAD),
    ]
    assert freecad.closed == ["kala-freecad"]
    assert noted == [[]]


def test_a_cylinder_is_built_from_a_radius_and_a_height(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    part = FakePart()
    outcome = _run(
        monkeypatch,
        tmp_path,
        _request(
            shape="cylinder",
            export_formats=("step",),
            length_mm=0.0,
            width_mm=0.0,
            radius_mm=12.5,
            height_mm=60.0,
        ),
        part=part,
    )
    assert part.dimensions == [("cylinder", (12.5, 60.0))]
    measurement = cast("dict[str, object]", outcome.result["measurement"])
    assert measurement["volume_mm3"] == pytest.approx(math.pi * 12.5**2 * 60.0)
    assert len(outcome.artifacts) == 1


def test_measurement_can_be_declined(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    outcome = _run(monkeypatch, tmp_path, _request(measure=False, export_formats=("step",)))
    assert outcome.result["measurement"] == {
        "area_mm2": 0.0,
        "bounding_box_mm": [0.0, 0.0, 0.0],
        "edge_count": 0,
        "face_count": 0,
        "solid": False,
        "vertex_count": 0,
        "volume_mm3": 0.0,
    }


def test_a_call_without_a_shape_is_refused(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    freecad = FakeFreeCAD()
    with pytest.raises(FreeCADOperationError, match="did not build a shape"):
        _run(
            monkeypatch,
            tmp_path,
            _request(export_formats=("step",)),
            part=FakePart(shape=silent_shape()),
            freecad=freecad,
        )
    assert freecad.closed == ["kala-freecad"]
    assert list(tmp_path.iterdir()) == []


def test_a_document_that_will_not_open_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="could not open a document") as failure:
        _run(
            monkeypatch,
            tmp_path,
            _request(),
            freecad=FakeFreeCAD(new_document_fails=True),
        )
    assert isinstance(failure.value.__cause__, RuntimeError)
    assert list(tmp_path.iterdir()) == []


def test_a_native_failure_while_building_is_reported_as_an_operation_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="could not build the box solid") as failure:
        _run(
            monkeypatch,
            tmp_path,
            _request(),
            freecad=FakeFreeCAD(add_error=RuntimeError("the feature is not allowed")),
        )
    assert isinstance(failure.value.__cause__, RuntimeError)
    assert "the feature is not allowed" in str(failure.value)


def test_an_adapter_failure_is_not_wrapped_twice(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    original = FreeCADWorkspaceError("the feature left the document")
    with pytest.raises(FreeCADWorkspaceError) as raised:
        _run(
            monkeypatch,
            tmp_path,
            _request(),
            freecad=FakeFreeCAD(add_error=original),
        )
    assert raised.value is original
    assert raised.value.__cause__ is None


def test_a_part_module_without_the_factory_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="does not offer makeBox"):
        _run(
            monkeypatch,
            tmp_path,
            _request(export_formats=("step",)),
            part=ExportOnlyPart(),
        )
    assert list(tmp_path.iterdir()) == []


def test_dimensions_the_module_refuses_are_reported_unchanged(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="refused the box dimensions") as failure:
        _run(
            monkeypatch,
            tmp_path,
            _request(export_formats=("step",)),
            part=FakePart(shape=raising_shape("non positive shape volume")),
        )
    assert isinstance(failure.value.__cause__, ValueError)
    assert "non positive shape volume" in str(failure.value)


def test_a_document_that_will_not_recompute_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="could not recompute the document") as failure:
        _run(
            monkeypatch,
            tmp_path,
            _request(export_formats=("step",)),
            freecad=FakeFreeCAD(recompute_failure_at=1),
        )
    assert isinstance(failure.value.__cause__, RuntimeError)
    assert list(tmp_path.iterdir()) == []


def test_a_shape_without_a_validity_flag_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="the FreeCAD shape has no isValid"):
        _run(
            monkeypatch,
            tmp_path,
            _request(export_formats=("step",)),
            part=FakePart(shape=_shape_with(isValid=None)),
        )


def test_a_shape_without_topology_counts_zero_of_everything(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    outcome = _run(
        monkeypatch,
        tmp_path,
        _request(export_formats=("step",)),
        part=FakePart(shape=_shape_with(Edges=None, Faces=None, Vertexes=None)),
    )
    measurement = cast("dict[str, object]", outcome.result["measurement"])
    assert measurement["edge_count"] == 0
    assert measurement["face_count"] == 0
    assert measurement["vertex_count"] == 0
    assert measurement["volume_mm3"] == pytest.approx(EXPECTED_VOLUME)


def test_a_measurement_that_is_a_word_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="was not a number") as failure:
        _run(
            monkeypatch,
            tmp_path,
            _request(export_formats=("step",)),
            part=FakePart(shape=_shape_with(Area="wide")),
        )
    assert isinstance(failure.value.__cause__, ValueError)


def test_a_measurement_that_is_not_finite_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="was not finite"):
        _run(
            monkeypatch,
            tmp_path,
            _request(export_formats=("step",)),
            part=FakePart(shape=_shape_with(Volume=float("inf"))),
        )
    with pytest.raises(FreeCADOperationError, match="was not finite"):
        _run(
            monkeypatch,
            tmp_path,
            _request(export_formats=("step",)),
            part=FakePart(shape=_shape_with(BoundBox=FakeBoundBox(float("nan"), 40.0, 20.0))),
        )


def test_an_exporter_that_fails_leaves_nothing_behind(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="could not write the step export") as failure:
        _run(
            monkeypatch,
            tmp_path,
            _request(export_formats=("step",)),
            part=FakePart(export_fails=True),
        )
    assert isinstance(failure.value.__cause__, RuntimeError)
    assert list((tmp_path / "artifacts").iterdir()) == []


def test_a_document_that_will_not_save_leaves_nothing_behind(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FreeCADOperationError, match="could not write the fcstd export"):
        _run(
            monkeypatch,
            tmp_path,
            _request(export_formats=("step", "fcstd")),
            freecad=FakeFreeCAD(save_fails=True),
        )
    assert not (tmp_path / "artifacts" / "kala-model.fcstd").exists()
    assert (tmp_path / "artifacts" / "kala-model.step").read_bytes() == STEP_PAYLOAD


def test_an_export_above_the_budget_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(geometry, "freecad_session", session_factory(modules_for()))
    with pytest.raises(Exception, match="artifact budget"):
        geometry.run(
            _request(export_formats=("step",)),
            workspace=tmp_path,
            import_roots=(tmp_path,),
            max_artifact_bytes=2,
            call_id=CALL_ID,
        )


def test_a_document_the_module_cannot_name_is_left_to_the_session(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    freecad = FakeFreeCAD(unnamed=True)
    noted: list[list[str]] = []
    outcome = _run(
        monkeypatch,
        tmp_path,
        _request(export_formats=("step",)),
        freecad=freecad,
        noted=noted,
    )
    assert noted == [["kala-freecad"]]
    assert len(outcome.artifacts) == 1
    assert freecad.listDocuments() == []
