"""End-to-end tests for the local mechanical reference loop.

These tests drive the same loop through both presentation layers: the HTTP API
and the command line adapter, including one real subprocess invocation. They
verify the properties the loop is supposed to have, not just that it returns:

* revisions are immutable, sequenced, chained, and content addressed;
* a revision is only ever written behind an approval bound to the digests of the
  state it commits;
* every artifact digest matches the bytes in the content-addressed store;
* provenance is recorded and can be read back;
* the workflow pauses at its declared pre-commit checkpoint and only completes
  once the policy checkpoint is approved;
* the journal is append only and every event digest verifies;
* failure paths are reported, not crashed through.

Every test client names the loopback interface and presents the launch token, so
no test reaches a route by weakening the request boundary. Nothing here opens a
socket, and no model provider is contacted.
"""

from __future__ import annotations

import hashlib
import io
import json
import os
import socket
import sqlite3
import subprocess
import sys
from collections.abc import Iterator
from pathlib import Path
from typing import Any

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient
from pydantic import ValidationError

from kala_agent.api import create_app
from kala_agent.api.boundary import read_api_token
from kala_agent.application import (
    ApplicationError,
    CheckpointDecisionDeniedError,
    KalaApplication,
    WorkflowFinishedError,
    mechanical_step_catalog,
)
from kala_agent.cli import main
from kala_agent.config import Settings

API = "/api/v1"
DESIGN = "bracket-integration"
WORKFLOW = "workflow-bracket-integration"
LOOPBACK_BASE_URL = "http://127.0.0.1"
LAUNCH_TOKEN = "integration-launch-token-0123456789"
REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE_ROOT = REPO_ROOT / "src"


def _client(app: FastAPI, *, token: str) -> TestClient:
    """Return a test client that names loopback and presents the launch token."""
    return TestClient(
        app,
        base_url=LOOPBACK_BASE_URL,
        headers={"Authorization": f"Bearer {token}"},
    )


@pytest.fixture(autouse=True)
def _forbid_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    def blocked(*args: object, **kwargs: object) -> None:
        raise AssertionError("integration tests must not open network connections")

    monkeypatch.setattr(socket, "create_connection", blocked)
    monkeypatch.setattr(socket.socket, "connect", blocked)
    yield


@pytest.fixture
def application(tmp_path: Path) -> Iterator[KalaApplication]:
    resolved = KalaApplication(Settings(data_dir=tmp_path / "data"))
    try:
        yield resolved
    finally:
        resolved.close()


@pytest.fixture
def client(application: KalaApplication) -> Iterator[TestClient]:
    with _client(create_app(application, api_token=LAUNCH_TOKEN), token=LAUNCH_TOKEN) as resolved:
        yield resolved


def _cli(
    argv: list[str],
    environ: dict[str, str],
) -> tuple[int, dict[str, Any], str]:
    out = io.StringIO()
    err = io.StringIO()
    code = main(argv, environ=environ, stdout=out, stderr=err)
    return code, (json.loads(out.getvalue()) if out.getvalue() else {}), err.getvalue()


def test_artifact_digest_matches_the_stored_bytes(application: KalaApplication) -> None:
    record = application.create_design_revision(design_id=DESIGN, domain="mechanical")
    artifact = record.revision.artifacts[0]
    stored = application.artifact_store.get_bytes(artifact.digest)
    assert hashlib.sha256(stored).hexdigest() == artifact.digest
    assert len(stored) == artifact.size_bytes
    assert stored.endswith(b"\n")
    document = json.loads(stored)
    assert document["domain"] == "mechanical"
    assert document["package_id"] == "mechanical.bracket"
    assert document["subject_id"] == DESIGN
    assert document["parameters"]["material"] == "aluminium-6061-t6"
    assert document["parameters"]["length_mm"] == 120.0
    assert document["non_authoritative_finding"].startswith("Surrogate reference")
    assert len(document["evaluations"]) == 4
    assert document["passed"] is True
    assert artifact.uri == f"artifact://sha256/{artifact.digest}"
    assert application.artifact_store.contains(artifact.digest)
    assert application.artifact_store.get_size(artifact.digest) == len(stored)


