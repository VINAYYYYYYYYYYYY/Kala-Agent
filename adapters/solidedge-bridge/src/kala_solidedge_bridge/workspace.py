"""Private data-directory layout, atomic persistence, artifacts, and document ledger.

Everything the bridge writes lives below one operator data directory. Files are
written to a sibling temporary name, flushed, and atomically replaced, so a
reader never observes a half-written request, status, manifest, or calibration
record. Artifact names are metadata only: a name is resolved through a completed
manifest and its size and digest are checked again on every read.
"""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import threading
from collections.abc import Mapping
from pathlib import Path
from typing import Any, Final

from .errors import UnavailableError
from .protocol import Artifact, ArtifactFormat

DATA_DIR_NAME: Final[tuple[str, str]] = ("Kala", "SolidEdgeBridge")
MANIFEST_NAME: Final[str] = "manifest.json"
MAX_ARTIFACT_BYTES: Final[int] = 256 * 1024 * 1024

_MEDIA_TYPES: Final[dict[ArtifactFormat, str]] = {
    ArtifactFormat.PAR: "application/octet-stream",
    ArtifactFormat.ASM: "application/octet-stream",
    ArtifactFormat.STEP: "application/step",
}


def default_data_dir(platform: str | None = None) -> Path:
    """Return the operator data directory for this platform.

    Windows uses ``%LOCALAPPDATA%``. Non-Windows hosts are development and test
    hosts only; their default keeps every test isolated from a user profile
    when an explicit directory is supplied.
    """
    current = platform if platform is not None else sys.platform
    if current == "win32":
        base = os.environ.get("LOCALAPPDATA")
        if not base:
            base = str(Path.home() / "AppData" / "Local")
    else:
        base = os.environ.get("XDG_DATA_HOME") or str(Path.home() / ".local" / "share")
    return Path(base).joinpath(*DATA_DIR_NAME)


def ensure_private_dir(path: Path) -> None:
    """Create ``path`` if needed and restrict it to the current user where possible."""
    path.mkdir(parents=True, exist_ok=True)
    if os.name != "nt":
        try:
            path.chmod(0o700)
        except OSError:
            pass


def restrict_windows_acl(path: Path) -> None:
    """Restrict a token/key file to the current Windows user.

    ``%LOCALAPPDATA%`` is already user-private on a normal installation, but the
    credential files are tightened explicitly. Failure is reported rather than
    silently ignored so a broken ACL does not look like a private file.
    """
    if os.name != "nt":
        return
    user = os.environ.get("USERNAME")
    if not user:
        raise OSError("USERNAME is unavailable; cannot restrict the bridge secret ACL")
    completed = subprocess.run(
        ["icacls", str(path), "/inheritance:r", "/grant:r", f"{user}:F"],
        check=False,
        capture_output=True,
        text=True,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )
    if completed.returncode != 0:
        raise OSError(f"icacls failed to restrict {path.name}: {completed.returncode}")


def _fsync_directory(path: Path) -> None:
    if os.name == "nt":
        return
    try:
        fd = os.open(path, os.O_RDONLY)
    except OSError:
        return
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def atomic_write_bytes(path: Path, data: bytes, *, mode: int = 0o600) -> None:
    """Write ``data`` to ``path`` atomically in the same directory."""
    ensure_private_dir(path.parent)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, path)
        _fsync_directory(path.parent)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


