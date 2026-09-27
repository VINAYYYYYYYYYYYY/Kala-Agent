"""Unit tests for the local Kala command line adapter.

Every test runs offline against a temporary local data directory and reads only
the JSON the adapter prints. No socket is opened and no credential is printed.
"""

from __future__ import annotations

import importlib
import io
import json
import sqlite3
from collections.abc import Iterator, Mapping
from pathlib import Path
from types import ModuleType
from typing import Any

import pytest
from fastapi.testclient import TestClient

from kala_agent.api.boundary import API_TOKEN_FILE_NAME, issue_api_token, read_api_token
from kala_agent.application import ApplicationError, KalaApplication
from kala_agent.cli import (
    EXIT_ERROR,
    EXIT_FAILED_OUTCOME,
    EXIT_OK,
    EXIT_USAGE,
    PROGRAM_NAME,
    SERVE_HOST,
    SERVE_PORT,
    build_parser,
    build_serve_plan,
    build_serve_target,
    load_uvicorn,
    main,
)
from kala_agent.config import Settings

CLI_MODULE: ModuleType = importlib.import_module("kala_agent.cli.main")
SECRET = "sk-cli-secret-value-must-not-be-printed-9876543210"
COMMANDS = (
    "init",
    "domains",
    "token",
    "serve",
    "evaluate",
    "run-mechanical",
    "workflow-status",
    "approve",
    "reject",
    "prepare-revision",
    "commit-revision",
    "events",
)


def _applied_schema_version(database_path: Path) -> int:
    with sqlite3.connect(database_path) as connection:
        row = connection.execute("SELECT MAX(version) FROM schema_migrations").fetchone()
    return 0 if row is None or row[0] is None else int(row[0])


@pytest.fixture(autouse=True)
def _forbid_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    def blocked(*args: object, **kwargs: object) -> None:
        raise AssertionError("the command line adapter must not open sockets")

    monkeypatch.setattr("socket.socket.connect", blocked)
    monkeypatch.setattr("socket.socket.connect_ex", blocked)
    monkeypatch.setattr("socket.create_connection", blocked)
    yield


@pytest.fixture
def data_dir(tmp_path: Path) -> Path:
    return tmp_path / "data"


@pytest.fixture
def environ(data_dir: Path) -> dict[str, str]:
    return {
        "KALA_AGENT_DATA_DIR": str(data_dir),
        "KALA_AGENT_MODEL_PROVIDER": "local",
        "KALA_AGENT_MODEL_ID": "local",
    }


def _run(
    argv: list[str],
    environ: Mapping[str, str],
) -> tuple[int, dict[str, Any], str]:
    out = io.StringIO()
    err = io.StringIO()
    code = main(argv, environ=environ, stdout=out, stderr=err)
    payload = json.loads(out.getvalue()) if out.getvalue() else {}
    return code, payload, err.getvalue()


def test_serve_binds_loopback_only() -> None:
    assert build_serve_target() == build_serve_target(SERVE_HOST, SERVE_PORT)
    assert (SERVE_HOST, SERVE_PORT) == ("127.0.0.1", 8000)
    assert build_serve_target("::1", 9_001).url == "http://[::1]:9001"
    for refused in ("0.0.0.0", "::", "[::]", "*", "192.168.1.10", "example.com", "", "  "):
        with pytest.raises(ApplicationError, match="loopback"):
            build_serve_target(refused, SERVE_PORT)
    for refused_port in (0, -1, 65_536, 1_000_000):
        with pytest.raises(ApplicationError, match="port"):
            build_serve_target(SERVE_HOST, refused_port)
    with pytest.raises(ApplicationError, match="port"):
        build_serve_target(SERVE_HOST, "8000")  # type: ignore[arg-type]
    with pytest.raises(SystemExit) as excinfo:
        build_parser().parse_args(["serve", "--port", "not-a-number"])
    assert excinfo.value.code == EXIT_USAGE


