"""Workspace-confined artifact publication.

Every file this adapter produces is written through the kernel's workspace
resolver, is refused if its directory or destination is a symlink, is bounded
by the caller's artifact budget, and is published with the SHA-256 digest of
the bytes that are actually on disk. Nothing here knows about FreeCAD.
"""

from __future__ import annotations

import hashlib
import os
import stat
from dataclasses import dataclass
from pathlib import Path
from typing import Final

from kala_agent.contracts import ArtifactClassification, ArtifactRef
from kala_agent.tools.protocol import (
    JSONValue,
    ToolWorkspaceError,
    resolve_workspace_path,
)
from kala_agent_freecad.contracts import EXPORT_MEDIA_TYPES
from kala_agent_freecad.errors import (
    FreeCADArtifactError,
    FreeCADOutputTooLargeError,
    FreeCADWorkspaceError,
)

CHUNK_BYTES: Final[int] = 1_048_576
MAX_MEDIA_TYPE_LENGTH: Final[int] = 127
MAX_LABEL_LENGTH: Final[int] = 1_024
DEFAULT_MEDIA_TYPE: Final[str] = "application/octet-stream"


@dataclass(frozen=True, slots=True)
class PublishedArtifact:
    """One exported file, described for both the envelope and the result."""

    artifact_id: str
    relative_path: str
    export_format: str
    media_type: str
    digest: str
    size_bytes: int
    uri: str

    @property
    def artifact(self) -> ArtifactRef:
        return ArtifactRef(
            artifact_id=self.artifact_id,
            uri=self.uri,
            digest=self.digest,
            media_type=self.media_type,
            size_bytes=self.size_bytes,
            classification=ArtifactClassification.DESIGN_BINARY,
        )

    def entry(self) -> dict[str, JSONValue]:
        return {
            "digest": self.digest,
            "format": self.export_format,
            "media_type": self.media_type,
            "relative_path": self.relative_path,
            "size_bytes": self.size_bytes,
        }


def resolve_target(workspace: Path, relative: str) -> Path:
    """Resolve a workspace relative artifact path or refuse it."""
    try:
        return resolve_workspace_path(workspace, relative)
    except ToolWorkspaceError as error:
        raise FreeCADWorkspaceError(str(error)) from error


def prepare_target(target: Path) -> None:
    """Create the artifact directory and refuse a symlinked destination."""
    parent = target.parent
    try:
        os.makedirs(parent, mode=0o700, exist_ok=True)
    except OSError as error:
        raise FreeCADWorkspaceError(
            f"the artifact directory could not be created: {error.strerror}"
        ) from error
    try:
        status = os.lstat(parent)
    except OSError as error:
        raise FreeCADWorkspaceError("the artifact directory is not accessible") from error
    if stat.S_ISLNK(status.st_mode) or not stat.S_ISDIR(status.st_mode):
        raise FreeCADWorkspaceError("the artifact directory must be a real directory")
    if target.is_symlink():
        raise FreeCADWorkspaceError("the artifact destination cannot be a symlink")


def finalize(
    target: Path,
    *,
    relative_path: str,
    export_format: str,
    artifact_id: str,
    max_artifact_bytes: int,
) -> PublishedArtifact:
    """Verify a written file and describe it as a publishable artifact."""
    try:
        size = target.stat().st_size
    except OSError as error:
        raise FreeCADArtifactError(f"an export could not be inspected: {error.strerror}") from error
    if size <= 0:
        raise FreeCADArtifactError(f"the {export_format} export produced an empty file")
    if size > max_artifact_bytes:
        raise FreeCADOutputTooLargeError(
            f"the {export_format} export is {size} bytes, above the "
            f"{max_artifact_bytes} byte artifact budget"
        )
    digest, measured = digest_artifact(target, max_artifact_bytes=max_artifact_bytes)
    if measured != size:
        raise FreeCADArtifactError(f"the {export_format} export changed while it was measured")
    return PublishedArtifact(
        artifact_id=artifact_id,
        relative_path=relative_path,
        export_format=export_format,
        media_type=_media_type(export_format),
        digest=digest,
        size_bytes=measured,
        uri=f"file://{target.as_posix()}",
    )


def digest_artifact(target: Path, *, max_artifact_bytes: int) -> tuple[str, int]:
    """Return the SHA-256 digest and byte count of a file on disk."""
    digest = hashlib.sha256()
    total = 0
    try:
        with target.open("rb") as handle:
            while True:
                chunk = handle.read(CHUNK_BYTES)
                if not chunk:
                    break
                total += len(chunk)
                if total > max_artifact_bytes:
                    raise FreeCADOutputTooLargeError(
                        f"an export exceeded the {max_artifact_bytes} byte artifact budget"
                    )
                digest.update(chunk)
    except OSError as error:
        raise FreeCADArtifactError(f"an export could not be read back: {error.strerror}") from error
    if total <= 0:
        raise FreeCADArtifactError("an export produced an empty file")
    return digest.hexdigest(), total


def discard(target: Path) -> None:
    """Remove a rejected export so a partial file is never published."""
    try:
        target.unlink(missing_ok=True)
    except OSError:  # pragma: no cover - teardown noise
        pass


def bounded_label(value: str, maximum: int) -> str:
    """Return a single line, bounded label for a result field."""
    collapsed = " ".join(str(value).split())
    return collapsed[:maximum] or "unknown"


def optional_message(text: str, maximum: int) -> str | None:
    """Return a single line, bounded message, or nothing at all."""
    collapsed = " ".join(str(text).split())[:maximum]
    return collapsed or None


def _media_type(export_format: str) -> str:
    return EXPORT_MEDIA_TYPES.get(export_format, DEFAULT_MEDIA_TYPE)[:MAX_MEDIA_TYPE_LENGTH]


__all__ = (
    "CHUNK_BYTES",
    "DEFAULT_MEDIA_TYPE",
    "MAX_LABEL_LENGTH",
    "MAX_MEDIA_TYPE_LENGTH",
    "PublishedArtifact",
    "bounded_label",
    "digest_artifact",
    "discard",
    "finalize",
    "optional_message",
    "prepare_target",
    "resolve_target",
)
