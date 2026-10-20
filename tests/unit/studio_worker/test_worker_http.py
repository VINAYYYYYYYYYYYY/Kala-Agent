"""HTTP contract tests for the geometry worker's internal surface.

These tests run the ASGI app in-process with ``TestClient`` so the gate and
route matrix is cheap to exercise. The real subprocess readiness contract,
SIGTERM, and the stdin lifeline live in ``test_worker_process.py``.
"""

from __future__ import annotations

import json
import os
from collections.abc import Iterator
from pathlib import Path
from typing import Any, cast

import pytest
from fastapi.testclient import TestClient

from kala_agent.studio.internal_api import (
    ALLOWED_APPEND_EVENT_TYPES,
    BUILDER_PROVIDER,
    BUILDER_PROVIDER_MODE,
    INTERNAL_PREFIX,
    MAX_APPEND_PAYLOAD_BYTES,
    create_worker_app,
    router,
)

WORKER_PORT = 51_234
LAUNCH_TOKEN = "worker-test-token-0123456789abcdef"
AUTH = {"Authorization": f"Bearer {LAUNCH_TOKEN}"}
ADR_VOLUME_MM3 = 35_571.416047
STITCH_PROMPT = (
    "Design a machined aluminum L-bracket for a 40 mm mounting plate, 8 mm wall "
    "thickness, with a 10 mm centered bore and 4x M5 corner mounting holes."
)
EXPECTED_ROUTES = {
    f"{INTERNAL_PREFIX}/preview",
    f"{INTERNAL_PREFIX}/export",
    f"{INTERNAL_PREFIX}/handoff",
    f"{INTERNAL_PREFIX}/design",
    f"{INTERNAL_PREFIX}/design/{{design_id}}/drawing",
    f"{INTERNAL_PREFIX}/design/{{design_id}}/section",
    f"{INTERNAL_PREFIX}/import",
    f"{INTERNAL_PREFIX}/intent/parse",
    f"{INTERNAL_PREFIX}/intent/validate",
    f"{INTERNAL_PREFIX}/intent/schema",
    f"{INTERNAL_PREFIX}/journal",
    f"{INTERNAL_PREFIX}/journal/verify",
    f"{INTERNAL_PREFIX}/journal/append",
    f"{INTERNAL_PREFIX}/providers/cad",
}


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


# --- route contract ---


def test_router_exposes_exactly_the_documented_internal_routes() -> None:
    paths = {getattr(route, "path", "") for route in router.routes}
    assert paths == EXPECTED_ROUTES
    assert not any(path.startswith("/api/") for path in paths)


def test_worker_has_no_public_settings_or_chat_provider_routes(
    client: TestClient,
) -> None:
    """The worker mounts no ``/api/*`` route and no chat provider surface.

    The one provider route it owns is ``providers/cad``: the CAD provider
    probe the front door proxies. It is not a chat provider and it never
    leaves the machine.
    """
    for path in (
        "/api/studio/health",
        "/api/studio/settings",
        "/api/studio/providers",
        "/api/studio/chat",
        "/api/studio/providers/cad",
        f"{INTERNAL_PREFIX}/chat",
        f"{INTERNAL_PREFIX}/settings",
        f"{INTERNAL_PREFIX}/providers",
        f"{INTERNAL_PREFIX}/health",
    ):
        response = (
            client.post(path, json={}, headers=AUTH)
            if "chat" in path
            else client.get(path, headers=AUTH)
        )
        assert response.status_code in (404, 405), (path, response.status_code)


def test_builder_routes_are_labelled_as_the_opt_in_opencascade_provider() -> None:
    routes = {getattr(route, "path", ""): route for route in router.routes}
    for path in (
        f"{INTERNAL_PREFIX}/design",
        f"{INTERNAL_PREFIX}/design/{{design_id}}/drawing",
        f"{INTERNAL_PREFIX}/design/{{design_id}}/section",
    ):
        route = routes[path]
        description = getattr(route, "description", "") or ""
        assert BUILDER_PROVIDER in description, path
        assert BUILDER_PROVIDER_MODE in description, path
        assert getattr(route, "openapi_extra", None) == {
            "x-kala-provider": BUILDER_PROVIDER,
            "x-kala-provider-mode": BUILDER_PROVIDER_MODE,
        }, path

    imported = routes[f"{INTERNAL_PREFIX}/import"]
    assert getattr(imported, "openapi_extra", None) == {
        "x-kala-provider": BUILDER_PROVIDER,
        "x-kala-role": "view-only",
    }


# --- gates ---


