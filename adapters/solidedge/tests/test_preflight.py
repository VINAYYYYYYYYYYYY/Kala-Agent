"""Preflight health gating, journal identity, oracle exit codes, and artifact safety.

Every test drives the public ``cli.run`` seam with a real loopback stub bridge,
so the transport, the health preflight, the journal, and the download path all
run for real. The stub is explicitly a test bridge: its geometry is the
closed-form corpus and never a native Solid Edge result.
"""

from __future__ import annotations

import hashlib
import io
import json
import threading
from collections.abc import Iterator
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Final

import pytest

from kala_agent_solidedge import cli
from kala_agent_solidedge.compile import build_request as build_request_for
from kala_agent_solidedge.journal import JOURNAL_FILENAME, Journal, JournalEntry
from kala_agent_solidedge.paths import build_output_dir, safe_build_id
from kala_solidedge_bridge.protocol import BuildRequest

TOKEN: Final[str] = "preflight-token-0123456789"
STEP_BYTES: Final[bytes] = b"ISO-10303-21;\nHEADER;\nENDSEC;\n"
EXAMPLES = Path(__file__).resolve().parents[1] / "examples"


@dataclass
class Stub:
    """A stub bridge whose health, result, and artifact behaviour are scripted."""

    health_overrides: dict[str, object] = field(default_factory=dict)
    volume_mm3: float = 3968.584073
    range_min: list[float] = field(default_factory=lambda: [0.0, 0.0, 0.0])
    range_max: list[float] = field(default_factory=lambda: [20.0, 20.0, 10.0])
    include_result: bool = True
    include_part: bool = True
    build_id: str = "build-1"
    artifact_name: str = "plate.step"
    artifact_bytes: bytes = STEP_BYTES
    requests: list[tuple[str, str, bytes]] = field(default_factory=list)
    submitted: list[BuildRequest] = field(default_factory=list)
    lock: threading.Lock = field(default_factory=threading.Lock)

    def __post_init__(self) -> None:
        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)

    @property
    def url(self) -> str:
        host, port = self.httpd.server_address[0], self.httpd.server_address[1]
        return f"http://{host!s}:{port}"

    def __enter__(self) -> Stub:
        self.thread.start()
        return self

    def __exit__(self, *exc: object) -> None:
        self.httpd.shutdown()
        self.httpd.server_close()
        self.thread.join(timeout=5)

    def _handler(self) -> type[BaseHTTPRequestHandler]:
        stub = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, format: str, *args: object) -> None:
                del format, args

            def do_GET(self) -> None:
                self._serve("GET")

            def do_POST(self) -> None:
                self._serve("POST")

            def _serve(self, method: str) -> None:
                length = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(length) if length else b""
                with stub.lock:
                    stub.requests.append((method, self.path, body))
                if self.headers.get("Authorization") != f"Bearer {TOKEN}":
                    self._json(401, {"code": "invalid_request", "message": "authentication failed"})
                    return
                if method == "GET" and self.path == "/v1/health":
                    self._json(200, stub._health())
                    return
                if method == "POST" and self.path == "/v1/builds":
                    stub.submitted.append(BuildRequest.model_validate_json(body))
                    self._json(202, {"build_id": stub.build_id, "state": "queued"})
                    return
                if method == "GET" and self.path == f"/v1/builds/{stub.build_id}":
                    self._json(200, stub._status())
                    return
                if method == "POST" and self.path == f"/v1/builds/{stub.build_id}/cancel":
                    self._json(200, stub._status())
                    return
                artifact_path = f"/v1/builds/{stub.build_id}/artifacts/{stub.artifact_name}"
                if method == "GET" and self.path == artifact_path:
                    self.send_response(200)
                    self.send_header("Content-Type", "application/step")
                    self.send_header("Content-Length", str(len(stub.artifact_bytes)))
                    self.end_headers()
                    self.wfile.write(stub.artifact_bytes)
                    return
                self._json(404, {"code": "invalid_request", "message": "not found"})

            def _json(self, status: int, payload: object) -> None:
                body = json.dumps(payload).encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        return Handler

    def _health(self) -> dict[str, object]:
        payload: dict[str, object] = {
            "protocol_version": 1,
            "bridge_version": "0.1.0",
            "platform": "win32",
            "supported": True,
            "available": True,
            "busy": False,
            "se_version": "2024",
            "calibration_id": "cal-1",
            "capabilities": ["protocol.v1"],
            "detail": "",
        }
        payload.update(self.health_overrides)
        return payload

    def _status(self) -> dict[str, object]:
        document: dict[str, object] = {
            "build_id": self.build_id,
            "state": "succeeded",
            "step_index": 8,
            "total": 8,
        }
        if not self.include_result:
            return document
        parts: list[dict[str, object]] = []
        if self.include_part:
            parts.append(
                {
                    "name": "plate",
                    "valid": True,
                    "volume_mm3": self.volume_mm3,
                    "surface_area_mm2": 1600.0,
                    "range_box": {"min_mm": self.range_min, "max_mm": self.range_max},
                }
            )
        document["result"] = {
            "build_id": self.build_id,
            "state": "succeeded",
            "parts": parts,
            "artifacts": [
                {
                    "name": self.artifact_name,
                    "format": "step",
                    "media_type": "application/step",
                    "size_bytes": len(self.artifact_bytes),
                    "sha256": hashlib.sha256(self.artifact_bytes).hexdigest(),
                }
            ],
        }
        return document


