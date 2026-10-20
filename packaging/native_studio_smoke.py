"""Smoke-test the staged native Kala Studio desktop distribution.

This helper validates the native stage produced by
``studio/scripts/native-backend.cjs`` in the same resource layout that
Electron packages:

    <temp>/resources/backend/kala-studio[.exe]
    <temp>/resources/backend/studio_dist/index.html
    <temp>/resources/backend/native-build.json

The staged tree is copied to an unrelated temporary directory, the
process is launched from yet another working directory with ``PATH``
free of Python, and no ``KALA_STUDIO_DIST`` override is supplied. This
proves the static UI resolves through the real packaged layout rather
than a source-relative ``studio/dist`` directory.

The checks are deliberately independent of ``packaging/studio_smoke.py``
(which remains the frozen-backend smoke and is unchanged): health, UI
root and assets, authenticated preview mesh, an unauthenticated refusal,
and a real ISO-10303-21 STEP export. The stage is also inspected for
Python interpreters, site-packages, OCP, PyInstaller and other vendor
Python artifacts.

Usage:

    python packaging/native_studio_smoke.py \
        --stage-dir studio/backend-native --timeout 60

Exit code ``0`` means every check passed. The helper never contacts a
provider, never installs anything, and always terminates the child.
"""

from __future__ import annotations

import argparse
import hashlib
import http.client
import json
import os
import re
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
from collections.abc import Sequence
from pathlib import Path
from typing import Final, NamedTuple

DEFAULT_HOST: Final[str] = "127.0.0.1"
DEFAULT_TIMEOUT_SECONDS: Final[float] = 60.0
READY_POLL_SECONDS: Final[float] = 0.25
SMOKE_BOX: Final[dict[str, float]] = {"size_x": 20.0, "size_y": 30.0, "size_z": 40.0}
STEP_MAGIC: Final[bytes] = b"ISO-10303-21"
STEP_TERMINATOR: Final[bytes] = b"END-ISO-10303-21"
TREE_HASH_PREFIX: Final[bytes] = b"kala-studio-native-tree\n"
HTML_ASSET_RE: Final[re.Pattern[str]] = re.compile(
    r"""(?:src|href)\s*=\s*["']([^"']+)["']""",
    re.IGNORECASE,
)
FORBIDDEN_EXTENSIONS: Final[frozenset[str]] = frozenset({".py", ".pyc", ".pyo", ".pyd"})
FORBIDDEN_NAMES: Final[frozenset[str]] = frozenset(
    {
        "__pycache__",
        "site-packages",
        "dist-info",
        "egg-info",
        "pip",
        "setuptools",
        "pyinstaller",
        "pydantic",
        "fastapi",
        "uvicorn",
        "cadquery",
        "ocp",
        "freecad",
        "numpy",
        "python",
    }
)
FORBIDDEN_PATTERNS: Final[tuple[re.Pattern[str], ...]] = (
    re.compile(r"^python(?:[0-9.]*)?(?:\.exe)?$", re.IGNORECASE),
    re.compile(r"^python[0-9.]*\.(?:dll|so|dylib|pyd)$", re.IGNORECASE),
    re.compile(r"^_?meipass", re.IGNORECASE),
    re.compile(r"^libpython", re.IGNORECASE),
    re.compile(r"^ocp[0-9._-]*\.(?:dll|so|dylib)$", re.IGNORECASE),
)


class SmokeFailure(RuntimeError):
    """A single native-distribution assertion failed."""


class HttpResponse(NamedTuple):
    status: int
    headers: dict[str, str]
    body: bytes

    def header(self, name: str) -> str:
        return self.headers.get(name.lower(), "")


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise SmokeFailure(message)


def _binary_name() -> str:
    return "kala-studio.exe" if sys.platform.startswith("win") else "kala-studio"


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _iter_files(root: Path) -> list[Path]:
    return sorted(
        (path for path in root.rglob("*") if path.is_file()),
        key=lambda p: p.relative_to(root).as_posix(),
    )


