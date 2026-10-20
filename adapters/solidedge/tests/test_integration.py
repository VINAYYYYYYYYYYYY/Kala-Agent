"""An end-to-end build against a stub loopback bridge on a simulated Windows host.

The stub is a real HTTP server, so the client's transport, auth header, journal,
download verification, and oracle comparison all run for real. It is explicitly
a *test* bridge: it never claims to be Solid Edge and its geometry is the
closed-form corpus, not native output.
"""

from __future__ import annotations

import hashlib
import json
import threading
from collections.abc import Iterator
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

from kala_agent_solidedge import cli
from kala_agent_solidedge.compile import build_request as build_request_for
from kala_agent_solidedge.journal import JOURNAL_FILENAME, Journal, JournalEntry
from kala_solidedge_bridge.protocol import BuildRequest

TOKEN = "integration-token-0123456789"
STEP_BYTES = b"ISO-10303-21;\nHEADER;\nENDSEC;\n"


@dataclass
class Stub:
    """A stub bridge that answers with the measured plate-with-hole result."""

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
                    self._json(200, _health())
                    return
                if method == "POST" and self.path == "/v1/builds":
                    stub.submitted.append(BuildRequest.model_validate_json(body))
                    self._json(202, {"build_id": "build-1", "state": "queued"})
                    return
                if method == "GET" and self.path == "/v1/builds/build-1":
                    self._json(200, _status())
                    return
                if method == "GET" and self.path == "/v1/builds/build-1/artifacts/plate.step":
                    self.send_response(200)
                    self.send_header("Content-Type", "application/step")
                    self.send_header("Content-Length", str(len(STEP_BYTES)))
                    self.end_headers()
                    self.wfile.write(STEP_BYTES)
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


def _health() -> dict[str, object]:
    return {
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


def _status() -> dict[str, object]:
    return {
        "build_id": "build-1",
        "state": "succeeded",
        "step_index": 8,
        "total": 8,
        "result": {
            "build_id": "build-1",
            "state": "succeeded",
            "parts": [
                {
                    "name": "plate",
                    "valid": True,
                    "volume_mm3": 3968.584073,
                    "surface_area_mm2": 1600.0,
                    "range_box": {"min_mm": [0.0, 0.0, 0.0], "max_mm": [20.0, 20.0, 10.0]},
                }
            ],
            "artifacts": [
                {
                    "name": "plate.step",
                    "format": "step",
                    "media_type": "application/step",
                    "size_bytes": len(STEP_BYTES),
                    "sha256": hashlib.sha256(STEP_BYTES).hexdigest(),
                }
            ],
        },
    }


@pytest.fixture(name="stub")
def stub_fixture() -> Iterator[Stub]:
    with Stub() as running:
        yield running


def test_a_confirmed_build_submits_downloads_and_matches_the_oracle(
    stub: Stub, tmp_path: Path
) -> None:
    token = tmp_path / "bridge.token"
    token.write_text(TOKEN, encoding="ascii")
    output = tmp_path / "runs"
    code, stdout, stderr = _run(
        [
            "build",
            "--script",
            str(Path(__file__).resolve().parents[1] / "examples" / "plate_with_hole.json"),
            "--calibration-id",
            "cal-1",
            "--yes",
            "--token-file",
            str(token),
            "--bridge-url",
            stub.url,
            "--output-dir",
            str(output),
            "--timeout",
            "5",
        ]
    )

    assert code == cli.EXIT_OK
    assert "oracle matched" in stderr
    assert (output / "builds" / "build-1" / "plate.step").read_bytes() == STEP_BYTES
    assert len(stub.submitted) == 1
    assert stub.submitted[0].calibration_id == "cal-1"
    assert stub.submitted[0].procedure[0].op == "part.new"

    journal = Journal(output / JOURNAL_FILENAME)
    entries = journal.read()
    assert len(entries) == 1
    assert entries[0].build_id == "build-1"
    assert entries[0].status == "succeeded"
    assert TOKEN not in (output / JOURNAL_FILENAME).read_text(encoding="utf-8")
    documents = _documents(stdout)
    assert documents[0]["request_digest"]
    assert documents[-1]["state"] == "succeeded"


def test_an_unconfirmed_build_never_opens_a_socket(stub: Stub, tmp_path: Path) -> None:
    token = tmp_path / "bridge.token"
    token.write_text(TOKEN, encoding="ascii")

    code, _, stderr = _run(
        [
            "build",
            "--script",
            str(Path(__file__).resolve().parents[1] / "examples" / "plate_with_hole.json"),
            "--calibration-id",
            "cal-1",
            "--token-file",
            str(token),
            "--bridge-url",
            stub.url,
            "--output-dir",
            str(tmp_path / "runs"),
        ]
    )

    assert code == cli.EXIT_ERROR
    assert "not confirmed" in stderr
    assert stub.requests == []


def test_a_missing_calibration_id_is_refused_before_submission(stub: Stub, tmp_path: Path) -> None:
    token = tmp_path / "bridge.token"
    token.write_text(TOKEN, encoding="ascii")

    code, _, stderr = _run(
        [
            "build",
            "--script",
            str(Path(__file__).resolve().parents[1] / "examples" / "plate_with_hole.json"),
            "--yes",
            "--token-file",
            str(token),
            "--bridge-url",
            stub.url,
            "--output-dir",
            str(tmp_path / "runs"),
        ]
    )

    assert code == cli.EXIT_ERROR
    assert "calibration" in stderr
    assert stub.requests == []


def test_a_pending_journal_entry_reuses_the_request_id(stub: Stub, tmp_path: Path) -> None:
    token = tmp_path / "bridge.token"
    token.write_text(TOKEN, encoding="ascii")
    output = tmp_path / "runs"
    output.mkdir()
    script = Path(__file__).resolve().parents[1] / "examples" / "plate_with_hole.json"

    loaded = cli._load_script(script)
    digest = cli._script_digest(loaded)
    request = build_request_for(loaded, request_id="lost-request-id", calibration_id="cal-1")
    journal = Journal(output / JOURNAL_FILENAME)
    journal.append(
        JournalEntry(
            script_digest=digest,
            request_id="lost-request-id",
            build_id=None,
            status="pending",
            summary="lost",
            request_digest=request.request_digest(),
            bridge_target=f"{stub.url}|cal-1",
        )
    )

    code, _, _ = _run(
        [
            "build",
            "--script",
            str(script),
            "--calibration-id",
            "cal-1",
            "--yes",
            "--token-file",
            str(token),
            "--bridge-url",
            stub.url,
            "--output-dir",
            str(output),
            "--timeout",
            "5",
        ]
    )

    assert code == cli.EXIT_OK
    assert [request.request_id for request in stub.submitted] == ["lost-request-id"]


def _documents(text: str) -> list[dict[str, object]]:
    """Decode the CLI's consecutive JSON documents."""
    decoder = json.JSONDecoder()
    documents: list[dict[str, object]] = []
    index = 0
    while index < len(text):
        while index < len(text) and text[index].isspace():
            index += 1
        if index >= len(text):
            break
        document, index = decoder.raw_decode(text, index)
        documents.append(document)
    return documents


def _run(argv: list[str]) -> tuple[int, str, str]:
    import io

    stdout = io.StringIO()
    stderr = io.StringIO()
    code = cli.run(
        argv,
        stdout=stdout,
        stderr=stderr,
        stdin=io.StringIO(""),
        platform="win32",
    )
    return code, stdout.getvalue(), stderr.getvalue()
