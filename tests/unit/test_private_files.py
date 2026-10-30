"""Regression tests for the owner-only credential files.

Both stores used to rely on POSIX-only primitives: the core API token was
written with ``O_TRUNC`` + ``os.fchmod`` (``fchmod`` does not exist on Windows,
and the truncation had already happened when it failed) and the studio settings
file was protected by ``chmod 0o600`` alone, which does not change a Windows
ACL at all. These tests run on the Windows deployment host as well as on Linux:
the writer must replace the file atomically, leave no truncated store behind,
and — on Windows — leave an ACL that names only the current user.
"""

from __future__ import annotations

import json
import os
import subprocess

import pytest

from kala_agent.api import boundary as api_boundary
from kala_agent.api.boundary import (
    API_TOKEN_FILE_NAME,
    API_TOKEN_FILE_MODE,
    issue_api_token,
    read_api_token,
)
from kala_agent.private_files import (
    atomic_write_private_text,
    restrict_windows_acl,
    tighten_file_permissions,
)
from kala_agent.studio.settings import (
    STUDIO_SETTINGS_FILE_NAME,
    read_stored_keys,
    write_stored_key,
)

IS_WINDOWS = os.name == "nt"


def _acl_entries(path: os.PathLike[str] | str) -> list[str]:
    """Return the access-control entries ``icacls`` reports for ``path``."""
    completed = subprocess.run(
        ["icacls", os.fspath(path)],
        check=False,
        capture_output=True,
        text=True,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )
    assert completed.returncode == 0, completed.stderr
    return [line for line in completed.stdout.splitlines() if ":(" in line]


def test_issue_api_token_replaces_a_stale_store_atomically(tmp_path) -> None:
    """A new launch replaces the old token and leaves no temporary file behind."""
    data_dir = tmp_path / "data"
    data_dir.mkdir()
    stale = data_dir / API_TOKEN_FILE_NAME
    stale.write_text("stale-token-0123456789abcdef\n", encoding="utf-8")

    issued = issue_api_token(data_dir)

    assert read_api_token(data_dir) == issued
    assert stale.read_text(encoding="utf-8") == f"{issued}\n"
    assert [entry.name for entry in data_dir.iterdir()] == [API_TOKEN_FILE_NAME]


def test_issue_api_token_degrades_to_memory_only(
    tmp_path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A store that cannot be written still yields a usable launch token."""

    def refuse(_path, _content, _mode=API_TOKEN_FILE_MODE) -> None:
        raise AttributeError("'module' object has no attribute 'fchmod'")

    monkeypatch.setattr(api_boundary, "atomic_write_private_text", refuse)

    issued = issue_api_token(tmp_path)

    assert issued
    assert read_api_token(tmp_path) is None


def test_atomic_write_private_text_replaces_the_destination(tmp_path) -> None:
    """The portable writer publishes the whole content under the final name."""
    data_dir = tmp_path / "data"
    data_dir.mkdir()
    destination = data_dir / "secret"
    destination.write_text("old\n", encoding="utf-8")

    atomic_write_private_text(destination, "new\n")

    assert destination.read_text(encoding="utf-8") == "new\n"
    assert [entry.name for entry in data_dir.iterdir()] == ["secret"]


@pytest.mark.skipif(not IS_WINDOWS, reason="the ACL is only applied on Windows")
def test_atomic_write_private_text_restricts_the_file_to_the_current_user(tmp_path) -> None:
    """A Windows credential file carries no inherited access entry."""
    destination = tmp_path / "secret"
    atomic_write_private_text(destination, "secret\n")

    entries = _acl_entries(destination)

    assert len(entries) == 1
    assert os.environ["USERNAME"] in entries[0]
    assert "(I)" not in entries[0]


@pytest.mark.skipif(not IS_WINDOWS, reason="the ACL is only applied on Windows")
def test_restrict_windows_acl_removes_inherited_entries(tmp_path) -> None:
    """The helper is the one the API and studio stores both rely on."""
    destination = tmp_path / "secret"
    destination.write_text("secret\n", encoding="utf-8")

    restrict_windows_acl(destination)

    entries = _acl_entries(destination)
    assert len(entries) == 1
    assert os.environ["USERNAME"] in entries[0]
    assert "(I)" not in entries[0]


def test_tighten_file_permissions_never_raises_for_an_unrestrictable_file(
    tmp_path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A settings file that cannot be tightened stays usable for this launch."""
    destination = tmp_path / "secret"
    destination.write_text("secret\n", encoding="utf-8")

    def refuse(*_args, **_kwargs) -> None:
        raise OSError("icacls failed")

    monkeypatch.setattr("kala_agent.private_files.restrict_windows_acl", refuse)
    monkeypatch.setattr(os, "chmod", refuse, raising=False)

    tighten_file_permissions(destination)


def test_write_stored_key_keeps_the_json_format_and_restricts_the_file(tmp_path) -> None:
    """The settings file stays a plain ``api_keys`` object and is owner-only."""
    data_dir = tmp_path / "data"
    data_dir.mkdir()
    write_stored_key(data_dir, "openai", "sk-test-key-0123456789")

    settings_path = data_dir / STUDIO_SETTINGS_FILE_NAME
    payload = json.loads(settings_path.read_text(encoding="utf-8"))
    assert payload == {"api_keys": {"openai": "sk-test-key-0123456789"}}
    assert read_stored_keys(data_dir) == {"openai": "sk-test-key-0123456789"}
    assert [entry.name for entry in data_dir.iterdir()] == [STUDIO_SETTINGS_FILE_NAME]
    if IS_WINDOWS:
        entries = _acl_entries(settings_path)
        assert len(entries) == 1
        assert os.environ["USERNAME"] in entries[0]
