"""Bridge command line: ``token``, ``probe``, and ``serve``.

``main`` returns an exit code and the package declares a module entry point.
``token`` is the only command that prints the token. ``probe`` writes a private
diagnostic JSON file. ``serve`` is the Windows-only authenticated loopback
bridge; on any other host it refuses with exit code 2 before resolving the
default data directory or creating a token. Pure tests exercise the HTTP
surface through the factory and server-loop seams instead.
"""

from __future__ import annotations

import argparse
import sys
from collections.abc import Sequence
from datetime import UTC, datetime
from pathlib import Path

from .auth import ensure_token, token_path
from .errors import BridgeError
from .probe import run_probe
from .server import is_windows_host, serve
from .workspace import default_data_dir


def _port(value: str) -> int:
    try:
        port = int(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("port must be an integer") from exc
    if not 1 <= port <= 65535:
        raise argparse.ArgumentTypeError("port must be between 1 and 65535")
    return port


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="kala-se-bridge",
        description=(
            "Local Windows Solid Edge bridge. The runtime target is Windows only; "
            "the serve command refuses non-Windows hosts, while pure tests can "
            "exercise the authenticated HTTP surface through injected factories."
        ),
    )
    parser.add_argument(
        "--version",
        action="version",
        version="%(prog)s 0.1.0",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    token_parser = subparsers.add_parser(
        "token",
        help="create the bearer token if needed and print it (the only command that prints it)",
    )
    token_parser.add_argument("--data-dir", type=Path, default=None)

    probe_parser = subparsers.add_parser(
        "probe",
        help="write host/COM diagnostics and optionally measure a private 10 mm cube",
    )
    probe_parser.add_argument(
        "--launch",
        action="store_true",
        help="launch Solid Edge if it is not already running (otherwise attach only)",
    )
    probe_parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="diagnostic JSON path (default: <data-dir>/probe/probe-<timestamp>.json)",
    )
    probe_parser.add_argument("--data-dir", type=Path, default=None)

    serve_parser = subparsers.add_parser(
        "serve",
        help="run the authenticated loopback bridge",
    )
    serve_parser.add_argument("--port", type=_port, default=8797)
    serve_parser.add_argument("--data-dir", type=Path, default=None)
    return parser


def _probe_output(data_dir: Path) -> Path:
    stamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
    return data_dir / "probe" / f"probe-{stamp}.json"


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    arguments = parser.parse_args(argv)

    # The public serve command is Windows-only. Refuse before resolving the
    # platform default data directory or creating a token on a development host.
    if arguments.command == "serve" and not is_windows_host():
        sys.stderr.write(
            "kala-se-bridge: serve requires Windows; no data directory or token was "
            "created. Use the probe command for this host's diagnostics.\n"
        )
        return 2

    data_dir = arguments.data_dir if arguments.data_dir is not None else default_data_dir()

    if arguments.command == "token":
        try:
            token = ensure_token(data_dir)
        except (OSError, BridgeError) as exc:
            sys.stderr.write(f"kala-se-bridge: cannot create token: {exc}\n")
            return 1
        sys.stdout.write(token + "\n")
        sys.stderr.write(f"token file: {token_path(data_dir)}\n")
        return 0

    if arguments.command == "probe":
        output = arguments.output if arguments.output is not None else _probe_output(data_dir)
        try:
            outcome = run_probe(
                launch=bool(arguments.launch),
                data_dir=data_dir,
                output=output,
            )
        except (OSError, BridgeError) as exc:
            sys.stderr.write(f"kala-se-bridge: probe failed: {exc}\n")
            return 1
        report = outcome.report
        attach = report.get("attach", {})
        attach_status = attach.get("status") if isinstance(attach, dict) else "unknown"
        sys.stdout.write(f"probe report: {outcome.output}\n")
        sys.stdout.write(f"attach: {attach_status}\n")
        sys.stdout.write(f"calibration: {'recorded' if outcome.calibration else 'required'}\n")
        sys.stdout.write("native build capability: false (no live-verified executor)\n")
        return outcome.exit_code

    if arguments.command == "serve":
        try:
            return serve(arguments.port, data_dir)
        except OSError as exc:
            sys.stderr.write(f"kala-se-bridge: cannot start the loopback server: {exc}\n")
            return 1

    parser.error("unknown command")


if __name__ == "__main__":  # pragma: no cover - module entry point
    raise SystemExit(main())


__all__ = ("build_parser", "main")
