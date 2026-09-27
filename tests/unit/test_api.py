"""Unit tests for the local Kala HTTP adapter.

Every test runs offline: no socket is opened, the model gateway is the
deterministic local client, and the tool plane is never dispatched. Every test
client states the two facts the boundary judges: the Host it presents is
loopback, and the mutating calls carry the launch token. No test bypasses a
control to reach a route.
"""

from __future__ import annotations

from collections.abc import Iterator
from pathlib import Path

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient

from kala_agent.api import create_app
from kala_agent.api.middleware import (
    MAX_REQUEST_BODY_BYTES,
    SECURITY_HEADERS,
    validate_max_body_bytes,
)
from kala_agent.application import KalaApplication
from kala_agent.config import Settings
from kala_agent.runtime.model_gateway import ModelGateway, OpenAICompatibleClient

API = "/api/v1"
SECRET = "sk-this-value-must-never-be-echoed-1234567890"
LOOPBACK_BASE_URL = "http://127.0.0.1"
LAUNCH_TOKEN = "unit-test-launch-token-0123456789abcdef"
AUTH = {"Authorization": f"Bearer {LAUNCH_TOKEN}"}


@pytest.fixture(autouse=True)
def _forbid_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    def blocked(*args: object, **kwargs: object) -> None:
        raise AssertionError("the local API must not open network connections")

    monkeypatch.setattr("socket.socket.connect", blocked)
    monkeypatch.setattr("socket.socket.connect_ex", blocked)
    monkeypatch.setattr("socket.create_connection", blocked)
    yield


def _settings(tmp_path: Path) -> Settings:
    return Settings(data_dir=tmp_path / "data")


def _client(app: FastAPI, *, token: str | None = LAUNCH_TOKEN) -> TestClient:
    """Return a test client that presents a loopback Host and the launch token."""
    return TestClient(
        app,
        base_url=LOOPBACK_BASE_URL,
        headers=AUTH if token is None else {"Authorization": f"Bearer {token}"},
        raise_server_exceptions=False,
    )


@pytest.fixture
def application(tmp_path: Path) -> Iterator[KalaApplication]:
    resolved = KalaApplication(_settings(tmp_path))
    try:
        yield resolved
    finally:
        resolved.close()


@pytest.fixture
def client(application: KalaApplication) -> Iterator[TestClient]:
    with _client(create_app(application, api_token=LAUNCH_TOKEN, max_body_bytes=4096)) as resolved:
        yield resolved


def test_health_reports_local_facts_and_security_headers(
    client: TestClient,
    application: KalaApplication,
) -> None:
    response = client.get("/health")
    assert response.status_code == 200
    body = response.json()
    assert body["status"] == "ok"
    assert body["provider"] == "local"
    assert body["offline"] is True
    assert body["egress_configured"] is False
    assert body["schema_version"] == application.database.schema_version
    assert body["data_dir"] == str(application.settings.data_dir)
    assert body["domains"] == ["building", "mechanical", "wing"]
    for header, value in SECURITY_HEADERS.items():
        assert response.headers[header] == value
    assert "x-powered-by" not in response.headers
    assert "set-cookie" not in response.headers
    assert LAUNCH_TOKEN not in response.text


def test_health_never_claims_a_provider_path_that_is_not_wired(tmp_path: Path) -> None:
    settings = Settings(data_dir=tmp_path / "data", model_endpoint="http://127.0.0.1:11434/v1")
    resolved = KalaApplication(settings)
    try:
        with _client(create_app(resolved)) as test_client:
            response = test_client.get("/health")
        assert response.status_code == 200
        body = response.json()
        assert body["egress_configured"] is False
        assert body["offline"] is True
        assert body["provider_endpoint_declared"] is True
        assert "11434" not in response.text
    finally:
        resolved.close()