def test_revisions_are_immutable_chained_and_listed(application: KalaApplication) -> None:
    first = application.create_design_revision(design_id=DESIGN, domain="mechanical")
    second = application.create_design_revision(design_id=DESIGN, domain="mechanical")
    assert first.revision.revision_id == "rev-bracket-integration-r0"
    assert second.revision.revision_id == "rev-bracket-integration-r1"
    assert second.revision.parent_revision_id == first.revision.revision_id
    assert second.provenance.parent_provenance_ids == (first.provenance.provenance_id,)
    assert second.artifact_digests == first.artifact_digests
    reread = application.get_design_revision(first.revision.revision_id)
    assert reread.revision == first.revision
    assert reread.provenance == first.provenance
    listed = application.list_design_revisions(DESIGN)
    assert [item.revision.revision_id for item in listed] == [
        first.revision.revision_id,
        second.revision.revision_id,
    ]
    assert [item.revision.sequence for item in listed] == [0, 1]
    with pytest.raises(ValidationError):
        first.revision.sequence = 99  # type: ignore[misc]
    with pytest.raises(ValidationError):
        first.revision.artifacts = ()  # type: ignore[misc]
    assert application.get_design_revision(first.revision.revision_id).revision == first.revision
    assert application.artifact_store.get_bytes(
        listed[0].revision.artifacts[0].digest
    ) == application.artifact_store.get_bytes(listed[1].revision.artifacts[0].digest)


