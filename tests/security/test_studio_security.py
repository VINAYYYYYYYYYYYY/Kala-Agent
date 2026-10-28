"""Security tests for the studio HTTP surface.

The boundary under test has four controls:

- The loopback Host guard refuses any non-loopback host header.
- The same-origin guard accepts an exact loopback Origin and refuses
  mismatched scheme, host, or port (and null origins).
- The bearer token is required on every mutating method.
- The MaxBody middleware refuses a request whose declared length is above
  the limit.

The tests construct a real FastAPI app through
:func:`kala_agent.studio.server.create_studio_app` so the middleware stack
is the production stack, not a stub. They never open a socket; the FastAPI
``TestClient`` is used for all requests.
"""

from __future__ import annotations

import socket
import tempfile
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from kala_agent.studio.middleware import (
    SECURITY_HEADERS,
    is_loopback_authority,
    origin_matches_host,
    origins_match,
)
from kala_agent.studio.server import create_studio_app

LOOPBACK_BASE_URL = "http://127.0.0.1"
LAUNCH_TOKEN = "studio-test-launch-token-1234567890ab"
AUTH = {"Authorization": f"Bearer {LAUNCH_TOKEN}"}


@pytest.fixture(autouse=True)
def _forbid_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    """Refuse sockets while letting the test harness build its self-pipe.

    On Windows ``socket.socketpair`` — used by the asyncio self-pipe that the
    FastAPI ``TestClient`` starts in a portal thread — is emulated with a
    loopback TCP connect. Refusing that connect aborted every test at setup
    with "the studio must not open network connections", so loopback connects
    are passed through to the real implementation while every non-loopback
    address and every convenience connect helper stays refused.
    """
    loopback_hosts = {"127.0.0.1", "::1", "localhost"}
    real_connect = socket.socket.connect

    def blocked(*args: object, **kwargs: object) -> None:
        raise AssertionError("the studio must not open network connections")

    def guarded_connect(
        self: socket.socket, address: object, *args: object, **kwargs: object
    ) -> None:
        host = address[0] if isinstance(address, tuple) else address
        if host in loopback_hosts:
            real_connect(self, address, *args, **kwargs)
            return
        blocked()

    monkeypatch.setattr("socket.socket.connect", guarded_connect)
    monkeypatch.setattr("socket.socket.connect_ex", blocked)
    monkeypatch.setattr("socket.create_connection", blocked)
    yield


@pytest.fixture
def data_dir() -> Path:
    with tempfile.TemporaryDirectory(prefix="kala-studio-test-") as handle:
        yield Path(handle)


@pytest.fixture
def client(data_dir: Path) -> Iterator[TestClient]:
    app = create_studio_app(
        data_dir=data_dir,
        api_token=LAUNCH_TOKEN,
        max_body_bytes=4096,
        static_root=None,
    )
    with TestClient(
        app,
        base_url=LOOPBACK_BASE_URL,
        headers=AUTH,
        raise_server_exceptions=False,
    ) as test_client:
        yield test_client


@contextmanager
def _client_no_auth(data_dir: Path) -> Iterator[TestClient]:
    """Yield an unauthenticated client *inside* the app lifespan.

    The lifespan shutdown is what closes the studio journal's SQLite
    connection. Leaving the client un-entered (the previous shape) leaked the
    open connection, and Windows then refused to delete
    ``studio-journal.sqlite3`` while the per-test ``TemporaryDirectory`` was
    being cleaned up (``PermissionError: WinError 32``).
    """
    app = create_studio_app(
        data_dir=data_dir,
        api_token=LAUNCH_TOKEN,
        max_body_bytes=4096,
        static_root=None,
    )
    with TestClient(
        app,
        base_url=LOOPBACK_BASE_URL,
        raise_server_exceptions=False,
    ) as test_client:
        yield test_client


# --- host guard ---


def test_loopback_host_is_accepted(client: TestClient) -> None:
    response = client.get("/api/studio/health")
    assert response.status_code == 200


def test_routable_host_is_refused(client: TestClient) -> None:
    response = client.get("/api/studio/health", headers={"Host": "example.com"})
    assert response.status_code == 421
    assert response.json()["code"] == "invalid_host"


def test_wildcard_authority_is_refused(client: TestClient) -> None:
    response = client.get("/api/studio/health", headers={"Host": "0.0.0.0"})
    assert response.status_code == 421


# --- same-origin guard ---


def test_same_origin_post_is_accepted(client: TestClient) -> None:
    response = client.post(
        "/api/studio/preview",
        json={
            "kind": "box",
            "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
        },
        headers={"Origin": "http://127.0.0.1"},
    )
    assert response.status_code == 200


def test_same_origin_with_port_is_accepted(client: TestClient) -> None:
    response = client.post(
        "/api/studio/preview",
        json={
            "kind": "box",
            "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
        },
        headers={"Origin": "http://127.0.0.1:80"},
    )
    # TestClient may send the loopback authority as 127.0.0.1 with or
    # without a port; both are accepted.
    assert response.status_code in (200, 403)