def test_health_reports_egress_only_for_a_supplied_gateway(tmp_path: Path) -> None:
    gateway = ModelGateway(
        OpenAICompatibleClient(
            endpoint="http://127.0.0.1:11434/v1",
            model_id="hosted-model",
            allowed_endpoints=("http://127.0.0.1:11434/v1",),
            consent=True,
        )
    )
    settings = Settings(data_dir=tmp_path / "data")
    resolved = KalaApplication(settings, model_gateway=gateway)
    try:
        assert resolved.egress_configured is True
        assert resolved.offline is False
        assert resolved.provider_endpoint_declared is False
        assert resolved.model_gateway_supplied is True
        with _client(create_app(resolved)) as test_client:
            response = test_client.get("/health")
        assert response.status_code == 200
        body = response.json()
        assert body["egress_configured"] is True
        assert body["offline"] is False
        assert body["provider_endpoint_declared"] is False
        assert body["provider"] == "openai-compatible"
        assert "11434" not in response.text
    finally:
        resolved.close()
    local = KalaApplication(Settings(data_dir=tmp_path / "other"))
    try:
        assert local.egress_configured is False
        assert local.offline is True
        assert local.model_gateway_supplied is False
    finally:
        local.close()


def test_domains_are_listed_with_digests_and_workflows(client: TestClient) -> None:
    response = client.get(f"{API}/domains")
    assert response.status_code == 200
    body = response.json()
    assert body["count"] == 3
    assert [package["domain"] for package in body["packages"]] == [
        "building",
        "mechanical",
        "wing",
    ]
    assert all(len(package["digest"]) == 64 for package in body["packages"])
    assert {workflow["domain"] for workflow in body["workflows"]} == {
        "building",
        "mechanical",
        "wing",
    }
    detail = client.get(f"{API}/domains/mechanical")
    assert detail.status_code == 200
    assert detail.json()["workflow"]["definition_id"] == "mechanical.bracket-evaluation"
    assert len(detail.json()["catalog_steps"]) == 4
    checkpoint_steps = [
        step for step in detail.json()["catalog_steps"] if step["checkpoint_kind"] is not None
    ]
    assert [step["step_id"] for step in checkpoint_steps] == ["mechanical.constraint-evaluation"]


def test_unknown_domain_is_reported_as_not_found(client: TestClient) -> None:
    for path in (f"{API}/domains/nope", f"{API}/domains/nope/evaluations"):
        response = client.post(path) if path.endswith("evaluations") else client.get(path)
        assert response.status_code in (404, 405)
    response = client.get(f"{API}/domains/nope")
    assert response.status_code == 404
    assert response.json()["error"]["code"] == "unknown_domain"
    assert "Traceback" not in response.text


def test_domain_evaluation_is_deterministic_and_bounded(client: TestClient) -> None:
    first = client.post(f"{API}/domains/mechanical/evaluations", json={})
    second = client.post(f"{API}/domains/mechanical/evaluations", json={})
    assert first.status_code == 200
    assert first.json() == second.json()
    report = first.json()["evaluation"]
    assert report["passed"] is True
    assert report["status"] == "completed"
    assert len(report["evaluations"]) == 4
    overridden = client.post(
        f"{API}/domains/wing/evaluations",
        json={"parameters": {"span_m": 14.0}, "design_id": "wing-14"},
    )
    assert overridden.status_code == 200
    assert overridden.json()["evaluation"]["subject_id"] == "wing-14"
    assert (
        overridden.json()["evaluation"]["parameters_digest"]
        != first.json()["evaluation"]["parameters_digest"]
    )


def test_invalid_evaluation_input_is_unprocessable(client: TestClient) -> None:
    out_of_range = client.post(
        f"{API}/domains/mechanical/evaluations",
        json={"parameters": {"load_kn": 100_000}},
    )
    assert out_of_range.status_code == 422
    assert out_of_range.json()["error"]["code"] == "invalid_parameters"
    unknown_parameter = client.post(
        f"{API}/domains/mechanical/evaluations",
        json={"parameters": {"not_a_parameter": 1}},
    )
    assert unknown_parameter.status_code == 422
    extra_field = client.post(
        f"{API}/domains/mechanical/evaluations",
        json={"unexpected": True},
    )
    assert extra_field.status_code == 422
    assert extra_field.json()["error"]["code"] == "invalid_request"
    non_finite = client.post(
        f"{API}/domains/mechanical/evaluations",
        json={"parameters": {"load_kn": "not-a-number"}},
    )
    assert non_finite.status_code == 422
    bad_identifier = client.post(
        f"{API}/domains/mechanical/evaluations",
        json={"design_id": "../escape"},
    )
    assert bad_identifier.status_code == 422
    assert SECRET not in non_finite.text


