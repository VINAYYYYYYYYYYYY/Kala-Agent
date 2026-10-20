"""Loopback HTTP authentication, request bounds, routing, and artifact serving."""

from __future__ import annotations

import hashlib
import http.client
import json
import os
import socket
import subprocess
import sys
import threading
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path

from kala_solidedge_bridge.auth import ensure_token
from kala_solidedge_bridge.jobs import (
    ArtifactSpec,
    BackendEvent,
    BackendHealth,
    JobContext,
    JobManager,
    Readiness,
)
from kala_solidedge_bridge.protocol import (
    ArtifactFormat,
    BuildAccepted,
    BuildStatus,
    Health,
    JobState,
    StepTrace,
)
from kala_solidedge_bridge.server import BridgeHTTPServer, create_server
from support import build_request


class StubBackend:
    def __init__(self, *, complete: bool = True) -> None:
        self.complete = complete
        self.started = 0
        self.submitted: list[JobContext] = []
        self.events: list[BackendEvent] = []
        self.cancelled = 0
        self.aborted = 0
        self.shutdowns = 0

    def start(self) -> None:
        self.started += 1

    def health(self, *, refresh: bool = False) -> BackendHealth:
        return BackendHealth(
            platform="win32",
            supported=True,
            available=True,
            attached=True,
            se_version="2024.0",
        )

    def submit(self, job: JobContext) -> None:
        self.submitted.append(job)
        if not self.complete:
            self.events = []
            return
        (job.run_dir / "plate.par").write_bytes(b"PAR-data")
        self.events = [
            BackendEvent(
                kind="step",
                trace=StepTrace(seq=0, op="part.measure", status="ok", duration_ms=1.0),
            ),
            BackendEvent(
                kind="completed",
                artifacts=(ArtifactSpec(name="plate.par", format=ArtifactFormat.PAR),),
            ),
        ]

    def poll(self, timeout: float = 0.0) -> list[BackendEvent]:
        if not self.events:
            return []
        return [self.events.pop(0)]

    def cancel(self) -> None:
        self.cancelled += 1
        self.events.append(BackendEvent(kind="cancelled", message="cancelled"))

    def abort(self) -> None:
        self.aborted += 1

    def shutdown(self) -> None:
        self.shutdowns += 1


def ready(_: BackendHealth) -> Readiness:
    return Readiness(
        supported=True,
        available=True,
        calibration_id="cal-test",
        se_version="2024.0",
    )


@dataclass
class Harness:
    server: BridgeHTTPServer
    manager: JobManager
    backend: StubBackend
    token: str

    @property
    def port(self) -> int:
        return int(self.server.server_address[1])


@contextmanager
def harness(tmp_path: Path, *, complete: bool = True) -> Iterator[Harness]:
    data = tmp_path / "data"
    token = ensure_token(data)
    backend = StubBackend(complete=complete)
    manager = JobManager(
        data,
        lambda: backend,
        readiness_provider=ready,
        start_supervisor=False,
    )
    server = create_server(0, manager, token)
    thread = threading.Thread(
        target=server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True
    )
    thread.start()
    try:
        yield Harness(server=server, manager=manager, backend=backend, token=token)
    finally:
        server.shutdown()
        server.server_close()
        manager.close()
        thread.join(5)


def request(
    harness_value: Harness,
    method: str,
    path: str,
    *,
    token: str | None = None,
    body: bytes | None = None,
    headers: dict[str, str] | None = None,
) -> tuple[int, dict[str, str], bytes]:
    connection = http.client.HTTPConnection("127.0.0.1", harness_value.port, timeout=5)
    all_headers = dict(headers or {})
    if token is not None:
        all_headers.setdefault("Authorization", f"Bearer {token}")
    connection.request(method, path, body=body, headers=all_headers)
    response = connection.getresponse()
    payload = response.read()
    result = (
        response.status,
        {key.lower(): value for key, value in response.getheaders()},
        payload,
    )
    connection.close()
    return result