@pytest.fixture
def stub() -> Iterator[Stub]:
    with Stub() as running:
        yield running


def _run(
    argv: list[str],
    *,
    platform: str = "win32",
) -> tuple[int, str, str]:
    stdout = io.StringIO()
    stderr = io.StringIO()
    code = cli.run(
        argv,
        stdout=stdout,
        stderr=stderr,
        stdin=io.StringIO(""),
        platform=platform,
    )
    return code, stdout.getvalue(), stderr.getvalue()


def _build_arguments(
    stub: Stub,
    tmp_path: Path,
    *,
    script: Path | None = None,
    calibration_id: str = "cal-1",
) -> list[str]:
    token = tmp_path / "bridge.token"
    token.write_text(TOKEN, encoding="ascii")
    return [
        "build",
        "--script",
        str(script or EXAMPLES / "plate_with_hole.json"),
        "--calibration-id",
        calibration_id,
        "--yes",
        "--token-file",
        str(token),
        "--bridge-url",
        stub.url,
        "--output-dir",
        str(tmp_path / "runs"),
        "--timeout",
        "5",
    ]


# ------------------------------------------------------------------ health preflight


@pytest.mark.parametrize(
    ("overrides", "expected"),
    [
        ({"platform": "linux"}, "non-Windows"),
        ({"supported": False}, "native build executor"),
        ({"available": False}, "cannot reach Solid Edge"),
        ({"busy": True}, "busy"),
        ({"calibration_id": None}, "no calibration"),
        ({"calibration_id": "cal-other"}, "stale"),
    ],
)
def test_an_unready_bridge_never_creates_a_job(
    stub: Stub, tmp_path: Path, overrides: dict[str, object], expected: str
) -> None:
    stub.health_overrides = overrides

    code, _, stderr = _run(_build_arguments(stub, tmp_path))

    assert code == cli.EXIT_ERROR
    assert expected in stderr
    assert stub.submitted == []
    assert not [request for request in stub.requests if request[0] == "POST"]
    assert not (tmp_path / "runs" / JOURNAL_FILENAME).exists()


def test_the_health_preflight_runs_before_the_journal_and_submission(
    stub: Stub, tmp_path: Path
) -> None:
    code, _, _ = _run(_build_arguments(stub, tmp_path))

    assert code == cli.EXIT_OK
    methods = [request[0] for request in stub.requests]
    assert methods == ["GET", "POST", "GET", "GET"]
    assert stub.requests[0][1] == "/v1/health"


# ------------------------------------------------------------------ oracle exit codes


def test_a_wrong_volume_is_a_nonzero_exit_despite_a_successful_bridge(
    stub: Stub, tmp_path: Path
) -> None:
    stub.volume_mm3 = 5000.0

    code, _, stderr = _run(_build_arguments(stub, tmp_path))

    assert code == cli.EXIT_ERROR
    assert "oracle mismatch" in stderr


def test_wrong_bounds_are_a_nonzero_exit_despite_a_successful_bridge(
    stub: Stub, tmp_path: Path
) -> None:
    stub.range_max = [25.0, 20.0, 10.0]

    code, _, stderr = _run(_build_arguments(stub, tmp_path))

    assert code == cli.EXIT_ERROR
    assert "oracle mismatch" in stderr


def test_a_successful_build_without_a_result_is_refused(stub: Stub, tmp_path: Path) -> None:
    stub.include_result = False

    code, _, stderr = _run(_build_arguments(stub, tmp_path))

    assert code == cli.EXIT_ERROR
    assert "no result" in stderr