def test_full_loop_through_the_api(client: TestClient, application: KalaApplication) -> None:
    health = client.get("/health")
    assert health.status_code == 200
    assert health.json()["offline"] is True

    assert client.get(f"{API}/domains").json()["count"] == 3
    report = client.post(f"{API}/domains/mechanical/evaluations", json={"design_id": DESIGN})
    assert report.status_code == 200
    assert report.json()["evaluation"]["passed"] is True

    seeded = client.post(f"{API}/revisions", json={"design_id": DESIGN, "domain": "mechanical"})
    assert seeded.status_code == 201
    seeded_digest = seeded.json()["artifact_digests"][0]

    started = client.post(f"{API}/workflows/mechanical/reference", json={"design_id": DESIGN})
    assert started.status_code == 201
    run = started.json()["run"]
    assert run["workflow_id"] == WORKFLOW
    assert run["status"] == "waiting_for_approval"
    assert run["checkpoint"]["step_id"] == "mechanical.constraint-evaluation"
    assert run["checkpoint"]["status"] == "pending"
    assert run["policy_status"] == "pending"
    assert run["policy_reason"] == "approval_required"
    assert run["revision"] is None
    assert all(evaluation["passed"] for evaluation in run["evaluations"])
    assert len(run["evaluations"]) == 4
    assert run["planned_task_ids"] == ["step-1", "step-2", "step-3", "step-4"]

    state = client.get(f"{API}/workflows/{WORKFLOW}").json()
    assert state["status"] == "waiting_for_approval"
    assert state["current_state"] == "load-cased"
    assert state["completed_step_ids"] == [
        "mechanical.geometry-check",
        "mechanical.material-check",
        "mechanical.load-check",
    ]
    assert state["pending_approval"]["approval_id"] == (f"approval-{WORKFLOW}-mechanical.assess")
    assert application.get_design_revision(f"rev-{DESIGN}-r0").revision.sequence == 0
    assert len(application.list_design_revisions(DESIGN)) == 1

    decided = client.post(
        f"{API}/workflows/{WORKFLOW}/checkpoints/approve",
        json={"actor_id": "integration-operator"},
    )
    assert decided.status_code == 200
    committed = decided.json()
    assert committed["run"]["status"] == "completed"
    assert committed["run"]["state"]["current_state"] == "assessed"
    assert committed["run"]["checkpoint"]["status"] == "approved"
    assert committed["run"]["checkpoint"]["actor_id"] == "integration-operator"
    revision = committed["revision"]["revision"]
    assert revision["revision_id"] == f"rev-{DESIGN}-r1"
    assert revision["parent_revision_id"] == f"rev-{DESIGN}-r0"
    assert committed["revision"]["artifact_digests"] == [seeded_digest]
    assert committed["revision"]["provenance"]["transformation_ids"] == [
        "mechanical-reference-workflow",
        "reference-workflow-evaluation",
    ]

    listed = client.get(f"{API}/revisions", params={"design_id": DESIGN}).json()
    assert listed["count"] == 2
    assert [item["revision"]["revision_id"] for item in listed["revisions"]] == [
        f"rev-{DESIGN}-r0",
        f"rev-{DESIGN}-r1",
    ]
    assert (
        listed["revisions"][0]["artifact_digests"]
        == listed["revisions"][1]["artifact_digests"]
        == [seeded_digest]
    )

    for record in listed["revisions"]:
        fetched = client.get(f"{API}/revisions/{record['revision']['revision_id']}")
        assert fetched.status_code == 200
        assert fetched.json() == record
        digest = record["artifact_digests"][0]
        assert hashlib.sha256(application.artifact_store.get_bytes(digest)).hexdigest() == (digest)

    events = client.get(f"{API}/events", params={"workflow_id": WORKFLOW, "limit": 100}).json()
    types = [event["event_type"] for event in events["events"]]
    assert types[0] == "plan.created"
    assert "workflow.started" in types
    assert "workflow.step_evaluated" in types
    assert "workflow.checkpoint" in types
    assert "policy.checkpoint_requested" in types
    assert "workflow.approval_resolved" in types
    assert "workflow.checkpoint_decided" in types
    assert "revision.prepared" in types
    assert "revision.created" in types
    prepared = [event for event in events["events"] if event["event_type"] == "revision.prepared"]
    assert len(prepared) == 1
    commit_request = prepared[0]["payload"]["request_id"]
    assert prepared[0]["payload"]["state_digest"] != prepared[0]["payload"]["action_digest"]
    committed_events = [
        event for event in events["events"] if event["event_type"] == "revision.created"
    ]
    assert committed_events[0]["payload"]["request_id"] == commit_request
    assert committed_events[0]["payload"]["state_digest"] == prepared[0]["payload"]["state_digest"]
    for event in events["events"]:
        payload = json.dumps(event["payload"], sort_keys=True, separators=(",", ":"))
        assert hashlib.sha256(payload.encode("utf-8")).hexdigest() == event["digest"]
    sequences = [event["sequence"] for event in events["events"]]
    assert sequences == sorted(sequences)
    assert events["payload_truncated"] is False

    readback = client.get(f"{API}/revision-commits/{commit_request}")
    assert readback.status_code == 200
    assert readback.json()["status"] == "approved"
    assert readback.json()["committed"] is True
    assert readback.json()["revision_id"] == f"rev-{DESIGN}-r1"

    with pytest.raises(sqlite3.IntegrityError):
        application.database.connection.execute("DELETE FROM events")
    remaining = client.get(f"{API}/events", params={"limit": 100}).json()
    assert remaining["count"] >= events["count"]


