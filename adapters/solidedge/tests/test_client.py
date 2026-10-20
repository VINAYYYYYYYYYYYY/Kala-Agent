"""The client is tested against a real loopback HTTP server, not a mock opener."""

from __future__ import annotations

import hashlib
import json
import threading
from collections.abc import Iterator
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Final

import pytest

from kala_agent_solidedge.client import BridgeClient, validate_bridge_url
from kala_agent_solidedge.errors import (
    ArtifactError,
    AuthenticationError,
    ClientError,
    NetworkError,
    ProtocolError,
)
from kala_solidedge_bridge.protocol import (
    Artifact,
    ArtifactFormat,
    BuildRequest,
    Health,
    JobState,
    PartMeasure,
    PartNew,
    Plane,
    SketchRectangle,
    SolidExtrude,
)

TOKEN: Final[str] = "test-token-0123456789abcdef"


@dataclass
class Response:
    status: int
    body: bytes
    content_type: str = "application/json"
    headers: dict[str, str] = field(default_factory=dict)


@dataclass
class Recorded:
    method: str
    path: str
    headers: dict[str, str]
    body: bytes


class Server:
    """A one-thread loopback server that returns queued responses."""

    def __init__(self) -> None:
        self.requests: list[Recorded] = []
        self.responses: list[Response] = []
        self.lock = threading.Lock()
        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)

    @property
    def url(self) -> str:
        host, port = self.httpd.server_address[0], self.httpd.server_address[1]
        return f"http://{host!s}:{port}"

    def __enter__(self) -> Server:
        self.thread.start()
        return self

    def __exit__(self, *exc: object) -> None:
        self.httpd.shutdown()
        self.httpd.server_close()
        self.thread.join(timeout=5)

    def queue(self, response: Response) -> None:
        with self.lock:
            self.responses.append(response)

    def _handler(self) -> type[BaseHTTPRequestHandler]:
        server = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, format: str, *args: object) -> None:
                del format, args

            def do_GET(self) -> None:
                self._serve()

            def do_POST(self) -> None:
                self._serve()

            def _serve(self) -> None:
                length = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(length) if length else b""
                server.requests.append(
                    Recorded(
                        method=self.command,
                        path=self.path,
                        headers={key.lower(): value for key, value in self.headers.items()},
                        body=body,
                    )
                )
                with server.lock:
                    response = (
                        server.responses.pop(0)
                        if server.responses
                        else Response(500, b'{"code":"internal_error","message":"no response"}')
                    )
                self.send_response(response.status)
                self.send_header("Content-Type", response.content_type)
                self.send_header("Content-Length", str(len(response.body)))
                for name, value in response.headers.items():
                    self.send_header(name, value)
                self.end_headers()
                self.wfile.write(response.body)

        return Handler


def request() -> BuildRequest:
    return BuildRequest(
        request_id="req-1",
        calibration_id="cal-1",
        summary="one part",
        procedure=(
            PartNew(name="plate"),
            SketchRectangle(plane=Plane.XY, width=20, height=20),
            SolidExtrude(length=10),
            PartMeasure(),
        ),
    )


def artifact(data: bytes, *, name: str = "plate.step") -> Artifact:
    return Artifact(
        name=name,
        format=ArtifactFormat.STEP,
        media_type="application/step",
        size_bytes=len(data),
        sha256=hashlib.sha256(data).hexdigest(),
    )


def health_body(**overrides: object) -> bytes:
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
    payload.update(overrides)
    return json.dumps(payload).encode("utf-8")


def status_body(state: str = "succeeded") -> bytes:
    return json.dumps(
        {
            "build_id": "build-1",
            "state": state,
            "step_index": 1,
            "total": 1,
        }
    ).encode("utf-8")


@pytest.fixture
def server() -> Iterator[Server]:
    with Server() as running:
        yield running


def test_health_is_authenticated_and_typed(server: Server) -> None:
    server.queue(Response(200, health_body()))
    client = BridgeClient(base_url=server.url, token=TOKEN)

    health = client.health()

    assert isinstance(health, Health)
    assert health.calibration_id == "cal-1"
    recorded = server.requests[0]
    assert recorded.headers["authorization"] == f"Bearer {TOKEN}"
    assert recorded.path == "/v1/health"


