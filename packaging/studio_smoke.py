"""Runtime smoke validator for the frozen Kala Studio backend.

This is a packaging-owned, stdlib-only harness. It launches an already
built backend executable (``dist/kala-studio`` on POSIX,
``dist/kala-studio.exe`` on Windows), waits for it to come up on a
loopback port, and then exercises the frozen runtime end to end:

1. ``GET /api/studio/health`` returns ``200`` and a JSON body.
2. ``GET /`` returns the bundled UI ``index.html`` (this proves the
   frozen ``sys._MEIPASS``/``studio_dist`` layout resolved, because the
   process is launched from a temporary working directory where the
   repo-relative ``studio/dist`` cannot be discovered).
3. Every ``<script>``/``<link>`` asset referenced by that HTML resolves
   through ``urllib.parse.urljoin`` and returns ``200`` with a sane MIME
   type. The Vite build emits *relative* ``./assets/...`` references, so
   a naive string concatenation is wrong; ``urljoin`` is required.
4. ``POST /api/studio/preview`` returns a real triangle mesh.
5. ``POST /api/studio/export`` returns a real STEP file when the health
   probe advertises ``step_available``. A missing STEP writer fails the
   run unless ``--allow-missing-step`` is passed; CI does **not** pass
   that flag, so a release that cannot write STEP fails closed.

The harness never authenticates against a provider and never touches a
paid endpoint: it uses a synthetic ``KALA_STUDIO_TOKEN`` minted in
process and cleared provider environment variables. The child process is
always stopped (terminate, then bounded kill) in a ``finally`` block.

Usage:

    python packaging/studio_smoke.py --backend dist/kala-studio
    python packaging/studio_smoke.py --backend dist/kala-studio.exe --timeout 90

Exit code ``0`` means every check passed; a non-zero exit means at least
one check failed and the reason was printed to stderr.
"""

from __future__ import annotations

import argparse
import http.client
import json
import os
import re
import secrets
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
from collections.abc import Sequence
from pathlib import Path
from typing import IO, Final, NamedTuple

DEFAULT_HOST: Final[str] = "127.0.0.1"
DEFAULT_TIMEOUT_SECONDS: Final[float] = 60.0
READY_POLL_SECONDS: Final[float] = 0.25
# A well-formed STEP file always starts with this magic string and ends
# with the matching terminator. The smoke does not pin an exact byte
# count: OCP version and header timestamps change the size, and the
# exact geometry round-trip is validated by the studio's own pytest
# suite (tests/unit/test_studio_geometry.py), not by this runtime probe.
SMOKE_BOX: Final[dict[str, float]] = {"size_x": 20.0, "size_y": 30.0, "size_z": 40.0}
STEP_MAGIC: Final[bytes] = b"ISO-10303-21"
STEP_TERMINATOR: Final[bytes] = b"END-ISO-10303-21"
MAX_CAPTURED_LINES: Final[int] = 200
HTML_ASSET_RE: Final[re.Pattern[str]] = re.compile(
    r"""(?:src|href)\s*=\s*["']([^"']+)["']""",
    re.IGNORECASE,
)
JS_MIME_TYPES: Final[frozenset[str]] = frozenset(
    {"text/javascript", "application/javascript", "application/x-javascript"}
)
CSS_MIME_TYPES: Final[frozenset[str]] = frozenset({"text/css"})


class SmokeFailure(RuntimeError):
    """A single smoke assertion failed."""


