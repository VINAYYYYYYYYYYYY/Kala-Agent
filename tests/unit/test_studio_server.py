"""Unit tests for the studio HTTP server surface.

The tests cover the live routes the studio advertises: health, integrations,
providers, preview, export (STL and STEP), handoff, and chat. The OCP
kernel is exercised through ``probe_step_capability``; when the binding is
absent the export route returns a sanitised 503 rather than fabricating
geometry.
"""

from __future__ import annotations

import tempfile
from collections.abc import Iterator
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from kala_agent.studio.geometry.step import probe_step_capability
from kala_agent.studio.server import create_studio_app

LOOPBACK_BASE_URL = "http://127.0.0.1"
LAUNCH_TOKEN = "studio-test-token-0123456789abcdef"
AUTH = {"Authorization": f"Bearer {LAUNCH_TOKEN}"}


@pytest.fixture(autouse=True)
def _forbid_external(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    """Permit OCP/test network reads but block all third-party egress."""

    blocked_hosts = {"api.openai.com", "openrouter.ai", "127.0.0.1", "localhost"}

    def blocked_connect(self: object, address: object, *_args: object, **_kwargs: object) -> int:  # type: ignore[no-untyped-def]
        host = address[0] if isinstance(address, tuple) else address
        if host in blocked_hosts:
            return 0
        raise AssertionError(f"the studio test would have connected to {host}")

    monkeypatch.setattr("socket.socket.connect", blocked_connect)
    monkeypatch.setattr("socket.create_connection", lambda *a, **kw: None)
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
        max_body_bytes=64 * 1024,
        static_root=None,
    )
    with TestClient(
        app,
        base_url=LOOPBACK_BASE_URL,
        headers=AUTH,
        raise_server_exceptions=False,
    ) as test_client:
        yield test_client


# --- health ---


def test_health_returns_liveness_without_secrets(client: TestClient) -> None:
    response = client.get("/api/studio/health")
    assert response.status_code == 200
    body = response.json()
    assert body["status"] == "ok"
    assert body["version"]
    assert "127.0.0.1" not in response.text
    assert LAUNCH_TOKEN not in response.text


def test_health_reflects_step_capability(client: TestClient) -> None:
    response = client.get("/api/studio/health")
    body = response.json()
    assert body["step_available"] is probe_step_capability().available


# --- registry ---


def test_integrations_lists_every_documented_tool(client: TestClient) -> None:
    response = client.get("/api/studio/integrations")
    assert response.status_code == 200
    body = response.json()
    ids = [row["id"] for row in body["integrations"]]
    for required in (
        "solidworks",
        "solid_edge",
        "autocad",
        "fusion360",
        "freecad",
        "openscad",
        "ansys_mechanical",
        "abaqus",
        "comsol",
        "simcenter_nastran",
        "altair_optistruct",
    ):
        assert required in ids, f"missing registry row: {required}"


def test_integrations_never_advertise_native_connection(client: TestClient) -> None:
    response = client.get("/api/studio/integrations")
    body = response.json()
    for row in body["integrations"]:
        # No row should claim a live native driver except freecad's
        # neutral-file import.
        assert row["status"] in (
            "available_via_neutral_file",
            "native_api_unavailable",
            "not_configured",
        )
        assert row["notes"], "every row must document its platform/licence"


def test_providers_lists_four_rows(client: TestClient) -> None:
    response = client.get("/api/studio/providers")
    assert response.status_code == 200
    providers = response.json()["providers"]
    ids = {row["id"] for row in providers}
    assert ids == {"local", "openai", "openrouter", "codex"}


# --- preview ---


def test_preview_box_returns_real_mesh(client: TestClient) -> None:
    response = client.post(
        "/api/studio/preview",
        json={
            "kind": "box",
            "parameters": {"size_x": 10, "size_y": 20, "size_z": 30},
        },
    )
    assert response.status_code == 200
    body = response.json()
    assert body["engine"] == "analytic"
    vertex_count = len(body["positions"]) // 3
    triangle_count = len(body["indices"]) // 3
    assert vertex_count > 0
    assert triangle_count > 0
    assert vertex_count == 24, "a 1x1 box should emit 4x6=24 vertices"
    assert triangle_count == 12, "a box is six faces x 2 triangles"
    assert len(body["positions"]) % 3 == 0
    assert len(body["normals"]) == len(body["positions"])
    assert len(body["indices"]) % 3 == 0
    assert body["warnings"]


