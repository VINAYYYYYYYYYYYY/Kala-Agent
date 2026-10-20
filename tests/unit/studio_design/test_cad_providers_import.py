"""The STEP import's own large limit and its one coarser retry (ADR-0011).

The import route must accept a real multi-megabyte provider export while every
other Studio route keeps the generic 256 KiB body limit. The fixture STEP is
generated with OpenCascade (200 disjoint boxes, about 3.4 MB) so the test is
the real reader, not a padded header.
"""

from __future__ import annotations

import hashlib
import os
import tempfile
from collections.abc import Iterator
from pathlib import Path
from typing import Any, cast

import pytest
from fastapi.testclient import TestClient
from OCP.BRep import BRep_Builder  # type: ignore[import-untyped]
from OCP.BRepPrimAPI import BRepPrimAPI_MakeBox  # type: ignore[import-untyped]
from OCP.gp import gp_Pnt  # type: ignore[import-untyped]
from OCP.IFSelect import IFSelect_RetDone  # type: ignore[import-untyped]
from OCP.STEPControl import (  # type: ignore[import-untyped]
    STEPControl_AsIs,
    STEPControl_Writer,
)
from OCP.TopoDS import TopoDS_Compound, TopoDS_Shape  # type: ignore[import-untyped]
from starlette.types import Message, Receive, Scope, Send

from kala_agent.studio.design.importer import (
    COARSENED_WARNING,
    COARSER_MESH_DEFLECTION_MM,
    DEFAULT_MAX_STEP_BYTES,
    STEP_IMPORT_MAX_BYTES_ENV,
    StepMeshError,
    import_step_bytes,
    resolve_step_import_max_bytes,
    validate_step_import_max_bytes,
)
from kala_agent.studio.design.mesh import (
    DEFAULT_ANGULAR_DEFLECTION_RAD,
    DEFAULT_MESH_DEFLECTION_MM,
    MeshData,
    MeshError,
)
from kala_agent.studio.design.mesh import (
    tessellate as real_tessellate,
)
from kala_agent.studio.geometry.limits import MAX_MESH_TRIANGLES, MAX_MESH_VERTICES
from kala_agent.studio.internal_api import (
    INTERNAL_PREFIX,
    WorkerBodyLimitMiddleware,
    create_worker_app,
)
from kala_agent.studio.middleware import MAX_REQUEST_BODY_BYTES

TOKEN = "worker-import-token-0123456789abcdef"
AUTH = {"Authorization": f"Bearer {TOKEN}"}
WORKER_PORT = 51_236
STEP_CONTENT_HEADERS = {**AUTH, "content-type": "application/step"}
LARGE_STEP_IMPORT_LIMIT = 8 * 1024 * 1024


