"""HTTP acceptance tests for the Studio design turns and journal routes."""

from __future__ import annotations

import hashlib
import json
import math
import sqlite3
from collections.abc import Iterator
from concurrent.futures import ThreadPoolExecutor
from inspect import signature
from pathlib import Path
from typing import Any, cast

import pytest
from fastapi.testclient import TestClient

from kala_agent.studio.design.intent import DESIGN_INTENT_INSTRUCTION
from kala_agent.studio.providers.base import ChatOutcome, ProviderTimeoutError
from kala_agent.studio.providers.dispatch import (
    MAX_REPAIR_PREVIOUS_CHARS,
    StudioChatTurn,
    _extract_design_json_block,
    _repair_prompt,
)
from kala_agent.studio.providers.dispatch import chat as dispatch_chat
from kala_agent.studio.providers.openai_compat import (
    STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS,
    STUDIO_REQUEST_TIMEOUT_SECONDS,
    openai_compat_chat,
)
from kala_agent.studio.server import create_studio_app

LOOPBACK_BASE_URL = "http://127.0.0.1"
LAUNCH_TOKEN = "studio-test-token-0123456789abcdef"
AUTH = {"Authorization": f"Bearer {LAUNCH_TOKEN}"}
ADR_VOLUME_MM3 = 35_571.416047
ADR_AREA_MM2 = 11_889.101731
STITCH_PROMPT = (
    "Design a machined aluminum L-bracket for a 40 mm mounting plate, 8 mm wall "
    "thickness, with a 10 mm centered bore and 4x M5 corner mounting holes."
)


def _lbracket_intent() -> dict[str, object]:
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
    }


def _lbracket_body() -> dict[str, object]:
    return {**_lbracket_intent(), "provider": "opencascade"}


def _two_part_assembly_script() -> dict[str, object]:
    """A place-only assembly of two boxes: the script declares two solids."""
    box = {
        "steps": [
            {
                "op": "sketch",
                "profile": "rectangle",
                "plane": "XY",
                "width": 10,
                "height": 10,
            },
            {"op": "extrude", "length": 5},
        ]
    }
    return {
        "summary": "two placed 10 x 10 x 5 boxes",
        "components": [
            {"name": "base", **box},
            {"name": "riser", **box},
        ],
        "assembly": [
            {"op": "place", "component": "base", "x": 0, "y": 0, "z": 0},
            {"op": "place", "component": "riser", "x": 20, "y": 0, "z": 0},
        ],
    }


@pytest.fixture
def data_dir(tmp_path: Path) -> Path:
    return tmp_path


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


def _make_design(client: TestClient) -> dict[str, Any]:
    response = client.post("/api/studio/design", json=_lbracket_body())
    assert response.status_code == 200, response.text
    return cast("dict[str, Any]", response.json())


def _events(client: TestClient) -> list[dict[str, Any]]:
    response = client.get("/api/studio/journal", params={"after": 0, "limit": 500})
    assert response.status_code == 200, response.text
    return cast("list[dict[str, Any]]", response.json()["events"])


# ---------------------------------------------------------------- criterion 2


def test_health_reports_the_design_capability(client: TestClient) -> None:
    body = client.get("/api/studio/health").json()
    assert body["design_available"] is True


def test_criterion_2_lbracket_end_to_end(client: TestClient) -> None:
    body = _make_design(client)
    assert len(body["design_id"]) == 64
    assert body["design_id"] != body["artifacts"]["script_sha256"]
    assert body["provider"] == "opencascade"
    assert body["provider_version"]
    assert body["measured_by"] == "OpenCascade (opt-in)"
    assert body["artifacts"]["step_sha256"]
    assert body["artifacts"]["native_sha256"] is None
    assert body["verified"] is True
    assert body["cached"] is False
    measured = body["measured"]
    assert measured["volume_mm3"] == pytest.approx(ADR_VOLUME_MM3, rel=1e-6)
    assert measured["surface_area_mm2"] == pytest.approx(ADR_AREA_MM2, rel=1e-6)
    assert measured["bounds"]["min"] == [-30.0, 0.0, 0.0]
    assert measured["bounds"]["max"] == [30.0, 45.0, 40.0]
    assert measured["size_mm"] == [60.0, 45.0, 40.0]
    assert measured["valid"] is True
    assert measured["solid_count"] == 1
    checks = {row["id"]: row for row in body["checks"]}
    for check_id in ("brep_valid", "single_solid", "envelope_x", "envelope_y", "envelope_z"):
        assert checks[check_id]["passed"] is True
    assert body["declared_envelope"] == {
        "min": [-30.0, 0.0, 0.0],
        "max": [30.0, 45.0, 40.0],
    }
    assert body["material"] == "aluminium"
    assert "ISO 2768-m (declared)" in body["tolerances"]
    view = body["view"]
    assert view["engine"] == "ocp"
    assert view["vertex_count"] == len(view["positions"]) // 3
    assert view["triangle_count"] == len(view["indices"]) // 3
    assert view["vertex_count"] > 0
    assert view["bounds"]["min"] == pytest.approx([-30.0, 0.0, 0.0], abs=0.001)
    assert view["bounds"]["max"] == pytest.approx([30.0, 45.0, 40.0], abs=0.001)
    assert view["solids"] == 1
    assert body["journal_head"]


def test_a_restarted_server_restores_the_measured_area_from_the_journal(
    data_dir: Path,
) -> None:
    """Cold reload through the HTTP route: the area must survive a restart.

    The second application owns a fresh service (and an empty in-memory
    cache), so the design comes back from the journal's stored build record.
    The writer stores ``surface_area_mm2``; a reader that looked for
    ``area_mm2`` reported null here, and the card's area row showed "—".
    """

    def application() -> TestClient:
        return TestClient(
            create_studio_app(
                data_dir=data_dir,
                api_token=LAUNCH_TOKEN,
                max_body_bytes=64 * 1024,
                static_root=None,
            ),
            base_url=LOOPBACK_BASE_URL,
            headers=AUTH,
            raise_server_exceptions=False,
        )

    with application() as first:
        created = _make_design(first)
    assert created["measured"]["surface_area_mm2"] == pytest.approx(ADR_AREA_MM2, rel=1e-6)

    with application() as second:
        restored = _make_design(second)
    assert restored["design_id"] == created["design_id"]
    assert restored["cached"] is True
    assert restored["measured"]["surface_area_mm2"] == pytest.approx(
        created["measured"]["surface_area_mm2"], rel=1e-12
    )
    assert restored["measured"]["volume_mm3"] == pytest.approx(
        created["measured"]["volume_mm3"], rel=1e-12
    )
    assert restored["verified"] is True


