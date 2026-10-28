"""Probe behavior with a fake COM seam. No test claims a Windows result.

The fakes mirror the shapes measured on Solid Edge 2024 (see
``live-evidence`` in the work log): ``GetNormal``/``GetRootPoint`` return a
plain 3-tuple, ``Body.GetRange`` returns ``(minimum, maximum)``, and
``ComputePhysicalProperties`` returns one tuple of eleven out parameters. No
fake exposes a by-reference ``VARIANT``, so a regression that reintroduces the
``.value`` assumption fails here.
"""

from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace
from typing import Any, ClassVar, cast

import pytest

from kala_solidedge_bridge.auth import ensure_token
from kala_solidedge_bridge.calibration import CalibrationStore
from kala_solidedge_bridge.probe import (
    ProbeOutcome,
    ProbeSession,
    run_probe,
)
from kala_solidedge_bridge.session import (
    E_INVALIDARG,
    MK_E_UNAVAILABLE,
    ConstantResolution,
)

DOC_MEMBERS = (
    "Models",
    "ProfileSets",
    "RefPlanes",
    "SaveAs",
    "SaveCopyAs",
    "Close",
)

#: The eleven out parameters measured on Solid Edge 2024 for a 10 mm cube at
#: 1000 kg/m3 and 0.001 m accuracy. Status 1 was the observed success value.
MEASURED_PROPERTIES: tuple[Any, ...] = (
    1.0000000000000002e-06,
    0.0006000000000000001,
    0.0010000000000000002,
    (0.0, 0.0, 0.0),
    (0.0, 0.0, 0.0),
    (1.6666666666666667e-08,) * 3 + (0.0, 5.5e-25, 5.5e-25),
    (1.6666666666666667e-08,) * 3,
    (1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0),
    (0.00408248290463863,) * 3,
    0.001,
    1,
)


class FakeComError(Exception):
    def __init__(self, hresult: int) -> None:
        super().__init__(f"fake COM failure {hresult:#x}")
        self.hresult = hresult


class FakeBinding:
    def __init__(
        self,
        *,
        app: FakeApp,
        constants: dict[str, int] | None = None,
        attach_error: Exception | None = None,
    ) -> None:
        self.app = app
        self.constants = (
            constants
            if constants is not None
            else {"igSymmetric": 3, "igProfileClosed": 1, "seModelingModeOrdered": 2}
        )
        self.attach_error = attach_error
        self.described: list[str] = []

    def attach(self, launch: bool) -> Any:  # noqa: ANN401
        if self.attach_error is not None:
            raise self.attach_error
        return self.app

    def version(self, application: Any) -> str:  # noqa: ANN401
        return str(application.Version)

    def resolve_constants(
        self,
        application: Any,  # noqa: ANN401
        names: tuple[str, ...],
    ) -> ConstantResolution:
        values = {name: self.constants[name] for name in names if name in self.constants}
        return ConstantResolution(
            values=values,
            missing=[name for name in names if name not in values],
            sources={name: f"SolidEdgePart.{name}Enum" for name in values},
        )

    def describe_typelib(self, obj: Any) -> dict[str, Any]:  # noqa: ANN401
        self.described.append(type(obj).__name__)
        typelib = getattr(obj, "typelib", None)
        if typelib is None:
            return {"status": "unavailable", "methods": {}}
        return cast(dict[str, Any], typelib)

    def call(self, operation: Any, **kwargs: Any) -> Any:  # noqa: ANN401
        return operation()


class FakeSession:
    def __init__(self, binding: FakeBinding) -> None:
        self.binding = binding
        self.started = False
        self.stopped = False

    def start(self, timeout: float = 15.0) -> None:
        self.started = True

    def stop(self) -> bool:
        self.stopped = True
        return True

    def run(self, operation: Any, *, timeout: float | None = None) -> Any:  # noqa: ANN401
        return operation(self.binding)


