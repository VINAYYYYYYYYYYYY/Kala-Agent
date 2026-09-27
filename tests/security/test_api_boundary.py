"""Security tests for the request boundary of the local Kala HTTP adapter.

These tests attack the boundary only: a rebound Host header, a missing or wrong
bearer token, a body that exceeds the limit on a method that carries none, a
wildcard address on the loopback allowlist, and a cross-origin request. Every
test runs offline against a temporary data directory, and no test reaches a
route by disabling a control.
"""

from __future__ import annotations

import importlib
import io
import json
import logging
import stat
from collections.abc import Iterator
from pathlib import Path
from types import ModuleType

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient
from httpx import Response

from kala_agent.api import create_app
from kala_agent.api.boundary import (
    API_TOKEN_FILE_MODE,
    API_TOKEN_FILE_NAME,
    MUTATING_METHODS,
    is_loopback_host,
    is_permitted_authority,
    read_api_token,
    validate_api_token,
)
from kala_agent.api.middleware import SECURITY_HEADERS
from kala_agent.application import KalaApplication
from kala_agent.cli import main
from kala_agent.config import Settings

CLI_MODULE: ModuleType = importlib.import_module("kala_agent.cli.main")

API = "/api/v1"
LOOPBACK_BASE_URL = "http://127.0.0.1"
REVISION = {"design_id": "boundary", "domain": "mechanical"}
INJECTED_TOKEN = "security-test-launch-token-0123456789"
AUTH = {"Authorization": f"Bearer {INJECTED_TOKEN}"}
SECRET_SHAPED = "sk-must-never-be-echoed-9876543210abcdef"


@pytest.fixture(autouse=True)
def _forbid_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    def blocked(*args: object, **kwargs: object) -> None:
        raise AssertionError("the request boundary must not open network connections")

    monkeypatch.setattr("socket.socket.connect", blocked)
    monkeypatch.setattr("socket.socket.connect_ex", blocked)
    monkeypatch.setattr("socket.create_connection", blocked)
    yield


def _client(app: FastAPI, *, token: str | None = INJECTED_TOKEN) -> TestClient:
    """Return a test client that names loopback and presents the launch token."""
    return TestClient(
        app,
        base_url=LOOPBACK_BASE_URL,
        headers={} if token is None else {"Authorization": f"Bearer {token}"},
        raise_server_exceptions=False,
    )


@pytest.fixture
def application(tmp_path: Path) -> Iterator[KalaApplication]:
    resolved = KalaApplication(Settings(data_dir=tmp_path / "data"))
    try:
        yield resolved
    finally:
        resolved.close()


def _assert_sanitized_refusal(response: Response, code: str) -> None:
    assert response.status_code in (401, 403, 413, 421)
    body = json.loads(response.text)
    assert set(body) == {"error", "details"}
    assert body["error"]["code"] == code
    assert body["error"]["message"]
    assert "Traceback" not in response.text
    for header, value in SECURITY_HEADERS.items():
        assert response.headers[header] == value
    assert "x-powered-by" not in response.headers


def test_a_rebound_host_header_never_reaches_a_route(application: KalaApplication) -> None:
    app = create_app(application, api_token=INJECTED_TOKEN)
    with _client(app) as client:
        for host in (
            "evil.example",
            "attacker.example:8000",
            "127.0.0.1.evil.example",
            "localhost.evil.example",
            "127.0.0.1@evil.example",
            "0.0.0.0",
            "0.0.0.0:8000",
            "::",
            "[::]:8000",
            "10.0.0.5:8000",
            "169.254.169.254",
            "127.0.0.1.",
            "127.0.0.1:0",
            "127.0.0.1:99999",
            "127.0.0.1:port",
            "[::1",
            "",
        ):
            read = client.get("/health", headers={"Host": host})
            _assert_sanitized_refusal(read, "invalid_host")
            assert read.status_code == 421, host
            write = client.post(f"{API}/revisions", json=REVISION, headers={"Host": host})
            assert write.status_code == 421, host
            assert write.json()["error"]["code"] == "invalid_host"
        for host in ("127.0.0.1", "127.0.0.1:8000", "localhost", "localhost:8000", "[::1]:8000"):
            allowed = client.get("/health", headers={"Host": host})
            assert allowed.status_code == 200, host
            assert allowed.json()["status"] == "ok"
    assert application.list_events(limit=200).count == 0


def test_duplicate_host_headers_are_refused(application: KalaApplication) -> None:
    app = create_app(application, api_token=INJECTED_TOKEN)
    with _client(app) as client:
        duplicated = client.get(
            "/health",
            headers=[("host", "127.0.0.1"), ("host", "evil.example")],
        )
    _assert_sanitized_refusal(duplicated, "invalid_host")
    assert duplicated.status_code == 421


