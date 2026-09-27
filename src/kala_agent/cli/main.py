"""The local Kala command line adapter.

Presentation only. Every command reads validated settings from the environment,
opens the same local data directory the API uses, prints one JSON document to
stdout, and returns an exit code. It holds no engineering rule, decides no
approval on its own, and never opens an outbound connection.

``serve`` is the one exception to "never opens a socket", and it is the narrow
one: it binds the loopback interface only, refuses any other address before a
socket exists, and serves the same application every other launch serves, with
the same loopback Host guard and the same per-launch bearer token. It reports
egress from the model client that is actually wired, never from a setting, and
a provider-neutral endpoint declared in the settings is reported as declared
rather than as a hosted path this process could use.

Exit codes:

* ``0`` the requested operation completed;
* ``1`` the operation could not be completed (bad input, unknown identifier,
  missing checkpoint, unusable local data directory, no installed server);
* ``2`` a usage error, raised by :mod:`argparse`;
* ``3`` the operation completed but the outcome is a failure: a reference
  evaluation that violates a hard constraint, or a workflow that ended failed.

A credential-shaped value in the environment is never printed: this module
reports provider and model identity, not endpoints or API keys. The one secret
it will print is the API launch token, and only when ``token`` is asked for by
name, on the operator's own terminal. No other command prints it, and it is
never written to a log or to stderr.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from importlib import import_module
from pathlib import Path
from types import ModuleType
from typing import Final, TextIO

from fastapi import FastAPI
from pydantic import ValidationError

from kala_agent.api.app import create_app
from kala_agent.api.boundary import (
    API_TOKEN_FILE_NAME,
    is_loopback_host,
    issue_api_token,
    read_api_token,
)
from kala_agent.application import (
    LOCAL_ACTOR_ID,
    ApplicationError,
    KalaApplication,
    MechanicalRunResult,
)
from kala_agent.config import Settings
from kala_agent.domains import NON_AUTHORITATIVE_FINDING

PROGRAM_NAME: Final[str] = "kala-agent"
EXIT_OK: Final[int] = 0
EXIT_ERROR: Final[int] = 1
EXIT_USAGE: Final[int] = 2
EXIT_FAILED_OUTCOME: Final[int] = 3
DATA_DIR_ENVIRONMENT_KEY: Final[str] = "KALA_AGENT_DATA_DIR"
MAX_PARAMETER_BYTES: Final[int] = 16_384
MAX_REASON_LENGTH: Final[int] = 512
MAX_ACTOR_LENGTH: Final[int] = 64
MAX_PARAMETERS_ENTRIES: Final[int] = 256
MAX_DIGEST_LENGTH: Final[int] = 64
DEFAULT_EVENT_LIMIT: Final[int] = 50
MAX_EVENT_LIMIT: Final[int] = 1_000
MAX_AFTER_SEQUENCE: Final[int] = 1_000_000_000
REVISION_DECISIONS: Final[tuple[str, ...]] = ("approve", "reject")
DEFAULT_REVISION_DECISION: Final[str] = "approve"
SERVE_HOST: Final[str] = "127.0.0.1"
SERVE_PORT: Final[int] = 8000
MIN_SERVE_PORT: Final[int] = 1
MAX_SERVE_PORT: Final[int] = 65_535
UVICORN_MODULE_NAME: Final[str] = "uvicorn"
SERVE_LOG_LEVEL: Final[str] = "warning"
SERVE_UNAVAILABLE_MESSAGE: Final[str] = (
    "the serve command needs the uvicorn server, which is an optional runtime dependency; "
    "install it with: pip install 'kala-agent[serve]'"
)
NON_LOOPBACK_HOST_MESSAGE: Final[str] = (
    "serve binds the loopback interface only; a routable or wildcard address is refused"
)
DISCLAIMER: Final[str] = (
    "Bounded surrogate reference arithmetic only; not CAD, CAE, a code check, or an "
    "engineering certification. No authentication and no network access."
)
EPILOG: Final[str] = "Exit codes: 0 completed, 1 error, 2 usage, 3 completed with a failed outcome."


@dataclass(frozen=True, slots=True)
class ServeTarget:
    """The loopback address the local API is served on."""

    host: str
    port: int

    @property
    def url(self) -> str:
        authority = f"[{self.host}]" if ":" in self.host else self.host
        return f"http://{authority}:{self.port}"


@dataclass(frozen=True, slots=True)
class ServePlan:
    """Everything the serve command resolves before a socket is opened.

    Building the plan mints the launch token and writes it to the local data
    directory, then builds the application with that token. The application is
    the unmodified one :func:`create_app` builds, so the loopback Host guard and
    the per-launch bearer token middleware are the same ones every other launch
    of this API carries; nothing here relaxes either of them.
    """

    target: ServeTarget
    app: FastAPI
    token_path: Path


def _emit(stream: TextIO, payload: object) -> None:
    stream.write(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    stream.flush()


def _emit_error(stream: TextIO, code: str, message: str) -> None:
    _emit(stream, {"error": {"code": code, "message": message[:512]}})


def _settings_for(data_dir: str | None, environ: Mapping[str, str] | None) -> Settings:
    source = dict(os.environ if environ is None else environ)
    if data_dir is not None:
        source[DATA_DIR_ENVIRONMENT_KEY] = data_dir
    return Settings.from_env(source)


def _safe_configuration_message(error: ValidationError) -> str:
    fields = ",".join(
        f"{'.'.join(str(item) for item in failure.get('loc', ())) or 'settings'}"
        f":{failure.get('type', 'invalid')}"
        for failure in error.errors()[:8]
    )
    return (
        f"the local configuration is invalid: {fields}"
        if fields
        else ("the local configuration is invalid")
    )


def _parse_parameter(raw: str) -> tuple[str, object]:
    name, separator, value = raw.partition("=")
    if not separator or not name.strip():
        raise ApplicationError("a parameter override must be written as name=value")
    try:
        decoded: object = json.loads(value)
    except (json.JSONDecodeError, ValueError):
        decoded = value
    return name.strip(), decoded


def _parse_parameters(
    overrides: Sequence[str],
    raw: str | None,
) -> dict[str, object] | None:
    if raw is None and not overrides:
        return None
    parameters: dict[str, object] = {}
    if raw is not None:
        try:
            decoded = json.loads(raw)
        except json.JSONDecodeError as error:
            raise ApplicationError("--parameters-json must be a JSON object") from error
        if not isinstance(decoded, dict):
            raise ApplicationError("--parameters-json must be a JSON object")
        parameters.update(decoded)
    for item in overrides:
        name, value = _parse_parameter(item)
        parameters[name] = value
    if len(parameters) > MAX_PARAMETERS_ENTRIES:
        raise ApplicationError("too many parameter overrides were supplied")
    if len(json.dumps(parameters)) > MAX_PARAMETER_BYTES:
        raise ApplicationError("the parameter overrides exceed the bounded size")
    return parameters


def _state_of(result: MechanicalRunResult) -> str:
    return str(result.status.value)


def _run_exit_code(result: MechanicalRunResult) -> int:
    return (
        EXIT_OK
        if result.status.value in {"waiting_for_approval", "completed"}
        else (EXIT_FAILED_OUTCOME)
    )


def _command_init(
    application: KalaApplication,
    *,
    created: bool,
) -> tuple[int, dict[str, object]]:
    settings = application.settings
    data = {
        "api_token_path": str(settings.data_dir / API_TOKEN_FILE_NAME),
        "artifact_dir": str(settings.artifact_dir),
        "created": created,
        "data_dir": str(settings.data_dir),
        "database_path": str(settings.database_path),
        "disclaimer": DISCLAIMER,
        "domains": list(application.domains),
        "egress_configured": application.egress_configured,
        "model": application.model_id,
        "non_authoritative_finding": NON_AUTHORITATIVE_FINDING,
        "offline": application.offline,
        "provider": application.provider_id,
        "provider_endpoint_declared": application.provider_endpoint_declared,
        "runs_dir": str(application.tool_executor.run_directory),
        "schema_version": application.database.schema_version,
        "tools": list(application.tool_registry.tool_ids()),
        "worker_count": settings.worker_count,
    }
    return EXIT_OK, {
        "command": "init",
        "data": data,
        "status": "ready",
    }


def _command_domains(application: KalaApplication) -> tuple[int, dict[str, object]]:
    packages = [package.model_dump(mode="json") for package in application.list_domain_packages()]
    workflows = [
        workflow.model_dump(mode="json") for workflow in application.list_workflow_definitions()
    ]
    return EXIT_OK, {
        "command": "domains",
        "data": {
            "domains": list(application.domains),
            "packages": packages,
            "workflows": workflows,
        },
        "status": "listed",
    }


def _command_evaluate(
    application: KalaApplication,
    arguments: argparse.Namespace,
) -> tuple[int, dict[str, object]]:
    report = application.evaluate_domain_design(
        arguments.domain,
        parameters=_parse_parameters(arguments.param, arguments.parameters_json),
        design_id=arguments.design_id,
    )
    status_code = EXIT_OK if report.passed else EXIT_FAILED_OUTCOME
    return status_code, {
        "command": "evaluate",
        "data": report.model_dump(mode="json"),
        "status": "passed" if report.passed else "failed",
    }


def _command_run_mechanical(
    application: KalaApplication,
    arguments: argparse.Namespace,
) -> tuple[int, dict[str, object]]:
    result = application.run_mechanical_reference_workflow(
        workflow_id=arguments.workflow_id,
        design_id=arguments.design_id,
        actor_id=arguments.actor,
    )
    return _run_exit_code(result), {
        "command": "run-mechanical",
        "data": result.model_dump(mode="json"),
        "status": _state_of(result),
    }


def _command_workflow_status(
    application: KalaApplication,
    arguments: argparse.Namespace,
) -> tuple[int, dict[str, object]]:
    state = application.get_workflow_state(arguments.workflow_id)
    return EXIT_OK, {
        "command": "workflow-status",
        "data": state.model_dump(mode="json"),
        "status": str(state.status.value),
    }


def _command_decide(
    application: KalaApplication,
    arguments: argparse.Namespace,
    decision: str,
) -> tuple[int, dict[str, object]]:
    result = application.decide_workflow_checkpoint(
        arguments.workflow_id,
        decision,
        actor_id=arguments.actor,
        reason=arguments.reason,
    )
    status_code = (
        EXIT_OK
        if decision == "reject" or _run_exit_code(result) == EXIT_OK
        else (EXIT_FAILED_OUTCOME)
    )
    return status_code, {
        "command": decision,
        "data": result.model_dump(mode="json"),
        "status": _state_of(result),
    }


def _command_events(
    application: KalaApplication,
    arguments: argparse.Namespace,
) -> tuple[int, dict[str, object]]:
    page = application.list_events(
        after_sequence=arguments.after_sequence,
        limit=arguments.limit,
        workflow_id=arguments.workflow_id,
        event_type=arguments.event_type,
        aggregate_id=arguments.aggregate_id,
    )
    return EXIT_OK, {
        "command": "events",
        "data": page.model_dump(mode="json"),
        "status": "listed",
    }


def _actor(value: str) -> str:
    if not value.strip() or len(value) > MAX_ACTOR_LENGTH:
        raise ApplicationError("the actor identifier must be nonempty and bounded")
    return value.strip()


def _reason(value: str | None) -> str | None:
    if value is None:
        return None
    if len(value) > MAX_REASON_LENGTH:
        raise ApplicationError("the reason is longer than the permitted length")
    return value


def _digest(value: str | None, field_name: str) -> str | None:
    if value is None:
        return None
    normalized = value.strip()
    if len(normalized) != MAX_DIGEST_LENGTH or any(
        character not in "0123456789abcdef" for character in normalized
    ):
        raise ApplicationError(f"{field_name} must be a lowercase SHA-256 digest")
    return normalized


def _command_prepare_revision(
    application: KalaApplication,
    arguments: argparse.Namespace,
) -> tuple[int, dict[str, object]]:
    prepared = application.prepare_design_revision(
        design_id=arguments.design_id,
        domain=arguments.domain,
        summary=arguments.summary,
        parameters=_parse_parameters(arguments.param, arguments.parameters_json),
        parent_revision_id=arguments.parent_revision_id,
    )
    return EXIT_OK, {
        "command": "prepare-revision",
        "data": prepared.model_dump(mode="json"),
        "status": prepared.status.value,
    }


def _command_commit_revision(
    application: KalaApplication,
    arguments: argparse.Namespace,
) -> tuple[int, dict[str, object]]:
    if arguments.decision not in REVISION_DECISIONS:
        raise ApplicationError(f"the decision must be one of {', '.join(REVISION_DECISIONS)}")
    state_digest = _digest(arguments.state_digest, "the state digest")
    action_digest = _digest(arguments.action_digest, "the action digest")
    actor = _actor(arguments.actor)
    reason = _reason(arguments.reason)
    if arguments.decision == "approve":
        record = application.commit_design_revision(
            arguments.request_id,
            actor_id=actor,
            state_digest=state_digest,
            action_digest=action_digest,
            reason=reason,
        )
        commit = application.get_revision_commit(arguments.request_id)
        return EXIT_OK, {
            "command": "commit-revision",
            "data": {
                "commit": commit.model_dump(mode="json"),
                "revision": record.model_dump(mode="json"),
            },
            "status": commit.status.value,
        }
    rejected = application.decide_revision_commit(
        arguments.request_id,
        "reject",
        actor_id=actor,
        state_digest=state_digest,
        action_digest=action_digest,
        reason=reason,
    )
    return EXIT_OK, {
        "command": "commit-revision",
        "data": {"commit": rejected.model_dump(mode="json")},
        "status": rejected.status.value,
    }


def _command_token(application: KalaApplication) -> tuple[int, dict[str, object]]:
    """Print the bearer token of the running launch.

    The token is written by the API process that serves the local API, not by
    this command: asking for it before a launch is an error, because there is
    nothing to hand out and minting one here would produce a token no server
    ever checks.
    """
    settings = application.settings
    token = read_api_token(settings.data_dir)
    if token is None:
        raise ApplicationError(
            "no launch token is stored in the local data directory; start the local api first"
        )
    return EXIT_OK, {
        "command": "token",
        "data": {
            "authentication": "bearer",
            "path": str(settings.data_dir / API_TOKEN_FILE_NAME),
            "token": token,
        },
        "status": "issued",
    }


def build_serve_target(host: str | None = None, port: int | None = None) -> ServeTarget:
    """Return the loopback address this command is allowed to bind.

    A host is only ever a loopback interface: a wildcard or routable address is
    refused here, before a socket exists, because binding either would publish a
    design surface the Host guard is not there to defend. The port is a plain
    bounded TCP port, and nothing else about the address is configurable.
    """
    resolved_host = SERVE_HOST if host is None else host.strip()
    if not is_loopback_host(resolved_host):
        raise ApplicationError(NON_LOOPBACK_HOST_MESSAGE)
    resolved_port = SERVE_PORT if port is None else port
    if isinstance(resolved_port, bool) or not isinstance(resolved_port, int):
        raise ApplicationError(
            f"the port must be an integer between {MIN_SERVE_PORT} and {MAX_SERVE_PORT}"
        )
    if not MIN_SERVE_PORT <= resolved_port <= MAX_SERVE_PORT:
        raise ApplicationError(f"the port must be between {MIN_SERVE_PORT} and {MAX_SERVE_PORT}")
    return ServeTarget(host=resolved_host, port=resolved_port)


def load_uvicorn() -> ModuleType:
    """Return the installed ASGI server module, or refuse to pretend it is there.

    The server is an optional runtime dependency because the library and every
    other command work without it. Its absence is reported as a plain message
    naming the extra to install, never as a traceback and never as a silent
    fallback to something that would open a socket some other way.
    """
    try:
        return import_module(UVICORN_MODULE_NAME)
    except ImportError as error:
        raise ApplicationError(SERVE_UNAVAILABLE_MESSAGE) from error


def build_serve_plan(
    application: KalaApplication,
    *,
    host: str | None = None,
    port: int | None = None,
) -> ServePlan:
    """Bind nothing and open nothing; resolve the launch this command would serve.

    The launch token is minted and written to the local data directory first, so
    the operator can read it with ``kala-agent token`` while the server runs, and
    it is the token the built application then requires. The token itself is
    never returned here, only the path it was stored at.
    """
    target = build_serve_target(host, port)
    data_dir = application.settings.data_dir
    token = issue_api_token(data_dir)
    return ServePlan(
        target=target,
        app=create_app(application, api_token=token),
        token_path=data_dir / API_TOKEN_FILE_NAME,
    )


def _command_serve(
    application: KalaApplication,
    arguments: argparse.Namespace,
) -> tuple[int, dict[str, object]]:
    target = build_serve_target(arguments.host, arguments.port)
    server = load_uvicorn()
    plan = build_serve_plan(application, host=target.host, port=target.port)
    server.run(
        plan.app,
        host=target.host,
        port=target.port,
        log_level=SERVE_LOG_LEVEL,
        access_log=False,
    )
    return EXIT_OK, {
        "command": "serve",
        "data": {
            "api_token_path": str(plan.token_path),
            "authentication": "bearer",
            "host": target.host,
            "loopback_only": True,
            "port": target.port,
            "url": target.url,
        },
        "status": "stopped",
    }


def _dispatch(
    application: KalaApplication,
    arguments: argparse.Namespace,
    created: bool,
) -> tuple[int, dict[str, object]]:
    if arguments.command == "init":
        return _command_init(application, created=created)
    if arguments.command == "token":
        return _command_token(application)
    if arguments.command == "serve":
        return _command_serve(application, arguments)
    if arguments.command == "domains":
        return _command_domains(application)
    if arguments.command == "evaluate":
        return _command_evaluate(application, arguments)
    if arguments.command == "prepare-revision":
        return _command_prepare_revision(application, arguments)
    if arguments.command == "commit-revision":
        return _command_commit_revision(application, arguments)
    if arguments.command == "run-mechanical":
        return _command_run_mechanical(application, arguments)
    if arguments.command == "workflow-status":
        return _command_workflow_status(application, arguments)
    if arguments.command in {"approve", "reject"}:
        return _command_decide(application, arguments, arguments.command)
    return _command_events(application, arguments)


def _actor_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--actor",
        default=LOCAL_ACTOR_ID,
        help="Local actor identifier recorded with the decision.",
    )
    parser.add_argument(
        "--reason",
        default=None,
        help="Optional operator reason recorded with the decision.",
    )


def build_parser() -> argparse.ArgumentParser:
    """Return the command line parser for this adapter."""
    parser = argparse.ArgumentParser(
        prog=PROGRAM_NAME,
        description=(
            "Local, offline engineering design agent over the declarative domain "
            "packages. Every command prints one JSON document and returns an exit "
            "code. No authentication, no network access."
        ),
        epilog=EPILOG,
    )
    parser.add_argument(
        "--data-dir",
        default=None,
        help=(
            "Local data directory, absolute. Overrides "
            f"{DATA_DIR_ENVIRONMENT_KEY} for this invocation only."
        ),
    )
    subparsers = parser.add_subparsers(dest="command", required=True, metavar="command")

    subparsers.add_parser(
        "init",
        help="Create or verify the local data directory and report local facts.",
        description=(
            "Create the local data directory, database, and artifact store, then "
            "report provider, model, and schema facts without printing credentials."
        ),
    )
    subparsers.add_parser(
        "domains",
        help="List every registered domain package and workflow definition.",
        description="List the registered domain packages, their digests, and workflows.",
    )
    subparsers.add_parser(
        "token",
        help="Print the bearer token of the running local API launch.",
        description=(
            "Read and print the bearer token the local API stored in the data directory "
            "when it started. This is the only place the token is printed; it is never "
            "logged, and no other command reports it."
        ),
    )
    serve = subparsers.add_parser(
        "serve",
        help="Serve the local API on the loopback interface until interrupted.",
        description=(
            "Mint a launch token, write it to the local data directory, and serve the "
            "local API on the loopback interface with the same Host guard and the same "
            "per-launch bearer token every other launch carries. The address is "
            "loopback only: a wildcard or routable bind is refused. Requires the "
            "optional uvicorn runtime dependency."
        ),
    )
    serve.add_argument(
        "--host",
        default=SERVE_HOST,
        help=(
            "Loopback address to bind, either 127.0.0.1 or ::1. Any other address, "
            "including a wildcard, is refused."
        ),
    )
    serve.add_argument(
        "--port",
        type=int,
        default=SERVE_PORT,
        help=f"TCP port to bind, {MIN_SERVE_PORT} to {MAX_SERVE_PORT}.",
    )

    evaluate = subparsers.add_parser(
        "evaluate",
        help="Evaluate one reference design of a domain package.",
        description=(
            "Run the deterministic reference evaluation of one domain package and "
            "print its bounded surrogate metrics and findings."
        ),
    )
    evaluate.add_argument(
        "--domain",
        required=True,
        help="Registered domain package name.",
    )
    evaluate.add_argument(
        "--design-id",
        default=None,
        help="Design identifier to evaluate the reference parameters under.",
    )
    evaluate.add_argument(
        "--param",
        action="append",
        default=[],
        metavar="NAME=VALUE",
        help="Parameter override, repeatable. Values are parsed as JSON when possible.",
    )
    evaluate.add_argument(
        "--parameters-json",
        default=None,
        help="A JSON object of parameter overrides.",
    )

    run = subparsers.add_parser(
        "run-mechanical",
        help="Run the mechanical reference workflow to its policy checkpoint.",
        description=(
            "Run the mechanical reference workflow until it pauses at the declared "
            "pre-commit policy checkpoint, or until it is completed."
        ),
    )
    run.add_argument(
        "--design-id",
        default="mechanical-reference",
        help="Design identifier the run belongs to.",
    )
    run.add_argument(
        "--workflow-id",
        default=None,
        help="Workflow identifier. Defaults to the identifier derived from the design.",
    )
    _actor_arguments(run)

    status = subparsers.add_parser(
        "workflow-status",
        help="Print the current state of one workflow.",
        description="Print the current state of one stored workflow run.",
    )
    status.add_argument(
        "--workflow-id",
        required=True,
        help="Workflow identifier to inspect.",
    )

    approve = subparsers.add_parser(
        "approve",
        help="Approve the pending policy checkpoint of a workflow.",
        description=(
            "Approve the pending policy checkpoint, drive the workflow to "
            "completion, and commit the declared design revision."
        ),
    )
    approve.add_argument(
        "--workflow-id",
        required=True,
        help="Workflow identifier whose checkpoint should be approved.",
    )
    _actor_arguments(approve)

    reject = subparsers.add_parser(
        "reject",
        help="Reject the pending policy checkpoint of a workflow.",
        description=(
            "Reject the pending policy checkpoint. The workflow ends as failed and "
            "no revision is committed."
        ),
    )
    reject.add_argument(
        "--workflow-id",
        required=True,
        help="Workflow identifier whose checkpoint should be rejected.",
    )
    _actor_arguments(reject)

    prepare_revision = subparsers.add_parser(
        "prepare-revision",
        help="Prepare one revision commit and open its pre-commit checkpoint.",
        description=(
            "Derive one design revision commit, publish the digests its approval is bound to, "
            "and write nothing to the design. Follow it with commit-revision."
        ),
    )
    prepare_revision.add_argument(
        "--design-id",
        required=True,
        help="Stable design identifier the revision belongs to.",
    )
    prepare_revision.add_argument(
        "--domain",
        required=True,
        help="Registered domain package name.",
    )
    prepare_revision.add_argument(
        "--summary",
        default=None,
        help="Optional summary recorded with the revision specification.",
    )
    prepare_revision.add_argument(
        "--parent-revision-id",
        default=None,
        help="Revision this commit extends. Defaults to the current head of the design.",
    )
    prepare_revision.add_argument(
        "--param",
        action="append",
        default=[],
        metavar="NAME=VALUE",
        help="Parameter override, repeatable. Values are parsed as JSON when possible.",
    )
    prepare_revision.add_argument(
        "--parameters-json",
        default=None,
        help="A JSON object of parameter overrides.",
    )

    commit_revision = subparsers.add_parser(
        "commit-revision",
        help="Approve or reject one prepared revision commit.",
        description=(
            "Record a decision for a prepared revision commit. An approval is honoured only "
            "against the digests the state carries now, so a design head that moved since the "
            "commit was prepared writes nothing. A rejection never writes."
        ),
    )
    commit_revision.add_argument(
        "--request-id",
        required=True,
        help="Identifier of the prepared revision commit.",
    )
    commit_revision.add_argument(
        "--decision",
        choices=REVISION_DECISIONS,
        default=DEFAULT_REVISION_DECISION,
        help="Record an approval or a rejection against the prepared digests.",
    )
    commit_revision.add_argument(
        "--state-digest",
        default=None,
        help="State digest the operator reviewed. Refused when it is stale.",
    )
    commit_revision.add_argument(
        "--action-digest",
        default=None,
        help="Action digest the operator reviewed. Refused when it is stale.",
    )
    _actor_arguments(commit_revision)

    events = subparsers.add_parser(
        "events",
        help="Print a bounded page of journal events.",
        description="Print journal events in sequence order, newest page last.",
    )
    events.add_argument(
        "--after-sequence",
        type=int,
        default=0,
        help="Return events after this sequence number.",
    )
    events.add_argument(
        "--limit",
        type=int,
        default=DEFAULT_EVENT_LIMIT,
        help=f"Maximum number of events to return, 1 to {MAX_EVENT_LIMIT}.",
    )
    events.add_argument(
        "--workflow-id",
        default=None,
        help="Restrict events to one workflow.",
    )
    events.add_argument(
        "--event-type",
        default=None,
        help="Restrict events to one event type.",
    )
    events.add_argument(
        "--aggregate-id",
        default=None,
        help="Restrict events to one aggregate.",
    )
    return parser


def main(
    argv: Sequence[str] | None = None,
    *,
    application: KalaApplication | None = None,
    environ: Mapping[str, str] | None = None,
    stdout: TextIO | None = None,
    stderr: TextIO | None = None,
) -> int:
    """Run one command and return its exit code.

    An application supplied by a caller is never closed by this function; the
    caller owns it.
    """
    out = sys.stdout if stdout is None else stdout
    err = sys.stderr if stderr is None else stderr
    parser = build_parser()
    arguments = parser.parse_args(list(argv) if argv is not None else None)
    resolved: KalaApplication | None = application
    owns = resolved is None
    created = False
    try:
        arguments.actor = _actor(getattr(arguments, "actor", LOCAL_ACTOR_ID))
        arguments.reason = _reason(getattr(arguments, "reason", None))
        if resolved is None:
            settings = _settings_for(arguments.data_dir, environ)
            created = not settings.data_dir.exists()
            resolved = KalaApplication(settings)
        status_code, payload = _dispatch(resolved, arguments, created)
    except ApplicationError as error:
        _emit_error(err, error.code, error.message)
        return EXIT_ERROR
    except ValidationError as error:
        _emit_error(err, "invalid_configuration", _safe_configuration_message(error))
        return EXIT_ERROR
    except (OSError, RuntimeError) as error:
        _emit_error(err, "local_failure", type(error).__name__)
        return EXIT_ERROR
    finally:
        if owns and resolved is not None:
            resolved.close()
    _emit(out, payload)
    return status_code


if __name__ == "__main__":
    raise SystemExit(main())