def test_an_unchecked_build_succeeds_but_says_unchecked(stub: Stub, tmp_path: Path) -> None:
    # Two additive features put the script outside the closed-form corpus, so the
    # oracle cannot predict it. Success is allowed, but it must be labelled.
    script = tmp_path / "two_features.json"
    script.write_text(
        json.dumps(
            {
                "summary": "two additive features",
                "components": [
                    {
                        "name": "plate",
                        "steps": [
                            {"op": "sketch", "profile": "rectangle", "width": 20, "height": 20},
                            {"op": "extrude", "length": 10},
                            {"op": "sketch", "profile": "rectangle", "width": 10, "height": 10},
                            {"op": "extrude", "length": 5},
                        ],
                    }
                ],
                "assembly": [],
            }
        ),
        encoding="utf-8",
    )

    code, _, stderr = _run(_build_arguments(stub, tmp_path, script=script))

    assert code == cli.EXIT_OK
    assert "unchecked" in stderr
    assert "not matched" in stderr
    assert "oracle matched" not in stderr


# ------------------------------------------------------------------ artifact directories


def test_two_builds_with_the_same_artifact_name_are_both_preserved(
    stub: Stub, tmp_path: Path
) -> None:
    first = tmp_path / "runs"
    code, _, _ = _run(_build_arguments(stub, tmp_path))
    assert code == cli.EXIT_OK
    first_file = first / "builds" / "build-1" / "plate.step"
    assert first_file.read_bytes() == STEP_BYTES

    stub.build_id = "build-2"
    stub.artifact_bytes = b"ISO-10303-21;\nSECOND;\n"
    code, _, _ = _run(_build_arguments(stub, tmp_path))
    assert code == cli.EXIT_OK

    second_file = first / "builds" / "build-2" / "plate.step"
    assert first_file.read_bytes() == STEP_BYTES
    assert second_file.read_bytes() == stub.artifact_bytes
    assert first_file != second_file


def test_a_traversing_build_id_is_rejected_before_a_path_is_built(tmp_path: Path) -> None:
    for bad in ("../escape", "a/b", "a\\b", "C:stream", ".", "..", "CON", "NUL", "x."):
        with pytest.raises(ValueError):
            build_output_dir(tmp_path, bad)
    assert safe_build_id("build-1") == "build-1"
    assert safe_build_id("build.1") == "build.1"
    assert build_output_dir(tmp_path, "build-1") == tmp_path / "builds" / "build-1"


def test_the_journal_stays_at_the_output_root(stub: Stub, tmp_path: Path) -> None:
    code, _, _ = _run(_build_arguments(stub, tmp_path))

    assert code == cli.EXIT_OK
    assert (tmp_path / "runs" / JOURNAL_FILENAME).is_file()


# ------------------------------------------------------------------ journal identity


def test_a_changed_calibration_does_not_reuse_the_old_request_id(
    stub: Stub, tmp_path: Path
) -> None:
    output = tmp_path / "runs"
    output.mkdir()
    script = EXAMPLES / "plate_with_hole.json"

    # A pending entry for the *same script* but the previous calibration.
    loaded = cli._load_script(script)
    old_request = build_request_for(loaded, request_id="old-cal-request", calibration_id="cal-old")
    Journal(output / JOURNAL_FILENAME).append(
        JournalEntry(
            script_digest=cli._script_digest(loaded),
            request_id="old-cal-request",
            build_id=None,
            status="pending",
            summary="old calibration",
            request_digest=old_request.request_digest(),
            bridge_target=f"{stub.url}|cal-old",
        )
    )

    code, _, _ = _run(_build_arguments(stub, tmp_path))

    assert code == cli.EXIT_OK
    assert len(stub.submitted) == 1
    assert stub.submitted[0].request_id != "old-cal-request"
    assert stub.submitted[0].calibration_id == "cal-1"


def test_a_lost_submission_retry_reuses_the_same_id_and_payload(stub: Stub, tmp_path: Path) -> None:
    output = tmp_path / "runs"
    output.mkdir()
    script = EXAMPLES / "plate_with_hole.json"
    loaded = cli._load_script(script)
    same_request = build_request_for(loaded, request_id="kept-request-id", calibration_id="cal-1")
    Journal(output / JOURNAL_FILENAME).append(
        JournalEntry(
            script_digest=cli._script_digest(loaded),
            request_id="kept-request-id",
            build_id=None,
            status="pending",
            summary="lost submission",
            request_digest=same_request.request_digest(),
            bridge_target=f"{stub.url}|cal-1",
        )
    )

    code, _, _ = _run(_build_arguments(stub, tmp_path))

    assert code == cli.EXIT_OK
    assert [request.request_id for request in stub.submitted] == ["kept-request-id"]
    assert stub.submitted[0].request_digest() == same_request.request_digest()


