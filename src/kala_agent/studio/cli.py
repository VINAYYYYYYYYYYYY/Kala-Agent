"""The studio command line adapter.

The CLI resolves a loopback bind, mints the per-launch studio token, and
starts the ASGI server through ``uvicorn``. The server is bound only to
the loopback interface; a wildcard or routable address is refused before
a socket exists. ``uvicorn`` is started without proxy headers because
trusting ``X-Forwarded-Host`` would defeat the Host guard.
"""

from __future__ import annotations

import argparse
import os
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import Final, TextIO

from fastapi import FastAPI

from kala_agent.studio.boundary import (
    NOT_LOOPBACK_MESSAGE,
    STUDIO_TOKEN_ENV_VAR,
    is_loopback_host,
)
from kala_agent.studio.server import create_studio_app
from kala_agent.studio.static import STUDIO_DIST_ENV_VAR, resolve_static_root

PROGRAM_NAME: Final[str] = "python -m kala_agent.studio"
DEFAULT_HOST: Final[str] = "127.0.0.1"
DEFAULT_PORT: Final[int] = 8765
MIN_PORT: Final[int] = 1
MAX_PORT: Final[int] = 65_535
LOG_LEVEL: Final[str] = "warning"
UVICORN_MODULE_NAME: Final[str] = "uvicorn"
EPILOG: Final[str] = (
    "Exit codes: 0 served; 1 the uvicorn module is not installed; 2 a usage error occurred."
)

EXIT_OK: Final[int] = 0
EXIT_ERROR: Final[int] = 1
EXIT_USAGE: Final[int] = 2


@dataclass(frozen=True, slots=True)
class ServeTarget:
    host: str
    port: int

    @property
    def url(self) -> str:
        authority = f"[{self.host}]" if ":" in self.host else self.host
        return f"http://{authority}:{self.port}"


def build_target(host: str | None = None, port: int | None = None) -> ServeTarget:
    """Resolve a loopback host/port pair, refusing anything else."""
    resolved_host = DEFAULT_HOST if host is None else host.strip()
    if not is_loopback_host(resolved_host):
        raise ValueError(NOT_LOOPBACK_MESSAGE)
    resolved_port = DEFAULT_PORT if port is None else port
    if isinstance(resolved_port, bool) or not isinstance(resolved_port, int):
        raise ValueError(f"port must be an integer between {MIN_PORT} and {MAX_PORT}")
    if not MIN_PORT <= resolved_port <= MAX_PORT:
        raise ValueError(f"port must be between {MIN_PORT} and {MAX_PORT}")
    return ServeTarget(host=resolved_host, port=resolved_port)


def load_uvicorn() -> ModuleType:
    try:
        from importlib import import_module

        return import_module(UVICORN_MODULE_NAME)
    except ImportError as error:
        raise RuntimeError(
            "the studio CLI needs uvicorn; install it with 'pip install kala-agent[serve]'"
        ) from error


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog=PROGRAM_NAME,
        description=(
            "Run the Kala Studio standalone HTTP service on a loopback "
            "interface. The host defaults to 127.0.0.1 and is refused for "
            "anything else; the token is taken from KALA_STUDIO_TOKEN or "
            "minted into the studio data directory."
        ),
        epilog=EPILOG,
    )
    parser.add_argument(
        "--host",
        default=DEFAULT_HOST,
        help="loopback address; refused if not 127.0.0.1, ::1, or localhost",
    )
    parser.add_argument(
        "--port",
        type=int,
        default=DEFAULT_PORT,
        help=f"TCP port to bind, {MIN_PORT} to {MAX_PORT}",
    )
    parser.add_argument(
        "--data-dir",
        default=None,
        help=(
            "absolute path to the studio data directory; defaults to the "
            "standard Kala data directory or the system temporary folder"
        ),
    )
    parser.add_argument(
        "--static-dist",
        default=None,
        help=(
            "override the static UI directory; otherwise the studio "
            "uses the frozen or repo-relative dist directory"
        ),
    )
    return parser


def _resolve_data_dir(arguments: argparse.Namespace) -> Path:
    raw = arguments.data_dir
    if raw:
        return Path(raw).expanduser().resolve(strict=False)
    from kala_agent.config import Settings
    from kala_agent.studio.boundary import resolve_studio_data_dir

    try:
        settings = Settings.from_env()
        return resolve_studio_data_dir(settings.data_dir)
    except Exception:
        import tempfile

        return Path(tempfile.gettempdir()) / "kala-studio"


def _resolve_static_root(arguments: argparse.Namespace) -> Path | None:
    if arguments.static_dist:
        return Path(arguments.static_dist).expanduser().resolve(strict=False)
    return resolve_static_root(os.environ)


def main(
    argv: Sequence[str] | None = None,
    *,
    stdout: TextIO | None = None,
    stderr: TextIO | None = None,
) -> int:
    """Run ``python -m kala_agent.studio`` and return the process exit code."""
    out = sys.stdout if stdout is None else stdout
    err = sys.stderr if stderr is None else stderr
    parser = build_parser()
    arguments = parser.parse_args(list(argv) if argv is not None else None)
    try:
        target = build_target(arguments.host, arguments.port)
    except ValueError as error:
        err.write(f"{NOT_LOOPBACK_MESSAGE}: {error}\n")
        return EXIT_USAGE
    try:
        uvicorn = load_uvicorn()
    except RuntimeError as error:
        err.write(f"{error}\n")
        return EXIT_ERROR
    data_dir = _resolve_data_dir(arguments)
    static_root = _resolve_static_root(arguments)
    app: FastAPI = create_studio_app(
        data_dir=data_dir,
        static_root=static_root,
    )
    _emit(
        out,
        {
            "host": target.host,
            "loopback": True,
            "port": target.port,
            "url": target.url,
            "token_environment_variable": STUDIO_TOKEN_ENV_VAR,
            "static_dist_environment_variable": STUDIO_DIST_ENV_VAR,
            "data_dir": str(data_dir),
        },
    )
    try:
        uvicorn.run(
            app,
            host=target.host,
            port=target.port,
            log_level=LOG_LEVEL,
            access_log=False,
            proxy_headers=False,
            forwarded_allow_ips="",  # explicit: refuse forwarded headers
        )
    except OSError as error:
        err.write(f"the studio could not bind {target.url}: {error}\n")
        return EXIT_ERROR
    return EXIT_OK


def _emit(stream: TextIO, payload: dict[str, object]) -> None:
    import json

    stream.write(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    stream.flush()


__all__ = (
    "DEFAULT_HOST",
    "DEFAULT_PORT",
    "PROGRAM_NAME",
    "build_parser",
    "build_target",
    "load_uvicorn",
    "main",
)


if __name__ == "__main__":  # pragma: no cover - module entry point
    raise SystemExit(main())