def test_revision_lifecycle_returns_location_and_provenance(client: TestClient) -> None:
    created = client.post(
        f"{API}/revisions",
        json={"design_id": "bracket-http", "domain": "mechanical"},
    )
    assert created.status_code == 201
    revision = created.json()["revision"]
    assert created.headers["location"] == f"{API}/revisions/{revision['revision_id']}"
    assert revision["sequence"] == 0
    assert revision["parent_revision_id"] is None
    assert created.json()["provenance"]["provenance_id"] == "prov-bracket-http-r0"
    assert created.json()["provenance"]["transformation_ids"] == ["reference-workflow-evaluation"]
    fetched = client.get(created.headers["location"])
    assert fetched.status_code == 200
    assert fetched.json() == created.json()
    second = client.post(
        f"{API}/revisions",
        json={"design_id": "bracket-http", "domain": "mechanical"},
    )
    assert second.status_code == 201
    assert second.json()["revision"]["sequence"] == 1
    assert second.json()["revision"]["parent_revision_id"] == revision["revision_id"]
    listed = client.get(f"{API}/revisions", params={"design_id": "bracket-http"})
    assert listed.status_code == 200
    assert [item["revision"]["revision_id"] for item in listed.json()["revisions"]] == [
        revision["revision_id"],
        second.json()["revision"]["revision_id"],
    ]
    assert (
        listed.json()["revisions"][0]["artifact_digests"]
        == listed.json()["revisions"][1]["artifact_digests"]
    )
    empty = client.get(f"{API}/revisions", params={"design_id": "never-created"})
    assert empty.status_code == 200
    assert empty.json() == {"design_id": "never-created", "count": 0, "revisions": []}


def test_missing_revision_records_are_not_found(client: TestClient) -> None:
    assert client.get(f"{API}/revisions/rev-absent-r0").status_code == 404
    assert client.get(f"{API}/revisions/rev-absent-r0").json()["error"]["code"] == (
        "unknown_revision"
    )
    assert client.get(f"{API}/revisions", params={"design_id": "x"}).status_code == 200
    assert client.get(f"{API}/revisions").status_code == 422
    orphan = client.post(
        f"{API}/revisions",
        json={
            "design_id": "orphan",
            "domain": "mechanical",
            "parent_revision_id": "rev-orphan-r9",
        },
    )
    assert orphan.status_code == 404
    assert orphan.json()["error"]["code"] == "unknown_revision"
    absent = client.get(f"{API}/revision-commits/commit-absent-r0")
    assert absent.status_code == 404
    assert absent.json()["error"]["code"] == "unknown_prepared_revision"
    unknown = client.post(f"{API}/revision-commits/commit-absent-r0/approve", json={})
    assert unknown.status_code == 404
    premature_digest = client.post(
        f"{API}/revision-commits",
        json={"design_id": "premature", "domain": "mechanical", "state_digest": "0" * 64},
    )
    assert premature_digest.status_code == 422
    assert premature_digest.json()["error"]["code"] == "invalid_request"


def test_a_prepared_commit_writes_nothing_until_it_is_approved(
    client: TestClient,
    application: KalaApplication,
) -> None:
    prepared = client.post(
        f"{API}/revision-commits",
        json={"design_id": "two-phase", "domain": "mechanical"},
    )
    assert prepared.status_code == 201
    commit = prepared.json()
    assert prepared.headers["location"] == f"{API}/revision-commits/{commit['request_id']}"
    assert commit["status"] == "pending"
    assert commit["committed"] is False
    assert commit["revision_id"] == "rev-two-phase-r0"
    assert commit["parent_revision_id"] is None
    assert len(commit["state_digest"]) == 64
    assert len(commit["action_digest"]) == 64
    assert commit["state_digest"] != commit["action_digest"]
    assert application.list_design_revisions("two-phase") == ()

    read_back = client.get(prepared.headers["location"])
    assert read_back.status_code == 200
    assert read_back.json() == commit

    approved = client.post(
        f"{API}/revision-commits/{commit['request_id']}/approve",
        json={
            "actor_id": "two-phase-operator",
            "state_digest": commit["state_digest"],
            "action_digest": commit["action_digest"],
        },
    )
    assert approved.status_code == 200
    assert approved.json()["revision"]["revision_id"] == "rev-two-phase-r0"
    decided = client.get(prepared.headers["location"]).json()
    assert decided["status"] == "approved"
    assert decided["committed"] is True
    assert decided["actor_id"] == "two-phase-operator"
    assert decided["decided_at"] is not None
    assert len(application.list_design_revisions("two-phase")) == 1

    replay = client.post(f"{API}/revision-commits/{commit['request_id']}/approve", json={})
    assert replay.status_code == 409
    assert replay.json()["error"]["code"] == "checkpoint_not_pending"
    assert len(application.list_design_revisions("two-phase")) == 1