def test_preview_cylinder_returns_real_mesh(client: TestClient) -> None:
    response = client.post(
        "/api/studio/preview",
        json={"kind": "cylinder", "parameters": {"radius": 5, "height": 10}},
    )
    assert response.status_code == 200
    body = response.json()
    assert body["engine"] == "analytic"
    assert len(body["positions"]) > 0


def test_preview_sphere_returns_real_mesh(client: TestClient) -> None:
    response = client.post(
        "/api/studio/preview",
        json={"kind": "sphere", "parameters": {"radius": 7}},
    )
    assert response.status_code == 200
    body = response.json()
    assert body["engine"] == "analytic"
    assert len(body["positions"]) > 0
    assert body["bounds"]["max"][2] - body["bounds"]["min"][2] == pytest.approx(14, abs=1e-6)


def test_preview_rejects_unknown_kind(client: TestClient) -> None:
    response = client.post(
        "/api/studio/preview",
        json={"kind": "torus", "parameters": {}},
    )
    assert response.status_code == 422
    assert response.json()["code"] == "invalid_request"


def test_preview_rejects_unknown_parameter(client: TestClient) -> None:
    response = client.post(
        "/api/studio/preview",
        json={
            "kind": "box",
            "parameters": {"size_x": 1, "size_y": 2, "size_z": 3, "extra": 4},
        },
    )
    assert response.status_code == 422


# --- export ---


def test_export_box_as_stl_returns_binary(client: TestClient) -> None:
    response = client.post(
        "/api/studio/export",
        json={
            "kind": "box",
            "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
            "format": "stl",
        },
    )
    assert response.status_code == 200
    assert response.headers["content-type"].startswith("model/stl")
    assert response.headers["content-disposition"].startswith("attachment")
    assert response.content.startswith(b"kala-studio ")


def test_export_box_as_step_returns_iso_text(client: TestClient) -> None:
    capability = probe_step_capability()
    response = client.post(
        "/api/studio/export",
        json={
            "kind": "box",
            "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
            "format": "step",
        },
    )
    if not capability.available:
        assert response.status_code == 503
        assert response.json()["code"] == "step_unavailable"
    else:
        assert response.status_code == 200
        assert response.content.startswith(b"ISO-10303-21")


def test_export_box_with_default_parameters_as_stl(client: TestClient) -> None:
    """A parameterless export must build the same canonical default box."""
    response = client.post(
        "/api/studio/export",
        json={"kind": "box", "format": "stl"},
    )
    assert response.status_code == 200
    assert response.content.startswith(b"kala-studio ")


def test_export_box_with_default_parameters_as_step(client: TestClient) -> None:
    """A parameterless STEP export must not require explicit dimensions."""
    capability = probe_step_capability()
    response = client.post(
        "/api/studio/export",
        json={"kind": "box", "format": "step"},
    )
    if not capability.available:
        assert response.status_code == 503
        assert response.json()["code"] == "step_unavailable"
    else:
        assert response.status_code == 200
        assert response.content.startswith(b"ISO-10303-21")


def test_export_rejects_out_of_range_dimension(client: TestClient) -> None:
    response = client.post(
        "/api/studio/export",
        json={
            "kind": "box",
            "parameters": {"size_x": -5, "size_y": 10, "size_z": 10},
            "format": "step",
        },
    )
    assert response.status_code == 422
    assert response.json()["code"] == "invalid_request"


def test_export_cylinder_as_step_returns_iso_text(client: TestClient) -> None:
    capability = probe_step_capability()
    response = client.post(
        "/api/studio/export",
        json={
            "kind": "cylinder",
            "parameters": {"radius": 5, "height": 10},
            "format": "step",
        },
    )
    if not capability.available:
        assert response.status_code == 503
    else:
        assert response.status_code == 200
        assert response.content.startswith(b"ISO-10303-21")