def test_full_loop_through_the_cli(tmp_path: Path) -> None:
    environ = {"KALA_AGENT_DATA_DIR": str(tmp_path / "cli-data")}

    code, payload, errors = _cli(["init"], environ)
    assert code == 0, errors
    assert payload["data"]["offline"] is True

    code, payload, _ = _cli(["domains"], environ)
    assert code == 0
    assert len(payload["data"]["packages"]) == 3

    code, payload, _ = _cli(["evaluate", "--domain", "mechanical", "--design-id", DESIGN], environ)
    assert code == 0
    assert payload["data"]["passed"] is True
    assert payload["data"]["subject_id"] == DESIGN

    code, payload, _ = _cli(
        ["run-mechanical", "--design-id", DESIGN, "--actor", "cli-operator"], environ
    )
    assert code == 0
    assert payload["status"] == "waiting_for_approval"
    assert payload["data"]["workflow_id"] == WORKFLOW
    checkpoint = payload["data"]["checkpoint"]
    assert checkpoint["step_id"] == "mechanical.constraint-evaluation"
    assert checkpoint["status"] == "pending"

    code, payload, _ = _cli(["workflow-status", "--workflow-id", WORKFLOW], environ)
    assert code == 0
    assert payload["data"]["status"] == "waiting_for_approval"
    assert len(payload["data"]["completed_step_ids"]) == 3

    code, payload, _ = _cli(
        ["approve", "--workflow-id", WORKFLOW, "--actor", "cli-operator"], environ
    )
    assert code == 0
    assert payload["status"] == "completed"
    assert payload["data"]["checkpoint"]["status"] == "approved"
    revision = payload["data"]["revision"]["revision"]
    assert revision["revision_id"] == f"rev-{DESIGN}-r0"
    digest = payload["data"]["revision"]["artifact_digests"][0]
    stored_path = tmp_path / "cli-data" / "artifacts" / "objects" / digest[:2] / digest
    assert hashlib.sha256(stored_path.read_bytes()).hexdigest() == digest
    assert json.loads(stored_path.read_text(encoding="utf-8"))["passed"] is True

    code, payload, _ = _cli(["events", "--limit", "100", "--workflow-id", WORKFLOW], environ)
    assert code == 0
    assert payload["data"]["count"] > 10
    revision_events = [
        event for event in payload["data"]["events"] if event["event_type"] == "revision.created"
    ]
    assert len(revision_events) == 1
    assert revision_events[0]["payload"]["revision_id"] == revision["revision_id"]


def test_cli_subprocess_runs_the_same_loop(tmp_path: Path) -> None:
    data_dir = tmp_path / "subprocess-data"
    environ = dict(os.environ)
    environ["KALA_AGENT_DATA_DIR"] = str(data_dir)
    environ["PYTHONPATH"] = os.pathsep.join(
        [str(SOURCE_ROOT), environ.get("PYTHONPATH", "")]
    ).rstrip(os.pathsep)

    def run(*arguments: str) -> dict[str, Any]:
        completed = subprocess.run(
            [sys.executable, "-m", "kala_agent.cli", *arguments],
            capture_output=True,
            text=True,
            timeout=120,
            env=environ,
            check=False,
        )
        assert completed.returncode in (0, 1, 3), completed.stderr
        return json.loads(completed.stdout) if completed.stdout.strip() else {}

    assert run("init")["status"] == "ready"
    assert run("run-mechanical", "--design-id", DESIGN)["status"] == "waiting_for_approval"
    status_payload = run("workflow-status", "--workflow-id", WORKFLOW)
    assert status_payload["data"]["status"] == "waiting_for_approval"
    approved = run("approve", "--workflow-id", WORKFLOW)
    assert approved["status"] == "completed"
    assert approved["data"]["revision"]["revision"]["revision_id"] == f"rev-{DESIGN}-r0"
    assert run("events", "--limit", "5")["data"]["count"] == 5


