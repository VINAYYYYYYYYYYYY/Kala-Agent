"""HTTP acceptance tests for the Studio design turns and journal routes."""

from __future__ import annotations

import hashlib
import json
import math
import sqlite3
from collections.abc import Iterator
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any, cast

import pytest
from fastapi.testclient import TestClient

from kala_agent.studio.providers.base import ChatOutcome
from kala_agent.studio.providers.dispatch import StudioChatTurn
from kala_agent.studio.providers.dispatch import chat as dispatch_chat
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


def test_remote_design_hook_parses_schema_json(monkeypatch: pytest.MonkeyPatch) -> None:
    prompts: list[str] = []

    def fake_chat(
        provider: str, *, model: str, prompt: str, allow_remote: bool, environ: object
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
            "I cannot do that.",
            _valid_reply(),
        ]
    )
    prompts: list[str] = []

    def fake_chat(
        provider: str, *, model: str, prompt: str, allow_remote: bool, environ: object
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


def test_remote_design_hook_fails_honestly_after_one_repair(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls = 0

    def fake_chat(
        provider: str, *, model: str, prompt: str, allow_remote: bool, environ: object
    ) -> ChatOutcome:
        nonlocal calls
        calls += 1
        return ChatOutcome(text="No JSON here.", provider=provider, model=model, offline=False)

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
    assert "No validated design was produced" in outcome.text
    assert outcome.design_error


def test_remote_non_design_prompt_is_left_alone(monkeypatch: pytest.MonkeyPatch) -> None:
    prompts: list[str] = []

    def fake_chat(
        provider: str, *, model: str, prompt: str, allow_remote: bool, environ: object
    ) -> ChatOutcome:
        prompts.append(prompt)
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
    assert outcome.design_intent is None