def _tree_sha256(root: Path) -> str:
    digest = hashlib.sha256()
    digest.update(TREE_HASH_PREFIX)
    for path in _iter_files(root):
        relative = path.relative_to(root).as_posix()
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(str(path.stat().st_size).encode("ascii"))
        digest.update(b"\0")
        digest.update(_sha256(path).encode("ascii"))
        digest.update(b"\n")
    return digest.hexdigest()


def _forbidden(path: Path, root: Path) -> str | None:
    relative = path.relative_to(root)
    for part in relative.parts:
        if part.lower() in FORBIDDEN_NAMES:
            return part
    if path.suffix.lower() in FORBIDDEN_EXTENSIONS:
        return path.name
    for pattern in FORBIDDEN_PATTERNS:
        if pattern.search(path.name):
            return path.name
    return None


def _validate_stage(
    stage_dir: Path, binary: Path, index: Path, metadata_path: Path
) -> dict[str, object]:
    _require(stage_dir.is_dir(), f"native stage directory is missing: {stage_dir}")
    _require(binary.is_file(), f"native backend binary is missing: {binary}")
    _require(index.is_file(), f"native static index is missing: {index}")
    _require(metadata_path.is_file(), f"native build metadata is missing: {metadata_path}")

    with binary.open("rb") as handle:
        magic = handle.read(4)
    if sys.platform.startswith("win"):
        _require(magic[:2] == b"MZ", f"{binary} is not a Windows PE executable")
    else:
        _require(magic == b"\x7fELF", f"{binary} is not an ELF executable")

    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    _require(isinstance(metadata, dict), "native-build.json was not an object")
    _require(metadata.get("package") == "kala-studio", "native-build.json package mismatch")
    _require(metadata.get("binary") == binary.name, "native-build.json binary name mismatch")
    _require(metadata.get("nativeStep") is True, "native stage was built without native STEP")
    _require(metadata.get("binarySha256") == _sha256(binary), "native binary digest mismatch")
    _require(metadata.get("binaryBytes") == binary.stat().st_size, "native binary size mismatch")
    _require(
        metadata.get("staticDistSha256") == _tree_sha256(index.parent),
        "native static UI digest mismatch",
    )
    symlinks = [path for path in stage_dir.rglob("*") if path.is_symlink()]
    _require(not symlinks, "native stage contained a symlink: " + ", ".join(map(str, symlinks)))

    offenders: list[str] = []
    for path in _iter_files(stage_dir):
        match = _forbidden(path, stage_dir)
        if match:
            offenders.append(f"{path.relative_to(stage_dir)} ({match})")
    _require(
        not offenders,
        "native stage contained Python/OCP/vendor artifacts: " + ", ".join(offenders),
    )

    assets = index.parent / "assets"
    _require(assets.is_dir(), f"static assets directory is missing: {assets}")
    _require(
        any(item.suffix.lower() in {".js", ".css"} for item in assets.iterdir() if item.is_file()),
        "static assets directory has no JS/CSS bundle",
    )
    return metadata


def _copy_packaged_layout(stage_dir: Path, destination: Path) -> Path:
    resources = destination / "resources"
    backend = resources / "backend"
    shutil.copytree(stage_dir, backend)
    return backend


def _pick_free_port(host: str) -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        probe.bind((host, 0))
        return int(probe.getsockname()[1])