def atomic_write_json(path: Path, payload: object) -> None:
    """Serialize ``payload`` deterministically and write it atomically."""
    encoded = json.dumps(
        payload,
        allow_nan=False,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    atomic_write_bytes(path, encoded)


def read_json(path: Path) -> Any | None:  # noqa: ANN401
    """Read JSON, returning ``None`` when the file is absent or invalid."""
    try:
        raw = path.read_bytes()
    except FileNotFoundError:
        return None
    try:
        return json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return None


def sha256_file(path: Path, *, chunk_size: int = 1024 * 1024) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(chunk_size)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def is_safe_basename(name: str) -> bool:
    """A served artifact name is one plain path component."""
    if not name or name in {".", ".."}:
        return False
    if "/" in name or "\\" in name or "\x00" in name or "%" in name:
        return False
    return Path(name).name == name


class ArtifactStore:
    """Completed-artifact manifest for one private run directory."""

    def __init__(self, run_dir: Path) -> None:
        self.run_dir = run_dir
        self.manifest_path = run_dir / MANIFEST_NAME

    def register(self, name: str, artifact_format: ArtifactFormat) -> Artifact:
        """Hash an artifact already written inside the run directory."""
        if not is_safe_basename(name):
            raise UnavailableError("artifact name is not a safe file name")
        path = self.run_dir / name
        resolved = path.resolve()
        if not resolved.is_relative_to(self.run_dir.resolve()):
            raise UnavailableError("artifact escaped the private run directory")
        if not resolved.is_file() or resolved.is_symlink():
            raise UnavailableError(f"artifact {name!r} was not produced as a regular file")
        size = resolved.stat().st_size
        if size > MAX_ARTIFACT_BYTES:
            raise UnavailableError(f"artifact {name!r} exceeds the bridge artifact cap")
        return Artifact(
            name=name,
            format=artifact_format,
            media_type=_MEDIA_TYPES[artifact_format],
            size_bytes=size,
            sha256=sha256_file(resolved),
        )

    def write_manifest(self, artifacts: tuple[Artifact, ...]) -> None:
        atomic_write_json(
            self.manifest_path,
            {
                "schema": 1,
                "artifacts": [artifact.model_dump(mode="json") for artifact in artifacts],
            },
        )

    def read_manifest(self) -> tuple[Artifact, ...] | None:
        payload = read_json(self.manifest_path)
        if not isinstance(payload, dict) or payload.get("schema") != 1:
            return None
        raw_artifacts = payload.get("artifacts")
        if not isinstance(raw_artifacts, list):
            return None
        artifacts: list[Artifact] = []
        try:
            for entry in raw_artifacts:
                artifacts.append(Artifact.model_validate(entry))
        except Exception:
            return None
        return tuple(artifacts)

    def read_verified(self, name: str) -> tuple[Artifact, bytes]:
        """Return a completed artifact only after re-checking size and digest."""
        if not is_safe_basename(name):
            raise UnavailableError("artifact name is not a safe file name")
        manifest = self.read_manifest()
        if manifest is None:
            raise KeyError(name)
        entry = next((artifact for artifact in manifest if artifact.name == name), None)
        if entry is None:
            raise KeyError(name)
        path = self.run_dir / name
        if not path.is_file() or path.is_symlink():
            raise UnavailableError(f"artifact {name!r} is missing")
        if path.stat().st_size != entry.size_bytes:
            raise UnavailableError(f"artifact {name!r} changed size after publication")
        data = path.read_bytes()
        if len(data) != entry.size_bytes:
            raise UnavailableError(f"artifact {name!r} changed size after publication")
        if hashlib.sha256(data).hexdigest() != entry.sha256:
            raise UnavailableError(f"artifact {name!r} failed digest verification")
        return entry, data


class DocumentLedger:
    """Append-only record of documents the bridge created and closed.

    The ledger records the bridge-owned tag and run identity. It never records
    credentials or contents of a user's documents.
    """

    def __init__(self, data_dir: Path) -> None:
        self.path = data_dir / "documents.jsonl"
        self._lock = threading.Lock()

    def append(self, event: Mapping[str, object]) -> None:
        line = json.dumps(
            dict(event),
            allow_nan=False,
            ensure_ascii=True,
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")
        with self._lock:
            ensure_private_dir(self.path.parent)
            descriptor = os.open(
                self.path,
                os.O_WRONLY | os.O_CREAT | os.O_APPEND,
                0o600,
            )
            try:
                os.write(descriptor, line + b"\n")
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