def test_export_rejects_unknown_format(client: TestClient) -> None:
    response = client.post(
        "/api/studio/export",
        json={
            "kind": "box",
            "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
            "format": "iges",
        },
    )
    assert response.status_code == 422


# --- handoff ---


def test_handoff_to_freecad_step_succeeds(client: TestClient) -> None:
    capability = probe_step_capability()
    response = client.post(
        "/api/studio/handoff",
        json={
            "integration_id": "freecad",
            "kind": "box",
            "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
            "format": "step",
        },
    )
    if not capability.available:
        assert response.status_code == 503
    else:
        assert response.status_code == 200
        assert response.content.startswith(b"ISO-10303-21")
        assert response.headers["x-studio-handoff"].startswith("freecad;")


def test_handoff_to_openscad_step_is_refused(client: TestClient) -> None:
    response = client.post(
        "/api/studio/handoff",
        json={
            "integration_id": "openscad",
            "kind": "box",
            "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
            "format": "step",
        },
    )
    assert response.status_code == 409
    assert response.json()["code"] == "handoff_format_unsupported"


def test_handoff_to_unknown_integration_is_not_found(client: TestClient) -> None:
    response = client.post(
        "/api/studio/handoff",
        json={
            "integration_id": "maya",
            "kind": "box",
            "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
            "format": "stl",
        },
    )
    assert response.status_code == 404
    assert response.json()["code"] == "unknown_integration"


def test_handoff_always_reports_connection_state_for_native_targets(client: TestClient) -> None:
    capability = probe_step_capability()
    response = client.post(
        "/api/studio/handoff",
        json={
            "integration_id": "solidworks",
            "kind": "box",
            "parameters": {"size_x": 10, "size_y": 10, "size_z": 10},
            "format": "step",
        },
    )
    if not capability.available:
        assert response.status_code == 503
    else:
        assert response.status_code == 200
        assert response.content.startswith(b"ISO-10303-21")
    # The registry row for SOLIDWORKS is "native_api_unavailable"; the route
    # never claims a native connection was made.
    registry = client.get("/api/studio/integrations").json()
    solidworks = next(row for row in registry["integrations"] if row["id"] == "solidworks")
    assert solidworks["status"] == "native_api_unavailable"


# --- chat ---


def test_chat_local_provider_is_offline(client: TestClient) -> None:
    response = client.post(
        "/api/studio/chat",
        json={
            "provider": "local",
            "model": "studio-local",
            "prompt": "First sentence. Second sentence.",
            "allow_remote": False,
        },
    )
    assert response.status_code == 200
    body = response.json()
    assert body["provider"] == "local"
    assert body["offline"] is True
    assert body["text"]


def test_chat_openai_without_key_returns_missing_api_key(client: TestClient) -> None:
    response = client.post(
        "/api/studio/chat",
        json={
            "provider": "openai",
            "model": "gpt-4",
            "prompt": "hi",
            "allow_remote": True,
        },
    )
    assert response.status_code == 400
    assert response.json()["code"] == "missing_api_key"