class FakeRefPlane:
    """Out parameters come back as the call's return value on pywin32 312."""

    def __init__(self) -> None:
        self.received: list[tuple[str, Any]] = []

    def GetNormal(self, buffer: Any) -> tuple[float, float, float]:  # noqa: ANN401
        self.received.append(("GetNormal", buffer))
        assert isinstance(buffer, list), "the probe must pass a plain list"
        return (0.0, 0.0, 1.0)

    def GetRootPoint(self, buffer: Any) -> tuple[float, float, float]:  # noqa: ANN401
        self.received.append(("GetRootPoint", buffer))
        assert isinstance(buffer, list), "the probe must pass a plain list"
        return (0.0, 0.0, 0.0)


class FakeLines:
    def __init__(self) -> None:
        self.fail = False
        self.points: list[tuple[float, float, float, float]] = []
        self.typelib = {"status": "ok", "methods": {"AddBy2Points": {"c_params": 4}}}

    def AddBy2Points(self, x1: float, y1: float, x2: float, y2: float) -> None:
        if self.fail:
            raise RuntimeError("line creation failed")
        self.points.append((x1, y1, x2, y2))


class FakeProfile:
    def __init__(self, *, fail_lines: bool = False, end_code: int = -103) -> None:
        self.Lines2d = FakeLines()
        self.Lines2d.fail = fail_lines
        self.end_code = end_code
        self.validation_arguments: list[int] = []

    def End(self, validation: int) -> int:
        self.validation_arguments.append(validation)
        return self.end_code


class FakeProfiles:
    def __init__(self, *, fail_lines: bool = False, end_code: int = -103) -> None:
        self.fail_lines = fail_lines
        self.end_code = end_code
        self.last: FakeProfile | None = None

    def Add(self, ref_plane: FakeRefPlane) -> FakeProfile:
        self.last_ref_plane = ref_plane
        self.last = FakeProfile(fail_lines=self.fail_lines, end_code=self.end_code)
        return self.last


class FakeProfileSet:
    def __init__(self, profiles: FakeProfiles) -> None:
        self.Profiles = profiles


class FakeProfileSets:
    def __init__(self, *, fail_lines: bool = False, end_code: int = -103) -> None:
        self.fail_lines = fail_lines
        self.end_code = end_code
        self.profiles = FakeProfiles(fail_lines=fail_lines, end_code=end_code)
        self.last: FakeProfileSet | None = None

    def Add(self) -> FakeProfileSet:
        self.last = FakeProfileSet(self.profiles)
        return self.last


class FakeBody:
    def __init__(self, minimum: tuple[float, ...], maximum: tuple[float, ...]) -> None:
        self.minimum = minimum
        self.maximum = maximum
        self.received: list[tuple[Any, Any]] = []
        self.typelib = {"status": "ok", "methods": {"GetRange": {"c_params": 2}}}

    def GetRange(self, minimum: Any, maximum: Any) -> tuple[Any, Any]:  # noqa: ANN401
        self.received.append((minimum, maximum))
        assert isinstance(minimum, list) and isinstance(maximum, list)
        return (self.minimum, self.maximum)


class FakeModel:
    def __init__(self, properties: tuple[Any, ...], body: FakeBody) -> None:
        self.Body = body
        self.properties = properties
        self.received: list[tuple[Any, ...]] = []
        self.typelib = {
            "status": "ok",
            "methods": {"ComputePhysicalProperties": {"c_params": 13}},
        }

    def ComputePhysicalProperties(self, *args: Any) -> Any:  # noqa: ANN401
        self.received.append(args)
        assert args == (1000.0, 0.001), "the probe must pass the measured inputs"
        return self.properties


class FakeModels:
    def __init__(self, properties: tuple[Any, ...], body: FakeBody) -> None:
        self.properties = properties
        self.body = body
        self.extrusions: list[tuple[Any, ...]] = []
        self.last_model: FakeModel | None = None
        self.typelib = {
            "status": "ok",
            "methods": {"AddFiniteExtrudedProtrusion": {"c_params": 4}},
        }

    def AddFiniteExtrudedProtrusion(self, *args: Any) -> FakeModel:  # noqa: ANN401
        self.extrusions.append(args)
        self.last_model = FakeModel(self.properties, self.body)
        return self.last_model