def _minimum_environment(data_dir: Path, token: str) -> dict[str, str]:
    environment = {
        "KALA_STUDIO_TOKEN": token,
        "KALA_STUDIO_DATA_DIR": str(data_dir),
        # Prove the native runtime does not need Python on PATH.
        "PATH": "",
        "TMPDIR": str(data_dir.parent),
        "TEMP": str(data_dir.parent),
        "TMP": str(data_dir.parent),
    }
    for key in ("SystemRoot", "SYSTEMROOT", "ComSpec", "WINDIR"):
        if key in os.environ:
            environment[key] = os.environ[key]
    if "HOME" in os.environ:
        environment["HOME"] = os.environ["HOME"]
    return environment


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
                f"native backend exited early with code {process.returncode} before becoming ready"
            )
        try:
            response = _request(host, port, "GET", "/api/studio/health", timeout=2.0)
        except (OSError, http.client.HTTPException) as error:
            last_error = str(error)
            time.sleep(READY_POLL_SECONDS)
            continue
        if response.status == 200:
            payload = json.loads(response.body.decode("utf-8"))
            _require(isinstance(payload, dict), "health body was not a JSON object")
            _require(payload.get("status") == "ok", f"health status was {payload.get('status')!r}")
            return payload
        last_error = f"HTTP {response.status}"
        time.sleep(READY_POLL_SECONDS)
    raise SmokeFailure(f"native backend did not become healthy within the deadline ({last_error})")


def _check_ui(host: str, port: int) -> None:
    response = _request(host, port, "GET", "/")
    _require(response.status == 200, f"GET / returned {response.status}, expected 200")
    _require(
        "text/html" in response.header("content-type"),
        f"GET / content-type was {response.header('content-type')!r}",
    )
    text = response.body.decode("utf-8", "replace")
    _require("<html" in text.lower(), "GET / body did not look like HTML")

    base = f"http://{host}:{port}/"
    references = [
        match.group(1)
        for match in HTML_ASSET_RE.finditer(text)
        if match.group(1).split("?")[0].lower().endswith((".js", ".css"))
    ]
    _require(bool(references), "index.html referenced no JS/CSS assets")
    for reference in references:
        url = urllib.parse.urljoin(base, reference)
        parsed = urllib.parse.urlparse(url)
        asset = _request(parsed.hostname or host, parsed.port or port, "GET", parsed.path or "/")
        _require(asset.status == 200, f"asset {reference!r} returned {asset.status}, expected 200")
        _require(len(asset.body) > 0, f"asset {reference!r} was empty")


def _check_api(host: str, port: int, token: str, *, step_available: bool) -> None:
    refused = _request(
        host,
        port,
        "POST",
        "/api/studio/preview",
        payload={"kind": "box", "parameters": dict(SMOKE_BOX)},
    )
    _require(
        refused.status == 401, f"unauthenticated preview returned {refused.status}, expected 401"
    )

    preview = _request(
        host,
        port,
        "POST",
        "/api/studio/preview",
        token=token,
        payload={"kind": "box", "parameters": dict(SMOKE_BOX)},
    )
    _require(preview.status == 200, f"preview returned {preview.status}, expected 200")
    mesh = json.loads(preview.body.decode("utf-8"))
    _require(isinstance(mesh, dict), "preview body was not an object")
    for field in ("positions", "normals", "indices"):
        value = mesh.get(field)
        _require(isinstance(value, list) and bool(value), f"preview field {field!r} was empty")
    _require(len(mesh["indices"]) % 3 == 0, "preview indices were not a multiple of three")

    step = _request(
        host,
        port,
        "POST",
        "/api/studio/export",
        token=token,
        payload={"kind": "box", "parameters": dict(SMOKE_BOX), "format": "step"},
    )
    _require(step_available, "health did not advertise the native STEP writer")
    _require(step.status == 200, f"STEP export returned {step.status}, expected 200")
    _require(
        step.header("content-type").split(";", 1)[0].strip().lower() == "application/step",
        f"STEP content-type was {step.header('content-type')!r}",
    )
    _require(
        "attachment" in step.header("content-disposition"), "STEP disposition was not attachment"
    )
    _require(step.body.startswith(STEP_MAGIC), "STEP body did not start with ISO-10303-21")
    _require(STEP_TERMINATOR in step.body, "STEP body did not contain the ISO-10303-21 terminator")