@pytest.fixture(scope="module")
def large_step() -> bytes:
    """Return a valid OpenCascade STEP of about 3.4 MB (200 boxes)."""
    compound = TopoDS_Compound()
    builder = BRep_Builder()
    builder.MakeCompound(compound)
    for index in range(200):
        box = BRepPrimAPI_MakeBox(
            gp_Pnt(
                (index % 20) * 12.0,
                ((index // 20) % 20) * 12.0,
                (index // 400) * 12.0,
            ),
            10.0,
            10.0,
            10.0,
        ).Shape()
        builder.Add(compound, box)
    descriptor, path = tempfile.mkstemp(suffix=".step")
    os.close(descriptor)
    try:
        writer = STEPControl_Writer()
        assert writer.Transfer(compound, STEPControl_AsIs) == IFSelect_RetDone
        assert writer.Write(path) == IFSelect_RetDone
        data = Path(path).read_bytes()
    finally:
        try:
            Path(path).unlink()
        except OSError:
            pass
    assert data.startswith(b"ISO-10303-21")
    assert len(data) > 3 * 1024 * 1024
    return data


@pytest.fixture
def data_dir(tmp_path: Path) -> Path:
    return tmp_path


def _app(data_dir: Path, *, step_limit: int | None) -> TestClient:
    app = create_worker_app(
        api_token=TOKEN,
        port=WORKER_PORT,
        data_dir=data_dir,
        step_import_max_bytes=step_limit,
    )
    return TestClient(
        app,
        base_url=f"http://127.0.0.1:{WORKER_PORT}",
        raise_server_exceptions=False,
    )


@pytest.fixture
def client(data_dir: Path) -> Iterator[TestClient]:
    with _app(data_dir, step_limit=LARGE_STEP_IMPORT_LIMIT) as test_client:
        yield test_client


def test_import_accepts_a_multi_megabyte_provider_export(
    client: TestClient, large_step: bytes
) -> None:
    response = client.post(
        f"{INTERNAL_PREFIX}/import",
        content=large_step,
        headers=STEP_CONTENT_HEADERS,
    )
    assert response.status_code == 200, response.text
    body = cast("dict[str, Any]", response.json())
    assert body["step_sha256"] == hashlib.sha256(large_step).hexdigest()
    assert body["solids"] == 200
    view = cast("dict[str, Any]", body["view"])
    bounds = cast("dict[str, list[float]]", view["bounds"])
    assert bounds["min"] == pytest.approx([0.0, 0.0, 0.0], abs=0.001)
    assert bounds["max"][0] == pytest.approx(238.0, abs=0.001)
    assert bounds["max"][1] == pytest.approx(118.0, abs=0.001)
    assert bounds["max"][2] == pytest.approx(10.0, abs=0.001)


def test_import_refuses_a_body_over_its_own_limit(data_dir: Path, large_step: bytes) -> None:
    with _app(data_dir, step_limit=1024 * 1024) as client:
        response = client.post(
            f"{INTERNAL_PREFIX}/import",
            content=large_step,
            headers=STEP_CONTENT_HEADERS,
        )
    assert response.status_code == 413
    assert response.json()["code"] == "payload_too_large"
    assert "1048576" in response.json()["details"][0]


def test_other_worker_routes_keep_the_generic_body_limit(client: TestClient) -> None:
    response = client.post(
        f"{INTERNAL_PREFIX}/intent/validate",
        content=b"x" * (MAX_REQUEST_BODY_BYTES + 1),
        headers={**AUTH, "content-type": "application/json"},
    )
    assert response.status_code == 413
    assert response.json()["code"] == "payload_too_large"


def test_the_step_limit_defaults_to_64_mib_and_reads_the_environment() -> None:
    assert DEFAULT_MAX_STEP_BYTES == 64 * 1024 * 1024
    assert resolve_step_import_max_bytes({}) == DEFAULT_MAX_STEP_BYTES
    assert resolve_step_import_max_bytes({STEP_IMPORT_MAX_BYTES_ENV: "1048576"}) == 1048576
    with pytest.raises(ValueError, match="integer"):
        resolve_step_import_max_bytes({STEP_IMPORT_MAX_BYTES_ENV: "lots"})
    with pytest.raises(ValueError, match="between"):
        resolve_step_import_max_bytes({STEP_IMPORT_MAX_BYTES_ENV: "0"})
    with pytest.raises(ValueError):
        validate_step_import_max_bytes(True)


@pytest.mark.asyncio
async def test_the_import_body_is_read_in_bounded_chunks() -> None:
    """The middleware stops reading as soon as the import ceiling is crossed."""
    calls: list[int] = []
    chunks: list[dict[str, object]] = [
        {"type": "http.request", "body": b"x" * 600, "more_body": True},
        {"type": "http.request", "body": b"x" * 600, "more_body": True},
        {"type": "http.request", "body": b"x" * 600, "more_body": False},
    ]
    sent: list[Message] = []

    async def app(scope: Scope, receive: Receive, send: Send) -> None:  # pragma: no cover
        del scope, receive, send
        raise AssertionError("the handler must not run for an oversized import")

    middleware = WorkerBodyLimitMiddleware(
        app,
        default_limit=MAX_REQUEST_BODY_BYTES,
        import_limit=1024,
        import_path=f"{INTERNAL_PREFIX}/import",
    )
    scope: Scope = {
        "type": "http",
        "method": "POST",
        "path": f"{INTERNAL_PREFIX}/import",
        "headers": [],
    }

    async def receive() -> dict[str, object]:
        calls.append(1)
        return chunks.pop(0)

    async def send(message: Message) -> None:
        sent.append(message)

    await middleware(scope, receive, send)
    assert len(calls) == 2, "the third chunk must never be read"
    assert sent[0]["type"] == "http.response.start"
    assert sent[0]["status"] == 413


def test_a_shape_over_the_mesh_budget_is_retried_once_with_a_coarser_deflection(
    large_step: bytes, monkeypatch: pytest.MonkeyPatch
) -> None:
    from kala_agent.studio.design import mesh as mesh_module

    calls: list[dict[str, object]] = []

    def fake_tessellate(
        shape: TopoDS_Shape,
        *,
        deflection_mm: float = DEFAULT_MESH_DEFLECTION_MM,
        angular_deflection_rad: float = DEFAULT_ANGULAR_DEFLECTION_RAD,
        max_vertices: int = MAX_MESH_VERTICES,
        max_triangles: int = MAX_MESH_TRIANGLES,
    ) -> MeshData:
        calls.append(
            {
                "deflection_mm": None
                if deflection_mm == DEFAULT_MESH_DEFLECTION_MM
                else deflection_mm,
                "angular_deflection_rad": angular_deflection_rad,
            }
        )
        if len(calls) == 1:
            raise MeshError("the mesh exceeds the vertex budget")
        return real_tessellate(
            shape,
            deflection_mm=deflection_mm,
            angular_deflection_rad=angular_deflection_rad,
            max_vertices=max_vertices,
            max_triangles=max_triangles,
        )

    monkeypatch.setattr(mesh_module, "tessellate", fake_tessellate)
    outcome = import_step_bytes(large_step, store_artifact=False)
    assert len(calls) == 2
    assert calls[0].get("deflection_mm") is None
    assert calls[1]["deflection_mm"] == COARSER_MESH_DEFLECTION_MM
    assert COARSENED_WARNING in outcome.warnings


def test_a_shape_over_the_mesh_budget_after_the_retry_is_refused(
    large_step: bytes, monkeypatch: pytest.MonkeyPatch
) -> None:
    from kala_agent.studio.design import mesh as mesh_module

    def fake_tessellate(
        shape: TopoDS_Shape,
        *,
        deflection_mm: float = DEFAULT_MESH_DEFLECTION_MM,
        angular_deflection_rad: float = DEFAULT_ANGULAR_DEFLECTION_RAD,
        max_vertices: int = MAX_MESH_VERTICES,
        max_triangles: int = MAX_MESH_TRIANGLES,
    ) -> MeshData:
        del shape, deflection_mm, angular_deflection_rad, max_vertices, max_triangles
        raise MeshError("the mesh exceeds the vertex budget")

    monkeypatch.setattr(mesh_module, "tessellate", fake_tessellate)
    with pytest.raises(StepMeshError):
        import_step_bytes(large_step, store_artifact=False)