class FakeDocuments:
    def __init__(self, document: FakeDocument) -> None:
        self.document = document
        self.added = 0

    def Add(self, progid: str) -> FakeDocument:
        self.added += 1
        return self.document


class FakeDocument:
    def __init__(self, *, end_code: int = -103) -> None:
        self.Body = FakeBody((-0.005, -0.005, -0.005), (0.005, 0.005, 0.005))
        self.Models = FakeModels(MEASURED_PROPERTIES, self.Body)
        self.ProfileSets = FakeProfileSets(end_code=end_code)
        self.ref_plane = FakeRefPlane()
        self.RefPlanes = SimpleNamespace(Item=lambda index: self.ref_plane)
        self.ModelingMode = 2
        self.Title = ""
        self.FullName = ""
        self.closed = 0
        self.closed_with: Any = None
        self.typelib: dict[str, Any] = {
            "status": "ok",
            "methods": {name: {} for name in DOC_MEMBERS},
        }

    def SaveAs(self, path: str) -> None:
        Path(path).write_bytes(b"PAR")
        self.FullName = path

    def SaveCopyAs(self, path: str) -> None:
        Path(path).write_bytes(b"ISO-10303-21;\nHEADER;\nENDSEC;\n")

    def Close(self, save: bool) -> None:
        self.closed += 1
        self.closed_with = save

    @property
    def profile(self) -> FakeProfile | None:
        return self.ProfileSets.profiles.last


class FakeApp:
    Version = "2024.0"
    typelib: ClassVar[dict[str, Any]] = {"status": "ok", "methods": {}}

    def __init__(self, *, end_code: int = -103) -> None:
        self.document = FakeDocument(end_code=end_code)
        self.Documents = FakeDocuments(self.document)
        self.quit_calls = 0

    def Quit(self) -> None:  # pragma: no cover - must never be called
        self.quit_calls += 1


def _run(
    tmp_path: Path,
    *,
    app: FakeApp | None = None,
    binding: FakeBinding | None = None,
    output: Path | None = None,
) -> tuple[ProbeOutcome, FakeApp, FakeBinding]:
    application = app if app is not None else FakeApp()
    active_binding = binding if binding is not None else FakeBinding(app=application)
    session = FakeSession(active_binding)
    outcome = run_probe(
        launch=False,
        data_dir=tmp_path / "data",
        output=output if output is not None else tmp_path / "probe.json",
        active_platform="win32",
        session_factory=lambda: cast(ProbeSession, session),
    )
    return outcome, application, active_binding


def test_non_windows_probe_reports_unsupported_and_native_build_false(tmp_path: Path) -> None:
    output = tmp_path / "probe.json"
    outcome = run_probe(
        launch=False,
        data_dir=tmp_path / "data",
        output=output,
        active_platform="linux",
    )

    assert outcome.exit_code == 2
    report = json.loads(output.read_text(encoding="utf-8"))
    assert report["attach"]["status"] == "unsupported"
    assert report["geometry"]["attempted"] is False
    assert report["calibration_required"] is True
    assert report["native_build_capability"] is False
    assert outcome.calibration is None


def test_attach_not_running_is_reported_without_geometry(tmp_path: Path) -> None:
    app = FakeApp()
    binding = FakeBinding(app=app, attach_error=FakeComError(MK_E_UNAVAILABLE))

    outcome, application, _ = _run(tmp_path, app=app, binding=binding)

    assert outcome.exit_code == 1
    assert outcome.report["attach"]["status"] == "not_running"
    assert outcome.report["geometry"]["attempted"] is False
    assert application.Documents.added == 0


def test_missing_required_enum_skips_geometry_and_never_creates_a_document(
    tmp_path: Path,
) -> None:
    app = FakeApp()
    binding = FakeBinding(app=app, constants={"igSymmetric": 3})

    outcome, application, _ = _run(tmp_path, app=app, binding=binding)

    assert outcome.report["geometry"]["attempted"] is False
    assert "igProfileClosed" in outcome.report["geometry"]["missing_constants"]
    assert application.Documents.added == 0
    assert outcome.calibration is None