def test_a_request_without_a_host_header_is_refused(application: KalaApplication) -> None:
    app = create_app(application, api_token=INJECTED_TOKEN)
    with _client(app) as client:
        response = client.get("/health", headers={"Host": ""})
    _assert_sanitized_refusal(response, "invalid_host")
    assert response.status_code == 421


def test_mutating_methods_require_the_launch_token(application: KalaApplication) -> None:
    app = create_app(application, api_token=INJECTED_TOKEN)
    with _client(app, token=None) as client:
        assert client.get(f"{API}/domains").status_code == 200
        assert client.get("/health").status_code == 200
        for authorization in (
            None,
            "",
            "Bearer",
            "Bearer ",
            f"Basic {INJECTED_TOKEN}",
            f"Bearer {INJECTED_TOKEN}x",
            f"bearer {INJECTED_TOKEN[:-1]}",
            f"Bearer {SECRET_SHAPED}",
            INJECTED_TOKEN,
            f"Bearer {INJECTED_TOKEN}, Bearer {INJECTED_TOKEN}",
        ):
            headers = {} if authorization is None else {"Authorization": authorization}
            response = client.post(f"{API}/revisions", json=REVISION, headers=headers)
            _assert_sanitized_refusal(response, "unauthorized")
            assert response.status_code == 401
            assert response.headers["www-authenticate"] == 'Bearer realm="kala-agent"'
            assert INJECTED_TOKEN not in response.text
            assert SECRET_SHAPED not in response.text
        wrong = client.post(
            f"{API}/revisions",
            json=REVISION,
            headers={"Authorization": f"Bearer {'w' * len(INJECTED_TOKEN)}"},
        )
        assert wrong.status_code == 401
        for prefix in ("Bearer ", "bearer ", "BEARER ", "Bearer  "):
            spaced = client.post(
                f"{API}/revisions",
                json={"design_id": "authorized", "domain": "mechanical"},
                headers={"Authorization": f"{prefix}{INJECTED_TOKEN}"},
            )
            assert spaced.status_code == 201, prefix
        authorized = client.post(
            f"{API}/revisions",
            json={"design_id": "authorized", "domain": "mechanical"},
            headers=AUTH,
        )
        assert authorized.status_code == 201
    assert application.list_design_revisions("boundary") == ()
    assert len(application.list_design_revisions("authorized")) == 5


def test_a_valid_token_authorizes_every_mutating_endpoint(application: KalaApplication) -> None:
    with _client(create_app(application, api_token=INJECTED_TOKEN)) as client:
        created = client.post(f"{API}/revisions", json=REVISION)
        assert created.status_code == 201
        started = client.post(
            f"{API}/workflows/mechanical/reference",
            json={"design_id": "boundary-run"},
        )
        assert started.status_code == 201
        workflow_id = started.json()["run"]["workflow_id"]
        approved = client.post(f"{API}/workflows/{workflow_id}/checkpoints/approve", json={})
        assert approved.status_code == 200
        evaluated = client.post(f"{API}/domains/mechanical/evaluations", json={})
        assert evaluated.status_code == 200
    assert len(application.list_design_revisions("boundary")) == 1


def test_a_route_added_after_launch_is_covered_by_the_boundary(
    application: KalaApplication,
) -> None:
    def mutate() -> dict[str, bool]:
        return {"mutated": True}

    app = create_app(application, api_token=INJECTED_TOKEN)
    app.router.add_api_route("/late", mutate, methods=["POST"], include_in_schema=False)
    with _client(app) as client:
        assert client.post("/late", json={}).status_code == 200
        assert client.post("/late", json={}, headers={"Authorization": ""}).status_code == 401


def test_every_mutating_method_is_judged(application: KalaApplication) -> None:
    def any_method() -> dict[str, bool]:
        return {"ok": True}

    app = create_app(application, api_token=INJECTED_TOKEN)
    app.router.add_api_route(
        "/verb",
        any_method,
        methods=["DELETE", "GET", "HEAD", "OPTIONS", "PATCH", "POST", "PUT"],
        include_in_schema=False,
    )
    with _client(app, token=None) as client:
        for method in ("DELETE", "GET", "HEAD", "OPTIONS", "PATCH", "POST", "PUT"):
            response = client.request(method, "/verb")
            expected = 401 if method in MUTATING_METHODS else 200
            assert response.status_code == expected, method
    assert MUTATING_METHODS == {"DELETE", "PATCH", "POST", "PUT"}