def raw_request(port: int, request_bytes: bytes, *, half_close: bool = False) -> tuple[int, bytes]:
    with socket.create_connection(("127.0.0.1", port), timeout=5) as connection:
        connection.sendall(request_bytes)
        if half_close:
            connection.shutdown(socket.SHUT_WR)
        chunks: list[bytes] = []
        while True:
            chunk = connection.recv(4096)
            if not chunk:
                break
            chunks.append(chunk)
    response = b"".join(chunks)
    status_line = response.split(b"\r\n", 1)[0]
    return int(status_line.split()[1]), response


def json_body(request_id: str = "server-request") -> bytes:
    return json.dumps(
        build_request(request_id).model_dump(mode="json"), separators=(",", ":")
    ).encode("utf-8")


def test_health_requires_authentication_even_on_get(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        status, _, _ = request(active, "GET", "/v1/health")
        assert status == 401

        status, _, _ = request(active, "GET", "/v1/health", token="x" * 32)
        assert status == 401

        status, _, body = request(active, "GET", "/v1/health", token=active.token)
        assert status == 200
        health = Health.model_validate_json(body)
        assert health.protocol_version == 1
        assert health.supported is True
        assert health.available is True


def test_browser_origin_is_rejected(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        status, _, _ = request(
            active,
            "GET",
            "/v1/health",
            token=active.token,
            headers={"Origin": "http://example.test"},
        )
        assert status == 400


def test_hostile_and_duplicate_host_headers_are_rejected(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        status, _, _ = request(
            active,
            "GET",
            "/v1/health",
            token=active.token,
            headers={"Host": "evil.example"},
        )
        assert status == 400

        status, _ = raw_request(
            active.port,
            (
                f"GET /v1/health HTTP/1.1\r\nHost: 127.0.0.1:{active.port}\r\n"
                f"Host: evil.example\r\nAuthorization: Bearer {active.token}\r\n"
                "Connection: close\r\n\r\n"
            ).encode("ascii"),
        )
        assert status == 400


def test_duplicate_authorization_is_rejected(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        status, _ = raw_request(
            active.port,
            (
                f"GET /v1/health HTTP/1.1\r\nHost: 127.0.0.1:{active.port}\r\n"
                f"Authorization: Bearer {active.token}\r\n"
                f"Authorization: Bearer {active.token}\r\n"
                "Connection: close\r\n\r\n"
            ).encode("ascii"),
        )
        assert status == 401


def test_transfer_encoding_and_malformed_length_are_rejected(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        status, _ = raw_request(
            active.port,
            (
                f"POST /v1/builds HTTP/1.1\r\nHost: 127.0.0.1:{active.port}\r\n"
                f"Authorization: Bearer {active.token}\r\n"
                "Transfer-Encoding: chunked\r\n\r\n0\r\n\r\n"
            ).encode("ascii"),
        )
        assert status == 400

        status, _ = raw_request(
            active.port,
            (
                f"POST /v1/builds HTTP/1.1\r\nHost: 127.0.0.1:{active.port}\r\n"
                f"Authorization: Bearer {active.token}\r\n"
                "Content-Length: nope\r\n\r\n"
            ).encode("ascii"),
        )
        assert status == 400


def test_oversized_body_is_rejected_before_reading(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        status, _ = raw_request(
            active.port,
            (
                f"POST /v1/builds HTTP/1.1\r\nHost: 127.0.0.1:{active.port}\r\n"
                f"Authorization: Bearer {active.token}\r\n"
                "Content-Type: application/json\r\nContent-Length: 300000\r\n\r\n"
            ).encode("ascii"),
        )
        assert status == 413


def test_json_objects_are_validated_and_builds_are_accepted(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        status, _, body = request(
            active,
            "POST",
            "/v1/builds",
            token=active.token,
            body=b"{}",
            headers={"Content-Type": "application/json"},
        )
        assert status == 400
        assert json.loads(body)["code"] == "invalid_request"

        status, _, body = request(
            active,
            "POST",
            "/v1/builds",
            token=active.token,
            body=json_body(),
            headers={"Content-Type": "application/json"},
        )
        assert status == 202
        accepted = BuildAccepted.model_validate_json(body)
        assert accepted.state is JobState.QUEUED


def test_build_status_cancel_and_artifact_flow(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        _, _, body = request(
            active,
            "POST",
            "/v1/builds",
            token=active.token,
            body=json_body(),
            headers={"Content-Type": "application/json"},
        )
        accepted = BuildAccepted.model_validate_json(body)
        active.manager.run_once()
        active.manager.run_once()

        status, _, body = request(
            active, "GET", f"/v1/builds/{accepted.build_id}", token=active.token
        )
        assert status == 200
        build_status = BuildStatus.model_validate_json(body)
        assert build_status.state is JobState.SUCCEEDED

        status, headers, artifact = request(
            active,
            "GET",
            f"/v1/builds/{accepted.build_id}/artifacts/plate.par",
            token=active.token,
        )
        assert status == 200
        assert artifact == b"PAR-data"
        assert headers["x-artifact-sha256"] == hashlib.sha256(b"PAR-data").hexdigest()

        status, _, _ = request(
            active,
            "GET",
            f"/v1/builds/{accepted.build_id}/artifacts/missing.par",
            token=active.token,
        )
        assert status == 404

        status, _, body = request(
            active,
            "POST",
            f"/v1/builds/{accepted.build_id}/cancel",
            token=active.token,
        )
        assert status == 200
        assert BuildStatus.model_validate_json(body).state is JobState.SUCCEEDED


def test_cancel_route_is_cooperative(tmp_path: Path) -> None:
    with harness(tmp_path, complete=False) as active:
        _, _, body = request(
            active,
            "POST",
            "/v1/builds",
            token=active.token,
            body=json_body("cancel-me"),
            headers={"Content-Type": "application/json"},
        )
        accepted = BuildAccepted.model_validate_json(body)
        active.manager.run_once()

        status, _, body = request(
            active,
            "POST",
            f"/v1/builds/{accepted.build_id}/cancel",
            token=active.token,
        )
        assert status == 200
        assert BuildStatus.model_validate_json(body).state is JobState.RUNNING

        active.manager.run_once()
        active.manager.run_once()
        assert active.manager.status(accepted.build_id).state is JobState.CANCELLED
        assert active.backend.cancelled == 1


def test_unknown_build_and_traversal_are_not_served(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        status, _, _ = request(active, "GET", "/v1/builds/does-not-exist", token=active.token)
        assert status == 404

        status, _, _ = request(
            active,
            "GET",
            "/v1/builds/abc/artifacts/..%2fjob.json",
            token=active.token,
        )
        assert status == 400


def test_query_fragment_and_wrong_method_are_rejected(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        status, _, _ = request(active, "GET", "/v1/health?x=1", token=active.token)
        assert status == 400

        status, _, _ = request(active, "PUT", "/v1/health", token=active.token)
        assert status == 405


def test_a_mismatched_calibration_is_a_typed_refusal(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        payload = json.dumps(
            build_request("stale-cal", calibration_id="cal-old").model_dump(mode="json"),
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
        assert status == 409
        assert json.loads(body)["code"] == "conflict"
        assert active.backend.submitted == []
        assert not (tmp_path / "data" / "jobs").exists() or not list(
            (tmp_path / "data" / "jobs").iterdir()
        )


def test_a_missing_calibration_is_a_typed_refusal(tmp_path: Path) -> None:
    with harness(tmp_path) as active:
        payload = json.dumps(
            build_request("no-cal", calibration_id=None).model_dump(mode="json"),
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
        assert status == 503
        assert json.loads(body)["code"] == "calibration_required"
        assert active.backend.submitted == []


def test_importing_the_http_surface_never_loads_com_modules() -> None:
    source = Path(__file__).resolve().parents[1] / "src"
    code = (
        "import sys; import kala_solidedge_bridge.server; "
        "assert 'kala_solidedge_bridge.session' not in sys.modules; "
        "assert 'pythoncom' not in sys.modules; "
        "assert 'win32com' not in sys.modules"
    )
    environment = dict(os.environ)
    environment["PYTHONPATH"] = str(source)
    completed = subprocess.run(
        [sys.executable, "-c", code],
        check=False,
        capture_output=True,
        text=True,
        env=environment,
        timeout=30,
    )
    assert completed.returncode == 0, completed.stderr