def test_resolved_constants_record_the_typelib_source(tmp_path: Path) -> None:
    outcome, _, _ = _run(tmp_path)

    constants = outcome.report["constants"]
    assert constants["resolved"] == {
        "igSymmetric": 3,
        "igProfileClosed": 1,
        "seModelingModeOrdered": 2,
    }
    assert constants["missing"] == []
    assert constants["sources"]["igProfileClosed"] == "SolidEdgePart.igProfileClosedEnum"


def test_missing_runtime_member_skips_geometry_and_closes_the_owned_document(
    tmp_path: Path,
) -> None:
    app = FakeApp()
    del app.document.typelib["methods"]["SaveAs"]

    outcome, application, _ = _run(tmp_path, app=app)

    assert outcome.report["geometry"]["attempted"] is False
    assert any("SaveAs" in item for item in outcome.report["geometry"]["missing_members"])
    assert application.document.closed == 1


def test_measured_cube_records_signed_calibration_and_closes_only_its_document(
    tmp_path: Path,
) -> None:
    outcome, application, _ = _run(tmp_path)

    geometry = outcome.report["geometry"]
    assert geometry["valid"] is True
    assert geometry["volume_mm3"] == pytest.approx(1000.0)
    assert geometry["surface_area_mm2"] == pytest.approx(600.0)
    assert geometry["mass_kg"] == pytest.approx(0.001)
    assert geometry["range_min_mm"] == [-5.0, -5.0, -5.0]
    assert geometry["range_max_mm"] == [5.0, 5.0, 5.0]
    assert geometry["physical_properties_status"] == 1
    assert geometry["ref_plane"] == {
        "index": 1,
        "normal": [0.0, 0.0, 1.0],
        "root_point": [0.0, 0.0, 0.0],
    }
    assert outcome.calibration is not None
    assert outcome.calibration.profile_end_code == -103
    assert outcome.exit_code == 0
    assert application.document.closed == 1
    assert application.document.closed_with is False
    assert application.quit_calls == 0
    assert outcome.report["native_build_capability"] is False
    assert outcome.report["document"]["tagged"] is True

    run_dir = tmp_path / "data" / str(outcome.report["probe_run"]["run_dir"])
    assert (run_dir / "probe.par").read_bytes() == b"PAR"
    assert b"ISO-10303-21" in (run_dir / "probe.stp").read_bytes()
    assert {artifact["name"] for artifact in outcome.report["artifacts"]} == {
        "probe.par",
        "probe.stp",
    }
    store = CalibrationStore(tmp_path / "data")
    assert store.find(se_version="2024.0") is not None


def test_out_parameters_are_plain_sequences_not_byref_variants(tmp_path: Path) -> None:
    outcome, application, _ = _run(tmp_path)

    geometry = outcome.report["geometry"]
    assert geometry["valid"] is True
    # The fakes assert that every out parameter was passed as a plain list and
    # that the returns were plain sequences.
    assert application.document.ref_plane.received == [
        ("GetNormal", [0.0, 0.0, 0.0]),
        ("GetRootPoint", [0.0, 0.0, 0.0]),
    ]
    assert application.document.Body.received == [([0.0, 0.0, 0.0], [0.0, 0.0, 0.0])]
    model = application.document.Models.last_model
    assert model is not None
    assert model.received == [(1000.0, 0.001)]


def test_a_nonzero_profile_end_is_accepted_only_after_measured_geometry(
    tmp_path: Path,
) -> None:
    outcome, application, _ = _run(tmp_path)

    geometry = outcome.report["geometry"]
    assert geometry["profile_end"] == -103
    assert geometry["profile_end_status"] == "accepted"
    assert "extrusion succeeded" in geometry["profile_end_evidence"]
    assert outcome.calibration is not None

    profile = application.document.profile
    assert profile is not None
    assert profile.validation_arguments == [1]