def _drain(stream: object, sink: list[str]) -> None:
    if stream is None:
        return
    try:
        for line in iter(stream.readline, b""):  # type: ignore[attr-defined]
            if len(sink) < 100:
                sink.append(line.decode("utf-8", "replace").rstrip())
    except (OSError, ValueError):
        pass


def _stop(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=8.0)
        return
    except subprocess.TimeoutExpired:
        pass
    process.kill()
    try:
        process.wait(timeout=8.0)
    except subprocess.TimeoutExpired:
        pass


def run(
    stage_dir: Path, *, host: str = DEFAULT_HOST, timeout: float = DEFAULT_TIMEOUT_SECONDS
) -> int:
    binary = stage_dir / _binary_name()
    index = stage_dir / "studio_dist" / "index.html"
    metadata_path = stage_dir / "native-build.json"
    metadata = _validate_stage(stage_dir, binary, index, metadata_path)
    print(f"[native-smoke] stage ok: {stage_dir}")
    print(f"[native-smoke] sha256 {metadata['binarySha256']} bytes {metadata['binaryBytes']}")
    parity = metadata.get("sourceParity")
    if isinstance(parity, dict):
        print(f"[native-smoke] current-feature readiness: {parity.get('summary', 'unknown')}")

    port = _pick_free_port(host)
    token = "kala-native-smoke-" + secrets.token_hex(16)
    captured: list[str] = []
    with tempfile.TemporaryDirectory(prefix="kala-native-smoke-") as raw_tmp:
        root = Path(raw_tmp)
        packaged = _copy_packaged_layout(stage_dir, root / "packaged")
        workdir = root / "cwd"
        workdir.mkdir()
        data_dir = root / "data"
        data_dir.mkdir()
        environment = _minimum_environment(data_dir, token)

        process = subprocess.Popen(
            [str(packaged / _binary_name()), "--host", host, "--port", str(port)],
            cwd=str(workdir),
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            creationflags=getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
            if sys.platform.startswith("win")
            else 0,
        )
        reader = threading.Thread(target=_drain, args=(process.stdout, captured), daemon=True)
        reader.start()
        try:
            health = _wait_for_health(
                host, port, deadline=time.monotonic() + timeout, process=process
            )
            print(f"[native-smoke] health ok: {json.dumps(health, sort_keys=True)}")
            _check_ui(host, port)
            print("[native-smoke] packaged UI root and assets ok.")
            _check_api(host, port, token, step_available=bool(health.get("step_available")))
            print("[native-smoke] preview, auth refusal and native STEP ok.")
            print("[native-smoke] PASS")
            return 0
        except (SmokeFailure, json.JSONDecodeError, UnicodeDecodeError) as error:
            print(f"[native-smoke] FAIL: {error}", file=sys.stderr)
            if captured:
                print("[native-smoke] backend output:", file=sys.stderr)
                for line in captured[-40:]:
                    print(f"    {line}", file=sys.stderr)
            return 1
        finally:
            _stop(process)
            reader.join(timeout=5.0)


def _parse_args(argv: Sequence[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="native_studio_smoke",
        description="Validate the staged native Kala Studio desktop distribution.",
    )
    parser.add_argument(
        "--stage-dir",
        default="studio/backend-native",
        help="native stage produced by studio/scripts/native-backend.cjs",
    )
    parser.add_argument("--host", default=DEFAULT_HOST, help="loopback host to bind")
    parser.add_argument(
        "--timeout",
        type=float,
        default=DEFAULT_TIMEOUT_SECONDS,
        help="seconds to wait for the native backend to become healthy",
    )
    return parser.parse_args(list(argv) if argv is not None else None)


def main(argv: Sequence[str] | None = None) -> int:
    arguments = _parse_args(argv)
    return run(Path(arguments.stage_dir).resolve(), host=arguments.host, timeout=arguments.timeout)


if __name__ == "__main__":  # pragma: no cover - module entry point
    sys.exit(main())