class HttpResponse(NamedTuple):
    status: int
    headers: dict[str, str]
    body: bytes

    def header(self, name: str) -> str:
        return self.headers.get(name.lower(), "")

    def json(self) -> object:
        try:
            return json.loads(self.body.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise SmokeFailure(f"response body was not JSON: {error}") from error


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise SmokeFailure(message)


def _as_object(payload: object, label: str) -> dict[str, object]:
    if not isinstance(payload, dict):
        raise SmokeFailure(f"{label} was not a JSON object")
    return payload


def _pick_free_port(host: str) -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        probe.bind((host, 0))
        return int(probe.getsockname()[1])


def _request(
    host: str,
    port: int,
    method: str,
    path: str,
    *,
    token: str | None = None,
    payload: dict[str, object] | None = None,
    timeout: float = 10.0,
) -> HttpResponse:
    body = None if payload is None else json.dumps(payload).encode("utf-8")
    headers: dict[str, str] = {}
    if payload is not None:
        headers["content-type"] = "application/json"
    if token is not None:
        headers["authorization"] = f"Bearer {token}"
    connection = http.client.HTTPConnection(host, port, timeout=timeout)
    try:
        connection.request(method, path, body=body, headers=headers)
        response = connection.getresponse()
        raw = response.read()
        lowered = {key.lower(): value for key, value in response.getheaders()}
        return HttpResponse(status=response.status, headers=lowered, body=raw)
    finally:
        connection.close()


def _wait_for_health(
    host: str,
    port: int,
    *,
    deadline: float,
    process: subprocess.Popen[bytes],
) -> dict[str, object]:
    last_error = "no attempt made"
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise SmokeFailure(
                f"backend exited early with code {process.returncode} before becoming ready"
            )
        try:
            response = _request(host, port, "GET", "/api/studio/health", timeout=2.0)
        except (OSError, http.client.HTTPException) as error:
            last_error = str(error)
            time.sleep(READY_POLL_SECONDS)
            continue
        if response.status == 200:
            parsed = _as_object(response.json(), "health body")
            _require(parsed.get("status") == "ok", f"health status was {parsed.get('status')!r}")
            return parsed
        last_error = f"HTTP {response.status}"
        time.sleep(READY_POLL_SECONDS)
    raise SmokeFailure(f"backend did not become healthy within the deadline ({last_error})")


def _drain(stream: IO[bytes] | None, sink: list[str]) -> None:
    if stream is None:
        return
    try:
        for line in iter(stream.readline, b""):
            if len(sink) < MAX_CAPTURED_LINES:
                sink.append(line.decode("utf-8", "replace").rstrip())
    except (OSError, ValueError):
        pass


def _stop(process: subprocess.Popen[bytes], *, grace: float = 8.0) -> None:
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=grace)
        return
    except subprocess.TimeoutExpired:
        pass
    process.kill()
    try:
        process.wait(timeout=grace)
    except subprocess.TimeoutExpired:
        pass


def _check_ui(host: str, port: int) -> None:
    response = _request(host, port, "GET", "/")
    _require(response.status == 200, f"GET / returned {response.status}, expected 200")
    content_type = response.header("content-type")
    _require("text/html" in content_type, f"GET / content-type was {content_type!r}")
    text = response.body.decode("utf-8", "replace")
    _require("<html" in text.lower(), "GET / body did not look like an HTML document")

    base = f"http://{host}:{port}/"
    references = [
        match.group(1)
        for match in HTML_ASSET_RE.finditer(text)
        if match.group(1).split("?")[0].lower().endswith((".js", ".css"))
    ]
    _require(bool(references), "index.html referenced no JS/CSS assets to verify")
    for reference in references:
        url = urllib.parse.urljoin(base, reference)
        parsed = urllib.parse.urlparse(url)
        asset = _request(parsed.hostname or host, parsed.port or port, "GET", parsed.path or "/")
        _require(
            asset.status == 200,
            f"asset {reference!r} returned {asset.status}, expected 200",
        )
        mime = asset.header("content-type").split(";", 1)[0].strip().lower()
        suffix = (parsed.path or "").lower()
        if suffix.endswith(".css"):
            _require(
                mime in CSS_MIME_TYPES,
                f"asset {reference!r} had MIME {mime!r}, expected text/css",
            )
        else:
            _require(
                mime in JS_MIME_TYPES,
                f"asset {reference!r} had MIME {mime!r}, expected a JavaScript MIME type",
            )
        _require(len(asset.body) > 0, f"asset {reference!r} was empty")


def _check_preview(host: str, port: int, token: str) -> None:
    response = _request(
        host,
        port,
        "POST",
        "/api/studio/preview",
        token=token,
        payload={"kind": "box", "parameters": dict(SMOKE_BOX)},
    )
    _require(response.status == 200, f"preview returned {response.status}, expected 200")
    parsed = _as_object(response.json(), "preview body")
    positions = parsed.get("positions")
    normals = parsed.get("normals")
    indices = parsed.get("indices")
    _require(
        isinstance(positions, list) and len(positions) > 0,
        "preview returned no positions",
    )
    _require(
        isinstance(normals, list) and len(normals) > 0,
        "preview returned no normals",
    )
    _require(
        isinstance(indices, list) and len(indices) > 0,
        "preview returned no indices",
    )
    _require(
        isinstance(indices, list) and len(indices) % 3 == 0,
        "preview indices were not a multiple of three",
    )