def test_the_body_size_limit_answers_every_method(application: KalaApplication) -> None:
    app = create_app(application, api_token=INJECTED_TOKEN, max_body_bytes=1024)
    with _client(app) as client:
        oversized = b"x" * 4096
        for method, path in (
            ("GET", "/health"),
            ("HEAD", "/health"),
            ("GET", f"{API}/events"),
            ("DELETE", f"{API}/domains"),
            ("OPTIONS", f"{API}/domains"),
            ("POST", f"{API}/revisions"),
            ("PUT", f"{API}/revisions"),
            ("PATCH", f"{API}/revisions"),
        ):
            declared = client.request(method, path, content=oversized)
            if method == "HEAD":
                assert declared.status_code == 413, method
                assert declared.headers["x-content-type-options"] == "nosniff"
                continue
            _assert_sanitized_refusal(declared, "payload_too_large")
            assert declared.status_code == 413, method

        def chunked() -> Iterator[bytes]:
            yield b"y" * 700
            yield b"y" * 700

        for method, path in (
            ("GET", "/health"),
            ("DELETE", f"{API}/domains"),
            ("OPTIONS", f"{API}/domains"),
            ("POST", f"{API}/revisions"),
        ):
            streamed = client.request(method, path, content=chunked())
            _assert_sanitized_refusal(streamed, "payload_too_large")
            assert streamed.status_code == 413, method
        within = client.request("GET", "/health", content=iter([b"y" * 64]))
        assert within.status_code == 200
    assert application.list_events(limit=200).count == 0


def test_the_wildcard_address_is_never_loopback() -> None:
    for host in ("0.0.0.0", "::", "[::]", "0:0:0:0:0:0:0:0", "0.0.0.0:8000", "[::]:8000", "*"):
        assert not is_loopback_host(host), host
        assert not is_permitted_authority(host), host
    for host in ("localhost", "127.0.0.1", "127.0.0.2", "[::1]", "[::1]:8000", "localhost:8000"):
        assert is_permitted_authority(host), host
    for host in ("localhost", "127.0.0.1", "127.0.0.2", "::1"):
        assert is_loopback_host(host), host
    for host in ("evil.example", "", "127.0.0.1.", "http://127.0.0.1", "127.0.0.1/path"):
        assert not is_loopback_host(host), host
        assert not is_permitted_authority(host), host
    assert not is_permitted_authority("::1")
    assert not is_permitted_authority(17)
    for authority in (
        "127.0.0.1 ",
        " 127.0.0.1",
        "[::1]x",
        "[::1]x:8000",
        ":8000",
        "1" * 300,
        "[::1",
    ):
        assert not is_permitted_authority(authority), authority
    assert not is_loopback_host(17)
    assert not is_loopback_host("1" * 300)


def test_cross_origin_requests_are_refused_with_no_grant(application: KalaApplication) -> None:
    with _client(create_app(application, api_token=INJECTED_TOKEN)) as client:
        for origin in (
            "http://evil.example",
            "http://localhost:5173",
            "http://127.0.0.1:5173",
            "http://0.0.0.0:5173",
            "https://127.0.0.1:5173",
            "null",
        ):
            read = client.get(f"{API}/domains", headers={"Origin": origin})
            _assert_sanitized_refusal(read, "cross_origin_denied")
            assert read.status_code == 403, origin
            assert not any(name.startswith("access-control-") for name in read.headers), origin
            write = client.post(f"{API}/revisions", json=REVISION, headers={"Origin": origin})
            assert write.status_code == 403, origin
        preflight = client.options(
            f"{API}/domains",
            headers={
                "Origin": "http://127.0.0.1:5173",
                "Access-Control-Request-Method": "GET",
                "Access-Control-Request-Headers": "authorization, content-type",
            },
        )
        assert preflight.status_code == 403
        assert not any(name.startswith("access-control-") for name in preflight.headers)
        assert client.get(f"{API}/domains").status_code == 200
        assert client.get("/health").status_code == 200


def test_the_launch_token_is_stored_never_returned_and_rotated(
    application: KalaApplication,
) -> None:
    first = create_app(application)
    stored = read_api_token(application.settings.data_dir)
    assert stored is not None
    path = application.settings.data_dir / API_TOKEN_FILE_NAME
    assert path.is_file()
    assert path.read_text(encoding="utf-8").strip() == stored
    assert stat.S_IMODE(path.stat().st_mode) == API_TOKEN_FILE_MODE
    assert validate_api_token(stored) == stored
    with _client(first, token=stored) as client:
        assert client.post(f"{API}/revisions", json=REVISION).status_code == 201
        for url in ("/health", f"{API}/domains", "/openapi.json", "/docs"):
            response = client.get(url)
            assert stored not in response.text, url
    second = create_app(application)
    rotated = read_api_token(application.settings.data_dir)
    assert rotated is not None
    assert rotated != stored
    with _client(second, token=stored) as client:
        assert client.post(f"{API}/revisions", json=REVISION).status_code == 401
    with _client(second, token=rotated) as client:
        assert client.post(f"{API}/revisions", json=REVISION).status_code == 201


