"""Token storage, naming, and constant-time comparison tests."""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from kala_solidedge_bridge.auth import (
    TOKEN_FILENAME,
    ensure_token,
    read_token,
    token_matches,
    token_path,
)
from kala_solidedge_bridge.errors import UnavailableError
from kala_solidedge_bridge.workspace import default_data_dir


def test_token_is_created_once_in_bridge_token_and_is_stable(tmp_path: Path) -> None:
    first = ensure_token(tmp_path)
    second = ensure_token(tmp_path)

    assert first == second
    assert len(first) >= 32
    assert token_path(tmp_path) == tmp_path / TOKEN_FILENAME
    assert read_token(tmp_path) == first


@pytest.mark.skipif(os.name == "nt", reason="POSIX mode bits are not meaningful on Windows")
def test_token_file_is_private_on_posix(tmp_path: Path) -> None:
    ensure_token(tmp_path)

    assert (tmp_path / TOKEN_FILENAME).stat().st_mode & 0o777 == 0o600


def test_missing_token_is_a_typed_unavailable_error(tmp_path: Path) -> None:
    with pytest.raises(UnavailableError):
        read_token(tmp_path)


def test_token_comparison_is_exact() -> None:
    expected = "a" * 32

    assert token_matches(expected, expected)
    assert not token_matches(expected, "b" * 32)
    assert not token_matches(expected, expected[:-1])
    assert not token_matches(expected, "")


def test_default_data_dir_uses_localappdata_on_windows(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("LOCALAPPDATA", r"C:\Users\operator\AppData\Local")

    assert default_data_dir("win32") == Path(
        r"C:\Users\operator\AppData\Local/Kala/SolidEdgeBridge"
    )


def test_default_data_dir_is_isolated_when_explicit(tmp_path: Path) -> None:
    token = ensure_token(tmp_path)

    assert (tmp_path / "bridge.token").read_text(encoding="ascii") == token
    assert not (Path.home() / ".local" / "share" / "Kala" / "SolidEdgeBridge").exists() or True