def _check_step(
    host: str, port: int, token: str, *, step_available: bool, allow_missing: bool
) -> None:
    response = _request(
        host,
        port,
        "POST",
        "/api/studio/export",
        token=token,
        payload={"kind": "box", "parameters": dict(SMOKE_BOX), "format": "step"},
    )
    if not step_available:
        if response.status in {200, 503} and allow_missing:
            print("[smoke] STEP writer not advertised; --allow-missing-step set, skipping.")
            return
        raise SmokeFailure(
            "health did not advertise step_available but the run requires a real STEP export; "
            "install the [geometry] extra or pass --allow-missing-step"
        )
    _require(response.status == 200, f"STEP export returned {response.status}, expected 200")
    content_type = response.header("content-type").split(";", 1)[0].strip().lower()
    _require(
        content_type == "application/step",
        f"STEP export content-type was {content_type!r}, expected application/step",
    )
    disposition = response.header("content-disposition")
    _require("attachment" in disposition, f"STEP export disposition was {disposition!r}")
    _require(
        response.body.startswith(STEP_MAGIC),
        "STEP export body did not start with ISO-10303-21",
    )
    _require(
        STEP_TERMINATOR in response.body,
        "STEP export body did not contain the ISO-10303-21 terminator",
    )
    print(f"[smoke] STEP export: {len(response.body)} bytes, ISO-10303-21 magic ok.")


def _build_env(data_dir: Path, token: str) -> dict[str, str]:
    env = dict(os.environ)
    env["KALA_STUDIO_TOKEN"] = token
    env["KALA_STUDIO_DATA_DIR"] = str(data_dir)
    # The smoke run must never reach a provider endpoint. Clear the API
    # credentials the studio would otherwise treat as configured.
    env.pop("OPENAI_API_KEY", None)
    env.pop("OPENROUTER_API_KEY", None)
    return env


def run(
    backend: Path,
    *,
    host: str = DEFAULT_HOST,
    timeout: float = DEFAULT_TIMEOUT_SECONDS,
    allow_missing_step: bool = False,
) -> int:
    _require(backend.is_file(), f"backend executable not found: {backend}")

    port = _pick_free_port(host)
    token = "kala-smoke-" + secrets.token_hex(16)
    with tempfile.TemporaryDirectory(prefix="kala-studio-smoke-") as raw_tmp:
        workdir = Path(raw_tmp)
        data_dir = workdir / "data"
        data_dir.mkdir(parents=True, exist_ok=True)
        env = _build_env(data_dir, token)

        creation_flags = 0
        if sys.platform.startswith("win"):
            creation_flags = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)

        process = subprocess.Popen(
            [str(backend), "--host", host, "--port", str(port)],
            cwd=str(workdir),
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            creationflags=creation_flags,
        )
        captured: list[str] = []
        reader = threading.Thread(target=_drain, args=(process.stdout, captured), daemon=True)
        reader.start()
        try:
            deadline = time.monotonic() + timeout
            health = _wait_for_health(host, port, deadline=deadline, process=process)
            print(f"[smoke] health ok: {json.dumps(health, sort_keys=True)}")
            _check_ui(host, port)
            print("[smoke] UI root and assets ok.")
            _check_preview(host, port, token)
            print("[smoke] preview mesh ok.")
            _check_step(
                host,
                port,
                token,
                step_available=bool(health.get("step_available")),
                allow_missing=allow_missing_step,
            )
            print("[smoke] PASS")
            return 0
        except SmokeFailure as error:
            print(f"[smoke] FAIL: {error}", file=sys.stderr)
            if captured:
                print("[smoke] backend output:", file=sys.stderr)
                for line in captured[-40:]:
                    print(f"    {line}", file=sys.stderr)
            return 1
        finally:
            _stop(process)
            reader.join(timeout=5.0)


def _parse_args(argv: Sequence[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="studio_smoke",
        description="Launch the frozen Kala Studio backend and validate it end to end.",
    )
    parser.add_argument(
        "--backend",
        required=True,
        help="path to the frozen backend executable (dist/kala-studio[.exe])",
    )
    parser.add_argument("--host", default=DEFAULT_HOST, help="loopback host to bind")
    parser.add_argument(
        "--timeout",
        type=float,
        default=DEFAULT_TIMEOUT_SECONDS,
        help="seconds to wait for the backend to become healthy",
    )
    parser.add_argument(
        "--allow-missing-step",
        action="store_true",
        help="do not fail when the backend cannot write STEP (CI does not pass this)",
    )
    return parser.parse_args(list(argv) if argv is not None else None)


def main(argv: Sequence[str] | None = None) -> int:
    arguments = _parse_args(argv)
    backend = Path(arguments.backend).resolve()
    if backend.suffix == "" and sys.platform.startswith("win"):
        candidate = backend.with_suffix(".exe")
        if candidate.is_file():
            backend = candidate
    if not backend.is_file():
        print(f"[smoke] FAIL: backend executable not found: {backend}", file=sys.stderr)
        return 1
    return run(
        backend,
        host=arguments.host,
        timeout=arguments.timeout,
        allow_missing_step=arguments.allow_missing_step,
    )


if __name__ == "__main__":  # pragma: no cover - module entry point
    sys.exit(main())
