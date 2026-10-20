"""Edge paths: bounds, malformed input, event decoding, and Linux-safe failures."""

from __future__ import annotations

import io
import json
import socket
import sys
from pathlib import Path
from typing import Any

import pytest

import kala_solidedge_bridge.__main__ as cli
from kala_solidedge_bridge.auth import (
    resolve_data_dir,
    secret_mode,
    token_matches,
    write_secret,
)
from kala_solidedge_bridge.calibration import (
    CalibrationStore,
    canonical_json,
)
from kala_solidedge_bridge.errors import (
    BridgeError,
    InputError,
    UnavailableError,
)
from kala_solidedge_bridge.jobs import (
    ArtifactSpec,
    BackendEvent,
    BackendHealth,
    ChildProcessBackend,
    JobContext,
    JobManager,
    Readiness,
    _event_from_wire,
)
from kala_solidedge_bridge.probe import (
    _cube_measurement_valid,
    _short,
    _step_header_valid,
    _vector,
)
from kala_solidedge_bridge.protocol import (
    ArtifactFormat,
    ErrorCode,
    ErrorResponse,
    JobState,
)
from kala_solidedge_bridge.worker import Worker
from kala_solidedge_bridge.workspace import (
    ArtifactStore,
    DocumentLedger,
    atomic_write_json,
    default_data_dir,
    is_safe_basename,
    read_json,
)
from support import build_request
from test_server import harness, json_body, raw_request, request


def ready(_: BackendHealth) -> Readiness:
    return Readiness(supported=True, available=True, calibration_id="cal-test", se_version="2024.0")


def test_auth_helpers_cover_the_small_surface(tmp_path: Path) -> None:
    assert secret_mode() == 0o600
    write_secret(tmp_path / "secret.txt", "value")
    assert (tmp_path / "secret.txt").read_text(encoding="utf-8") == "value"
    assert resolve_data_dir(None) == default_data_dir()
    assert resolve_data_dir(tmp_path) == tmp_path
    assert not token_matches("a" * 32, "b" * 32)


def test_malformed_token_file_is_refused(tmp_path: Path) -> None:
    from kala_solidedge_bridge.auth import read_token

    (tmp_path / "bridge.token").write_text("short", encoding="ascii")
    with pytest.raises(UnavailableError):
        read_token(tmp_path)
    (tmp_path / "bridge.token").write_bytes(b"\xff\xfe")
    with pytest.raises(UnavailableError):
        read_token(tmp_path)