def test_the_journal_never_records_the_token(stub: Stub, tmp_path: Path) -> None:
    code, _, _ = _run(_build_arguments(stub, tmp_path))

    assert code == cli.EXIT_OK
    text = (tmp_path / "runs" / JOURNAL_FILENAME).read_text(encoding="utf-8")
    assert TOKEN not in text
    assert "bearer" not in text.casefold()
    assert "authorization" not in text.casefold()


def test_a_legacy_ambiguous_entry_refuses_instead_of_resubmitting(
    stub: Stub, tmp_path: Path
) -> None:
    output = tmp_path / "runs"
    output.mkdir()
    loaded = cli._load_script(EXAMPLES / "plate_with_hole.json")
    Journal(output / JOURNAL_FILENAME).append(
        JournalEntry(
            script_digest=cli._script_digest(loaded),
            request_id="legacy-id",
            build_id=None,
            status="pending",
            summary="pre-upgrade entry",
        )
    )

    code, _, stderr = _run(_build_arguments(stub, tmp_path))

    assert code == cli.EXIT_ERROR
    assert stub.submitted == []
    assert not [request for request in stub.requests if request[0] == "POST"]
    assert "legacy-id" in stderr
    assert "cannot prove" in stderr
    assert "reconcile or cancel" in stderr
    # The refusal happens before the journal is appended to.
    entries = Journal(output / JOURNAL_FILENAME).read()
    assert [entry.request_id for entry in entries] == ["legacy-id"]


# ------------------------------------------------------------------ doctor conditions


def _doctor(stub: Stub, tmp_path: Path, *, token: str = TOKEN) -> tuple[int, str, str]:
    token_file = tmp_path / "bridge.token"
    token_file.write_text(token, encoding="ascii")
    stdout = io.StringIO()
    stderr = io.StringIO()
    code = cli.run(
        [
            "doctor",
            "--token-file",
            str(token_file),
            "--bridge-url",
            stub.url,
        ],
        stdout=stdout,
        stderr=stderr,
        stdin=io.StringIO(""),
        platform="win32",
    )
    return code, stdout.getvalue(), stderr.getvalue()


@pytest.mark.parametrize(
    ("overrides", "condition"),
    [
        ({}, "ok"),
        ({"platform": "linux"}, "unsupported"),
        ({"supported": False}, "unsupported"),
        ({"available": False}, "unsupported"),
        ({"busy": True}, "busy"),
        ({"calibration_id": None}, "uncalibrated"),
        ({"protocol_version": 2}, "version"),
    ],
)
def test_doctor_conditions(
    stub: Stub, tmp_path: Path, overrides: dict[str, object], condition: str
) -> None:
    stub.health_overrides = overrides

    code, stdout, _ = _doctor(stub, tmp_path)

    assert json.loads(stdout)["condition"] == condition
    assert (code == cli.EXIT_OK) == (condition == "ok")


def test_doctor_reports_authentication_separately(stub: Stub, tmp_path: Path) -> None:
    code, stdout, _ = _doctor(stub, tmp_path, token="a-different-token-value")

    assert code == cli.EXIT_ERROR
    assert json.loads(stdout)["condition"] == "auth"


def test_status_reports_a_finished_build(stub: Stub, tmp_path: Path) -> None:
    token_file = tmp_path / "bridge.token"
    token_file.write_text(TOKEN, encoding="ascii")
    stdout = io.StringIO()
    stderr = io.StringIO()

    code = cli.run(
        [
            "status",
            "build-1",
            "--token-file",
            str(token_file),
            "--bridge-url",
            stub.url,
        ],
        stdout=stdout,
        stderr=stderr,
        stdin=io.StringIO(""),
        platform="win32",
    )

    assert code == cli.EXIT_OK
    document = json.loads(stdout.getvalue())
    assert document["state"] == "succeeded"
    assert document["result"]["parts"][0]["name"] == "plate"


def test_cancel_says_it_is_cooperative(stub: Stub, tmp_path: Path) -> None:
    token_file = tmp_path / "bridge.token"
    token_file.write_text(TOKEN, encoding="ascii")
    stdout = io.StringIO()
    stderr = io.StringIO()

    code = cli.run(
        [
            "cancel",
            "build-1",
            "--token-file",
            str(token_file),
            "--bridge-url",
            stub.url,
        ],
        stdout=stdout,
        stderr=stderr,
        stdin=io.StringIO(""),
        platform="win32",
    )

    assert code == cli.EXIT_OK
    assert "cooperative" in stderr.getvalue()
    assert "not proven to have stopped" in stderr.getvalue()