def test_missing_bearer_is_401(client: TestClient) -> None:
    response = client.get(f"{INTERNAL_PREFIX}/intent/schema")
    assert response.status_code == 401
    assert response.json()["code"] == "unauthorized"
    assert response.headers["www-authenticate"].startswith("Bearer ")


def test_wrong_bearer_is_401(client: TestClient) -> None:
    response = client.get(
        f"{INTERNAL_PREFIX}/intent/schema",
        headers={"Authorization": "Bearer wrong-token-0123456789abcdef"},
    )
    assert response.status_code == 401
    assert response.json()["code"] == "unauthorized"


def test_bearer_is_required_on_get_routes_too(client: TestClient) -> None:
    for path in (
        f"{INTERNAL_PREFIX}/intent/schema",
        f"{INTERNAL_PREFIX}/journal",
        f"{INTERNAL_PREFIX}/journal/verify",
    ):
        response = client.get(path)
        assert response.status_code == 401, path


def test_non_exact_host_is_421(client: TestClient) -> None:
    for host in (f"localhost:{WORKER_PORT}", "127.0.0.1", f"127.0.0.1:{WORKER_PORT + 1}"):
        response = client.get(
            f"{INTERNAL_PREFIX}/intent/schema",
            headers={**AUTH, "host": host},
        )
        assert response.status_code == 421, host
        assert response.json()["code"] == "invalid_host"


def test_any_origin_header_is_403(client: TestClient) -> None:
    for origin in ("http://127.0.0.1:51234", "https://evil.example", "null"):
        response = client.get(
            f"{INTERNAL_PREFIX}/intent/schema",
            headers={**AUTH, "origin": origin},
        )
        assert response.status_code == 403, origin
        assert response.json()["code"] == "cross_origin_denied"


def test_oversized_body_is_413(client: TestClient) -> None:
    response = client.post(
        f"{INTERNAL_PREFIX}/journal/append",
        content=b"x" * (256 * 1024 + 1),
        headers={**AUTH, "content-type": "application/json"},
    )
    assert response.status_code == 413
    assert response.json()["code"] == "payload_too_large"


# --- intent ---


