"""Artifact publication limits and confined path failures.

A produced file is only ever an artifact if it is inside the workspace, on a
real directory, and inside the caller's budget, and only if it is still the
same file when it is measured. Each of those refusals is proved here against
the exact filesystem condition that produces it.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Final

import pytest

from kala_agent_freecad.cad import artifacts
from kala_agent_freecad.cad.artifacts import (
    digest_artifact,
    finalize,
    prepare_target,
)
from kala_agent_freecad.errors import (
    FreeCADArtifactError,
    FreeCADOutputTooLargeError,
    FreeCADWorkspaceError,
)

PAYLOAD: Final[bytes] = b"ISO-10303-21;\n"
BUDGET: Final[int] = 4_096


def _written(target: Path, payload: bytes = PAYLOAD) -> Path:
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(payload)
    return target


def test_a_parent_that_is_a_file_cannot_become_the_artifact_directory(
    tmp_path: Path,
) -> None:
    blocker = tmp_path / "blocker"
    blocker.write_bytes(b"not a directory\n")
    with pytest.raises(FreeCADWorkspaceError, match="could not be created") as failure:
        prepare_target(blocker / "nested" / "model.step")
    assert failure.value.__cause__ is not None
    assert blocker.is_file()


def test_an_artifact_directory_that_cannot_be_inspected_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target = tmp_path / "artifacts" / "model.step"

    def refuse(*_path: str) -> os.stat_result:
        raise PermissionError(13, "Permission denied")

    monkeypatch.setattr("os.lstat", refuse)
    with pytest.raises(FreeCADWorkspaceError, match="is not accessible") as failure:
        prepare_target(target)
    monkeypatch.undo()
    assert failure.value.__cause__ is not None
    assert target.parent.is_dir()


def test_an_export_that_changes_while_it_is_measured_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target = _written(tmp_path / "artifacts" / "model.step")
    measured = digest_artifact

    def drifted(path: Path, *, max_artifact_bytes: int) -> tuple[str, int]:
        digest, size = measured(path, max_artifact_bytes=max_artifact_bytes)
        return digest, size + 1

    monkeypatch.setattr(artifacts, "digest_artifact", drifted)
    with pytest.raises(FreeCADArtifactError, match="changed while it was measured"):
        finalize(
            target,
            relative_path="artifacts/model.step",
            export_format="step",
            artifact_id="kala-call-1-step",
            max_artifact_bytes=BUDGET,
        )
    assert target.read_bytes() == PAYLOAD


def test_a_file_above_the_budget_is_measured_refused(tmp_path: Path) -> None:
    target = _written(tmp_path / "artifacts" / "model.step", b"x" * 8_192)
    with pytest.raises(FreeCADOutputTooLargeError, match=r"is 8192 bytes"):
        finalize(
            target,
            relative_path="artifacts/model.step",
            export_format="step",
            artifact_id="kala-call-1-step",
            max_artifact_bytes=4_096,
        )
    digest, size = digest_artifact(target, max_artifact_bytes=65_536)
    assert size == 8_192
    assert len(digest) == 64


def test_an_artifact_directory_created_for_a_call_is_private(tmp_path: Path) -> None:
    prepare_target(tmp_path / "artifacts" / "model.step")
    mode = (tmp_path / "artifacts").stat().st_mode
    assert mode & 0o777 == 0o700
    assert os.path.isdir(tmp_path / "artifacts")