def test_a_rejected_prepared_commit_writes_nothing(
    client: TestClient,
    application: KalaApplication,
) -> None:
    commit = client.post(
        f"{API}/revision-commits",
        json={"design_id": "rejected-commit", "domain": "mechanical"},
    ).json()
    refused = client.post(
        f"{API}/revision-commits/{commit['request_id']}/reject",
        json={"actor_id": "reviewer", "reason": "not the configuration under review"},
    )
    assert refused.status_code == 200
    body = refused.json()
    assert body["status"] == "rejected"
    assert body["committed"] is False
    assert body["actor_id"] == "reviewer"
    assert body["reason"] == "not the configuration under review"
    assert application.list_design_revisions("rejected-commit") == ()
    assert client.get(f"{API}/revisions/rev-rejected-commit-r0").status_code == 404
    assert (
        client.post(f"{API}/revision-commits/{commit['request_id']}/approve", json={}).status_code
        == 409
    )
    assert application.list_design_revisions("rejected-commit") == ()


def test_a_stale_prepared_commit_is_refused_without_writing(
    client: TestClient,
    application: KalaApplication,
) -> None:
    prepared = client.post(
        f"{API}/revision-commits",
        json={
            "design_id": "stale-commit",
            "domain": "mechanical",
            "summary": "the configuration under review",
        },
    ).json()
    moved = client.post(
        f"{API}/revisions",
        json={
            "design_id": "stale-commit",
            "domain": "mechanical",
            "summary": "a different configuration",
        },
    )
    assert moved.status_code == 201
    assert moved.json()["revision"]["revision_id"] == "rev-stale-commit-r0"
    assert application.get_revision_commit(prepared["request_id"]).status.value == "pending"

    stale = client.post(f"{API}/revision-commits/{prepared['request_id']}/approve", json={})
    assert stale.status_code == 409
    assert stale.json()["error"]["code"] == "checkpoint_digest_mismatch"
    assert [
        record.revision.revision_id for record in application.list_design_revisions("stale-commit")
    ] == ["rev-stale-commit-r0"]

    wrong_digest = client.post(
        f"{API}/revisions",
        json={
            "design_id": "wrong-digest",
            "domain": "mechanical",
            "state_digest": "0" * 64,
        },
    )
    assert wrong_digest.status_code == 409
    assert wrong_digest.json()["error"]["code"] == "checkpoint_digest_mismatch"
    assert application.list_design_revisions("wrong-digest") == ()


def test_a_commit_must_extend_the_current_head(client: TestClient) -> None:
    first = client.post(f"{API}/revisions", json={"design_id": "linear", "domain": "mechanical"})
    second = client.post(f"{API}/revisions", json={"design_id": "linear", "domain": "mechanical"})
    assert first.json()["revision"]["revision_id"] == "rev-linear-r0"
    assert second.json()["revision"]["revision_id"] == "rev-linear-r1"
    forked = client.post(
        f"{API}/revisions",
        json={
            "design_id": "linear",
            "domain": "mechanical",
            "parent_revision_id": "rev-linear-r0",
        },
    )
    assert forked.status_code == 409
    assert forked.json()["error"]["code"] == "revision_fork_refused"
    assert client.get(f"{API}/revisions", params={"design_id": "linear"}).json()["count"] == 2