def test_intent_parse_stitch_sentence_yields_a_complete_intent(client: TestClient) -> None:
    response = client.post(
        f"{INTERNAL_PREFIX}/intent/parse",
        json={"prompt": STITCH_PROMPT},
        headers=AUTH,
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["family"] == "l_bracket"
    assert body["missing"] == []
    intent = body["intent"]
    assert intent["template"]["template"] == "l_bracket"
    assert intent["template"]["width"] == 60.0
    assert intent["template"]["depth"] == 45.0
    assert intent["template"]["height"] == 40.0
    assert intent["template"]["wall"] == 8.0
    assert intent["template"]["bore_d"] == 10.0
    assert intent["template"]["hole_d"] == 5.5
    assert intent["template"]["hole_count"] == 4
    assert intent["material"] == "aluminium"


def test_intent_parse_incomplete_prompt_names_missing_values(client: TestClient) -> None:
    response = client.post(
        f"{INTERNAL_PREFIX}/intent/parse",
        json={"prompt": "make me a bracket"},
        headers=AUTH,
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["family"] == "l_bracket"
    assert body["intent"] is None
    assert body["missing"] == [
        "width (mm)",
        "depth (mm)",
        "height (mm)",
        "wall thickness (mm)",
    ]


def test_intent_parse_unrecognised_prompt_has_no_family(client: TestClient) -> None:
    response = client.post(
        f"{INTERNAL_PREFIX}/intent/parse",
        json={"prompt": "what is the weather"},
        headers=AUTH,
    )
    assert response.status_code == 200
    assert response.json() == {
        "family": None,
        "intent": None,
        "missing": [],
        "notes": [],
    }


def test_intent_validate_accepts_a_fenced_reply(client: TestClient) -> None:
    reply = "Here is the part.\n```json\n" + json.dumps(_lbracket_intent()) + "\n```\n"
    response = client.post(
        f"{INTERNAL_PREFIX}/intent/validate",
        json={"text": reply},
        headers=AUTH,
    )
    assert response.status_code == 200, response.text
    intent = response.json()["intent"]
    assert intent["template"]["template"] == "l_bracket"
    assert intent["template"]["width"] == 60.0
    assert intent["material"] == "aluminium"


def test_intent_validate_refuses_text_without_json(client: TestClient) -> None:
    response = client.post(
        f"{INTERNAL_PREFIX}/intent/validate",
        json={"text": "I need more information before I can design this."},
        headers=AUTH,
    )
    assert response.status_code == 422
    assert response.json()["code"] == "invalid_intent"


def test_intent_validate_refuses_an_invalid_intent(client: TestClient) -> None:
    broken = {"template": {"template": "l_bracket", "depth": 45, "height": 40, "wall": 8}}
    response = client.post(
        f"{INTERNAL_PREFIX}/intent/validate",
        json={"text": "```json\n" + json.dumps(broken) + "\n```"},
        headers=AUTH,
    )
    assert response.status_code == 422
    body = response.json()
    assert body["code"] == "invalid_intent"
    assert any("width" in detail for detail in body["details"])


def test_intent_schema_is_the_design_intent_json_schema(client: TestClient) -> None:
    response = client.get(f"{INTERNAL_PREFIX}/intent/schema", headers=AUTH)
    assert response.status_code == 200
    assert response.headers["content-type"].startswith("text/plain")
    schema = cast("dict[str, Any]", json.loads(response.text))
    assert "properties" in schema
    assert "DesignIntent" in schema.get("title", "")


# --- journal ---


def test_journal_append_accepts_only_the_allow_list(client: TestClient) -> None:
    sequence = 0
    for event_type in sorted(ALLOWED_APPEND_EVENT_TYPES):
        response = client.post(
            f"{INTERNAL_PREFIX}/journal/append",
            json={"event_type": event_type, "payload": {"source": "rust"}},
            headers=AUTH,
        )
        assert response.status_code == 201, (event_type, response.text)
        body = response.json()
        assert body["event_type"] == event_type
        assert body["payload"] == {"source": "rust"}
        assert body["sequence"] > sequence
        sequence = body["sequence"]

    for rejected in ("design.compiled", "design.intent", "artifact.stored", "chat.message"):
        response = client.post(
            f"{INTERNAL_PREFIX}/journal/append",
            json={"event_type": rejected, "payload": {}},
            headers=AUTH,
        )
        assert response.status_code == 422, rejected
        assert response.json()["code"] == "event_type_not_allowed"


def test_journal_append_requires_an_object_payload(client: TestClient) -> None:
    response = client.post(
        f"{INTERNAL_PREFIX}/journal/append",
        json={"event_type": "chat.turn", "payload": [1, 2, 3]},
        headers=AUTH,
    )
    assert response.status_code == 422


def test_journal_append_refuses_an_oversized_payload(client: TestClient) -> None:
    response = client.post(
        f"{INTERNAL_PREFIX}/journal/append",
        json={
            "event_type": "chat.turn",
            "payload": {"blob": "x" * (MAX_APPEND_PAYLOAD_BYTES + 1)},
        },
        headers=AUTH,
    )
    assert response.status_code == 422
    assert response.json()["code"] == "invalid_request"


def test_journal_append_extends_the_verified_chain(client: TestClient) -> None:
    response = client.post(
        f"{INTERNAL_PREFIX}/journal/append",
        json={
            "event_type": "settings.changed",
            "payload": {"setting": "provider", "changed": True},
            "aggregate_id": "rust-control-plane",
        },
        headers=AUTH,
    )
    assert response.status_code == 201
    body = response.json()
    assert body["aggregate_id"] == "rust-control-plane"

    page = client.get(
        f"{INTERNAL_PREFIX}/journal",
        params={"after": 0, "limit": 10},
        headers=AUTH,
    )
    assert page.status_code == 200
    assert page.json()["events"][-1]["event_type"] == "settings.changed"

    verification = client.get(f"{INTERNAL_PREFIX}/journal/verify", headers=AUTH)
    assert verification.status_code == 200
    assert verification.json() == {
        "ok": True,
        "event_count": 1,
        "head_digest": body["event_digest"],
        "first_bad_sequence": None,
    }


# --- design ---


def test_design_route_measures_the_adr_lbracket(client: TestClient) -> None:
    response = client.post(
        f"{INTERNAL_PREFIX}/design",
        json=_lbracket_body(),
        headers=AUTH,
    )
    assert response.status_code == 200, response.text
    body = response.json()
    volume = body["measured"]["volume_mm3"]
    assert abs(volume - ADR_VOLUME_MM3) / ADR_VOLUME_MM3 < 1e-6
    assert body["measured"]["bounds"] == {
        "min": [-30.0, 0.0, 0.0],
        "max": [30.0, 45.0, 40.0],
    }
    assert body["verified"] is True


def test_design_drawing_and_section_routes_reuse_the_service(client: TestClient) -> None:
    design = client.post(
        f"{INTERNAL_PREFIX}/design",
        json=_lbracket_body(),
        headers=AUTH,
    )
    assert design.status_code == 200
    design_id = design.json()["design_id"]

    drawing = client.get(
        f"{INTERNAL_PREFIX}/design/{design_id}/drawing",
        params={"views": "front"},
        headers=AUTH,
    )
    assert drawing.status_code == 200
    assert [view["name"] for view in drawing.json()["views"]] == ["front"]

    section = client.get(
        f"{INTERNAL_PREFIX}/design/{design_id}/section",
        params={"plane": "XZ", "offset": 4},
        headers=AUTH,
    )
    assert section.status_code == 200
    body = section.json()
    assert body["outer_loop_count"] == 1
    assert body["inner_loop_count"] == 1
    assert abs(body["area_mm2"] - 2321.4601836602546) < 1e-6


def test_design_route_rejects_an_invalid_intent(client: TestClient) -> None:
    response = client.post(
        f"{INTERNAL_PREFIX}/design",
        json={
            "template": {"template": "l_bracket", "width": 60},
            "provider": "opencascade",
        },
        headers=AUTH,
    )
    assert response.status_code == 422
    assert response.json()["code"] == "invalid_request"


# --- CAD providers (ADR-0011) ---


def test_the_cad_route_reuses_the_public_probe_handler() -> None:
    """One implementation: the worker's route is the public server's handler."""
    from fastapi.routing import APIRoute

    from kala_agent.studio import server

    routes = {getattr(route, "path", ""): route for route in router.routes}
    route = routes[f"{INTERNAL_PREFIX}/providers/cad"]
    assert isinstance(route, APIRoute)
    assert route.endpoint is server.cad_providers


def test_cad_providers_route_reports_the_live_probe(client: TestClient) -> None:
    response = client.get(f"{INTERNAL_PREFIX}/providers/cad", headers=AUTH)
    assert response.status_code == 200, response.text
    body = cast("dict[str, Any]", response.json())
    rows = cast("list[dict[str, Any]]", body["providers"])
    assert [row["id"] for row in rows] == ["opencascade", "freecad", "solidedge"]
    for row in rows:
        assert set(row) == {
            "id",
            "name",
            "available",
            "version",
            "reason",
            "measured_by",
            "mode",
        }
        assert isinstance(row["available"], bool)
        assert row["mode"] in {"opt-in", "external", "bridge"}
    # Availability is evidence, not a constant: an absent provider reports a
    # reason instead of a silent "true".
    for row in rows:
        if not row["available"]:
            assert row["reason"], row


def test_cad_providers_route_requires_the_worker_bearer(client: TestClient) -> None:
    missing = client.get(f"{INTERNAL_PREFIX}/providers/cad")
    assert missing.status_code == 401
    assert missing.json()["code"] == "unauthorized"
    wrong = client.get(
        f"{INTERNAL_PREFIX}/providers/cad",
        headers={"Authorization": "Bearer wrong-token-0123456789abcdef"},
    )
    assert wrong.status_code == 401


def test_cad_providers_route_is_behind_the_host_and_origin_guards(
    client: TestClient,
) -> None:
    bad_host = client.get(
        f"{INTERNAL_PREFIX}/providers/cad",
        headers={**AUTH, "host": f"localhost:{WORKER_PORT}"},
    )
    assert bad_host.status_code == 421
    assert bad_host.json()["code"] == "invalid_host"
    cross_origin = client.get(
        f"{INTERNAL_PREFIX}/providers/cad",
        headers={**AUTH, "origin": "https://evil.example"},
    )
    assert cross_origin.status_code == 403
    assert cross_origin.json()["code"] == "cross_origin_denied"


def test_cad_providers_route_matches_the_public_server(data_dir: Path) -> None:
    """The worker and the public server report the same availability facts.

    Both applications are built with the same environment view, so the two
    provider registries are the same registry and the two documents must be
    equal. This is the parity the Rust front door's differential needs.
    """
    from kala_agent.studio.server import create_studio_app

    environ = dict(os.environ)
    worker = create_worker_app(
        api_token=LAUNCH_TOKEN,
        port=WORKER_PORT,
        data_dir=data_dir / "worker",
        environ=environ,
    )
    public = create_studio_app(
        data_dir=data_dir / "public",
        api_token=LAUNCH_TOKEN,
        environ=environ,
    )
    with (
        TestClient(
            worker,
            base_url=f"http://127.0.0.1:{WORKER_PORT}",
            raise_server_exceptions=False,
        ) as worker_client,
        TestClient(
            public,
            base_url=f"http://127.0.0.1:{WORKER_PORT}",
            raise_server_exceptions=False,
        ) as public_client,
    ):
        worker_response = worker_client.get(f"{INTERNAL_PREFIX}/providers/cad", headers=AUTH)
        public_response = public_client.get("/api/studio/providers/cad", headers=AUTH)
    assert worker_response.status_code == 200, worker_response.text
    assert public_response.status_code == 200, public_response.text
    assert worker_response.json() == public_response.json()