def test_criterion_2_place_only_assembly_verifies_with_a_solid_count(
    client: TestClient,
) -> None:
    response = client.post(
        "/api/studio/design",
        json={"provider": "opencascade", "script": _two_part_assembly_script()},
    )
    assert response.status_code == 200, response.text
    body = cast("dict[str, Any]", response.json())
    assert body["provider"] == "opencascade"
    assert body["component_count"] == 2
    assert body["fused"] is False
    assert body["measured"]["solid_count"] == 2
    assert body["view"]["solids"] == 2
    checks = {row["id"]: row for row in body["checks"]}
    assert set(checks) == {"brep_valid", "solid_count"}
    row = checks["solid_count"]
    assert row["label"] == "Solid count"
    assert row["expected"] == 2.0
    assert row["measured"] == 2.0
    assert row["tolerance"] == 0.0
    assert row["unit"] == "solid"
    assert row["passed"] is True
    assert row["status"] == "passed"
    assert "single_solid" not in checks
    assert body["verified"] is True


# ---------------------------------------------------------------- criterion 4


def test_criterion_4_section_at_xz_offset_4(client: TestClient) -> None:
    design = _make_design(client)
    response = client.get(
        f"/api/studio/design/{design['design_id']}/section",
        params={"plane": "XZ", "offset": 4},
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["plane"] == "XZ"
    assert body["offset_mm"] == 4
    assert body["outer_loop_count"] == 1
    assert body["inner_loop_count"] == 1
    assert len(body["loops"]) == 2
    assert body["area_exact"] is True
    assert body["area_mm2"] == pytest.approx(2400 - 25 * math.pi, rel=1e-3)
    outer = next(loop for loop in body["loops"] if loop["outer"])
    inner = next(loop for loop in body["loops"] if not loop["outer"])
    assert outer["bounds"]["width"] == pytest.approx(60, abs=0.05)
    assert outer["bounds"]["height"] == pytest.approx(40, abs=0.05)
    assert inner["area_mm2"] == pytest.approx(25 * math.pi, rel=1e-3)
    assert inner["centroid"][0] == pytest.approx(0, abs=0.05)
    assert inner["centroid"][1] == pytest.approx(24, abs=0.05)
    assert all(loop["area_exact"] is True for loop in body["loops"])


def test_section_plane_is_case_insensitive(client: TestClient) -> None:
    design = _make_design(client)
    response = client.get(
        f"/api/studio/design/{design['design_id']}/section",
        params={"plane": "xz", "offset": 4},
    )
    assert response.status_code == 200
    assert response.json()["plane"] == "XZ"


# ---------------------------------------------------------------- criterion 5


def test_criterion_5_verify_ok_then_tamper(client: TestClient, data_dir: Path) -> None:
    _make_design(client)
    verify = client.get("/api/studio/journal/verify")
    assert verify.status_code == 200
    assert verify.json()["ok"] is True
    assert verify.json()["first_bad_sequence"] is None
    assert verify.json()["event_count"] == 9

    connection = sqlite3.connect(data_dir / "studio-journal.sqlite3")
    connection.execute(
        "UPDATE studio_events SET payload_json = payload_json || ' ' WHERE sequence = 4"
    )
    connection.commit()
    connection.close()

    tampered = client.get("/api/studio/journal/verify")
    assert tampered.status_code == 200
    assert tampered.json()["ok"] is False
    assert tampered.json()["first_bad_sequence"] == 4


# ---------------------------------------------------------------- criterion 6


def test_criterion_6_local_helper_asks_for_missing_values(client: TestClient) -> None:
    response = client.post(
        "/api/studio/chat",
        json={"provider": "local", "prompt": "make me a bracket"},
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["design"] is None
    text = body["text"].lower()
    for required in ("width (mm)", "depth (mm)", "height (mm)", "wall thickness (mm)"):
        assert required in text
    assert "no design was compiled" in text
    turn = [event for event in _events(client) if event["event_type"] == "chat.turn"]
    assert turn[-1]["payload"]["design_id"] is None
    assert turn[-1]["payload"]["design_requested"] is True


def test_criterion_6_stitch_sentence_builds_the_criterion_2_design(
    client: TestClient,
) -> None:
    response = client.post(
        "/api/studio/chat",
        json={
            "provider": "local",
            "prompt": STITCH_PROMPT,
            "cad_provider": "opencascade",
        },
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["design"] is not None
    design = body["design"]
    assert design["measured"]["volume_mm3"] == pytest.approx(ADR_VOLUME_MM3, rel=1e-6)
    assert design["verified"] is True
    assert design["measured"]["bounds"]["min"] == [-30.0, 0.0, 0.0]
    text = body["text"]
    assert "Measured volume" in text
    assert "ISO 273 medium" in text
    assert "5.5 mm clearance holes" in text
    assert design["design_id"] in text


def test_local_non_design_reply_still_returns_no_design(client: TestClient) -> None:
    response = client.post(
        "/api/studio/chat",
        json={"provider": "local", "prompt": "Estimate a box 20x30x40 mm."},
    )
    assert response.status_code == 200
    body = response.json()
    assert body["design"] is None
    assert "24000 mm^3" in body["text"]


# ---------------------------------------------------------------- criterion 7


def test_new_get_routes_require_the_bearer(client: TestClient, data_dir: Path) -> None:
    design = _make_design(client)
    design_id = str(design["design_id"])
    unauth = TestClient(
        client.app,
        base_url=LOOPBACK_BASE_URL,
        raise_server_exceptions=False,
    )
    for path, params in (
        ("/api/studio/journal", {"after": 0, "limit": 10}),
        ("/api/studio/journal/verify", {}),
        ("/api/studio/providers/cad", {}),
        (f"/api/studio/design/{design_id}/drawing", {"views": "front"}),
        (f"/api/studio/design/{design_id}/section", {"plane": "XZ"}),
    ):
        response = unauth.get(path, params=params)
        assert response.status_code == 401, (path, response.text)
        assert response.json()["code"] == "unauthorized"

    wrong = {"Authorization": "Bearer wrong-token-123456789012345678"}
    assert unauth.get("/api/studio/journal", headers=wrong).status_code == 401


def test_new_post_routes_require_the_bearer(client: TestClient) -> None:
    unauth = TestClient(
        client.app,
        base_url=LOOPBACK_BASE_URL,
        raise_server_exceptions=False,
    )
    assert unauth.post("/api/studio/design", json=_lbracket_body()).status_code == 401
    assert (
        unauth.post(
            "/api/studio/export",
            json={"design_id": "0" * 64, "format": "step"},
        ).status_code
        == 401
    )


def test_new_routes_refuse_a_bad_host_and_origin(client: TestClient) -> None:
    design = _make_design(client)
    design_id = str(design["design_id"])
    assert client.get("/api/studio/journal", headers={"Host": "example.com"}).status_code == 421
    assert (
        client.post(
            "/api/studio/design",
            json=_lbracket_body(),
            headers={"Host": "example.com"},
        ).status_code
        == 421
    )
    assert (
        client.get(
            f"/api/studio/design/{design_id}/section",
            params={"plane": "XZ"},
            headers={"Origin": "https://evil.example"},
        ).status_code
        == 403
    )
    assert (
        client.post(
            "/api/studio/design",
            json=_lbracket_body(),
            headers={"Origin": "https://evil.example"},
        ).status_code
        == 403
    )


def test_new_post_routes_enforce_the_body_limit(client: TestClient) -> None:
    oversized = "{" + '"x":"' + ("a" * 70_000) + '"}'
    for path in ("/api/studio/design", "/api/studio/export", "/api/studio/chat"):
        response = client.post(
            path,
            content=oversized,
            headers={"Content-Type": "application/json"},
        )
        assert response.status_code == 413, (path, response.text)
        assert response.json()["code"] == "payload_too_large"


def test_views_and_plane_and_offset_are_validated(client: TestClient) -> None:
    design = _make_design(client)
    design_id = str(design["design_id"])
    base = f"/api/studio/design/{design_id}"
    assert client.get(f"{base}/drawing", params={"views": "front,top"}).status_code == 200
    assert client.get(f"{base}/drawing", params={"views": "front,bogus"}).status_code == 422
    assert client.get(f"{base}/drawing", params={"views": "front,front"}).status_code == 422
    assert client.get(f"{base}/drawing", params={"views": "front,"}).status_code == 422
    assert client.get(f"{base}/section", params={"plane": "AA"}).status_code == 422
    assert client.get(f"{base}/section", params={"offset": "nan"}).status_code == 422
    assert client.get(f"{base}/section", params={"offset": "inf"}).status_code == 422
    assert client.get(f"{base}/section", params={"offset": 999_999}).status_code == 422


def test_design_id_is_validated_and_unknown_ids_are_404(client: TestClient) -> None:
    base = "/api/studio/design"
    assert client.get(f"{base}/not-a-digest/section").status_code == 422
    unknown = "0" * 64
    assert client.get(f"{base}/{unknown}/drawing").status_code == 404
    assert client.get(f"{base}/{unknown}/section").status_code == 404


# ---------------------------------------------------------- artifact download


def test_artifact_download_serves_the_exact_stored_bytes(client: TestClient) -> None:
    design = _make_design(client)
    design_id = str(design["design_id"])
    base = f"/api/studio/design/{design_id}/artifact"

    script = client.get(f"{base}/script")
    assert script.status_code == 200, script.text
    assert script.headers["content-type"] == "application/json"
    assert script.headers["x-studio-design-id"] == design_id
    assert script.headers["x-studio-artifact-kind"] == "script"
    assert script.headers["x-studio-digest"] == design["artifacts"]["script_sha256"]
    assert hashlib.sha256(script.content).hexdigest() == design["artifacts"]["script_sha256"]
    assert script.headers["content-disposition"].startswith("attachment; filename=")
    assert script.headers["content-disposition"].endswith(".json\"")
    # The served script is the canonical construction script, not a re-export.
    assert json.loads(script.content)["components"]

    step = client.get(f"{base}/step")
    assert step.status_code == 200, step.text
    assert step.headers["content-type"] == "application/step"
    assert step.content.startswith(b"ISO-10303-21")
    assert step.headers["x-studio-digest"] == design["artifacts"]["step_sha256"]
    assert hashlib.sha256(step.content).hexdigest() == design["artifacts"]["step_sha256"]
    assert step.headers["content-disposition"].endswith(".step\"")


def test_artifact_download_refuses_a_kind_the_design_did_not_store(client: TestClient) -> None:
    design = _make_design(client)
    base = f"/api/studio/design/{design['design_id']}/artifact"
    # The OpenCascade provider stores no native document and no STL.
    for kind in ("fcstd", "stl"):
        response = client.get(f"{base}/{kind}")
        assert response.status_code == 404, response.text
        body = response.json()
        assert body["code"] == "artifact_not_found"
        assert body["message"] == "no artifact of that kind is stored for this design"
        assert body["details"] == [f"design_id={design['design_id']}; kind={kind}"]
    unknown = client.get(f"{base}/bogus")
    assert unknown.status_code == 422
    assert unknown.json()["details"] == ["the artifact kind must be one of script, step, fcstd, stl"]


def test_artifact_download_validates_the_design_id_and_the_bearer(client: TestClient) -> None:
    base = "/api/studio/design"
    invalid = client.get(f"{base}/not-a-digest/artifact/step")
    assert invalid.status_code == 422
    assert invalid.json()["details"] == ["the design id is not a sha256 digest"]
    denied = client.get(
        f"{base}/{'0' * 64}/artifact/step",
        headers={"Authorization": "Bearer wrong-token"},
    )
    assert denied.status_code == 401


def test_artifact_download_verifies_the_journalled_digest(client: TestClient, tmp_path: Path) -> None:
    design = _make_design(client)
    digest = str(design["artifacts"]["script_sha256"])
    object_path = tmp_path / "artifacts" / "objects" / digest[:2] / digest
    assert object_path.is_file()
    object_path.write_bytes(b'{"tampered":true}')
    response = client.get(f"/api/studio/design/{design['design_id']}/artifact/script")
    assert response.status_code == 500
    assert response.json()["code"] == "artifact_digest_mismatch"
    assert response.json()["message"] == (
        "the stored artifact does not match its journalled digest"
    )


# ------------------------------------------------------------ idempotency


def test_same_script_twice_is_the_same_design_and_a_cache_hit(client: TestClient) -> None:
    first = _make_design(client)
    second = _make_design(client)
    assert first["design_id"] == second["design_id"]
    assert second["cached"] is True
    events = _events(client)
    compiled = [event for event in events if event["event_type"] == "design.compiled"]
    intents = [event for event in events if event["event_type"] == "design.intent"]
    assert len(compiled) == 1
    assert intents[-1]["payload"]["cache_hit"] is True


def test_eight_parallel_design_posts_all_succeed_and_verify(client: TestClient) -> None:
    with ThreadPoolExecutor(max_workers=8) as pool:
        responses = list(
            pool.map(
                lambda _index: client.post("/api/studio/design", json=_lbracket_body()), range(8)
            )
        )
    assert all(response.status_code == 200 for response in responses)
    assert len({response.json()["design_id"] for response in responses}) == 1
    verify = client.get("/api/studio/journal/verify")
    assert verify.status_code == 200
    assert verify.json()["ok"] is True
    compiled = [event for event in _events(client) if event["event_type"] == "design.compiled"]
    assert len(compiled) == 1


# ---------------------------------------------------------------- drawing/export


def test_drawing_returns_the_requested_views(client: TestClient) -> None:
    design = _make_design(client)
    response = client.get(
        f"/api/studio/design/{design['design_id']}/drawing",
        params={"views": "front,top,right,iso"},
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert [view["name"] for view in body["views"]] == ["front", "top", "right", "iso"]
    assert body["max_points"] > 0
    assert body["point_count"] == sum(view["point_count"] for view in body["views"])
    for view in body["views"]:
        assert view["visible"]
        assert all(
            len(polyline["points"]) == polyline["point_count"] for polyline in view["visible"]
        )


def test_export_design_step_returns_digest_and_artifact(client: TestClient) -> None:
    design = _make_design(client)
    design_id = str(design["design_id"])
    response = client.post(
        "/api/studio/export",
        json={"design_id": design_id, "format": "step"},
    )
    assert response.status_code == 200, response.text
    assert response.content.startswith(b"ISO-10303-21")
    digest = response.headers["x-studio-digest"]
    assert digest == hashlib.sha256(response.content).hexdigest()
    assert response.headers["x-studio-design-id"] == design_id
    assert response.headers["content-disposition"].startswith("attachment")
    assert any(
        event["event_type"] == "export.completed" and event["payload"]["sha256"] == digest
        for event in _events(client)
    )
    refreshed = _make_design(client)
    assert refreshed["artifacts"]["step_sha256"] == digest


def test_export_design_stl_is_binary(client: TestClient) -> None:
    design = _make_design(client)
    response = client.post(
        "/api/studio/export",
        json={"design_id": design["design_id"], "format": "stl"},
    )
    assert response.status_code == 200
    assert response.headers["content-type"].startswith("model/stl")
    assert response.content.startswith(b"kala-studio design mesh")


def test_export_request_needs_exactly_one_source(client: TestClient) -> None:
    design = _make_design(client)
    response = client.post(
        "/api/studio/export",
        json={
            "design_id": design["design_id"],
            "kind": "box",
            "format": "step",
        },
    )
    assert response.status_code == 422
    assert client.post("/api/studio/export", json={"format": "step"}).status_code == 422


# ---------------------------------------------------------------- refusal / 503


def test_compile_refusal_is_journalled_without_an_artifact(client: TestClient) -> None:
    script = {
        "summary": "unsupported helix",
        "components": [
            {
                "name": "shaft",
                "steps": [
                    {"op": "sketch", "profile": "circle", "plane": "XY", "diameter": 10},
                    {"op": "extrude", "length": 20},
                    {"op": "helix", "diameter": 10, "pitch": 2, "length": 10},
                ],
            }
        ],
    }
    response = client.post("/api/studio/design", json={"script": script, "provider": "opencascade"})
    assert response.status_code == 422
    assert response.json()["code"] == "design_rejected"
    assert "helix" in response.json()["details"][0]
    events = _events(client)
    types = [event["event_type"] for event in events]
    assert "design.rejected" in types
    assert "design.compiled" not in types
    rejected = next(event for event in events if event["event_type"] == "design.rejected")
    design_id = rejected["payload"]["design_id"]
    service = cast("Any", client.app).state.design_service
    assert service.journal.has_artifact(design_id) is False
    assert client.get("/api/studio/journal/verify").json()["ok"] is True


def test_design_routes_return_503_when_ocp_is_missing(
    client: TestClient, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("kala_agent.studio.server.design_available", lambda: False)
    response = client.post("/api/studio/design", json=_lbracket_body())
    assert response.status_code == 503
    assert response.json()["code"] == "design_unavailable"
    assert "occurring" not in response.text
    section = client.get(f"/api/studio/design/{'0' * 64}/section")
    assert section.status_code == 503
    assert client.get("/api/studio/health").json()["design_available"] is False


# ---------------------------------------------------------------- journal page


def test_journal_page_is_bounded_and_paginates(client: TestClient) -> None:
    _make_design(client)
    _make_design(client)
    first_page = client.get("/api/studio/journal", params={"after": 0, "limit": 2})
    assert first_page.status_code == 200
    body = first_page.json()
    assert [event["sequence"] for event in body["events"]] == [1, 2]
    assert body["next_after"] == 2
    assert body["has_more"] is True
    second_page = client.get("/api/studio/journal", params={"after": 2, "limit": 500})
    assert second_page.json()["events"][0]["sequence"] == 3
    assert second_page.json()["has_more"] is False
    empty = client.get("/api/studio/journal", params={"after": 500, "limit": 10})
    assert empty.json()["events"] == []
    assert empty.json()["next_after"] == 500


def test_create_app_honours_kala_studio_data_dir(tmp_path: Path) -> None:
    studio_dir = tmp_path / "explicit-studio-dir"
    env = {
        "KALA_STUDIO_DATA_DIR": str(studio_dir),
        "KALA_STUDIO_TOKEN": LAUNCH_TOKEN,
    }
    app = create_studio_app(environ=env)
    assert app.state.studio_data_dir == studio_dir
    with TestClient(
        app,
        base_url=LOOPBACK_BASE_URL,
        headers=AUTH,
        raise_server_exceptions=False,
    ) as scoped:
        response = scoped.post("/api/studio/design", json=_lbracket_body())
        assert response.status_code == 200
    assert (studio_dir / "studio-journal.sqlite3").is_file()


# ---------------------------------------------------------------- remote hook


def test_provider_control_plane_does_not_import_the_kernel() -> None:
    import ast

    import kala_agent.studio.providers.dispatch as dispatch_module
    import kala_agent.studio.providers.local as local_module

    for module in (local_module, dispatch_module):
        assert module.__file__ is not None
        tree = ast.parse(Path(module.__file__).read_text(encoding="utf-8"))
        imported: set[str] = set()
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                imported.update(alias.name for alias in node.names)
            elif isinstance(node, ast.ImportFrom) and node.module:
                imported.add(node.module)
        assert not any(name == "OCP" or name.startswith("OCP.") for name in imported)
        assert "kala_agent.studio.design.templates" not in imported
        assert "kala_agent.studio.design.measure" not in imported
        assert "kala_agent.studio.design.mesh" not in imported
        assert "kala_agent.studio.design.compile_ocp" not in imported


def _valid_reply() -> str:
    return "Compiling the bracket.\n\n```json\n" + json.dumps(_lbracket_intent()) + "\n```"


def _long_design_reply(pairs: int = 100) -> str:
    """Return prose plus a design JSON longer than the 4 096 display bound."""
    steps: list[dict[str, object]] = [
        {"op": "sketch", "plane": "XY", "profile": "rectangle", "width": 200, "height": 200},
        {"op": "extrude", "length": 5},
    ]
    for index in range(pairs):
        steps.append(
            {
                "op": "sketch",
                "plane": "XY",
                "profile": "circle",
                "diameter": 3,
                "x": 20 + 4 * index,
                "y": 20,
            }
        )
        steps.append({"op": "pocket"})
    payload = {
        "script": {
            "summary": "200 x 200 x 5 mm plate with through holes",
            "components": [{"name": "plate", "steps": steps}],
        }
    }
    return "Here is the plate.\n\n```json\n" + json.dumps(payload) + "\n```"


def test_a_reply_longer_than_the_display_bound_is_not_truncated_before_parsing(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    reply = _long_design_reply()
    assert len(reply) > 6_000
    assert len(reply[:4096]) < len(reply)

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        del prompt, allow_remote, environ, timeout
        # Exactly what the HTTP provider does: the display keeps the 4 096
        # bound and the full reply is kept for design extraction.
        return ChatOutcome(
            text=reply[:4096],
            reply=reply,
            provider=provider,
            model=model,
            offline=False,
        )

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    outcome = dispatch_chat(
        provider="openai",
        model="gpt-4o-mini",
        prompt=STITCH_PROMPT,
        allow_remote=True,
        environ={"OPENAI_API_KEY": "sk-test-1234567890abcdef"},
    )
    assert isinstance(outcome, StudioChatTurn)
    assert outcome.design_intent is not None
    script = outcome.design_intent.script
    assert script is not None
    assert len(script.components[0].steps) == 202
    assert len(outcome.text) <= 4096
    assert outcome.text.startswith("Here is the plate.")
    assert "```json" not in outcome.text


def test_the_provider_reader_keeps_the_full_reply() -> None:
    from kala_agent.studio.providers import openai_compat

    content = "x" * 5_000 + '```json\n{"a":1}\n```'
    data = json.dumps({"choices": [{"message": {"content": content}}]}).encode("utf-8")

    text, _model = openai_compat._read_response(data)

    assert text == content


def test_remote_design_hook_parses_schema_json(monkeypatch: pytest.MonkeyPatch) -> None:
    prompts: list[str] = []

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        prompts.append(prompt)
        return ChatOutcome(text=_valid_reply(), provider=provider, model=model, offline=False)

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    outcome = dispatch_chat(
        provider="openai",
        model="gpt-4o-mini",
        prompt=STITCH_PROMPT,
        allow_remote=True,
        environ={"OPENAI_API_KEY": "sk-test-1234567890abcdef"},
    )
    assert isinstance(outcome, StudioChatTurn)
    assert outcome.design_intent is not None
    assert len(prompts) == 1
    assert "DesignIntent JSON Schema" in prompts[0]
    assert "l_bracket" in prompts[0]
    assert STITCH_PROMPT in prompts[0]


def test_remote_design_hook_allows_one_repair_round_trip(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    responses = iter(
        [
            "```json\n{not json}\n```",
            _valid_reply(),
        ]
    )
    prompts: list[str] = []

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        prompts.append(prompt)
        return ChatOutcome(text=next(responses), provider=provider, model=model, offline=False)

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    outcome = dispatch_chat(
        provider="openai",
        model="gpt-4o-mini",
        prompt=STITCH_PROMPT,
        allow_remote=True,
        environ={"OPENAI_API_KEY": "sk-test-1234567890abcdef"},
    )
    assert isinstance(outcome, StudioChatTurn)
    assert outcome.design_intent is not None
    assert len(prompts) == 2
    assert "could not be validated" in prompts[1]
    assert "not valid JSON" in prompts[1]
    assert "Schema for" in prompts[1]


def test_the_validation_repair_prompt_carries_the_instruction_the_schema_and_the_previous_json(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The repair call must see the same world the first call saw.

    Every model call is stateless. Without the instruction, the full schema
    and its own previous JSON the repair re-authors the design blind: it
    invents a different shape and loses the values it had.
    """
    prompts: list[str] = []

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        prompts.append(prompt)
        return ChatOutcome(
            text='```json\n{"template": {"template": "l_bracket", "width": -3}}\n```',
            provider=provider,
            model=model,
            offline=False,
        )

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    outcome = dispatch_chat(
        provider="openai",
        model="gpt-4o-mini",
        prompt=STITCH_PROMPT,
        allow_remote=True,
        environ={"OPENAI_API_KEY": "sk-test-1234567890abcdef"},
    )
    assert isinstance(outcome, StudioChatTurn)
    assert len(prompts) == 2
    repair = prompts[1]
    assert repair.startswith(DESIGN_INTENT_INSTRUCTION), repair
    assert "When no template matches, author a ConstructionScript" in repair
    assert "Worked examples" in repair
    assert "DesignIntent JSON Schema:" in repair
    assert "The JSON block you sent:" in repair
    assert '{"template": {"template": "l_bracket", "width": -3}' in repair
    assert "could not be validated" in repair
    assert "Keep every value the user stated" in repair
    assert "If a required value is missing, reply in prose" in repair
    assert STITCH_PROMPT in repair
    # The order of the prompt is the order of the facts the model needs.
    assert repair.index("DesignIntent JSON Schema:") < repair.index("The JSON block you sent:")
    assert repair.index("The JSON block you sent:") < repair.index("could not be validated")


def test_the_validation_repair_prompt_bounds_the_previous_json() -> None:
    huge = (
        "```json\n"
        + '{"script": {"summary": "'
        + "x" * (MAX_REPAIR_PREVIOUS_CHARS + 5_000)
        + '"}}\n```'
    )
    prompt = _repair_prompt("a plate 80 x 40 x 8 mm", "script: Field required", huge)
    assert "The JSON block you sent:" in prompt
    carried = prompt.split("The JSON block you sent:\n", 1)[1].split("\n\n", 1)[0]
    assert len(carried) <= MAX_REPAIR_PREVIOUS_CHARS


def test_the_previous_json_block_is_extracted_from_the_reply() -> None:
    assert _extract_design_json_block('prose\n```json\n{"script": {}}\n```') == '{"script": {}}'
    assert _extract_design_json_block('prose {"script": {}} more') == '{"script": {}}'
    assert _extract_design_json_block('```json\n{"a": {"b": 1}}\n```') == '{"a": {"b": 1}}'
    assert _extract_design_json_block("no json here") is None


def test_the_validation_repair_text_names_both_errors(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The honest text names what went wrong the first time too."""
    replies = iter(
        [
            '```json\n{"template": 3}\n```',
            '```json\n{"script": {"components": []}}\n```',
        ]
    )

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        return ChatOutcome(
            text=next(replies),
            provider=provider,
            model=model,
            offline=False,
        )

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    outcome = dispatch_chat(
        provider="openai",
        model="gpt-4o-mini",
        prompt=STITCH_PROMPT,
        allow_remote=True,
        environ={"OPENAI_API_KEY": "sk-test-1234567890abcdef"},
    )
    assert isinstance(outcome, StudioChatTurn)
    assert outcome.design_intent is None
    assert "No part was built" in outcome.text
    assert "the first reply was not a valid design (template:" in outcome.text
    assert (
        "and the reply after one repair attempt was not a valid design either (script."
        in outcome.text
    )
    assert outcome.design_error


def test_remote_design_hook_fails_honestly_after_one_repair(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls = 0

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        nonlocal calls
        calls += 1
        return ChatOutcome(
            text='```json\n{"template": 3}\n```',
            provider=provider,
            model=model,
            offline=False,
        )

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    outcome = dispatch_chat(
        provider="openai",
        model="gpt-4o-mini",
        prompt=STITCH_PROMPT,
        allow_remote=True,
        environ={"OPENAI_API_KEY": "sk-test-1234567890abcdef"},
    )
    assert isinstance(outcome, StudioChatTurn)
    assert outcome.design_intent is None
    assert calls == 2
    assert "No part was built" in outcome.text
    assert "after one repair attempt" in outcome.text
    assert outcome.design_error


def test_remote_missing_values_is_a_question_without_a_repair(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    prompts: list[str] = []

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        prompts.append(prompt)
        return ChatOutcome(
            text="What is the plate thickness and the hole diameter?",
            provider=provider,
            model=model,
            offline=False,
        )

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    outcome = dispatch_chat(
        provider="openai",
        model="gpt-4o-mini",
        prompt=STITCH_PROMPT,
        allow_remote=True,
        environ={"OPENAI_API_KEY": "sk-test-1234567890abcdef"},
    )
    assert isinstance(outcome, StudioChatTurn)
    assert outcome.design_intent is None
    assert len(prompts) == 1
    assert "What is the plate thickness" in outcome.text
    assert "No part was built" in outcome.text
    assert "repair" not in outcome.text


def test_remote_repair_that_asks_for_values_is_a_question(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    responses = iter(
        [
            '{"template": 3}',
            "What wall thickness should the bracket use?",
        ]
    )
    prompts: list[str] = []

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        prompts.append(prompt)
        return ChatOutcome(text=next(responses), provider=provider, model=model, offline=False)

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    outcome = dispatch_chat(
        provider="openai",
        model="gpt-4o-mini",
        prompt=STITCH_PROMPT,
        allow_remote=True,
        environ={"OPENAI_API_KEY": "sk-test-1234567890abcdef"},
    )
    assert isinstance(outcome, StudioChatTurn)
    assert outcome.design_intent is None
    assert outcome.design_error is None
    assert len(prompts) == 2
    assert "What wall thickness" in outcome.text
    assert "No part was built" in outcome.text
    assert "asks for values" in outcome.text
    assert "not a valid design" not in outcome.text


def test_remote_script_is_authored_from_a_non_template_prompt_with_a_cad_provider(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    plate = {
        "script": {
            "summary": "80 x 40 x 8 mm plate with four 6 mm through holes",
            "components": [
                {
                    "name": "plate",
                    "steps": [
                        {
                            "op": "sketch",
                            "plane": "XY",
                            "profile": "rectangle",
                            "width": 80,
                            "height": 40,
                        },
                        {"op": "extrude", "length": 8},
                        {
                            "op": "sketch",
                            "plane": "XY",
                            "profile": "circle",
                            "diameter": 6,
                            "x": 8,
                            "y": 8,
                        },
                        {"op": "pocket"},
                    ],
                }
            ],
        }
    }
    prompts: list[str] = []

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        prompts.append(prompt)
        return ChatOutcome(
            text=json.dumps(plate),
            provider=provider,
            model=model,
            offline=False,
        )

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    outcome = dispatch_chat(
        provider="openai",
        model="gpt-4o-mini",
        prompt="aluminium mounting plate 80 x 40 x 8 mm with four 6 mm through holes",
        allow_remote=True,
        cad_provider="opencascade",
        environ={"OPENAI_API_KEY": "sk-test-1234567890abcdef"},
    )
    assert isinstance(outcome, StudioChatTurn)
    assert outcome.design_intent is not None
    assert outcome.design_intent.script is not None
    assert outcome.design_family is None
    assert len(prompts) == 1
    assert "When no template matches, author a ConstructionScript" in prompts[0]


def test_remote_non_design_prompt_is_left_alone(monkeypatch: pytest.MonkeyPatch) -> None:
    prompts: list[str] = []
    timeouts: list[float | None] = []

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        prompts.append(prompt)
        timeouts.append(timeout)
        return ChatOutcome(text="plain reply", provider=provider, model=model, offline=False)

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    outcome = dispatch_chat(
        provider="openai",
        model="gpt-4o-mini",
        prompt="What is a STEP file?",
        allow_remote=True,
        environ={"OPENAI_API_KEY": "sk-test-1234567890abcdef"},
    )
    assert isinstance(outcome, StudioChatTurn)
    assert prompts == ["What is a STEP file?"]
    assert timeouts == [None], "the plain path must leave the 30 s provider default"
    assert outcome.design_intent is None


def test_design_turn_uses_the_longer_remote_timeout(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    replies = iter(["```json\n{not json}\n```", _valid_reply()])
    timeouts: list[float | None] = []

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        timeouts.append(timeout)
        return ChatOutcome(text=next(replies), provider=provider, model=model, offline=False)

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    outcome = dispatch_chat(
        provider="openai",
        model="gpt-4o-mini",
        prompt=STITCH_PROMPT,
        allow_remote=True,
        environ={"OPENAI_API_KEY": "sk-test-1234567890abcdef"},
    )
    assert isinstance(outcome, StudioChatTurn)
    assert outcome.design_intent is not None
    assert timeouts == [
        STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS,
        STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS,
    ], "the first design call and its repair call must both carry 180 s"
    assert STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS == 180.0
    assert (
        signature(openai_compat_chat).parameters["timeout"].default
        == STUDIO_REQUEST_TIMEOUT_SECONDS
    )
    assert STUDIO_REQUEST_TIMEOUT_SECONDS == 30.0


def test_a_timed_out_design_turn_is_journalled_once(
    client: TestClient, monkeypatch: pytest.MonkeyPatch
) -> None:
    prompt = "round steel flange 100 mm OD, 12 mm thick, 30 mm bore, six 8 mm holes on an 80 mm PCD"

    def timeout_chat(*_args: object, **_kwargs: object) -> ChatOutcome:
        raise ProviderTimeoutError("the provider request exceeded its bounded timeout")

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", timeout_chat)
    response = client.post(
        "/api/studio/chat",
        json={
            "provider": "openrouter",
            "model": "minimax/minimax-m3.1-flash:free",
            "prompt": prompt,
            "allow_remote": True,
            "cad_provider": "opencascade",
        },
    )
    assert response.status_code == 504
    assert response.json() == {
        "code": "provider_timeout",
        "message": "the chat provider exceeded its bounded timeout",
        "details": ["the provider request exceeded its bounded timeout"],
    }
    turns = [event for event in _events(client) if event["event_type"] == "chat.turn"]
    assert len(turns) == 1, "a failed design turn is journalled exactly once"
    payload = turns[0]["payload"]
    assert payload["provider"] == "openrouter"
    assert payload["offline"] is False
    assert payload["design_requested"] is True
    assert payload["design_id"] is None
    assert payload["cad_provider"] == "opencascade"
    assert payload["missing"] == []
    assert payload["failure"] == "provider_timeout"
    assert payload["prompt_chars"] == len(prompt)
    assert len(payload["prompt_sha256"]) == 64
    assert "prompt" not in payload


def test_a_broken_journal_does_not_mask_the_provider_error(
    client: TestClient, monkeypatch: pytest.MonkeyPatch
) -> None:
    def timeout_chat(*_args: object, **_kwargs: object) -> ChatOutcome:
        raise ProviderTimeoutError("the provider request exceeded its bounded timeout")

    def broken_append(*_args: object, **_kwargs: object) -> None:
        raise RuntimeError("the journal is broken")

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", timeout_chat)
    monkeypatch.setattr(
        "kala_agent.studio.session_journal.StudioSessionJournal.append", broken_append
    )
    response = client.post(
        "/api/studio/chat",
        json={
            "provider": "openai",
            "model": "gpt-4o-mini",
            "prompt": "round steel flange 100 mm OD, 12 mm thick, 30 mm bore",
            "allow_remote": True,
            "cad_provider": "opencascade",
        },
    )
    assert response.status_code == 504
    assert response.json()["code"] == "provider_timeout"
    assert response.json()["details"] == ["the provider request exceeded its bounded timeout"]
    assert [event for event in _events(client) if event["event_type"] == "chat.turn"] == []


# ------------------------------------------------------- the build-feedback repair

PLATE_PROMPT = "aluminium mounting plate 80 x 40 x 8 mm with four 6 mm through holes"


def _plate_script() -> dict[str, object]:
    """A plate with four through holes: every cut removes material."""
    return {
        "script": {
            "summary": "80 x 40 x 8 mm plate with four 6 mm through holes",
            "components": [
                {
                    "name": "plate",
                    "steps": [
                        {
                            "op": "sketch",
                            "plane": "XY",
                            "profile": "rectangle",
                            "width": 80,
                            "height": 40,
                        },
                        {"op": "extrude", "length": 8},
                        {
                            "op": "sketch",
                            "plane": "XY",
                            "profile": "circle",
                            "diameter": 6,
                            "x": 8,
                            "y": 8,
                        },
                        {"op": "pocket"},
                    ],
                }
            ],
        }
    }


def _ineffective_cut_script() -> dict[str, object]:
    """A plate with one hole sketched entirely outside the plate.

    The build succeeds, but the cut removed no material, so the design is not
    verified.
    """
    return {
        "script": {
            "summary": "80 x 40 x 8 mm plate with one 6 mm hole outside the plate",
            "components": [
                {
                    "name": "plate",
                    "steps": [
                        {
                            "op": "sketch",
                            "plane": "XY",
                            "profile": "rectangle",
                            "width": 80,
                            "height": 40,
                        },
                        {"op": "extrude", "length": 8},
                        {
                            "op": "sketch",
                            "plane": "XY",
                            "profile": "circle",
                            "diameter": 6,
                            "x": 200,
                            "y": 200,
                        },
                        {"op": "pocket"},
                    ],
                }
            ],
        }
    }


def _chat_reply(script: dict[str, object]) -> str:
    return "Compiling the plate.\n\n```json\n" + json.dumps(script) + "\n```"


def test_an_unverified_build_is_repaired_with_the_build_feedback(
    client: TestClient, monkeypatch: pytest.MonkeyPatch
) -> None:
    prompts: list[str] = []
    replies = iter(
        [
            _chat_reply(_ineffective_cut_script()),
            _chat_reply(_plate_script()),
        ]
    )

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        prompts.append(prompt)
        return ChatOutcome(text=next(replies), provider=provider, model=model, offline=False)

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    response = client.post(
        "/api/studio/chat",
        json={
            "provider": "openrouter",
            "model": "minimax/minimax-m3.1-flash:free",
            "prompt": PLATE_PROMPT,
            "allow_remote": True,
            "cad_provider": "opencascade",
        },
    )
    assert response.status_code == 200, response.text
    body = response.json()
    # The second attempt is the turn's design, and it verifies.
    assert body["design"]["verified"] is True, body
    text = body["text"]
    assert "The first attempt was rejected" in text, text
    assert "cuts_effective" in text, text
    assert "removed no material" in text, text
    assert len(prompts) == 2, "the first call and the one build repair"
    assert "the build was rejected" in prompts[1], prompts[1]
    assert "cuts_effective" in prompts[1], prompts[1]
    assert "removed no material" in prompts[1], prompts[1]
    assert PLATE_PROMPT in prompts[1], prompts[1]
    # Every model call is stateless: the repair call must still see the
    # supported subset, the plane rule and the worked examples.
    assert "When no template matches, author a ConstructionScript" in prompts[1], prompts[1]
    assert "Never invent a dimension the user did not state" in prompts[1], prompts[1]
    assert "Worked examples" in prompts[1], prompts[1]


def test_a_spent_validation_repair_buys_no_build_repair(
    client: TestClient, monkeypatch: pytest.MonkeyPatch
) -> None:
    prompts: list[str] = []
    replies = iter(
        [
            "```json\n{not json}\n```",
            _chat_reply(_ineffective_cut_script()),
        ]
    )

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        prompts.append(prompt)
        return ChatOutcome(text=next(replies), provider=provider, model=model, offline=False)

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    response = client.post(
        "/api/studio/chat",
        json={
            "provider": "openrouter",
            "model": "minimax/minimax-m3.1-flash:free",
            "prompt": PLATE_PROMPT,
            "allow_remote": True,
            "cad_provider": "opencascade",
        },
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["design"]["verified"] is False, body
    text = body["text"]
    assert "B-Rep NOT verified" in text, text
    assert "The first attempt was rejected" not in text, (
        "a turn that spent its repair on the validation never repairs the build"
    )
    assert len(prompts) == 2, "the whole turn never spends more than one repair call"


def test_a_failed_repair_call_keeps_the_first_build(
    client: TestClient, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A repair CALL that fails must not throw the first build away.

    The first design was built and journalled already: a provider that times
    out on the repair call must not turn the whole turn into an error
    envelope. The turn keeps the first design and says why the repair did not
    happen.
    """
    prompts: list[str] = []
    replies = iter([_chat_reply(_ineffective_cut_script())])

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        prompts.append(prompt)
        if len(prompts) == 1:
            return ChatOutcome(text=next(replies), provider=provider, model=model, offline=False)
        raise ProviderTimeoutError("the provider request exceeded its bounded timeout")

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    response = client.post(
        "/api/studio/chat",
        json={
            "provider": "openrouter",
            "model": "minimax/minimax-m3.1-flash:free",
            "prompt": PLATE_PROMPT,
            "allow_remote": True,
            "cad_provider": "opencascade",
        },
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["design"] is not None, "the first build must not be thrown away"
    assert body["design"]["verified"] is False, body
    text = body["text"]
    assert "The first attempt was rejected" in text, text
    assert "cuts_effective" in text, text
    assert "removed no material" in text, text
    assert "the repair attempt failed: provider_timeout" in text, text
    assert "B-Rep NOT verified" in text, text
    assert "B-Rep verified" not in text, text
    assert len(prompts) == 2, "the first call and the one build repair"
    # The turn is journalled exactly once, and it points at the first design.
    turns = [event for event in _events(client) if event["event_type"] == "chat.turn"]
    assert len(turns) == 1, "the turn is journalled exactly once"
    assert turns[0]["payload"]["design_id"] == body["design"]["design_id"]


def test_a_failed_repair_call_after_a_refusal_says_no_part_was_built(
    client: TestClient, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A refused script and a repair call that times out never built a part.

    The turn must say so in the house wording instead of claiming a design it
    does not have.
    """
    prompts: list[str] = []
    helix = {
        "script": {
            "summary": "a shaft with a helix",
            "components": [
                {
                    "name": "shaft",
                    "steps": [
                        {"op": "sketch", "plane": "XY", "profile": "circle", "diameter": 10},
                        {"op": "extrude", "length": 20},
                        {"op": "helix", "diameter": 10, "pitch": 2, "length": 10},
                    ],
                }
            ],
        }
    }

    def fake_chat(
        provider: str,
        *,
        model: str,
        prompt: str,
        allow_remote: bool,
        environ: object,
        timeout: float | None = None,
    ) -> ChatOutcome:
        prompts.append(prompt)
        if len(prompts) == 1:
            return ChatOutcome(
                text=_chat_reply(helix), provider=provider, model=model, offline=False
            )
        raise ProviderTimeoutError("the provider request exceeded its bounded timeout")

    monkeypatch.setattr("kala_agent.studio.providers.dispatch.openai_compat_chat", fake_chat)
    response = client.post(
        "/api/studio/chat",
        json={
            "provider": "openrouter",
            "model": "minimax/minimax-m3.1-flash:free",
            "prompt": PLATE_PROMPT,
            "allow_remote": True,
            "cad_provider": "opencascade",
        },
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["design"] is None, body
    text = body["text"]
    assert "The first attempt was rejected" in text, text
    assert "the builder refused the script" in text, text
    assert "the repair attempt failed: provider_timeout" in text, text
    assert "No part was built" in text, text
    assert len(prompts) == 2, "the first call and the one build repair"