def test_mechanical_run_pauses_at_the_policy_checkpoint(client: TestClient) -> None:
    started = client.post(
        f"{API}/workflows/mechanical/reference",
        json={"design_id": "bracket-run"},
    )
    assert started.status_code == 201
    assert started.headers["location"] == f"{API}/workflows/workflow-bracket-run"
    run = started.json()["run"]
    assert run["started"] is True
    assert run["status"] == "waiting_for_approval"
    assert run["policy_status"] == "pending"
    assert run["policy_reason"] == "approval_required"
    assert run["checkpoint"]["checkpoint_type"] == "pre_commit"
    assert run["checkpoint"]["step_id"] == "mechanical.constraint-evaluation"
    assert run["revision"] is None
    assert [evaluation["passed"] for evaluation in run["evaluations"]] == [True] * 4
    resumed = client.post(
        f"{API}/workflows/mechanical/reference",
        json={"design_id": "bracket-run"},
    )
    assert resumed.status_code == 200
    assert resumed.json()["run"]["started"] is False
    assert resumed.json()["run"]["workflow_id"] == run["workflow_id"]
    state = client.get(f"{API}/workflows/workflow-bracket-run")
    assert state.status_code == 200
    assert state.json()["status"] == "waiting_for_approval"
    assert state.json()["current_state"] == "load-cased"
    assert len(state.json()["completed_step_ids"]) == 3
    checkpoints = client.get(f"{API}/workflows/workflow-bracket-run/checkpoints")
    assert checkpoints.status_code == 200
    assert checkpoints.json()["count"] == 1
    assert checkpoints.json()["approvals"][0]["status"] == "pending"
    assert checkpoints.json()["pending"] is not None


def test_approving_the_checkpoint_commits_the_revision(client: TestClient) -> None:
    started = client.post(
        f"{API}/workflows/mechanical/reference",
        json={"design_id": "bracket-approve"},
    )
    assert started.status_code == 201
    workflow_id = started.json()["run"]["workflow_id"]
    decided = client.post(f"{API}/workflows/{workflow_id}/checkpoints/approve", json={})
    assert decided.status_code == 200
    run = decided.json()["run"]
    assert run["status"] == "completed"
    assert run["state"]["current_state"] == "assessed"
    assert run["checkpoint"]["status"] == "approved"
    assert run["revision"]["revision"]["sequence"] == 0
    assert run["revision"]["artifact_digests"] == run["revision"]["artifact_digests"]
    again = client.post(f"{API}/workflows/{workflow_id}/checkpoints/approve", json={})
    assert again.status_code == 409
    assert again.json()["error"]["code"] == "workflow_finished"
    restart = client.post(
        f"{API}/workflows/mechanical/reference",
        json={"design_id": "bracket-approve"},
    )
    assert restart.status_code == 409


def test_rejecting_the_checkpoint_ends_the_workflow(client: TestClient) -> None:
    started = client.post(
        f"{API}/workflows/mechanical/reference",
        json={"design_id": "bracket-reject"},
    )
    workflow_id = started.json()["run"]["workflow_id"]
    decided = client.post(
        f"{API}/workflows/{workflow_id}/checkpoints/reject",
        json={"reason": "operator declined the pre-commit checkpoint"},
    )
    assert decided.status_code == 200
    assert decided.json()["run"]["status"] == "failed"
    assert decided.json()["run"]["revision"] is None
    assert decided.json()["run"]["state"]["last_error"] == (
        "operator declined the pre-commit checkpoint"
    )
    assert decided.json()["run"]["checkpoint"]["status"] == "rejected"
    replay = client.post(f"{API}/workflows/{workflow_id}/checkpoints/reject", json={})
    assert replay.status_code == 409