def test_api_and_cli_share_one_local_data_directory(tmp_path: Path) -> None:
    data_dir = tmp_path / "shared"
    environ = {"KALA_AGENT_DATA_DIR": str(data_dir)}
    assert _cli(["init"], environ)[0] == 0
    code, payload, _ = _cli(
        ["run-mechanical", "--design-id", DESIGN, "--workflow-id", WORKFLOW], environ
    )
    assert code == 0
    assert payload["data"]["workflow_id"] == WORKFLOW

    launch = create_app(settings=Settings(data_dir=data_dir))
    token = read_api_token(data_dir)
    assert token is not None
    with _client(launch, token=token) as client:
        state = client.get(f"{API}/workflows/{WORKFLOW}")
        assert state.status_code == 200
        assert state.json()["status"] == "waiting_for_approval"
        approved = client.post(f"{API}/workflows/{WORKFLOW}/checkpoints/approve", json={})
        assert approved.status_code == 200
        assert approved.json()["run"]["status"] == "completed"
        assert approved.json()["revision"]["revision"]["revision_id"] == f"rev-{DESIGN}-r0"

    code, payload, _ = _cli(["workflow-status", "--workflow-id", WORKFLOW], environ)
    assert code == 0
    assert payload["data"]["status"] == "completed"
    code, payload, _ = _cli(["approve", "--workflow-id", WORKFLOW], environ)
    assert code == 1
    assert (
        json.loads(_cli(["approve", "--workflow-id", WORKFLOW], environ)[2])["error"]["code"]
        == "workflow_finished"
    )


def test_failure_paths_are_reported(client: TestClient, application: KalaApplication) -> None:
    assert client.get(f"{API}/domains/absent").status_code == 404
    assert client.get(f"{API}/workflows/absent").status_code == 404
    assert client.get(f"{API}/revisions/rev-absent-r0").status_code == 404
    assert client.get(f"{API}/revision-commits/commit-absent").status_code == 404
    assert client.post(f"{API}/revisions", json={"design_id": "x"}).status_code == 422
    assert client.post(f"{API}/revision-commits", json={"design_id": "x"}).status_code == 422

    code, payload, errors = _cli(
        ["evaluate", "--domain", "mechanical", "--param", "thickness_mm=0.5"],
        {"KALA_AGENT_DATA_DIR": str(application.settings.data_dir)},
    )
    assert code == 3
    assert payload["data"]["passed"] is False
    assert errors == ""

    with pytest.raises(ApplicationError) as excinfo:
        application.evaluate_domain_design("absent")
    assert excinfo.value.code == "unknown_domain"


def test_rejected_checkpoint_leaves_no_revision(application: KalaApplication) -> None:
    run = application.run_mechanical_reference_workflow(design_id=DESIGN)
    rejected = application.reject_workflow_checkpoint(
        run.workflow_id, reason="integration rejection"
    )
    assert rejected.status is not None
    assert rejected.status.value == "failed"
    assert rejected.revision is None
    assert rejected.checkpoint is not None
    assert rejected.checkpoint.status.value == "rejected"
    assert rejected.state.last_error == "integration rejection"
    assert application.list_design_revisions(DESIGN) == ()
    with pytest.raises(WorkflowFinishedError):
        application.approve_workflow_checkpoint(run.workflow_id)
    with pytest.raises(WorkflowFinishedError):
        application.run_mechanical_reference_workflow(design_id=DESIGN)


def test_revoked_capability_blocks_and_recovers_the_checkpoint(
    application: KalaApplication,
) -> None:
    run = application.run_mechanical_reference_workflow(design_id=DESIGN)
    assert application.policy.revoke("design.write") is True
    with pytest.raises(CheckpointDecisionDeniedError) as excinfo:
        application.approve_workflow_checkpoint(run.workflow_id)
    assert "capability_not_granted" in str(excinfo.value)
    assert application.get_workflow_state(run.workflow_id).status.value == ("waiting_for_approval")
    assert application.list_design_revisions(DESIGN) == ()
    application.policy.grant("design.write")
    approved = application.approve_workflow_checkpoint(run.workflow_id)
    assert approved.status.value == "completed"
    assert approved.revision is not None
    assert len(application.list_design_revisions(DESIGN)) == 1


def test_stale_checkpoint_digests_are_refused(application: KalaApplication) -> None:
    run = application.run_mechanical_reference_workflow(design_id=DESIGN)
    with pytest.raises(CheckpointDecisionDeniedError) as excinfo:
        application.approve_workflow_checkpoint(run.workflow_id, state_digest="0" * 64)
    assert "stale" in str(excinfo.value)
    assert application.get_workflow_state(run.workflow_id).status.value == ("waiting_for_approval")