def test_a_rejected_token_is_an_authentication_error_not_a_leak(server: Server) -> None:
    server.queue(Response(401, b'{"code":"invalid_request","message":"authentication failed"}'))
    client = BridgeClient(base_url=server.url, token=TOKEN)

    with pytest.raises(AuthenticationError) as caught:
        client.health()

    assert TOKEN not in str(caught.value)
    assert "token" in str(caught.value)


def test_a_redirect_is_refused_not_followed(server: Server) -> None:
    server.queue(Response(302, b"", headers={"Location": "http://example.com/"}))
    client = BridgeClient(base_url=server.url, token=TOKEN)

    with pytest.raises(ProtocolError):
        client.health()
    assert len(server.requests) == 1


def test_an_oversize_json_body_is_refused_before_it_is_parsed(server: Server) -> None:
    server.queue(Response(200, b"{" + b" " * (1024 * 1024) + b"}"))
    client = BridgeClient(base_url=server.url, token=TOKEN)

    with pytest.raises(ProtocolError, match="1 MiB"):
        client.health()


def test_a_non_protocol_document_is_a_protocol_error(server: Server) -> None:
    server.queue(Response(200, json.dumps({"unexpected": True}).encode("utf-8")))
    client = BridgeClient(base_url=server.url, token=TOKEN)

    with pytest.raises(ProtocolError):
        client.health()


def test_submit_posts_the_typed_request(server: Server) -> None:
    server.queue(Response(202, b'{"build_id":"build-1","state":"queued"}'))
    client = BridgeClient(base_url=server.url, token=TOKEN)

    accepted = client.submit(request())

    assert accepted.build_id == "build-1"
    recorded = server.requests[0]
    assert recorded.method == "POST"
    assert recorded.headers["content-type"] == "application/json"
    assert BuildRequest.model_validate_json(recorded.body).request_id == "req-1"


def test_a_lost_submission_is_reported_as_ambiguous(server: Server) -> None:
    client = BridgeClient(base_url=server.url, token=TOKEN)
    server.httpd.shutdown()
    server.httpd.server_close()
    server.thread.join(timeout=5)

    with pytest.raises(NetworkError, match="may or may not"):
        client.submit(request())


def test_status_and_cancel_use_the_build_id(server: Server) -> None:
    server.queue(Response(200, status_body("running")))
    server.queue(Response(200, status_body("cancelled")))
    client = BridgeClient(base_url=server.url, token=TOKEN)

    assert client.status("build-1").state is JobState.RUNNING
    assert client.cancel("build-1").state is JobState.CANCELLED
    assert server.requests[0].path == "/v1/builds/build-1"
    assert server.requests[1].path == "/v1/builds/build-1/cancel"


def test_a_build_id_with_a_separator_is_refused_locally(server: Server) -> None:
    client = BridgeClient(base_url=server.url, token=TOKEN)

    with pytest.raises(ClientError):
        client.status("../etc/passwd")
    assert server.requests == []


def test_download_verifies_and_publishes_atomically(server: Server, tmp_path: Path) -> None:
    data = b"ISO-10303-21;\nHEADER;\n"
    server.queue(Response(200, data, content_type="application/step"))
    client = BridgeClient(base_url=server.url, token=TOKEN)

    download = client.download("build-1", artifact(data), tmp_path)

    assert download.path.read_bytes() == data
    assert download.path.parent == tmp_path
    assert sorted(item.name for item in tmp_path.iterdir()) == ["plate.step"]


def test_a_hash_mismatch_discards_the_file(server: Server, tmp_path: Path) -> None:
    declared = artifact(b"good bytes")
    server.queue(Response(200, b"evil bytes"))
    client = BridgeClient(base_url=server.url, token=TOKEN)

    with pytest.raises(ArtifactError, match="digest"):
        client.download("build-1", declared, tmp_path)
    assert list(tmp_path.iterdir()) == []


def test_a_size_mismatch_discards_the_file(server: Server, tmp_path: Path) -> None:
    declared = artifact(b"1234")
    server.queue(Response(200, b"12345"))
    client = BridgeClient(base_url=server.url, token=TOKEN)

    with pytest.raises(ArtifactError, match="bytes"):
        client.download("build-1", declared, tmp_path)
    assert list(tmp_path.iterdir()) == []