def test_checkpoint_decision_conflicts_are_reported(client: TestClient) -> None:
    started = client.post(
        f"{API}/workflows/mechanical/reference",
        json={"design_id": "bracket-conflict"},
    )
    workflow_id = started.json()["run"]["workflow_id"]
    stale = client.post(
        f"{API}/workflows/{workflow_id}/checkpoints/approve",
        json={"state_digest": "0" * 64},
    )
    assert stale.status_code == 409
    assert stale.json()["error"]["code"] == "checkpoint_digest_mismatch"
    malformed = client.post(
        f"{API}/workflows/{workflow_id}/checkpoints/approve",
        json={"state_digest": "not-a-digest"},
    )
    assert malformed.status_code == 422
    unknown = client.post(f"{API}/workflows/workflow-absent/checkpoints/approve", json={})
    assert unknown.status_code == 404
    assert unknown.json()["error"]["code"] == "unknown_workflow"
    unroutable = client.post(f"{API}/workflows/..%2Fescape/checkpoints/approve", json={})
    assert unroutable.status_code in (404, 405)
    still_pending = client.get(f"{API}/workflows/{workflow_id}").json()
    assert still_pending["status"] == "waiting_for_approval"


def test_events_are_paged_filtered_and_verifiable(client: TestClient) -> None:
    evaluated = client.post(f"{API}/domains/mechanical/evaluations", json={})
    assert evaluated.status_code == 200
    for index in range(2):
        created = client.post(
            f"{API}/revisions",
            json={"design_id": f"evented-{index}", "domain": "mechanical"},
        )
        assert created.status_code == 201
    page = client.get(f"{API}/events", params={"limit": 3})
    assert page.status_code == 200
    body = page.json()
    assert body["count"] == 3
    assert body["after_sequence"] == 0
    assert body["payload_truncated"] is False
    assert all(len(event["digest"]) == 64 for event in body["events"])
    assert [event["sequence"] for event in body["events"]] == [1, 2, 3]
    assert [event["event_type"] for event in body["events"]] == [
        "domain.evaluated",
        "revision.prepared",
        "revision.commit_decided",
    ]
    resumed = client.get(
        f"{API}/events",
        params={"after_sequence": body["next_after_sequence"], "limit": 4},
    )
    assert [event["sequence"] for event in resumed.json()["events"]] == [4, 5, 6, 7]
    assert [event["event_type"] for event in resumed.json()["events"]] == [
        "revision.created",
        "revision.prepared",
        "revision.commit_decided",
        "revision.created",
    ]
    drained = client.get(
        f"{API}/events",
        params={"after_sequence": resumed.json()["next_after_sequence"]},
    )
    assert drained.json()["events"] == []
    assert drained.json()["count"] == 0
    filtered = client.get(
        f"{API}/events",
        params={"event_type": "revision.created", "limit": 20},
    )
    assert filtered.status_code == 200
    assert filtered.json()["count"] == 2
    assert {event["event_type"] for event in filtered.json()["events"]} == {"revision.created"}
    aggregate = client.get(f"{API}/events", params={"aggregate_id": "rev-evented-0-r0"})
    assert aggregate.status_code == 200
    assert aggregate.json()["count"] == 1
    assert aggregate.json()["events"][0]["payload"]["design_id"] == "evented-0"
    assert (
        aggregate.json()["events"][0]["payload"]["provenance"]["provenance_id"]
        == "prov-evented-0-r0"
    )
    for invalid in (
        {"limit": 0},
        {"limit": 10_000},
        {"after_sequence": -1},
        {"workflow_id": "../escape"},
    ):
        rejected = client.get(f"{API}/events", params=invalid)
        assert rejected.status_code == 422
        assert rejected.json()["error"]["code"] == "invalid_request"


def test_oversized_bodies_are_refused_on_every_method(client: TestClient) -> None:
    payload = {
        "design_id": "too-big",
        "domain": "mechanical",
        "summary": "x" * 8192,
    }
    response = client.post(f"{API}/revisions", json=payload)
    assert response.status_code == 413
    assert response.json()["error"]["code"] == "payload_too_large"
    assert response.headers["x-content-type-options"] == "nosniff"
    small = client.post(
        f"{API}/revisions",
        json={"design_id": "small", "domain": "mechanical"},
    )
    assert small.status_code == 201