def test_a_zero_profile_end_is_recorded_as_zero_and_still_valid(tmp_path: Path) -> None:
    outcome, application, _ = _run(tmp_path, app=FakeApp(end_code=0))

    geometry = outcome.report["geometry"]
    assert geometry["profile_end"] == 0
    assert geometry["profile_end_status"] == "zero"
    assert geometry["valid"] is True
    assert outcome.calibration is not None
    assert outcome.calibration.profile_end_code == 0
    assert application.document.closed == 1


def test_a_nonzero_profile_end_without_an_extrusion_is_not_calibrated(
    tmp_path: Path,
) -> None:
    app = FakeApp(end_code=-103)

    def reject_once(*args: Any) -> FakeModel:  # noqa: ANN401
        raise FakeComError(E_INVALIDARG)

    app.document.Models.AddFiniteExtrudedProtrusion = reject_once  # type: ignore[method-assign]

    outcome, application, _ = _run(tmp_path, app=app)

    geometry = outcome.report["geometry"]
    assert geometry["profile_end"] == -103
    assert geometry["valid"] is False
    assert "profile_end_status" not in geometry
    assert outcome.calibration is None
    assert application.document.closed == 1


def test_wrong_measured_volume_is_not_calibrated(tmp_path: Path) -> None:
    app = FakeApp()
    app.document.Models.properties = (2.0000000000000004e-06, *MEASURED_PROPERTIES[1:])

    outcome, application, _ = _run(tmp_path, app=app)

    assert outcome.report["geometry"]["valid"] is False
    assert outcome.calibration is None
    assert outcome.exit_code == 1
    assert application.document.closed == 1


def test_a_non_success_physical_status_is_not_calibrated(tmp_path: Path) -> None:
    app = FakeApp()
    app.document.Models.properties = (*MEASURED_PROPERTIES[:-1], 2)

    outcome, application, _ = _run(tmp_path, app=app)

    assert outcome.report["geometry"]["physical_properties_status"] == 2
    assert outcome.report["geometry"]["valid"] is False
    assert outcome.calibration is None
    assert application.document.closed == 1


def test_a_short_physical_properties_tuple_is_not_calibrated(tmp_path: Path) -> None:
    app = FakeApp()
    app.document.Models.properties = (1.0e-06, 6.0e-04)

    outcome, application, _ = _run(tmp_path, app=app)

    geometry = outcome.report["geometry"]
    assert geometry["valid"] is False
    assert geometry["physical_properties_shape"] == "tuple[2]"
    assert "out-parameter tuple" in geometry["reason"]
    assert outcome.calibration is None
    assert application.document.closed == 1


def test_geometry_exception_still_closes_the_owned_document(tmp_path: Path) -> None:
    app = FakeApp()
    app.document.ProfileSets.profiles.fail_lines = True

    outcome, application, _ = _run(tmp_path, app=app)

    assert outcome.report["geometry"]["attempted"] is True
    assert outcome.report["geometry"]["valid"] is False
    assert "error" in outcome.report["geometry"]
    assert application.document.closed == 1
    assert outcome.calibration is None


def test_a_rejected_extrusion_is_not_retried_with_another_argument_form(
    tmp_path: Path,
) -> None:
    app = FakeApp()
    calls: list[tuple[Any, ...]] = []

    def reject_once(*args: Any) -> FakeModel:  # noqa: ANN401
        calls.append(args)
        raise FakeComError(E_INVALIDARG)

    app.document.Models.AddFiniteExtrudedProtrusion = reject_once  # type: ignore[method-assign]

    outcome, application, _ = _run(tmp_path, app=app)

    assert len(calls) == 1
    geometry = outcome.report["geometry"]
    assert geometry["valid"] is False
    assert geometry["attempted"] is True
    assert len(geometry["attempts"]) == 1
    assert geometry["attempts"][0]["ok"] is False
    assert "not retried" in geometry["reason"]
    assert outcome.calibration is None
    assert application.document.closed == 1
    assert application.quit_calls == 0


def test_probe_report_never_contains_the_bearer_token(tmp_path: Path) -> None:
    token = ensure_token(tmp_path / "data")
    outcome, _, _ = _run(tmp_path)

    text = outcome.output.read_text(encoding="utf-8")
    assert token not in text
    assert "Authorization" not in text