def test_default_data_dir_honors_xdg(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    assert default_data_dir("linux") == tmp_path / "Kala" / "SolidEdgeBridge"


def test_workspace_rejects_unsafe_and_missing_artifacts(tmp_path: Path) -> None:
    assert not is_safe_basename("../x")
    assert not is_safe_basename("a/b")
    assert not is_safe_basename("")
    assert is_safe_basename("plate.par")
    store = ArtifactStore(tmp_path)
    with pytest.raises(UnavailableError):
        store.register("../escape.par", ArtifactFormat.PAR)
    with pytest.raises(UnavailableError):
        store.register("missing.par", ArtifactFormat.PAR)
    assert store.read_manifest() is None
    with pytest.raises(KeyError):
        store.read_verified("anything.par")
    atomic_write_json(tmp_path / "manifest.json", {"schema": 2, "artifacts": []})
    assert store.read_manifest() is None
    atomic_write_json(tmp_path / "manifest.json", {"schema": 1, "artifacts": "bad"})
    assert store.read_manifest() is None
    assert read_json(tmp_path / "absent.json") is None


def test_document_ledger_appends_privately(tmp_path: Path) -> None:
    ledger = DocumentLedger(tmp_path)
    ledger.append({"event": "created", "tag": "t"})
    ledger.append({"event": "closed", "tag": "t"})
    lines = (tmp_path / "documents.jsonl").read_text(encoding="utf-8").strip().splitlines()
    assert len(lines) == 2
    assert json.loads(lines[0])["event"] == "created"


def test_calibration_helpers_and_invalid_records(tmp_path: Path) -> None:
    assert canonical_json({"b": 1, "a": 2}) == b'{"a":2,"b":1}'
    store = CalibrationStore(tmp_path)
    assert store.load_all() == ()
    records = tmp_path / "calibration"
    records.mkdir(parents=True)
    (records / "broken.json").write_text("{not json", encoding="utf-8")
    (records / "wrong.json").write_text(json.dumps({"schema": 1}), encoding="utf-8")
    assert store.load_all() == ()
    signed = store.record_probe(
        se_version="2024.0",
        volume_mm3=1000.0,
        range_min_mm=(-5.0, -5.0, -5.0),
        range_max_mm=(5.0, 5.0, 5.0),
        par_sha256="a" * 64,
        step_sha256="b" * 64,
    )
    payload = json.loads((records / f"{signed.calibration_id}.json").read_text(encoding="utf-8"))
    payload["signature"] = "0" * 64
    (records / f"{signed.calibration_id}.json").write_text(json.dumps(payload), encoding="utf-8")
    assert store.find(se_version="2024.0") is None


def test_event_decoding_rejects_and_accepts_wire_shapes() -> None:
    assert _event_from_wire("not a dict") is None
    assert _event_from_wire({"kind": "unknown"}) is None
    event = _event_from_wire(
        {
            "kind": "completed",
            "trace": {"seq": 0, "op": "part.measure", "status": "ok", "duration_ms": 0.0},
            "result": {"build_id": "b", "state": "succeeded"},
            "error": {"code": "unavailable", "message": "x"},
            "artifacts": [{"name": "plate.par", "format": "par"}, {"bad": True}, "nope"],
        }
    )
    assert event is not None
    assert event.trace is not None
    assert event.result is not None
    assert event.error is not None
    assert event.artifacts[0].name == "plate.par"


def test_backend_start_failure_marks_the_build_failed(tmp_path: Path) -> None:
    def failing_factory() -> Any:  # noqa: ANN401
        raise UnavailableError("no worker binary")

    manager = JobManager(
        tmp_path,
        failing_factory,
        readiness_provider=ready,
        start_supervisor=False,
    )
    accepted = manager.submit(build_request())
    manager.run_once()

    status = manager.status(accepted.build_id)
    assert status.state is JobState.FAILED
    assert status.result is not None
    assert status.result.error is not None
    assert status.result.error.code is ErrorCode.UNAVAILABLE
    assert manager.health().detail
    manager.close()


def test_warning_and_indeterminate_events_are_handled(tmp_path: Path) -> None:
    backend = _Scripted()
    backend.script = [
        BackendEvent(kind="warning", message="careful"),
        BackendEvent(kind="indeterminate", message="unknown state"),
    ]
    manager = JobManager(
        tmp_path, lambda: backend, readiness_provider=ready, start_supervisor=False
    )
    accepted = manager.submit(build_request())
    manager.run_once()
    manager.run_once()
    manager.run_once()

    status = manager.status(accepted.build_id)
    assert status.state is JobState.INDETERMINATE
    assert status.result is not None
    assert status.result.warnings == ("careful",)
    assert status.result.error is not None
    assert status.result.error.code is ErrorCode.TIMEOUT_INDETERMINATE
    with pytest.raises(KeyError):
        manager.reconcile("missing-build")
    manager.reconcile(accepted.build_id)
    manager.close()


def test_missing_artifact_and_terminal_cancel_paths(tmp_path: Path) -> None:
    backend = _Scripted()
    backend.script = [
        BackendEvent(kind="completed", artifacts=(ArtifactSpec("gone.par", ArtifactFormat.PAR),))
    ]
    manager = JobManager(
        tmp_path, lambda: backend, readiness_provider=ready, start_supervisor=False
    )
    accepted = manager.submit(build_request())
    manager.run_once()
    manager.run_once()

    status = manager.status(accepted.build_id)
    assert status.state is JobState.SUCCEEDED
    assert status.result is not None
    assert status.result.warnings
    assert manager.cancel(accepted.build_id).state is JobState.SUCCEEDED
    with pytest.raises(InputError):
        manager.reconcile(accepted.build_id)
    with pytest.raises(UnavailableError):
        manager.artifact(accepted.build_id, "../escape.par")
    assert manager.health().busy is False
    manager.close()


class _Scripted:
    def __init__(self) -> None:
        self.script: list[BackendEvent] = []
        self.events: list[BackendEvent] = []
        self.submitted: list[JobContext] = []

    def start(self) -> None:
        return None

    def health(self, *, refresh: bool = False) -> BackendHealth:
        return BackendHealth(platform="win32", supported=True, available=True, se_version="2024.0")

    def submit(self, job: JobContext) -> None:
        self.submitted.append(job)
        self.events.extend(self.script)

    def poll(self, timeout: float = 0.0) -> list[BackendEvent]:
        return [self.events.pop(0)] if self.events else []

    def cancel(self) -> None:
        return None

    def abort(self) -> None:
        return None

    def shutdown(self) -> None:
        return None


def test_child_backend_health_refresh_and_double_abort(tmp_path: Path) -> None:
    backend = ChildProcessBackend(tmp_path)
    backend.start()
    try:
        assert backend.health(refresh=True).platform == sys.platform
    finally:
        backend.abort()
        backend.abort()


def test_probe_small_helpers() -> None:
    assert _short("a\nb", 10) == "a b"
    assert _vector([1, 2, 3], 3) == (1.0, 2.0, 3.0)
    with pytest.raises(ValueError):
        _vector([1, 2], 3)
    assert _cube_measurement_valid(1000.0, (-5, -5, -5), (5, 5, 5))
    assert not _cube_measurement_valid(2000.0, (-5, -5, -5), (5, 5, 5))
    assert not _cube_measurement_valid(1000.0, (-5, -5, -5), (5, 5, 6))
    assert not _step_header_valid(Path("/nonexistent/step"))


def test_cli_token_failure_on_unusable_data_dir(tmp_path: Path) -> None:
    blocked = tmp_path / "blocked"
    blocked.write_text("not a directory", encoding="utf-8")
    assert cli.main(["token", "--data-dir", str(blocked)]) == 1


def test_cli_serve_reports_a_port_conflict(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(sys, "platform", "win32")
    with socket.socket() as holder:
        holder.bind(("127.0.0.1", 0))
        holder.listen(1)
        port = holder.getsockname()[1]
        assert cli.main(["serve", "--port", str(port), "--data-dir", str(tmp_path)]) == 1


def test_public_serve_refuses_non_windows_before_creating_anything(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    data_dir = tmp_path / "data"
    monkeypatch.setattr(sys, "platform", "linux")

    assert cli.main(["serve", "--port", "8797", "--data-dir", str(data_dir)]) == 2

    assert not data_dir.exists()


def test_public_serve_never_calls_same_thread_shutdown(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A KeyboardInterrupt inside the serving loop must return, not deadlock."""
    from kala_solidedge_bridge import server as server_module

    monkeypatch.setattr(sys, "platform", "win32")
    closed: list[bool] = []
    shutdowns: list[bool] = []

    class LoopServer:
        server_address = ("127.0.0.1", 8797)

        def serve_forever(self, poll_interval: float = 0.5) -> None:
            raise KeyboardInterrupt

        def server_close(self) -> None:
            closed.append(True)

        def shutdown(self) -> None:  # pragma: no cover - must not be called
            shutdowns.append(True)

    class Manager:
        def close(self) -> None:
            closed.append(True)

    monkeypatch.setattr(server_module, "ensure_token", lambda _: "t" * 32)
    monkeypatch.setattr(server_module, "production_manager", lambda _: Manager())
    monkeypatch.setattr(server_module, "create_server", lambda *_, **__: LoopServer())
    monkeypatch.setattr(server_module, "token_path", lambda _: tmp_path / "bridge.token")

    assert server_module.serve(8797, tmp_path) == 0
    assert shutdowns == []
    assert closed == [True, True]


def test_server_method_and_route_edges(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        for method in ("HEAD", "PUT", "PATCH", "DELETE"):
            status, _, _ = request(active, method, "/v1/health", token=active.token)
            assert status == 405

        status, _, _ = request(active, "OPTIONS", "/v1/health", token=active.token)
        assert status == 405

        status, _, _ = request(active, "POST", "/v1/health", token=active.token)
        assert status == 404

        status, _, _ = request(active, "GET", "/v1/unknown", token=active.token)
        assert status == 404

        status, _, _ = request(active, "GET", "/v1/builds/abc/artifacts", token=active.token)
        assert status == 404

        status, _, _ = request(active, "GET", "/v1/builds/unknown-build", token=active.token)
        assert status == 404

        status, _, _ = request(
            active, "GET", "/v1/builds/abc/artifacts/bad$name", token=active.token
        )
        assert status == 400

        status, _, _ = request(
            active,
            "POST",
            "/v1/builds",
            token=active.token,
            body=json_body(),
            headers={"Content-Type": "text/plain"},
        )
        assert status == 400

        status, _, _ = request(
            active,
            "POST",
            "/v1/builds",
            token=active.token,
            body=b"\xff\xfe",
            headers={"Content-Type": "application/json"},
        )
        assert status == 400


def test_server_rejects_duplicate_length_and_short_body(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        status, _ = raw_request(
            active.port,
            (
                f"POST /v1/builds HTTP/1.1\r\nHost: 127.0.0.1:{active.port}\r\n"
                f"Authorization: Bearer {active.token}\r\n"
                "Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}"
            ).encode("ascii"),
        )
        assert status == 400

        status, _ = raw_request(
            active.port,
            (
                f"POST /v1/builds HTTP/1.1\r\nHost: 127.0.0.1:{active.port}\r\n"
                f"Authorization: Bearer {active.token}\r\n"
                "Content-Type: application/json\r\nContent-Length: 10\r\n\r\nabc"
            ).encode("ascii"),
            half_close=True,
        )
        assert status == 400


def test_server_conflict_on_same_id_different_payload(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        for summary in ("first", "second"):
            payload = json.dumps(
                build_request("same-id", summary=summary).model_dump(mode="json"),
                separators=(",", ":"),
            ).encode("utf-8")
            status, _, body = request(
                active,
                "POST",
                "/v1/builds",
                token=active.token,
                body=payload,
                headers={"Content-Type": "application/json"},
            )
            if summary == "first":
                assert status == 202
            else:
                assert status == 409
                assert json.loads(body)["code"] == "conflict"


def test_worker_health_on_emulated_windows_paths(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(sys, "platform", "win32")

    class Attached:
        Version = "2024.0"

    class Binding:
        def __init__(self, error: Exception | None) -> None:
            self.error = error

        def attach(self, launch: bool) -> Any:  # noqa: ANN401
            if self.error is not None:
                raise self.error
            return Attached()

        def version(self, application: Any) -> str:  # noqa: ANN401
            return str(application.Version)

    class Session:
        def __init__(
            self, *, error: Exception | None = None, start_error: Exception | None = None
        ) -> None:
            self.error = error
            self.start_error = start_error
            self.stopped = False

        def start(self, timeout: float = 15.0) -> None:
            if self.start_error is not None:
                raise self.start_error

        def stop(self) -> bool:
            self.stopped = True
            return True

        def run(self, operation: Any, *, timeout: float | None = None) -> Any:  # noqa: ANN401
            return operation(Binding(self.error))

    healthy = Session()
    worker = Worker(tmp_path, session_factory=lambda: healthy)  # type: ignore[arg-type]
    assert worker.health()["available"] is True
    assert worker.health()["se_version"] == "2024.0"

    from kala_solidedge_bridge.session import ComUnavailable
    from kala_solidedge_bridge.session import Session as RealSession

    not_running = Worker(
        tmp_path,
        session_factory=lambda: Session(error=ComUnavailable("not running")),  # type: ignore[arg-type]
    )
    assert not_running.health()["available"] is False

    broken = Worker(
        tmp_path,
        session_factory=lambda: Session(start_error=ComUnavailable("no pywin32")),  # type: ignore[arg-type]
    )
    assert broken.health()["available"] is False
    assert RealSession is not None


def test_worker_run_loop_processes_commands(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    commands = "\n".join(
        [
            "not json",
            json.dumps({"cmd": "unknown"}),
            json.dumps({"cmd": "cancel"}),
            json.dumps({"cmd": "build", "build_id": "b1"}),
            json.dumps({"cmd": "health"}),
            json.dumps({"cmd": "shutdown"}),
        ]
    )
    monkeypatch.setattr(sys, "stdin", io.StringIO(commands + "\n"))
    output = io.StringIO()
    monkeypatch.setattr(sys, "stdout", output)

    assert Worker(tmp_path).run() == 0
    lines = [json.loads(line) for line in output.getvalue().strip().splitlines()]
    kinds = [line["type"] for line in lines]
    assert "error" in kinds
    assert "cancel_ack" in kinds
    assert "build_started" in kinds
    assert "health" in kinds
    assert "shutdown_ack" in kinds
    failed = [line for line in lines if line["type"] == "event"]
    assert failed and failed[0]["event"]["error"]["code"] == "unavailable"
    assert capsys.readouterr().out == ""


def test_bridge_error_exposes_a_response() -> None:
    error = BridgeError(ErrorCode.BUSY, "x" * 600)
    response = error.to_response()
    assert isinstance(response, ErrorResponse)
    assert len(response.message) == 500