def test_oversized_bodies_are_refused_without_a_declared_length(client: TestClient) -> None:
    def chunked() -> Iterator[bytes]:
        yield b"x" * 3000
        yield b"x" * 3000

    for method, path in (
        ("GET", "/health"),
        ("HEAD", "/health"),
        ("DELETE", f"{API}/domains"),
        ("OPTIONS", f"{API}/domains"),
        ("POST", f"{API}/revisions"),
        ("PUT", f"{API}/revisions"),
        ("PATCH", f"{API}/revisions"),
    ):
        response = client.request(method, path, content=chunked())
        assert response.status_code == 413, method
        assert response.headers["x-content-type-options"] == "nosniff"
        if method != "HEAD":
            assert response.json()["error"]["code"] == "payload_too_large"
    under = client.request("GET", "/health", content=iter([b"x" * 16]))
    assert under.status_code == 200


def test_error_responses_never_echo_submitted_values(client: TestClient) -> None:
    response = client.post(
        f"{API}/domains/mechanical/evaluations",
        json={"parameters": {"load_kn": SECRET}},
    )
    assert response.status_code == 422
    assert SECRET not in response.text
    unsafe_identifier = client.post(
        f"{API}/domains/mechanical/evaluations",
        json={"design_id": f"{SECRET} with spaces"},
    )
    assert unsafe_identifier.status_code == 422
    assert SECRET not in unsafe_identifier.text
    failed_revision = client.post(
        f"{API}/revisions",
        json={"design_id": f"{SECRET} escape", "domain": "mechanical"},
    )
    assert failed_revision.status_code == 422
    assert SECRET not in failed_revision.text
    assert "Traceback" not in response.text
    assert set(response.json()) == {"error", "details"}
    assert set(response.json()["error"]) == {"code", "message"}
    assert len(response.json()["error"]["message"]) <= 256


def test_cross_origin_requests_are_refused_and_never_granted(
    application: KalaApplication,
) -> None:
    with _client(create_app(application, api_token=LAUNCH_TOKEN)) as client:
        for origin in (
            "http://localhost:5173",
            "http://127.0.0.1:5173",
            "http://0.0.0.0:5173",
            "http://evil.example",
            "null",
        ):
            refused = client.get(f"{API}/domains", headers={"Origin": origin})
            assert refused.status_code == 403, origin
            assert refused.json()["error"]["code"] == "cross_origin_denied"
            assert not any(name.startswith("access-control-") for name in refused.headers)
        preflight = client.options(
            f"{API}/domains",
            headers={
                "Origin": "http://localhost:5173",
                "Access-Control-Request-Method": "GET",
            },
        )
        assert preflight.status_code == 403
        assert not any(name.startswith("access-control-") for name in preflight.headers)
        write = client.post(
            f"{API}/revisions",
            json={"design_id": "cross-origin", "domain": "mechanical"},
            headers={"Origin": "http://localhost:5173"},
        )
        assert write.status_code == 403
        assert not any(name.startswith("access-control-") for name in write.headers)
        same = client.get(f"{API}/domains")
        assert same.status_code == 200
    with pytest.raises(ValueError, match="max_body_bytes"):
        validate_max_body_bytes(0)
    assert validate_max_body_bytes(MAX_REQUEST_BODY_BYTES) == MAX_REQUEST_BODY_BYTES


def test_lifespan_closes_only_an_owned_application(tmp_path: Path) -> None:
    injected = KalaApplication(_settings(tmp_path / "injected"))
    with _client(create_app(injected, api_token=LAUNCH_TOKEN)) as client:
        assert client.get("/health").status_code == 200
    assert injected.closed is False
    injected.close()

    owned = create_app(settings=_settings(tmp_path / "owned"), api_token=LAUNCH_TOKEN)
    with _client(owned) as client:
        assert client.get("/health").status_code == 200
    assert owned.state.kala_application.closed is True
    with pytest.raises(ValueError, match="not both"):
        create_app(
            KalaApplication(_settings(tmp_path / "both")),
            settings=_settings(tmp_path),
        )