def test_an_unsafe_artifact_name_is_refused_locally(server: Server, tmp_path: Path) -> None:
    # Defense in depth: the wire pattern already forbids separators, so build an
    # Artifact that only this local guard can refuse.
    declared = Artifact.model_construct(
        name="../escape.step",
        format=ArtifactFormat.STEP,
        media_type="application/step",
        size_bytes=4,
        sha256=hashlib.sha256(b"data").hexdigest(),
    )
    client = BridgeClient(base_url=server.url, token=TOKEN)

    with pytest.raises(ArtifactError):
        client.download("build-1", declared, tmp_path)
    assert server.requests == []


@pytest.mark.parametrize(
    "url",
    [
        "https://127.0.0.1:8797",
        "http://localhost:8797",
        "http://192.168.1.5:8797",
        "http://user:pass@127.0.0.1:8797",
        "http://127.0.0.1:8797/base",
        "http://127.0.0.1:8797/?a=1",
        "http://127.0.0.1:8797/#frag",
        "ftp://127.0.0.1:8797",
    ],
)
def test_only_a_plain_loopback_base_url_is_accepted(url: str) -> None:
    with pytest.raises(ClientError):
        validate_bridge_url(url)


def test_a_loopback_url_is_normalised() -> None:
    assert validate_bridge_url("http://127.0.0.1:8797/") == "http://127.0.0.1:8797"
    assert validate_bridge_url("http://127.0.0.1") == "http://127.0.0.1"


def test_an_empty_token_is_refused() -> None:
    with pytest.raises(AuthenticationError):
        BridgeClient(base_url="http://127.0.0.1:8797", token="  ")


def test_a_missing_token_file_is_an_authentication_error(tmp_path: Path) -> None:
    with pytest.raises(AuthenticationError, match="does not exist"):
        BridgeClient.from_token_file(tmp_path / "bridge.token")


def test_wait_returns_a_terminal_status_without_unbounded_polling(server: Server) -> None:
    server.queue(Response(200, status_body("queued")))
    server.queue(Response(200, status_body("succeeded")))
    client = BridgeClient(base_url=server.url, token=TOKEN)

    status = client.wait("build-1", timeout=5.0, interval=0.0, sleep=lambda _: None)

    assert status.state is JobState.SUCCEEDED
    assert len(server.requests) == 2


def test_wait_times_out_with_a_message_that_keeps_the_build_id(server: Server) -> None:
    server.queue(Response(200, status_body("running")))
    server.queue(Response(200, status_body("running")))
    client = BridgeClient(base_url=server.url, token=TOKEN)
    clock = iter([0.0, 0.0, 5.0])

    with pytest.raises(NetworkError, match="keep the build id"):
        client.wait(
            "build-1",
            timeout=1.0,
            interval=0.0,
            clock=lambda: next(clock, 5.0),
            sleep=lambda _: None,
        )


class _Drip:
    """A response-like object that returns one chunk per call."""

    def __init__(self, chunks: list[bytes]) -> None:
        self._chunks = chunks

    def read(self, size: int) -> bytes:
        del size
        return self._chunks.pop(0) if self._chunks else b""


def test_an_overall_deadline_abandons_a_drip_fed_json_body() -> None:
    from kala_agent_solidedge.client import _read_capped

    clock = iter([0.0, 0.0, 99.0])
    with pytest.raises(NetworkError, match="overall timeout"):
        _read_capped(
            _Drip([b"{}", b"{}"]),
            deadline=1.0,
            clock=lambda: next(clock, 99.0),
        )


def test_the_json_reader_stops_at_the_cap_without_trusting_content_length() -> None:
    from kala_agent_solidedge.client import MAX_JSON_BYTES, _read_capped

    over = _Drip([b"x" * (MAX_JSON_BYTES + 1)])
    with pytest.raises(ProtocolError, match="1 MiB"):
        _read_capped(over)


def test_an_artifact_stream_that_exceeds_the_overall_timeout_is_discarded(
    server: Server, tmp_path: Path
) -> None:
    data = b"abcdefgh"
    declared = artifact(data)
    server.queue(Response(200, data, content_type="application/step"))
    clock = iter([0.0, 5.0, 5.0, 5.0])
    client = BridgeClient(
        base_url=server.url,
        token=TOKEN,
        artifact_timeout=1.0,
        clock=lambda: next(clock, 5.0),
    )

    with pytest.raises(NetworkError, match="overall timeout"):
        client.download("build-1", declared, tmp_path)
    assert list(tmp_path.iterdir()) == []