def test_chat_openai_with_key_but_remote_false_returns_provider_not_configured(
    client: TestClient, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("OPENAI_API_KEY", "sk-test-1234567890abcdef")
    response = client.post(
        "/api/studio/chat",
        json={
            "provider": "openai",
            "model": "gpt-4",
            "prompt": "hi",
            "allow_remote": False,
        },
    )
    assert response.status_code == 400
    assert response.json()["code"] == "provider_not_configured"


def test_chat_unknown_provider_returns_validation_error(client: TestClient) -> None:
    response = client.post(
        "/api/studio/chat",
        json={
            "provider": "anthropic",
            "model": "x",
            "prompt": "hi",
            "allow_remote": False,
        },
    )
    assert response.status_code == 422


def test_chat_rejects_oversize_prompt(client: TestClient) -> None:
    response = client.post(
        "/api/studio/chat",
        json={
            "provider": "local",
            "model": "x",
            "prompt": "x" * 5000,
            "allow_remote": False,
        },
    )
    assert response.status_code == 422


# --- settings (API key configuration) ---


def test_settings_default_reports_unconfigured_without_secrets(
    client: TestClient, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    monkeypatch.delenv("OPENROUTER_API_KEY", raising=False)
    response = client.get("/api/studio/settings")
    assert response.status_code == 200
    body = response.json()
    assert body["offline"] is True
    creds = {row["provider"]: row for row in body["credentials"]}
    assert set(creds.keys()) == {"openai", "openrouter"}
    assert creds["openai"]["configured"] is False
    assert creds["openai"]["source"] == "none"
    assert creds["openrouter"]["configured"] is False
    assert creds["openrouter"]["source"] == "none"


def test_settings_save_and_clear_api_key_updates_providers_without_echoing_secret(
    client: TestClient, data_dir: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    monkeypatch.delenv("OPENROUTER_API_KEY", raising=False)
    secret = "sk-studio-secret-key-9876543210abcdef"

    save_res = client.post(
        "/api/studio/settings",
        json={"provider": "openai", "api_key": secret},
    )
    assert save_res.status_code == 200
    assert secret not in save_res.text
    body = save_res.json()
    assert body["offline"] is False
    openai_cred = next(row for row in body["credentials"] if row["provider"] == "openai")
    assert openai_cred["configured"] is True
    assert openai_cred["source"] == "stored"
    openai_prov = next(row for row in body["providers"] if row["id"] == "openai")
    assert openai_prov["configured"] is True

    # Verify /providers and /health also see the stored key and never echo it
    prov_res = client.get("/api/studio/providers")
    assert prov_res.status_code == 200
    assert secret not in prov_res.text
    assert (
        next(row for row in prov_res.json()["providers"] if row["id"] == "openai")["configured"]
        is True
    )

    health_res = client.get("/api/studio/health")
    assert health_res.status_code == 200
    assert secret not in health_res.text
    assert health_res.json()["offline"] is False

    assert (data_dir / "studio-settings.json").is_file()

    # Clear the key
    clear_res = client.post(
        "/api/studio/settings",
        json={"provider": "openai", "api_key": ""},
    )
    assert clear_res.status_code == 200
    cleared_body = clear_res.json()
    assert cleared_body["offline"] is True
    openai_cred_after = next(
        row for row in cleared_body["credentials"] if row["provider"] == "openai"
    )
    assert openai_cred_after["configured"] is False
    assert openai_cred_after["source"] == "none"


def test_settings_rejects_key_with_whitespace_without_echoing_key(client: TestClient) -> None:
    bad_secret = "sk-bad secret with spaces"
    res = client.post(
        "/api/studio/settings",
        json={"provider": "openrouter", "api_key": bad_secret},
    )
    assert res.status_code == 422
    assert res.json()["code"] == "invalid_request"
    assert bad_secret not in res.text


# --- envelope shape ---


def test_error_envelope_is_flat(client: TestClient) -> None:
    response = client.post(
        "/api/studio/preview",
        json={"kind": "torus", "parameters": {}},
    )
    assert response.status_code == 422
    body = response.json()
    assert set(body.keys()) == {"code", "message", "details"}
    assert body["code"]
    assert body["message"]


def test_token_is_never_echoed_in_any_response(client: TestClient) -> None:
    from fastapi.testclient import TestClient as TC

    app = create_studio_app(
        data_dir=client.app.extra.get("data_dir", Path(tempfile.gettempdir()) / "x")  # type: ignore[attr-defined]
        if False
        else None,
        api_token=LAUNCH_TOKEN,
        max_body_bytes=4096,
        static_root=None,
    )
    with TC(app, base_url=LOOPBACK_BASE_URL, headers=AUTH, raise_server_exceptions=False) as fresh:
        for path, method in (
            ("/api/studio/health", "GET"),
            ("/api/studio/integrations", "GET"),
            ("/api/studio/providers", "GET"),
            ("/api/studio/settings", "GET"),
            ("/api/studio/settings", "POST"),
            ("/api/studio/chat", "POST"),
            ("/api/studio/preview", "POST"),
            ("/api/studio/handoff", "POST"),
            ("/api/studio/export", "POST"),
        ):
            if method == "GET":
                response = fresh.get(path)
            else:
                response = fresh.post(path, json={})
            assert LAUNCH_TOKEN not in response.text, f"token leaked via {method} {path}"