def test_serve_reports_a_missing_server_with_a_clear_message(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def missing(name: str, *args: object, **kwargs: object) -> object:
        raise ImportError(f"No module named {name!r}")

    monkeypatch.setattr(CLI_MODULE, "import_module", missing)
    with pytest.raises(ApplicationError) as excinfo:
        load_uvicorn()
    assert "uvicorn" in excinfo.value.message
    assert "kala-agent[serve]" in excinfo.value.message
    assert excinfo.value.code == "application_error"


class _RecordingServer:
    def __init__(self) -> None:
        self.calls: list[tuple[object, dict[str, object]]] = []

    def run(self, app: object, **kwargs: object) -> None:
        self.calls.append((app, kwargs))


def test_serve_mints_the_launch_token_and_keeps_the_boundary(
    data_dir: Path,
    environ: dict[str, str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    recorded = _RecordingServer()
    monkeypatch.setattr(CLI_MODULE, "load_uvicorn", lambda: recorded)
    assert read_api_token(data_dir) is None
    code, payload, errors = _run(["serve", "--port", "8931"], environ)
    assert code == EXIT_OK
    assert errors == ""
    assert payload["command"] == "serve"
    assert payload["data"]["host"] == "127.0.0.1"
    assert payload["data"]["port"] == 8931
    assert payload["data"]["url"] == "http://127.0.0.1:8931"
    assert payload["data"]["loopback_only"] is True
    assert payload["data"]["authentication"] == "bearer"
    assert payload["data"]["api_token_path"] == str(data_dir / API_TOKEN_FILE_NAME)
    assert len(recorded.calls) == 1
    app, kwargs = recorded.calls[0]
    assert kwargs["host"] == "127.0.0.1"
    assert kwargs["port"] == 8931
    assert kwargs["access_log"] is False
    token = read_api_token(data_dir)
    assert token is not None
    assert token not in json.dumps(payload)
    assert token not in errors

    classes = [middleware.cls for middleware in getattr(app, "user_middleware", [])]
    names = {cls.__name__ for cls in classes}
    assert "HostGuardMiddleware" in names
    assert "BearerTokenMiddleware" in names
    assert "CrossOriginGuardMiddleware" in names
    assert "SecurityHeadersMiddleware" in names
    assert "MaxBodySizeMiddleware" in names


def test_the_served_health_reports_local_facts_without_a_socket(
    data_dir: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(CLI_MODULE, "load_uvicorn", _RecordingServer)
    application = KalaApplication(Settings(data_dir=data_dir))
    try:
        plan = build_serve_plan(application, port=8_777)
        assert (plan.target.host, plan.target.port) == ("127.0.0.1", 8_777)
        assert plan.token_path == data_dir / API_TOKEN_FILE_NAME
        token = read_api_token(data_dir)
        assert token is not None
        assert token not in repr(plan)
        with TestClient(
            plan.app, base_url="http://127.0.0.1", raise_server_exceptions=False
        ) as client:
            health = client.get("/health")
            assert health.status_code == 200
            body = health.json()
            assert body["offline"] is True
            assert body["egress_configured"] is False
            assert body["provider_endpoint_declared"] is False
            unauthenticated = client.post("/api/v1/domains/mechanical/evaluations", json={})
            assert unauthenticated.status_code == 401
            rebound = TestClient(
                plan.app,
                base_url="http://192.168.1.10",
                raise_server_exceptions=False,
            )
            with rebound:
                assert rebound.get("/health").status_code == 421
    finally:
        application.close()


def test_serve_never_prints_the_launch_token(
    data_dir: Path,
    environ: dict[str, str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    recorded = _RecordingServer()
    monkeypatch.setattr(CLI_MODULE, "load_uvicorn", lambda: recorded)
    _run(["serve"], environ)
    token = read_api_token(data_dir)
    assert token is not None
    code, payload, errors = _run(["serve"], environ)
    assert code == EXIT_OK
    assert token not in json.dumps(payload)
    assert token not in errors
    for path in data_dir.rglob("*"):
        if path.is_file() and path.name != API_TOKEN_FILE_NAME:
            assert token.encode() not in path.read_bytes()


def test_parser_declares_every_command() -> None:
    parser = build_parser()
    assert parser.prog == PROGRAM_NAME
    actions = [
        action for action in parser._actions if isinstance(getattr(action, "choices", None), dict)
    ]
    assert len(actions) == 1
    choices = actions[0].choices
    assert isinstance(choices, dict)
    assert tuple(choices) == COMMANDS
    with pytest.raises(SystemExit) as excinfo:
        parser.parse_args(["not-a-command"])
    assert excinfo.value.code == EXIT_USAGE
    with pytest.raises(SystemExit) as excinfo:
        parser.parse_args([])
    assert excinfo.value.code == EXIT_USAGE


def test_init_creates_the_local_data_directory(
    data_dir: Path,
    environ: dict[str, str],
) -> None:
    assert not data_dir.exists()
    code, payload, errors = _run(["init"], environ)
    assert code == EXIT_OK
    assert errors == ""
    assert payload["command"] == "init"
    assert payload["status"] == "ready"
    assert payload["data"]["created"] is True
    assert payload["data"]["schema_version"] == _applied_schema_version(
        Path(payload["data"]["database_path"])
    )
    assert payload["data"]["offline"] is True
    assert payload["data"]["provider"] == "local"
    assert payload["data"]["domains"] == ["building", "mechanical", "wing"]
    assert payload["data"]["api_token_path"] == str(data_dir / API_TOKEN_FILE_NAME)
    assert Path(payload["data"]["database_path"]).is_file()
    assert Path(payload["data"]["artifact_dir"]).is_dir()
    assert Path(payload["data"]["runs_dir"]).is_dir()
    assert payload["data"]["tools"] == ["mechanical.reference-simulator"]
    assert not (data_dir / API_TOKEN_FILE_NAME).exists()
    code, payload, _ = _run(["init"], environ)
    assert code == EXIT_OK
    assert payload["data"]["created"] is False


def test_token_prints_the_stored_launch_token_and_nothing_else(
    data_dir: Path,
    environ: dict[str, str],
) -> None:
    code, payload, errors = _run(["token"], environ)
    assert code == EXIT_ERROR
    assert payload == {}
    assert "start the local api" in json.loads(errors)["error"]["message"]
    assert "Traceback" not in errors

    issued = issue_api_token(data_dir)
    code, payload, errors = _run(["token"], environ)
    assert code == EXIT_OK
    assert errors == ""
    assert payload["command"] == "token"
    assert payload["status"] == "issued"
    assert payload["data"]["token"] == issued
    assert payload["data"]["authentication"] == "bearer"
    assert payload["data"]["path"] == str(data_dir / API_TOKEN_FILE_NAME)
    assert read_api_token(data_dir) == issued


def test_no_command_prints_the_launch_token_but_token(
    data_dir: Path,
    environ: dict[str, str],
) -> None:
    issued = issue_api_token(data_dir)
    for argv in (
        ["init"],
        ["domains"],
        ["events"],
        ["evaluate", "--domain", "mechanical"],
        ["run-mechanical", "--design-id", "token-run"],
        ["workflow-status", "--workflow-id", "workflow-token-run"],
        ["approve", "--workflow-id", "workflow-token-run"],
    ):
        code, payload, errors = _run(argv, environ)
        assert code in (EXIT_OK, EXIT_ERROR), argv
        assert issued not in errors
        assert issued not in json.dumps(payload)
    for path in data_dir.rglob("*"):
        if path.is_file() and path.name != API_TOKEN_FILE_NAME:
            assert issued.encode() not in path.read_bytes()
    rotated = issue_api_token(data_dir)
    assert rotated != issued
    code, payload, _ = _run(["token"], environ)
    assert payload["data"]["token"] == rotated


def test_domains_lists_packages_and_workflows(environ: dict[str, str]) -> None:
    code, payload, _ = _run(["domains"], environ)
    assert code == EXIT_OK
    assert payload["command"] == "domains"
    assert [package["domain"] for package in payload["data"]["packages"]] == [
        "building",
        "mechanical",
        "wing",
    ]
    assert all(len(package["digest"]) == 64 for package in payload["data"]["packages"])
    assert {workflow["domain"] for workflow in payload["data"]["workflows"]} == {
        "building",
        "mechanical",
        "wing",
    }


def test_evaluate_reports_passed_and_failed_outcomes(environ: dict[str, str]) -> None:
    code, payload, _ = _run(["evaluate", "--domain", "mechanical"], environ)
    assert code == EXIT_OK
    assert payload["status"] == "passed"
    assert payload["data"]["passed"] is True
    assert len(payload["data"]["evaluations"]) == 4
    code, payload, _ = _run(
        [
            "evaluate",
            "--domain",
            "mechanical",
            "--param",
            "thickness_mm=0.5",
            "--param",
            "minimum_thickness_mm=2.0",
        ],
        environ,
    )
    assert code == EXIT_FAILED_OUTCOME
    assert payload["status"] == "failed"
    assert payload["data"]["passed"] is False
    assert payload["data"]["blocked_step_id"] == "mechanical.geometry-check"
    code, payload, _ = _run(
        [
            "evaluate",
            "--domain",
            "wing",
            "--parameters-json",
            '{"span_m": 8.0}',
            "--design-id",
            "wing-8",
        ],
        environ,
    )
    assert code == EXIT_OK
    assert payload["data"]["subject_id"] == "wing-8"
    assert payload["data"]["passed"] is True
    code, payload, _ = _run(["evaluate", "--domain", "wing", "--param", "span_m=14.5"], environ)
    assert code == EXIT_FAILED_OUTCOME
    assert payload["data"]["blocked_step_id"] is not None


def test_evaluate_reports_errors_without_a_traceback(environ: dict[str, str]) -> None:
    code, payload, errors = _run(["evaluate", "--domain", "not-a-domain"], environ)
    assert code == EXIT_ERROR
    assert payload == {}
    assert json.loads(errors)["error"]["code"] == "unknown_domain"
    assert "Traceback" not in errors
    code, payload, errors = _run(
        ["evaluate", "--domain", "mechanical", "--parameters-json", "{"], environ
    )
    assert code == EXIT_ERROR
    assert json.loads(errors)["error"]["code"] == "application_error"
    code, payload, errors = _run(
        ["evaluate", "--domain", "mechanical", "--param", "broken"], environ
    )
    assert code == EXIT_ERROR
    assert "name=value" in json.loads(errors)["error"]["message"]
    code, payload, errors = _run(
        ["evaluate", "--domain", "mechanical", "--param", f"load_kn={SECRET}"], environ
    )
    assert code == EXIT_ERROR
    assert SECRET not in errors
    code, payload, errors = _run(
        ["evaluate", "--domain", "mechanical", "--parameters-json", "[1, 2]"], environ
    )
    assert code == EXIT_ERROR


def test_mechanical_run_reports_a_pending_checkpoint(environ: dict[str, str]) -> None:
    code, payload, _ = _run(["run-mechanical", "--design-id", "cli-bracket"], environ)
    assert code == EXIT_OK
    run = payload["data"]
    assert payload["status"] == "waiting_for_approval"
    assert run["workflow_id"] == "workflow-cli-bracket"
    assert run["started"] is True
    assert run["checkpoint"]["status"] == "pending"
    assert run["checkpoint"]["checkpoint_type"] == "pre_commit"
    assert run["policy_status"] == "pending"
    assert run["revision"] is None
    assert len(run["planned_task_ids"]) == 4
    code, payload, _ = _run(["run-mechanical", "--design-id", "cli-bracket"], environ)
    assert code == EXIT_OK
    assert payload["data"]["started"] is False
    code, payload, _ = _run(["workflow-status", "--workflow-id", "workflow-cli-bracket"], environ)
    assert code == EXIT_OK
    assert payload["data"]["status"] == "waiting_for_approval"
    assert payload["data"]["current_state"] == "load-cased"


def test_approve_then_reject_drive_workflows(environ: dict[str, str]) -> None:
    _run(["run-mechanical", "--design-id", "approved"], environ)
    code, payload, _ = _run(
        ["approve", "--workflow-id", "workflow-approved", "--actor", "engineer-a"],
        environ,
    )
    assert code == EXIT_OK
    assert payload["status"] == "completed"
    assert payload["data"]["checkpoint"]["status"] == "approved"
    assert payload["data"]["checkpoint"]["actor_id"] == "engineer-a"
    assert payload["data"]["revision"]["revision"]["revision_id"] == "rev-approved-r0"
    code, payload, errors = _run(["approve", "--workflow-id", "workflow-approved"], environ)
    assert code == EXIT_ERROR
    assert payload == {}
    assert json.loads(errors)["error"]["code"] == "workflow_finished"

    _run(["run-mechanical", "--design-id", "rejected"], environ)
    code, payload, _ = _run(
        [
            "reject",
            "--workflow-id",
            "workflow-rejected",
            "--reason",
            "operator declined",
        ],
        environ,
    )
    assert code == EXIT_OK
    assert payload["status"] == "failed"
    assert payload["data"]["revision"] is None
    assert payload["data"]["state"]["last_error"] == "operator declined"
    code, payload, errors = _run(["workflow-status", "--workflow-id", "workflow-absent"], environ)
    assert code == EXIT_ERROR
    assert json.loads(errors)["error"]["code"] == "unknown_workflow"


def test_prepare_then_commit_is_the_two_phase_local_flow(environ: dict[str, str]) -> None:
    code, payload, errors = _run(
        [
            "prepare-revision",
            "--design-id",
            "cli-commit",
            "--domain",
            "mechanical",
            "--summary",
            "reviewed configuration",
        ],
        environ,
    )
    assert code == EXIT_OK, errors
    assert errors == ""
    prepared = payload["data"]
    assert payload["command"] == "prepare-revision"
    assert payload["status"] == "pending"
    assert prepared["revision_id"] == "rev-cli-commit-r0"
    assert prepared["committed"] is False
    assert len(prepared["state_digest"]) == 64
    assert len(prepared["action_digest"]) == 64
    code, payload, _ = _run(["events", "--limit", "5"], environ)
    assert code == EXIT_OK
    assert [event["event_type"] for event in payload["data"]["events"]][-1] == ("revision.prepared")
    assert _run(["commit-revision", "--request-id", "commit-absent"], environ)[0] == EXIT_ERROR

    code, payload, errors = _run(
        [
            "commit-revision",
            "--request-id",
            prepared["request_id"],
            "--state-digest",
            prepared["state_digest"],
            "--action-digest",
            prepared["action_digest"],
            "--actor",
            "cli-reviewer",
        ],
        environ,
    )
    assert code == EXIT_OK, errors
    assert payload["command"] == "commit-revision"
    assert payload["status"] == "approved"
    assert payload["data"]["commit"]["committed"] is True
    assert payload["data"]["commit"]["actor_id"] == "cli-reviewer"
    assert payload["data"]["revision"]["revision"]["revision_id"] == "rev-cli-commit-r0"

    code, payload, errors = _run(
        ["commit-revision", "--request-id", prepared["request_id"]], environ
    )
    assert code == EXIT_ERROR
    assert payload == {}
    assert json.loads(errors)["error"]["code"] == "checkpoint_not_pending"

    code, payload, errors = _run(
        [
            "prepare-revision",
            "--design-id",
            "cli-stale",
            "--domain",
            "mechanical",
        ],
        environ,
    )
    assert code == EXIT_OK
    stale = payload["data"]
    code, payload, _ = _run(
        [
            "prepare-revision",
            "--design-id",
            "cli-stale",
            "--domain",
            "mechanical",
            "--summary",
            "moved on",
        ],
        environ,
    )
    assert code == EXIT_OK
    assert _run(["commit-revision", "--request-id", payload["data"]["request_id"]], environ)[0] == (
        EXIT_OK
    )
    code, payload, errors = _run(["commit-revision", "--request-id", stale["request_id"]], environ)
    assert code == EXIT_ERROR
    assert json.loads(errors)["error"]["code"] == "checkpoint_digest_mismatch"

    code, payload, errors = _run(
        [
            "prepare-revision",
            "--design-id",
            "cli-declined",
            "--domain",
            "mechanical",
        ],
        environ,
    )
    assert code == EXIT_OK
    declined = payload["data"]
    code, payload, _ = _run(
        [
            "commit-revision",
            "--request-id",
            declined["request_id"],
            "--decision",
            "reject",
            "--reason",
            "declined locally",
        ],
        environ,
    )
    assert code == EXIT_OK
    assert payload["status"] == "rejected"
    assert payload["data"]["commit"]["committed"] is False
    assert payload["data"]["commit"]["reason"] == "declined locally"
    assert "revision" not in payload["data"]
    code, payload, errors = _run(["workflow-status", "--workflow-id", "cli-declined"], environ)
    assert code == EXIT_ERROR

    for argv, expected in (
        (["commit-revision", "--request-id", "commit-x", "--state-digest", "nope"], EXIT_ERROR),
        (["prepare-revision", "--design-id", "../escape", "--domain", "mechanical"], EXIT_ERROR),
        (["prepare-revision", "--design-id", "cli-bad", "--domain", "absent"], EXIT_ERROR),
    ):
        code, payload, errors = _run(argv, environ)
        assert code == expected, argv
        assert payload == {}
        assert json.loads(errors)["error"]["code"] != ""


def test_events_are_printed_as_a_bounded_page(environ: dict[str, str]) -> None:
    _run(["run-mechanical", "--design-id", "evented"], environ)
    code, payload, _ = _run(["events", "--limit", "3"], environ)
    assert code == EXIT_OK
    page = payload["data"]
    assert page["count"] == 3
    assert page["next_after_sequence"] == 3
    assert next(event["event_type"] for event in page["events"]) == "plan.created"
    code, payload, _ = _run(
        ["events", "--after-sequence", "3", "--workflow-id", "workflow-evented"],
        environ,
    )
    assert code == EXIT_OK
    assert all(event["workflow_id"] == "workflow-evented" for event in payload["data"]["events"])
    code, payload, errors = _run(["events", "--limit", "0"], environ)
    assert code == EXIT_ERROR
    assert json.loads(errors)["error"]["code"] == "invalid_parameters"
    with pytest.raises(SystemExit) as excinfo:
        _run(["events", "--limit", "not-a-number"], environ)
    assert excinfo.value.code == EXIT_USAGE


def test_secrets_in_the_environment_are_never_printed(
    data_dir: Path,
    environ: dict[str, str],
) -> None:
    hostile = dict(environ)
    hostile["KALA_AGENT_API_KEY"] = SECRET
    hostile["OPENAI_API_KEY"] = SECRET
    hostile["KALA_AGENT_MODEL_ENDPOINT"] = "http://127.0.0.1:11434/v1"
    for argv in (
        ["init"],
        ["domains"],
        ["events"],
        ["evaluate", "--domain", "mechanical"],
    ):
        code, payload, errors = _run(argv, hostile)
        assert code == EXIT_OK
        assert SECRET not in errors
        assert SECRET not in json.dumps(payload)
        assert "11434" not in json.dumps(payload)
        assert payload["data"].get("offline", True) is True
    code, payload, errors = _run(["init"], hostile)
    assert payload["data"]["egress_configured"] is False
    assert payload["data"]["provider_endpoint_declared"] is True
    assert data_dir.is_dir()


def test_invalid_configuration_is_reported_without_values(tmp_path: Path) -> None:
    broken = {"KALA_AGENT_DATA_DIR": "relative/path", "KALA_AGENT_WORKER_COUNT": "0"}
    out = io.StringIO()
    err = io.StringIO()
    code = main(["init"], environ=broken, stdout=out, stderr=err)
    assert code == EXIT_ERROR
    assert out.getvalue() == ""
    body = json.loads(err.getvalue())
    assert body["error"]["code"] == "invalid_configuration"
    assert "relative/path" not in err.getvalue()
    assert "Traceback" not in err.getvalue()
    out = io.StringIO()
    err = io.StringIO()
    code = main(["--data-dir", "relative", "init"], environ={}, stdout=out, stderr=err)
    assert code == EXIT_ERROR
    assert json.loads(err.getvalue())["error"]["code"] == "invalid_configuration"
    assert not tmp_path.joinpath("relative").exists()
    with pytest.raises(SystemExit) as excinfo:
        main(["init", "--unknown-flag"], environ={})
    assert excinfo.value.code == EXIT_USAGE


def test_injected_application_is_never_closed(
    data_dir: Path,
    environ: dict[str, str],
) -> None:
    application = KalaApplication(Settings(data_dir=data_dir))
    try:
        out = io.StringIO()
        assert main(["domains"], application=application, environ=environ, stdout=out) == 0
        assert json.loads(out.getvalue())["command"] == "domains"
        assert application.closed is False
    finally:
        application.close()
    assert application.closed is True
