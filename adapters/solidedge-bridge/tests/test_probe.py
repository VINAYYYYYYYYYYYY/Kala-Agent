"""Probe behavior with a fake COM seam. No test claims a Windows result."""

from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace
from typing import Any, ClassVar, cast

from kala_solidedge_bridge.auth import ensure_token
from kala_solidedge_bridge.calibration import CalibrationStore
from kala_solidedge_bridge.probe import (
    ProbeOutcome,
    ProbeSession,
    run_probe,
)
from kala_solidedge_bridge.session import E_INVALIDARG, MK_E_UNAVAILABLE

DOC_MEMBERS = (
    "Models",
    "ProfileSets",
    "RefPlanes",
    "SaveAs",
    "SaveCopyAs",
    "Close",
)


class FakeComError(Exception):
    def __init__(self, hresult: int) -> None:
        super().__init__(f"fake COM failure {hresult:#x}")
        self.hresult = hresult


class FakeVariant:
    def __init__(self, value: Any) -> None:  # noqa: ANN401
        self.value = value


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
    ) -> tuple[dict[str, int], list[str]]:
        return (
            {name: self.constants[name] for name in names if name in self.constants},
            [name for name in names if name not in self.constants],
        )

    def describe_typelib(self, obj: Any) -> dict[str, Any]:  # noqa: ANN401
        self.described.append(type(obj).__name__)
        typelib = getattr(obj, "typelib", None)
        if typelib is None:
            return {"status": "unavailable", "methods": {}}
        return cast(dict[str, Any], typelib)

    def variant_byref_float(self, value: float) -> FakeVariant:
        return FakeVariant(float(value))

    def variant_byref_int(self, value: int) -> FakeVariant:
        return FakeVariant(int(value))

    def variant_byref_float_array(self, values: list[float]) -> FakeVariant:
        return FakeVariant([float(value) for value in values])

    def variant_byref_variant_array(self, values: list[Any]) -> FakeVariant:
        return FakeVariant(list(values))

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
    def GetNormal(self, buffer: FakeVariant) -> None:
        buffer.value = [0.0, 0.0, 1.0]

    def GetRootPoint(self, buffer: FakeVariant) -> None:
        buffer.value = [0.0, 0.0, 0.0]


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
    def __init__(self, *, fail_lines: bool = False) -> None:
        self.Lines2d = FakeLines()
        self.Lines2d.fail = fail_lines
        self.validation = 0

    def End(self, validation: int) -> int:
        self.validation = validation
        return 0


class FakeProfileSet:
    def __init__(self, *, fail_lines: bool = False) -> None:
        self.Profiles = FakeProfiles(fail_lines=fail_lines)


class FakeProfiles:
    def __init__(self, *, fail_lines: bool = False) -> None:
        self.fail_lines = fail_lines

    def Add(self, ref_plane: FakeRefPlane) -> FakeProfile:
        self.last_ref_plane = ref_plane
        return FakeProfile(fail_lines=self.fail_lines)


class FakeProfileSets:
    def __init__(self, *, fail_lines: bool = False) -> None:
        self.fail_lines = fail_lines

    def Add(self) -> FakeProfileSet:
        self.last = FakeProfileSet(fail_lines=self.fail_lines)
        return self.last


class FakeBody:
    def __init__(self) -> None:
        self.typelib = {"status": "ok", "methods": {"GetRange": {"c_params": 2}}}

    def GetRange(self, minimum: FakeVariant, maximum: FakeVariant) -> None:
        minimum.value = [-0.005, -0.005, -0.005]
        maximum.value = [0.005, 0.005, 0.005]


class FakeModel:
    def __init__(self, volume_m3: float) -> None:
        self.Body = FakeBody()
        self.volume_m3 = volume_m3
        self.typelib = {
            "status": "ok",
            "methods": {"ComputePhysicalProperties": {"c_params": 13}},
        }

    def ComputePhysicalProperties(self, *args: Any) -> None:  # noqa: ANN401
        args[2].value = self.volume_m3
        args[3].value = 0.0006
        args[-2].value = 0.001
        args[-1].value = 0


class FakeModels:
    def __init__(self, volume_m3: float) -> None:
        self.volume_m3 = volume_m3
        self.extrusions: list[tuple[Any, ...]] = []
        self.typelib = {
            "status": "ok",
            "methods": {"AddFiniteExtrudedProtrusion": {"c_params": 4}},
        }

    def AddFiniteExtrudedProtrusion(self, *args: Any) -> FakeModel:  # noqa: ANN401
        self.extrusions.append(args)
        return FakeModel(self.volume_m3)


class FakeDocuments:
    def __init__(self, document: FakeDocument) -> None:
        self.document = document
        self.added = 0

    def Add(self, progid: str) -> FakeDocument:
        self.added += 1
        return self.document


class FakeDocument:
    def __init__(self) -> None:
        self.Models = FakeModels(volume_m3=0.000001)
        self.ProfileSets = FakeProfileSets()
        self.RefPlanes = SimpleNamespace(Item=lambda index: FakeRefPlane())
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


class FakeApp:
    Version = "2024.0"
    typelib: ClassVar[dict[str, Any]] = {"status": "ok", "methods": {}}

    def __init__(self) -> None:
        self.document = FakeDocument()
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
    assert geometry["volume_mm3"] == 1000.0
    assert geometry["range_min_mm"] == [-5.0, -5.0, -5.0]
    assert geometry["range_max_mm"] == [5.0, 5.0, 5.0]
    assert outcome.calibration is not None
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


def test_wrong_measured_volume_is_not_calibrated(tmp_path: Path) -> None:
    app = FakeApp()
    app.document.Models.volume_m3 = 0.000002

    outcome, application, _ = _run(tmp_path, app=app)

    assert outcome.report["geometry"]["valid"] is False
    assert outcome.calibration is None
    assert outcome.exit_code == 1
    assert application.document.closed == 1


def test_geometry_exception_still_closes_the_owned_document(tmp_path: Path) -> None:
    app = FakeApp()
    app.document.ProfileSets.fail_lines = True

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
    original = app.document.Models.AddFiniteExtrudedProtrusion

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
    assert original is not None


def test_probe_report_never_contains_the_bearer_token(tmp_path: Path) -> None:
    token = ensure_token(tmp_path / "data")
    outcome, _, _ = _run(tmp_path)

    text = outcome.output.read_text(encoding="utf-8")
    assert token not in text
    assert "Authorization" not in text