def test_closed_application_refuses_further_work(tmp_path: Path) -> None:
    resolved = KalaApplication(Settings(data_dir=tmp_path / "closed"))
    resolved.create_design_revision(design_id=DESIGN, domain="mechanical")
    resolved.close()
    with pytest.raises(ApplicationError) as excinfo:
        resolved.list_events()
    assert excinfo.value.code == "application_closed"
    with _client(create_app(resolved, api_token=LAUNCH_TOKEN), token=LAUNCH_TOKEN) as client:
        assert client.get("/health").status_code == 503
        assert client.get(f"{API}/domains").status_code == 503


def test_every_component_can_be_injected_and_injected_resources_survive_close(
    tmp_path: Path,
) -> None:
    from datetime import UTC, datetime, timedelta

    from kala_agent.config import Settings
    from kala_agent.kernel.budget import BudgetLedger
    from kala_agent.kernel.policy import PolicyEngine
    from kala_agent.persistence.artifacts import ContentAddressedArtifactStore
    from kala_agent.persistence.database import Database
    from kala_agent.persistence.journal import EventJournal
    from kala_agent.runtime.agent import OfflinePlanner
    from kala_agent.runtime.model_gateway import LocalModelClient, ModelGateway
    from kala_agent.tools.executor import ToolExecutor
    from kala_agent.tools.registry import ToolRegistry
    from kala_agent.tools.simulator import register_simulator

    settings = Settings(data_dir=tmp_path / "injected")
    database = Database(settings.database_path)
    journal = EventJournal(database)
    artifacts = ContentAddressedArtifactStore(settings.artifact_dir)
    ledger = BudgetLedger()
    policy = PolicyEngine(["artifact.write", "design.write"])
    registry = ToolRegistry()
    register_simulator(registry)
    executor = ToolExecutor(
        registry,
        run_directory=settings.data_dir / "injected-runs",
        granted_capabilities=("artifact.read", "tool.execute"),
        policy=policy,
        ledger=ledger,
    )
    gateway = ModelGateway(LocalModelClient(model_id="injected-model"), ledger=ledger)
    planner = OfflinePlanner(
        mechanical_step_catalog(),
        fallback_step_id="mechanical.geometry-check",
    )
    fixed = datetime(2026, 5, 6, 7, 8, 9, tzinfo=UTC)
    clock_values = iter([fixed + timedelta(seconds=index) for index in range(64)])

    resolved = KalaApplication(
        settings,
        database=database,
        journal=journal,
        artifact_store=artifacts,
        budget=ledger,
        policy=policy,
        tool_registry=registry,
        tool_executor=executor,
        model_gateway=gateway,
        planner=planner,
        clock=lambda: next(clock_values),
    )
    try:
        assert resolved.database is database
        assert resolved.journal is journal
        assert resolved.artifact_store is artifacts
        assert resolved.budget is ledger
        assert resolved.policy is policy
        assert resolved.tool_registry is registry
        assert resolved.tool_executor is executor
        assert resolved.model_gateway is gateway
        assert resolved.planner is planner
        assert resolved.model_id == "injected-model"
        assert resolved.domains == ("building", "mechanical", "wing")
        record = resolved.create_design_revision(design_id=DESIGN, domain="mechanical")
        assert record.revision.created_at >= fixed
        assert record.revision.created_at.tzinfo is not None
        run = resolved.run_mechanical_reference_workflow(design_id=DESIGN)
        assert run.started is True
        assert run.planned_task_ids == ("step-1", "step-2", "step-3", "step-4")
        decided = resolved.approve_workflow_checkpoint(run.workflow_id)
        assert decided.status.value == "completed"
        assert decided.revision is not None
        assert decided.revision.revision.created_at >= record.revision.created_at
        assert ledger.usage.artifact_bytes > 0
    finally:
        resolved.close()
    assert resolved.closed is True
    assert database.closed is False
    assert journal.get(1) is not None
    artifacts.put_text("still writable after close")
    database.close()
    assert database.closed is True