def test_a_missing_or_corrupt_token_store_reads_as_absent(tmp_path: Path) -> None:
    data_dir = tmp_path / "empty"
    data_dir.mkdir()
    assert read_api_token(data_dir) is None
    (data_dir / API_TOKEN_FILE_NAME).write_text("short\n", encoding="utf-8")
    assert read_api_token(data_dir) is None
    (data_dir / API_TOKEN_FILE_NAME).write_text("x" * 40 + "\n", encoding="utf-8")
    assert read_api_token(data_dir) == "x" * 40
    (data_dir / API_TOKEN_FILE_NAME).write_bytes(b"\xff\xfe not utf 8")
    assert read_api_token(data_dir) is None
    for unusable in (
        "",
        "   ",
        "a" * 15,
        "a" * 129,
        f"{'a' * 15} ",
        f"{'a' * 15}\t",
        f"{'a' * 15}\n",
        f"{'a' * 15}/",
        f"{'a' * 15}é",
        17,
        None,
    ):
        with pytest.raises(ValueError, match="api token"):
            validate_api_token(unusable)
    assert validate_api_token("a" * 16) == "a" * 16
    assert validate_api_token("a" * 128) == "a" * 128
    assert validate_api_token("a-b_c.d~e" + "f" * 8) == "a-b_c.d~e" + "f" * 8


def test_the_token_and_the_store_are_never_logged(
    application: KalaApplication,
    caplog: pytest.LogCaptureFixture,
) -> None:
    with caplog.at_level(logging.DEBUG, logger="kala_agent"):
        app = create_app(application)
        stored = read_api_token(application.settings.data_dir)
        assert stored is not None
        with _client(app, token=stored) as client:
            assert client.get("/health").status_code == 200
            assert client.post(f"{API}/revisions", json=REVISION).status_code == 201
            refused = client.post(
                f"{API}/revisions",
                json=REVISION,
                headers={"Authorization": f"Bearer {SECRET_SHAPED}"},
            )
            assert refused.status_code == 401
            assert client.get("/health", headers={"Host": "evil.example"}).status_code == 421
    logged = "\n".join(record.getMessage() for record in caplog.records)
    assert stored not in logged
    assert SECRET_SHAPED not in logged
    assert INJECTED_TOKEN not in logged


def test_a_token_that_cannot_be_stored_still_guards_the_process(tmp_path: Path) -> None:
    data_dir = tmp_path / "read-only"
    data_dir.mkdir()
    (data_dir / API_TOKEN_FILE_NAME).mkdir()
    settings = Settings(data_dir=data_dir)
    unresolved = KalaApplication(settings)
    try:
        app = create_app(unresolved)
        assert read_api_token(data_dir) is None
        with _client(app, token=None) as client:
            assert client.post(f"{API}/revisions", json=REVISION).status_code == 401
            assert client.get(f"{API}/domains").status_code == 200
    finally:
        unresolved.close()


def test_the_served_launch_keeps_the_host_and_token_boundary(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    data_dir = tmp_path / "data"
    recorded: list[dict[str, object]] = []

    class _Server:
        def run(self, app: FastAPI, **kwargs: object) -> None:
            recorded.append({"app": app, **kwargs})

    monkeypatch.setattr(CLI_MODULE, "load_uvicorn", _Server)
    application = KalaApplication(Settings(data_dir=data_dir))
    try:
        exit_code = main(
            ["serve", "--port", "8123"],
            application=application,
            environ={"KALA_AGENT_DATA_DIR": str(data_dir)},
            stdout=io.StringIO(),
            stderr=io.StringIO(),
        )
        assert exit_code == 0
        assert application.closed is False
        assert len(recorded) == 1
        assert recorded[0]["host"] == "127.0.0.1"
        assert recorded[0]["port"] == 8123
        app = recorded[0]["app"]
        assert isinstance(app, FastAPI)
        names = {str(getattr(middleware.cls, "__name__", "")) for middleware in app.user_middleware}
        assert {
            "HostGuardMiddleware",
            "BearerTokenMiddleware",
            "CrossOriginGuardMiddleware",
        } <= names

        token = read_api_token(data_dir)
        assert token is not None
        with TestClient(
            app,
            base_url=LOOPBACK_BASE_URL,
            headers={"Authorization": f"Bearer {token}"},
            raise_server_exceptions=False,
        ) as client:
            assert client.get("/health", headers={"Host": "0.0.0.0"}).status_code == 421
            assert client.get("/health").json()["egress_configured"] is False
            assert client.post(f"{API}/revisions", json=REVISION).status_code == 201
        with TestClient(
            app, base_url=LOOPBACK_BASE_URL, raise_server_exceptions=False
        ) as anonymous:
            assert anonymous.post(f"{API}/revisions", json=REVISION).status_code == 401
            assert anonymous.get(f"{API}/domains").status_code == 200
    finally:
        application.close()
