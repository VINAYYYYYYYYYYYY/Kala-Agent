"""The Windows-local ``kala-solidedge`` command line.

The command is honest about its platform. ``doctor``, ``build --dry-run``, and
``--help`` are useful on a development host; a real ``build``, ``status``, or
``cancel`` requires Windows and exits ``2`` with a clear message elsewhere.

A build validates the script and shows a review summary before it opens a
socket. ``--yes`` is required when stdin is not a terminal. The journal records
the normalized script digest and the request id, so a retry after an ambiguous
submission reuses the same id instead of creating a second job.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Final, TextIO

from pydantic import ValidationError

from kala_agent.contracts.construction import (
    ConstructionError,
    ConstructionScript,
    load_construction,
)
from kala_solidedge_bridge.protocol import (
    BuildRequest,
    BuildResult,
    BuildStatus,
    Health,
    JobState,
)

from . import oracle
from .client import BridgeClient
from .compile import build_request, new_request_id
from .errors import (
    ArtifactError,
    AuthenticationError,
    ClientError,
    CompileError,
    NetworkError,
    ProtocolError,
    SolidEdgeClientError,
)
from .journal import JOURNAL_FILENAME, Journal, JournalEntry
from .paths import (
    DEFAULT_BRIDGE_URL,
    build_output_dir,
    is_windows,
    resolve_bridge_url,
    resolve_output_dir,
    resolve_token_file,
)

PROGRAM_NAME: Final[str] = "kala-solidedge"
EXIT_OK: Final[int] = 0
EXIT_ERROR: Final[int] = 1
EXIT_USAGE: Final[int] = 2
DEFAULT_POLL_SECONDS: Final[float] = 900.0
DEFAULT_POLL_INTERVAL_SECONDS: Final[float] = 2.0
_TERMINAL_STATES: Final[frozenset[JobState]] = frozenset(
    {
        JobState.SUCCEEDED,
        JobState.FAILED,
        JobState.CANCELLED,
        JobState.INDETERMINATE,
        JobState.INTERRUPTED,
    }
)
_READY_STATES: Final[dict[str, str]] = {
    "ok": "the bridge reports a calibrated, available host",
    "unreachable": "the bridge is not reachable at the configured loopback URL",
    "auth": "the bridge rejected the token",
    "unsupported": "this host cannot run native Solid Edge automation",
    "uncalibrated": "Solid Edge is present but this host is not calibrated",
    "busy": "another build owns the single Solid Edge worker",
    "version": "the bridge protocol or version does not match this client",
}


@dataclass(frozen=True)
class Context:
    """Everything one command needs, resolved once."""

    bridge_url: str
    token_file: Path | None
    output_dir: Path | None
    stdout: TextIO
    stderr: TextIO
    platform: str | None = None


def main(argv: Sequence[str] | None = None) -> int:
    """Parse arguments and return an exit code."""
    return run(argv, stdout=sys.stdout, stderr=sys.stderr)


def run(
    argv: Sequence[str] | None,
    *,
    stdout: TextIO,
    stderr: TextIO,
    stdin: TextIO | None = None,
    platform: str | None = None,
) -> int:
    """Run one command against injected streams and platform (the test seam)."""
    parser = _parser()
    arguments = parser.parse_args(argv)
    environment = dict(os.environ)
    context = Context(
        bridge_url=resolve_bridge_url(
            getattr(arguments, "bridge_url", None), environment=environment
        ),
        token_file=resolve_token_file(
            getattr(arguments, "token_file", None),
            environment=environment,
            platform=platform,
        ),
        output_dir=resolve_output_dir(
            getattr(arguments, "output_dir", None),
            environment=environment,
            platform=platform,
        ),
        stdout=stdout,
        stderr=stderr,
        platform=platform,
    )
    command = getattr(arguments, "command", None)
    original_stdin = sys.stdin
    if stdin is not None:
        sys.stdin = stdin
    try:
        return _dispatch(command, arguments, context, parser, stdout)
    finally:
        sys.stdin = original_stdin


def _dispatch(
    command: object,
    arguments: argparse.Namespace,
    context: Context,
    parser: argparse.ArgumentParser,
    stdout: TextIO,
) -> int:
    stderr = context.stderr
    try:
        if command == "doctor":
            return _doctor(context)
        if command == "build":
            return _build(arguments, context)
        if command == "status":
            return _status(arguments, context)
        if command == "cancel":
            return _cancel(arguments, context)
        parser.print_help(stdout)
        return EXIT_USAGE
    except CompileError as error:
        _print(stderr, f"{PROGRAM_NAME}: refused before submission: {error}")
        return EXIT_ERROR
    except AuthenticationError as error:
        _print(stderr, f"{PROGRAM_NAME}: authentication: {error}")
        return EXIT_ERROR
    except ProtocolError as error:
        _print(stderr, f"{PROGRAM_NAME}: protocol: {error}")
        return EXIT_ERROR
    except NetworkError as error:
        _print(stderr, f"{PROGRAM_NAME}: network: {error}")
        return EXIT_ERROR
    except (ClientError, ArtifactError) as error:
        _print(stderr, f"{PROGRAM_NAME}: {error}")
        return EXIT_ERROR
    except SolidEdgeClientError as error:  # pragma: no cover - base class fallback
        _print(stderr, f"{PROGRAM_NAME}: {error}")
        return EXIT_ERROR


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog=PROGRAM_NAME,
        description=(
            "Compile a Kala construction script and drive the local Solid Edge bridge. "
            "The bridge must run in the same logged-in Windows desktop session."
        ),
        epilog=(
            "Current behavior: a real build, status, and cancel require Windows. "
            "build --dry-run compiles and prints the review summary without a token or "
            "a network connection. Prompt and Studio modes are not implemented; this "
            "command is the supported surface."
        ),
    )
    common = argparse.ArgumentParser(add_help=False)
    _add_connection_arguments(common, suppress=True)
    _add_connection_arguments(parser, suppress=False)
    subparsers = parser.add_subparsers(dest="command", metavar="COMMAND")

    doctor = subparsers.add_parser(
        "doctor",
        parents=[common],
        help="diagnose the bridge, token, host support, calibration, and busy state",
    )
    doctor.set_defaults(handler="doctor")

    build = subparsers.add_parser(
        "build",
        parents=[common],
        help="compile and submit a construction script",
    )
    build.add_argument("--script", required=True, help="path to a ConstructionScript JSON file")
    build.add_argument(
        "--dry-run",
        action="store_true",
        help="validate and print the review summary without a token or a socket",
    )
    build.add_argument(
        "--yes",
        action="store_true",
        help="confirm the review summary non-interactively",
    )
    build.add_argument(
        "--calibration-id",
        default=None,
        help="calibration id reported by doctor; required for a native build",
    )
    build.add_argument(
        "--timeout",
        type=float,
        default=DEFAULT_POLL_SECONDS,
        help="overall polling deadline in seconds (default: 900)",
    )

    status = subparsers.add_parser(
        "status",
        parents=[common],
        help="show one build's status",
    )
    status.add_argument("build_id", help="build id returned by build")

    cancel = subparsers.add_parser(
        "cancel",
        parents=[common],
        help="ask the bridge to cancel one build",
    )
    cancel.add_argument("build_id", help="build id returned by build")
    return parser


def _add_connection_arguments(
    parser: argparse.ArgumentParser,
    *,
    suppress: bool,
) -> None:
    """Add the shared connection flags so either position parses."""
    default: object = argparse.SUPPRESS if suppress else None
    parser.add_argument(
        "--bridge-url",
        default=default,
        help=f"loopback bridge URL (default: ${'KALA_SOLIDEDGE_BRIDGE_URL'} or {DEFAULT_BRIDGE_URL})",
    )
    parser.add_argument(
        "--token-file",
        default=default,
        help=(
            "bearer token file (default: $KALA_SOLIDEDGE_TOKEN_FILE, or "
            "%%LOCALAPPDATA%%/Kala/SolidEdgeBridge/bridge.token on Windows)"
        ),
    )
    parser.add_argument(
        "--output-dir",
        default=default,
        help=(
            "artifact directory (default: %%LOCALAPPDATA%%/Kala/SolidEdge/runs on Windows; "
            "required elsewhere for a real build)"
        ),
    )


# ---------------------------------------------------------------------- commands


def _doctor(context: Context) -> int:
    if context.token_file is None:
        if not is_windows(context.platform):
            document = _doctor_document(
                "unsupported",
                "this development host has no Windows token path and no local Solid Edge "
                "runtime; pass --token-file to probe a reachable bridge",
                context.bridge_url,
            )
            _print(context.stdout, json.dumps(document, indent=2))
            return EXIT_ERROR
        _print(
            context.stderr,
            f"{PROGRAM_NAME}: no token file is configured; "
            "pass --token-file or set KALA_SOLIDEDGE_TOKEN_FILE",
        )
        return EXIT_ERROR
    try:
        client = BridgeClient.from_token_file(
            context.token_file,
            base_url=context.bridge_url,
        )
        health = client.health()
    except AuthenticationError as error:
        _print(
            context.stdout,
            json.dumps(_doctor_document("auth", str(error), context.bridge_url), indent=2),
        )
        return EXIT_ERROR
    except NetworkError as error:
        _print(
            context.stdout,
            json.dumps(_doctor_document("unreachable", str(error), context.bridge_url), indent=2),
        )
        return EXIT_ERROR
    except ProtocolError as error:
        _print(
            context.stdout,
            json.dumps(_doctor_document("version", str(error), context.bridge_url), indent=2),
        )
        return EXIT_ERROR
    condition = _condition(health)
    document = _doctor_document(condition, _READY_STATES[condition], context.bridge_url)
    document.update(_health_document(health))
    _print(context.stdout, json.dumps(document, indent=2))
    return EXIT_OK if condition == "ok" else EXIT_ERROR


def _condition(health: Health) -> str:
    if health.platform != "win32":
        return "unsupported"
    if not health.available or not health.supported:
        return "unsupported"
    if health.busy:
        return "busy"
    if health.calibration_id is None:
        return "uncalibrated"
    return "ok"


def _doctor_document(condition: str, detail: str, bridge_url: str) -> dict[str, object]:
    return {"condition": condition, "detail": detail, "bridge_url": bridge_url}


def _health_document(health: Health) -> dict[str, object]:
    return {
        "bridge_version": health.bridge_version,
        "platform": health.platform,
        "supported": health.supported,
        "available": health.available,
        "busy": health.busy,
        "se_version": health.se_version,
        "calibration_id": health.calibration_id,
        "capabilities": list(health.capabilities),
        "bridge_detail": health.detail,
    }


def _build(arguments: argparse.Namespace, context: Context) -> int:
    script = _load_script(Path(arguments.script))
    request_id = new_request_id()
    try:
        request = build_request(
            script,
            request_id=request_id,
            calibration_id=arguments.calibration_id,
        )
    except ValidationError as error:
        raise CompileError(str(error)) from None

    _print(context.stdout, json.dumps(_review(script, request), indent=2))
    if arguments.dry_run:
        return EXIT_OK

    if not is_windows(context.platform):
        _print(
            context.stderr,
            f"{PROGRAM_NAME}: a real build requires Windows and a running local bridge; "
            "use --dry-run on this host",
        )
        return EXIT_USAGE
    if arguments.calibration_id is None:
        raise ClientError(
            "a native build requires --calibration-id from doctor; the bridge refuses an "
            "uncalibrated host"
        )
    if not arguments.yes and not _confirmed(context):
        _print(context.stderr, f"{PROGRAM_NAME}: build not confirmed; pass --yes")
        return EXIT_ERROR
    if context.token_file is None:
        raise AuthenticationError(
            "no token file is configured; pass --token-file or set KALA_SOLIDEDGE_TOKEN_FILE"
        )
    if context.output_dir is None:
        raise ClientError("no output directory is configured; pass --output-dir")

    client = BridgeClient.from_token_file(context.token_file, base_url=context.bridge_url)
    # Prove the host is ready *before* a journal entry or a submission exists.
    # This is a live check, not a cached claim: a stale calibration id, a busy
    # bridge, or an unsupported host must never create a job.
    _require_ready(client, arguments.calibration_id)

    journal = Journal(context.output_dir / JOURNAL_FILENAME)
    digest = _script_digest(script)
    request_digest = request.request_digest()
    bridge_target = _bridge_target(client, request)
    fresh_request_id = request_id
    request_id = _reusable_request_id(
        journal, digest, request_digest, bridge_target, fresh_request_id
    )
    if request_id == fresh_request_id:
        # No exact replay was found. A legacy unfinished entry cannot be proven
        # to be this request, and submitting a fresh id could duplicate a job
        # that already reached the bridge, so refuse until it is resolved.
        legacy = _legacy_ambiguous_entry(journal, digest)
        if legacy is not None:
            raise ClientError(_legacy_refusal_message(legacy, journal))
    request = request.model_copy(update={"request_id": request_id})

    # Record the intent *before* the socket: if the submission is lost, the
    # retry finds this id and resends the same semantic request instead of
    # minting a second one. The normalized script travels with the entry so an
    # operator can see exactly what was sent without the token or the artifacts.
    journal.append(
        JournalEntry(
            script_digest=digest,
            request_id=request_id,
            build_id=None,
            status="pending",
            summary=request.summary[:200],
            request_digest=request_digest,
            bridge_target=bridge_target,
            script=_normalized_script(script),
        )
    )
    accepted = client.submit(request)
    journal.update(request_id, build_id=accepted.build_id, status=accepted.state.value)
    status = client.wait(
        accepted.build_id,
        timeout=arguments.timeout,
        interval=DEFAULT_POLL_INTERVAL_SECONDS,
    )
    journal.update(request_id, build_id=status.build_id, status=status.state.value)
    return _report(context, client, script, status, context.output_dir)


def _status(arguments: argparse.Namespace, context: Context) -> int:
    if not is_windows(context.platform):
        _print(
            context.stderr,
            f"{PROGRAM_NAME}: status requires Windows and a running local bridge",
        )
        return EXIT_USAGE
    client = _client(context)
    status = client.status(arguments.build_id)
    _print(context.stdout, json.dumps(_status_document(status), indent=2))
    return EXIT_OK if status.state is JobState.SUCCEEDED else EXIT_ERROR


def _cancel(arguments: argparse.Namespace, context: Context) -> int:
    if not is_windows(context.platform):
        _print(
            context.stderr,
            f"{PROGRAM_NAME}: cancel requires Windows and a running local bridge",
        )
        return EXIT_USAGE
    client = _client(context)
    status = client.cancel(arguments.build_id)
    _print(context.stdout, json.dumps(_status_document(status), indent=2))
    _print(
        context.stderr,
        f"{PROGRAM_NAME}: cancellation is cooperative; a COM call already in progress is "
        "not proven to have stopped",
    )
    return EXIT_OK


def _client(context: Context) -> BridgeClient:
    if context.token_file is None:
        raise AuthenticationError(
            "no token file is configured; pass --token-file or set KALA_SOLIDEDGE_TOKEN_FILE"
        )
    return BridgeClient.from_token_file(context.token_file, base_url=context.bridge_url)


# ---------------------------------------------------------------------- readiness


def _require_ready(client: BridgeClient, calibration_id: str) -> Health:
    """Refuse a build unless the live bridge reports this host ready.

    The bridge already refuses at admission, but the client must not create a
    journal entry or a submission for a host that cannot accept one. Every
    condition is checked explicitly so the operator gets the precise reason
    instead of a generic 503.
    """
    health = client.health()
    if health.platform != "win32":
        raise ClientError(
            "the bridge reports a non-Windows platform; a native Solid Edge build requires "
            "Windows and a local Solid Edge desktop session"
        )
    if not health.supported:
        raise ClientError(
            "the bridge reports no live-verified native build executor; a build would fail "
            f"honestly instead of producing geometry ({health.detail or 'no detail'})"
        )
    if not health.available:
        raise ClientError(
            "the bridge cannot reach Solid Edge in this desktop session"
            + (f": {health.detail}" if health.detail else "")
        )
    if health.busy:
        raise ClientError("the bridge is busy with another build; wait for it to finish")
    if not health.calibration_id:
        raise ClientError("the bridge reports no calibration; run the probe and doctor first")
    if health.calibration_id != calibration_id:
        raise ClientError(
            f"the calibration id {calibration_id!r} is stale; doctor reports "
            f"{health.calibration_id!r}. Re-run doctor and pass the current id"
        )
    return health


def _bridge_target(client: BridgeClient, request: BuildRequest) -> str:
    """A normalized identity for the bridge a request is bound to.

    The journal reuses a request id only for the same semantic request against
    the same bridge URL and calibration; a different bridge or a re-probed host
    must not silently inherit an old job.
    """
    calibration = request.calibration_id or ""
    return f"{client.base_url}|{calibration}"


def _reusable_request_id(
    journal: Journal,
    script_digest: str,
    request_digest: str,
    bridge_target: str,
    fresh: str,
) -> str:
    """Reuse the id of an unfinished identical request, else use a fresh one.

    Reuse requires an exact semantic match: the same normalized script, the same
    canonical request digest (which covers calibration, timeout, and procedure),
    and the same bridge target. A legacy entry without those fields is
    ambiguous, so it is never reused: an old job must not be mistaken for this
    one. Reusing the id after a lost submission is what keeps one semantic build
    from becoming two jobs.
    """
    entry = journal.find_exact(
        script_digest=script_digest,
        request_digest=request_digest,
        bridge_target=bridge_target,
    )
    if entry is None:
        return fresh
    if entry.status not in {"pending", "queued", "running", "indeterminate", "interrupted"}:
        return fresh
    return entry.request_id


def _legacy_ambiguous_entry(
    journal: Journal,
    script_digest: str,
) -> JournalEntry | None:
    """Return any unfinished same-script entry that has no exact identity.

    Such an entry cannot be proven to be this semantic request. Because a fresh
    request id could duplicate a job that already reached the bridge, the CLI
    refuses to submit until the operator resolves it; this helper only locates
    the entry so the refusal can say exactly which id is in question.
    """
    for entry in reversed(journal.read()):
        if entry.script_digest != script_digest:
            continue
        if entry.status not in {"pending", "queued", "running", "indeterminate", "interrupted"}:
            continue
        if entry.request_digest and entry.bridge_target:
            # A complete identity is a different semantic request (for example a
            # changed calibration), not a legacy ambiguous entry.
            continue
        return entry
    return None


def _legacy_refusal_message(entry: JournalEntry, journal: Journal) -> str:
    """Explain how to resolve an ambiguous pre-upgrade journal entry."""
    if entry.build_id:
        resolution = f"run `{PROGRAM_NAME} status {entry.build_id}` in the desktop session"
    else:
        resolution = (
            f"find request id {entry.request_id} in the bridge ledger and reconcile or cancel it"
        )
    return (
        f"the journal has an earlier unfinished submission ({entry.request_id}) for this "
        "script that predates exact request identity, so this client cannot prove whether "
        f"submitting again would duplicate it. Resolve it first: {resolution}; then remove "
        f"that entry from {journal.path.name} (or complete it) before retrying."
    )


def _load_script(path: Path) -> ConstructionScript:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise CompileError(f"the script file {path.name} does not exist") from exc
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise CompileError(f"the script file {path.name} is not readable UTF-8 JSON") from exc
    try:
        return load_construction(payload)
    except ConstructionError as exc:
        raise CompileError(str(exc)) from exc


def _normalized_script(script: ConstructionScript) -> str:
    """The deterministic JSON form of the script, as recorded in the journal."""
    payload = script.model_dump(mode="json")
    return json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def _script_digest(script: ConstructionScript) -> str:
    return hashlib.sha256(_normalized_script(script).encode("utf-8")).hexdigest()


def _review(script: ConstructionScript, request: BuildRequest) -> dict[str, object]:
    return {
        "summary": request.summary,
        "request_id": request.request_id,
        "request_digest": request.request_digest(),
        "components": [component.name for component in script.components],
        "operations": len(request.procedure),
        "assembly": any(op.op.startswith("assembly.") for op in request.procedure),
        "calibration_id": request.calibration_id,
        "timeout_seconds": request.timeout_seconds,
    }


def _confirmed(context: Context) -> bool:
    if not sys.stdin.isatty():
        return False
    _print(context.stdout, "Submit this build to the local Solid Edge bridge? [y/N] ")
    answer = sys.stdin.readline().strip().casefold()
    return answer in {"y", "yes"}


def _report(
    context: Context,
    client: BridgeClient,
    script: ConstructionScript,
    status: BuildStatus,
    output_dir: Path,
) -> int:
    result = status.result
    document = _status_document(status)
    downloads: list[str] = []
    if status.state is JobState.SUCCEEDED:
        if result is None:
            _print(
                context.stderr,
                f"{PROGRAM_NAME}: the bridge reported success for {status.build_id} but "
                "returned no result; the build is not accepted as verified",
            )
            return EXIT_ERROR
        build_dir = build_output_dir(output_dir, status.build_id)
        try:
            for artifact in result.artifacts:
                download = client.download(status.build_id, artifact, build_dir)
                downloads.append(str(download.path))
        except ArtifactError as error:
            _print(
                context.stderr,
                f"{PROGRAM_NAME}: an artifact for {status.build_id} could not be downloaded: "
                f"{error}",
            )
            return EXIT_ERROR
        document["artifacts"] = downloads
        document["output_dir"] = str(build_dir)
    _print(context.stdout, json.dumps(document, indent=2))
    if status.state is JobState.SUCCEEDED:
        return _report_oracle(context, script, result)
    if status.state is JobState.INDETERMINATE:
        _print(
            context.stderr,
            f"{PROGRAM_NAME}: the bridge marked build {status.build_id} indeterminate; keep "
            "this id and reconcile instead of resubmitting",
        )
    return EXIT_ERROR


def _report_oracle(
    context: Context,
    script: ConstructionScript,
    result: BuildResult | None,
) -> int:
    """Compare the result with the closed-form corpus and return the exit code.

    A checked geometry mismatch is a failed build even when the bridge reported
    success. An unchecked script is not a failure, but the message says so
    explicitly: "unchecked" must never be read as "matched".
    """
    if result is None:
        _print(context.stderr, f"{PROGRAM_NAME}: the bridge returned no result to compare")
        return EXIT_ERROR
    report = oracle.compare(script, result)
    if not report.checked:
        for note in report.notes:
            _print(context.stderr, f"{PROGRAM_NAME}: unchecked: {note}")
        _print(
            context.stderr,
            f"{PROGRAM_NAME}: the build succeeded, but the geometry is unchecked and was "
            "not matched against a prediction",
        )
        return EXIT_OK
    for failure in report.failures:
        _print(context.stderr, f"{PROGRAM_NAME}: oracle mismatch: {failure}")
    if report.failures:
        return EXIT_ERROR
    _print(context.stderr, f"{PROGRAM_NAME}: oracle matched the closed-form corpus")
    return EXIT_OK


def _status_document(status: BuildStatus) -> dict[str, object]:
    document: dict[str, object] = {
        "build_id": status.build_id,
        "state": status.state.value,
        "step_index": status.step_index,
        "total": status.total,
        "message": status.message,
    }
    if status.result is not None:
        document["result"] = {
            "parts": [
                {
                    "name": part.name,
                    "valid": part.valid,
                    "volume_mm3": part.volume_mm3,
                    "range_box": {
                        "min_mm": list(part.range_box.min_mm),
                        "max_mm": list(part.range_box.max_mm),
                    },
                }
                for part in status.result.parts
            ],
            "assembly_bounds": (
                None
                if status.result.assembly_bounds is None
                else {
                    "min_mm": list(status.result.assembly_bounds.min_mm),
                    "max_mm": list(status.result.assembly_bounds.max_mm),
                }
            ),
            "artifacts": [
                {"name": artifact.name, "size_bytes": artifact.size_bytes}
                for artifact in status.result.artifacts
            ],
        }
    return document


def _print(stream: TextIO, text: str) -> None:
    stream.write(text.rstrip("\n") + "\n")
    stream.flush()


__all__ = (
    "DEFAULT_POLL_SECONDS",
    "EXIT_ERROR",
    "EXIT_OK",
    "EXIT_USAGE",
    "PROGRAM_NAME",
    "Context",
    "main",
    "run",
)