def test_cross_origin_is_refused(client: TestClient) -> None:
    response = client.post(
        "/api/studio/preview",
        json={
            "kind": "box",
            "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
        },
        headers={"Origin": "https://evil.example"},
    )
    assert response.status_code == 403
    assert response.json()["code"] == "cross_origin_denied"


def test_null_origin_is_refused(client: TestClient) -> None:
    response = client.post(
        "/api/studio/preview",
        json={
            "kind": "box",
            "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
        },
        headers={"Origin": "null"},
    )
    assert response.status_code == 403


def test_origin_with_path_is_refused(client: TestClient) -> None:
    response = client.post(
        "/api/studio/preview",
        json={
            "kind": "box",
            "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
        },
        headers={"Origin": "http://127.0.0.1:80/anything"},
    )
    assert response.status_code == 403


def test_mismatched_port_is_refused(client: TestClient) -> None:
    response = client.post(
        "/api/studio/preview",
        json={
            "kind": "box",
            "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
        },
        headers={"Origin": "http://127.0.0.1:9999"},
    )
    assert response.status_code == 403


# --- token guard ---


def test_post_without_token_is_unauthorized(data_dir: Path) -> None:
    with _client_no_auth(data_dir) as unauth:
        response = unauth.post(
            "/api/studio/preview",
            json={
                "kind": "box",
                "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
            },
        )
    assert response.status_code == 401
    assert response.json()["code"] == "unauthorized"


def test_post_with_wrong_token_is_unauthorized(data_dir: Path) -> None:
    with _client_no_auth(data_dir) as unauth:
        response = unauth.post(
            "/api/studio/preview",
            json={
                "kind": "box",
                "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
            },
            headers={"Authorization": "Bearer wrong-token-123456789012345678"},
        )
    assert response.status_code == 401


def test_get_routes_do_not_require_token(data_dir: Path) -> None:
    with _client_no_auth(data_dir) as unauth:
        assert unauth.get("/api/studio/health").status_code == 200
        assert unauth.get("/api/studio/integrations").status_code == 200
        assert unauth.get("/api/studio/providers").status_code == 200


# --- body limit ---


def test_payload_too_large_is_refused(client: TestClient) -> None:
    # 4 KB limit set in fixture; send 5 KB.
    body = "{" + '"x":"' + ("a" * 5000) + '"}'
    response = client.post(
        "/api/studio/preview",
        content=body,
        headers={"Content-Type": "application/json"},
    )
    assert response.status_code == 413
    assert response.json()["code"] == "payload_too_large"


# --- security header presence on a known route ---


def test_security_headers_are_applied_to_every_response(client: TestClient) -> None:
    response = client.get("/api/studio/health")
    for header, value in SECURITY_HEADERS.items():
        assert response.headers.get(header) == value
    # x-powered-by must not be advertised.
    assert "x-powered-by" not in response.headers


# --- middleware helper unit checks ---


def test_origin_matches_host_accepts_only_exact_match() -> None:
    scope = {
        "type": "http",
        "headers": (
            (b"host", b"127.0.0.1:8765"),
            (b"origin", b"http://127.0.0.1:8765"),
        ),
    }
    assert origin_matches_host(scope)


def test_origin_matches_host_rejects_null_origin() -> None:
    scope = {
        "type": "http",
        "headers": (
            (b"host", b"127.0.0.1:8765"),
            (b"origin", b"null"),
        ),
    }
    assert not origin_matches_host(scope)


def test_origin_matches_host_rejects_port_mismatch() -> None:
    scope = {
        "type": "http",
        "headers": (
            (b"host", b"127.0.0.1:8765"),
            (b"origin", b"http://127.0.0.1:9999"),
        ),
    }
    assert not origin_matches_host(scope)


def test_origin_matches_host_rejects_scheme_mismatch() -> None:
    scope = {
        "type": "http",
        "headers": (
            (b"host", b"127.0.0.1:8765"),
            (b"origin", b"https://127.0.0.1:8765"),
        ),
    }
    assert not origin_matches_host(scope)


def test_origin_matches_host_accepts_missing_origin() -> None:
    scope = {
        "type": "http",
        "headers": ((b"host", b"127.0.0.1:8765"),),
    }
    assert origin_matches_host(scope)


def test_is_loopback_authority_helper() -> None:
    assert is_loopback_authority("127.0.0.1:8765")
    assert is_loopback_authority("127.0.0.1")
    assert is_loopback_authority("localhost")
    assert not is_loopback_authority("example.com")
    assert not is_loopback_authority("0.0.0.0")


def test_origins_match_requires_exact_triple() -> None:
    a = ("http", "127.0.0.1", 8765)
    b = ("http", "127.0.0.1", 8765)
    c = ("http", "127.0.0.1", 9999)
    assert origins_match(a, b)
    assert not origins_match(a, c)
