"""View-only STEP import tests for the geometry worker (ADR-0011).

The fixture is the real thing: the opt-in OpenCascade builder compiles the
ADR-0009 L-bracket, the existing ``/export`` route writes its STEP file, and
``/import`` reads those exact bytes back. No fixture STEP file is checked in,
so the import path is exercised against what the OpenCascade exporter actually
produces.
"""

from __future__ import annotations

import hashlib
from collections.abc import Iterator
from pathlib import Path
from typing import Any, cast

import pytest
from fastapi.testclient import TestClient
from httpx import Response

from kala_agent.studio.design.service import design_available
from kala_agent.studio.internal_api import INTERNAL_PREFIX, create_worker_app
from kala_agent.studio.middleware import MAX_REQUEST_BODY_BYTES

WORKER_PORT = 51_235
LAUNCH_TOKEN = "worker-import-token-0123456789abcdef"
AUTH = {"Authorization": f"Bearer {LAUNCH_TOKEN}"}
EXPECTED_MIN = (-30.0, 0.0, 0.0)
EXPECTED_MAX = (30.0, 45.0, 40.0)
BOUNDS_TOLERANCE_MM = 1e-6

pytestmark = pytest.mark.skipif(
    not design_available(), reason="the OCP geometry kernel is not installed"
)


def _lbracket_body() -> dict[str, object]:
    return {
        "template": {
            "template": "l_bracket",
            "width": 60,
            "depth": 45,
            "height": 40,
            "wall": 8,
            "bore_d": 10,
            "hole_d": 5.5,
            "hole_count": 4,
        },
        "material": "aluminium",
        "provider": "opencascade",
    }


@pytest.fixture
def data_dir(tmp_path: Path) -> Path:
    return tmp_path


@pytest.fixture
def client(data_dir: Path) -> Iterator[TestClient]:
    app = create_worker_app(
        api_token=LAUNCH_TOKEN,
        port=WORKER_PORT,
        data_dir=data_dir,
    )
    with TestClient(
        app,
        base_url=f"http://127.0.0.1:{WORKER_PORT}",
        raise_server_exceptions=False,
    ) as test_client:
        yield test_client


def _export_lbracket_step(client: TestClient) -> bytes:
    design = client.post(
        f"{INTERNAL_PREFIX}/design",
        json=_lbracket_body(),
        headers=AUTH,
    )
    assert design.status_code == 200, design.text
    design_id = design.json()["design_id"]
    exported = client.post(
        f"{INTERNAL_PREFIX}/export",
        json={"design_id": design_id, "format": "step"},
        headers=AUTH,
    )
    assert exported.status_code == 200, exported.text
    assert exported.content.startswith(b"ISO-10303-21")
    return cast("bytes", exported.content)


def _import(client: TestClient, data: bytes) -> Response:
    response = client.post(
        f"{INTERNAL_PREFIX}/import",
        content=data,
        headers={**AUTH, "content-type": "application/step"},
    )
    return cast("Response", response)


def test_import_returns_a_view_of_the_exported_lbracket(client: TestClient) -> None:
    step = _export_lbracket_step(client)
    response = _import(client, step)
    assert response.status_code == 200, response.text
    body = cast("dict[str, Any]", response.json())

    assert body["step_sha256"] == hashlib.sha256(step).hexdigest()
    assert body["solids"] == 1
    assert body["warnings"] == []

    view = cast("dict[str, Any]", body["view"])
    positions = cast("list[float]", view["positions"])
    normals = cast("list[float]", view["normals"])
    indices = cast("list[int]", view["indices"])
    assert positions
    assert len(positions) % 3 == 0
    assert len(normals) == len(positions)
    assert len(indices) % 3 == 0
    vertex_count = len(positions) // 3
    assert all(0 <= index < vertex_count for index in indices)

    bounds = cast("dict[str, list[float]]", view["bounds"])
    for measured, expected in zip(bounds["min"], EXPECTED_MIN, strict=True):
        assert measured == pytest.approx(expected, abs=BOUNDS_TOLERANCE_MM)
    for measured, expected in zip(bounds["max"], EXPECTED_MAX, strict=True):
        assert measured == pytest.approx(expected, abs=BOUNDS_TOLERANCE_MM)


def test_import_stores_the_original_step_and_journals_the_view(
    client: TestClient,
) -> None:
    step = _export_lbracket_step(client)
    digest = hashlib.sha256(step).hexdigest()

    response = _import(client, step)
    assert response.status_code == 200, response.text

    journal = client.app.state.design_service.journal  # type: ignore[attr-defined]
    assert journal.has_artifact(digest)
    assert journal.get_artifact(digest) == step

    events = client.get(
        f"{INTERNAL_PREFIX}/journal",
        params={"after": 0, "limit": 100},
        headers=AUTH,
    )
    assert events.status_code == 200
    recorded = cast("list[dict[str, Any]]", events.json()["events"])
    imported = [
        event
        for event in recorded
        if event["event_type"] == "view.imported" and event["payload"].get("provider") is None
    ]
    assert len(imported) == 1
    payload = cast("dict[str, Any]", imported[0]["payload"])
    assert payload["step_sha256"] == digest
    assert payload["viewer"] == "opencascade"
    assert payload["role"] == "view-only"
    assert payload["solids"] == 1


def test_import_requires_the_worker_bearer(client: TestClient) -> None:
    response = client.post(
        f"{INTERNAL_PREFIX}/import",
        content=b"ISO-10303-21;",
        headers={"content-type": "application/step"},
    )
    assert response.status_code == 401
    assert response.json()["code"] == "unauthorized"


def test_import_rejects_a_malformed_step_file(client: TestClient) -> None:
    _export_lbracket_step(client)
    before = {
        path
        for path in client.app.state.design_service.journal.artifact_root.rglob("*")  # type: ignore[attr-defined]
        if path.is_file()
    }
    for broken in (
        b"this is not a STEP file",
        b"ISO-10303-21; but the body is not an exchange structure",
        b"",
    ):
        response = _import(client, broken)
        assert response.status_code == 422, broken[:32]
        body = cast("dict[str, Any]", response.json())
        assert body["code"] == "invalid_step"
        assert body["details"]

    after = {
        path
        for path in client.app.state.design_service.journal.artifact_root.rglob("*")  # type: ignore[attr-defined]
        if path.is_file()
    }
    assert after == before, "a malformed STEP file must not be stored"


def test_import_route_is_not_capped_at_the_generic_studio_limit(
    client: TestClient,
) -> None:
    """The import route has its own large ceiling, not the 256 KiB body limit."""
    response = _import(client, b"x" * (MAX_REQUEST_BODY_BYTES + 1))
    assert response.status_code == 422
    assert response.json()["code"] == "invalid_step"


def test_import_refuses_a_body_over_the_configured_step_limit(
    data_dir: Path,
) -> None:
    app = create_worker_app(
        api_token=LAUNCH_TOKEN,
        port=WORKER_PORT,
        data_dir=data_dir,
        step_import_max_bytes=MAX_REQUEST_BODY_BYTES,
    )
    with TestClient(
        app,
        base_url=f"http://127.0.0.1:{WORKER_PORT}",
        raise_server_exceptions=False,
    ) as scoped:
        response = scoped.post(
            f"{INTERNAL_PREFIX}/import",
            content=b"x" * (MAX_REQUEST_BODY_BYTES + 1),
            headers={**AUTH, "content-type": "application/step"},
        )
        assert response.status_code == 413
        assert response.json()["code"] == "payload_too_large"
