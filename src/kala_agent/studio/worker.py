"""Process lifecycle for the Studio geometry worker (ADR-0010).

``python -m kala_agent.studio --worker`` starts this module. The worker binds
``127.0.0.1:0``, prints exactly one readiness line to stdout once the socket is
listening and uvicorn has started, then redirects process stdout to stderr so
neither uvicorn nor the geometry kernel can add a second stdout line.

The worker token is read only from ``KALA_WORKER_TOKEN``; it is never minted,
written to disk, or accepted from argv. A missing token, or one shorter than
32 characters, refuses startup before a socket exists.

The data directory is explicit. The CLI resolves ``--data-dir`` (or
``KALA_STUDIO_DATA_DIR``) for ``--worker`` and refuses to start when neither
names one; the worker never falls back to the home directory or a shared
temporary directory. Its journal, artifacts and settings therefore live in
exactly the front door's data directory, never in an unrelated shared store.

The process exits cleanly (status 0) on SIGTERM or SIGINT. When
``KALA_WORKER_STDIN_LIFELINE=1`` it also exits when stdin reaches EOF, so a
supervisor that dies cannot leave an orphaned worker behind. The design service
is closed by the FastAPI lifespan on the way out.
"""

from __future__ import annotations

import json
import os
import signal
import socket
import sys
import threading
import time
from collections.abc import Mapping
from pathlib import Path
from types import FrameType, ModuleType
from typing import Final, TextIO

from kala_agent.studio.cli import EXIT_ERROR, EXIT_OK, EXIT_USAGE, load_uvicorn
from kala_agent.studio.design.service import design_available
from kala_agent.studio.internal_api import (
    WORKER_HOST,
    WORKER_READY_PREFIX,
    WORKER_STDIN_LIFELINE_ENV_VAR,
    WORKER_TOKEN_ENV_VAR,
    create_worker_app,
    validate_worker_token,
)
from kala_agent.studio.server import STUDIO_VERSION

START_TIMEOUT_SECONDS: Final[float] = 20.0
SHUTDOWN_TIMEOUT_SECONDS: Final[float] = 10.0
FORCE_SHUTDOWN_TIMEOUT_SECONDS: Final[float] = 2.0
LISTEN_BACKLOG: Final[int] = 128
LOG_LEVEL: Final[str] = "warning"

__all__ = (
    "FORCE_SHUTDOWN_TIMEOUT_SECONDS",
    "LISTEN_BACKLOG",
    "SHUTDOWN_TIMEOUT_SECONDS",
    "START_TIMEOUT_SECONDS",
    "readiness_payload",
    "run_worker",
)


def readiness_payload(port: int) -> dict[str, object]:
    """Return the documented readiness object for the bound port."""
    return {
        "port": port,
        "version": STUDIO_VERSION,
        "design_available": design_available(),
    }


def run_worker(
    *,
    data_dir: Path,
    environ: Mapping[str, str] | None = None,
    uvicorn_module: ModuleType | None = None,
    stdin: TextIO | None = None,
    stdout: TextIO | None = None,
    stderr: TextIO | None = None,
) -> int:
    """Run the geometry worker until SIGTERM/SIGINT or the stdin lifeline ends."""
    out = sys.stdout if stdout is None else stdout
    err = sys.stderr if stderr is None else stderr
    source = environ if environ is not None else os.environ
    try:
        token = validate_worker_token(source.get(WORKER_TOKEN_ENV_VAR))
    except ValueError as error:
        err.write(f"{WORKER_TOKEN_ENV_VAR}: {error}\n")
        err.flush()
        return EXIT_USAGE
    module = uvicorn_module if uvicorn_module is not None else _load_uvicorn(err)
    if module is None:
        return EXIT_ERROR
    resolved_dir = Path(data_dir).expanduser().resolve(strict=False)
    try:
        resolved_dir.mkdir(parents=True, exist_ok=True)
    except OSError as error:
        err.write(f"the geometry worker data directory is unusable: {type(error).__name__}\n")
        err.flush()
        return EXIT_ERROR
    try:
        listener = _bind_listener()
    except OSError as error:
        err.write(f"the geometry worker could not bind {WORKER_HOST}:0: {type(error).__name__}\n")
        err.flush()
        return EXIT_ERROR
    port = int(listener.getsockname()[1])
    try:
        app = create_worker_app(
            api_token=token,
            port=port,
            data_dir=resolved_dir,
            environ=source,
        )
    except Exception as error:  # pragma: no cover - startup failures are fatal
        _close(listener)
        err.write(f"the geometry worker could not start: {type(error).__name__}\n")
        err.flush()
        return EXIT_ERROR

    config = module.Config(
        app,
        log_level=LOG_LEVEL,
        access_log=False,
        proxy_headers=False,
        forwarded_allow_ips="",
        timeout_graceful_shutdown=SHUTDOWN_TIMEOUT_SECONDS,
    )
    server = module.Server(config)
    stop = threading.Event()
    failures: list[BaseException] = []

    def serve() -> None:
        try:
            server.run(sockets=[listener])
        except BaseException as error:  # pragma: no cover - uvicorn failure path
            failures.append(error)
        finally:
            stop.set()

    _install_signal_handlers(stop)
    server_thread = threading.Thread(target=serve, name="kala-studio-worker", daemon=True)
    server_thread.start()
    try:
        if not _wait_until_started(server, server_thread, stop, failures):
            if stop.is_set() and not failures:
                return EXIT_OK
            detail = f": {type(failures[0]).__name__}" if failures else ""
            err.write(f"the geometry worker did not start{detail}\n")
            err.flush()
            return EXIT_ERROR
        _emit_readiness(out, port)
        _redirect_stdout_to_stderr(out)
        if source.get(WORKER_STDIN_LIFELINE_ENV_VAR, "").strip() == "1":
            _start_stdin_lifeline(sys.stdin if stdin is None else stdin, stop)
        _wait_for_stop(stop, server_thread)
    finally:
        server.should_exit = True
        server_thread.join(SHUTDOWN_TIMEOUT_SECONDS)
        if server_thread.is_alive():
            server.force_exit = True
            server_thread.join(FORCE_SHUTDOWN_TIMEOUT_SECONDS)
        _close(listener)
    return EXIT_OK