def test_openapi_documents_the_local_trust_model(client: TestClient) -> None:
    spec = client.get("/openapi.json")
    assert spec.status_code == 200
    document = spec.json()
    assert document["info"]["description"] is not None
    description = document["info"]["description"]
    assert "bearer token" in description
    assert "loopback" in description
    assert "no multi-user isolation" in description
    assert document["servers"] == [
        {"url": "http://127.0.0.1:8000", "description": "Local loopback only"}
    ]
    assert set(document["paths"]) == {
        "/health",
        f"{API}/domains",
        f"{API}/domains/{{domain}}",
        f"{API}/domains/{{domain}}/evaluations",
        f"{API}/revisions",
        f"{API}/revisions/{{revision_id}}",
        f"{API}/revision-commits",
        f"{API}/revision-commits/{{request_id}}",
        f"{API}/revision-commits/{{request_id}}/approve",
        f"{API}/revision-commits/{{request_id}}/reject",
        f"{API}/workflows/mechanical/reference",
        f"{API}/workflows/{{workflow_id}}",
        f"{API}/workflows/{{workflow_id}}/checkpoints",
        f"{API}/workflows/{{workflow_id}}/checkpoints/approve",
        f"{API}/workflows/{{workflow_id}}/checkpoints/reject",
        f"{API}/events",
    }
    assert "a revision is never written without a pre-commit" in description
    assert "does not wire one" in description
    assert "reserves for itself" in description
    health_schema = document["paths"]["/health"]["get"]["responses"]["200"]["content"][
        "application/json"
    ]["schema"]
    assert health_schema["$ref"].endswith("HealthResponse")
    create_schema = document["paths"][f"{API}/revisions"]["post"]["responses"]["201"]["content"][
        "application/json"
    ]["schema"]
    assert create_schema["$ref"].endswith("RevisionRecord")
    prepare_schema = document["paths"][f"{API}/revision-commits"]["post"]["responses"]["201"][
        "content"
    ]["application/json"]["schema"]
    assert prepare_schema["$ref"].endswith("RevisionCommitView")


def test_a_reserved_workflow_identifier_is_refused_over_http(client: TestClient) -> None:
    for reserved in ("kernel-policy-state", "kernel-budget-state", "kernel-state:any"):
        response = client.post(
            f"{API}/workflows/mechanical/reference",
            headers=AUTH,
            json={"workflow_id": reserved},
        )
        assert response.status_code == 422
        assert response.json()["error"]["code"] == "reserved_identifier"
        assert client.get(f"{API}/workflows/{reserved}").status_code == 422
        assert (
            client.post(
                f"{API}/workflows/{reserved}/checkpoints/approve",
                headers=AUTH,
                json={},
            ).status_code
            == 422
        )


def test_docs_are_local_only_and_uncached(client: TestClient) -> None:
    docs = client.get("/docs")
    assert docs.status_code == 200
    assert "swagger" in docs.text.lower()
    assert docs.headers["cache-control"] == "no-store"
    assert client.get("/redoc").status_code == 404


def test_create_app_returns_a_fastapi_application(application: KalaApplication) -> None:
    app: FastAPI = create_app(application, api_token=LAUNCH_TOKEN)
    assert isinstance(app, FastAPI)
    assert app.state.kala_application is application
    assert app.state.kala_owns_application is False


def test_unexpected_failures_fail_closed(application: KalaApplication) -> None:
    def boom() -> dict[str, str]:
        raise RuntimeError(f"internal detail {SECRET}")

    app = create_app(application, api_token=LAUNCH_TOKEN)
    app.router.add_api_route("/boom", boom, methods=["GET"], include_in_schema=False)
    with _client(app) as client:
        response = client.get("/boom")
    assert response.status_code == 500
    assert response.json() == {
        "details": [],
        "error": {
            "code": "internal_error",
            "message": "the request could not be completed",
        },
    }
    assert SECRET not in response.text
    assert "Traceback" not in response.text
    assert response.headers["x-content-type-options"] == "nosniff"


def test_unsupported_methods_and_routes_are_reported_safely(
    client: TestClient,
) -> None:
    method_not_allowed = client.delete(f"{API}/domains")
    assert method_not_allowed.status_code == 405
    assert method_not_allowed.json()["error"]["code"] == "method_not_allowed"
    unknown_route = client.get(f"{API}/does-not-exist")
    assert unknown_route.status_code == 404
    assert unknown_route.json() == {
        "details": [],
        "error": {
            "code": "not_found",
            "message": "the requested resource does not exist",
        },
    }
    assert client.get("/").status_code == 404