def _load_uvicorn(err: TextIO) -> ModuleType | None:
    try:
        return load_uvicorn()
    except RuntimeError as error:
        err.write(f"{error}\n")
        err.flush()
        return None


def _bind_listener() -> socket.socket:
    """Bind and listen on an ephemeral loopback port, without reusing it."""
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        listener.bind((WORKER_HOST, 0))
        listener.listen(LISTEN_BACKLOG)
    except OSError:
        _close(listener)
        raise
    return listener


def _emit_readiness(stream: TextIO, port: int) -> None:
    payload = json.dumps(readiness_payload(port), sort_keys=True, separators=(",", ":"))
    stream.write(f"{WORKER_READY_PREFIX} {payload}\n")
    stream.flush()


def _redirect_stdout_to_stderr(out: TextIO) -> None:
    """Send every later process-level stdout write to stderr.

    Only the real process stdout is redirected; a caller-supplied stream (a
    unit test) is left untouched. The fd-level replacement also catches writes
    from C libraries such as the OCP STEP writer.
    """
    if out is not sys.stdout:
        return
    try:
        sys.stdout.flush()
    except (OSError, ValueError):
        pass
    try:
        os.dup2(sys.stderr.fileno(), sys.stdout.fileno())
    except (AttributeError, OSError, ValueError):
        sys.stdout = sys.stderr


def _start_stdin_lifeline(stream: TextIO, stop: threading.Event) -> None:
    """Exit when the supervised parent closes the worker's stdin."""

    def watch() -> None:
        try:
            while stream.read(1) != "":
                pass
        except (OSError, ValueError):
            pass
        stop.set()

    threading.Thread(target=watch, name="kala-worker-lifeline", daemon=True).start()


def _wait_until_started(
    server: object,
    server_thread: threading.Thread,
    stop: threading.Event,
    failures: list[BaseException],
) -> bool:
    deadline = time.monotonic() + START_TIMEOUT_SECONDS
    while time.monotonic() < deadline:
        if getattr(server, "started", False):
            return True
        if stop.is_set() or failures or not server_thread.is_alive():
            return False
        time.sleep(0.01)
    return False


def _wait_for_stop(stop: threading.Event, server_thread: threading.Thread) -> None:
    while not stop.wait(0.1):
        if not server_thread.is_alive():
            return


def _install_signal_handlers(stop: threading.Event) -> None:
    if threading.current_thread() is not threading.main_thread():
        return

    def handle(signum: int, frame: FrameType | None) -> None:
        stop.set()

    for candidate in (signal.SIGTERM, signal.SIGINT):
        try:
            signal.signal(candidate, handle)
        except (OSError, ValueError):  # pragma: no cover - platform-specific
            continue


def _close(listener: socket.socket) -> None:
    try:
        listener.close()
    except OSError:  # pragma: no cover - already closed
        pass
