"""Shared fixtures for the Kala Agent Python suite.

Every test runs with ``HOME``, ``USERPROFILE`` (on Windows), ``XDG_DATA_HOME``,
``KALA_AGENT_DATA_DIR`` and ``KALA_STUDIO_DATA_DIR`` pointed at per-test
temporary directories. The suite therefore cannot resolve, create, or open the
developer's real data directory (``$HOME/.local/share/kala-agent`` or
``%USERPROFILE%\\AppData\\Local``). That leak existed in
``tests/unit/test_studio_server.py``, whose default-resolved studio app opened
the real ``studio/studio-journal.sqlite3`` and briefly created its ``-shm`` /
``-wal`` siblings.

The fixture is autouse and function-scoped, so each test starts from a clean
environment and ``monkeypatch`` restores the real values afterwards. Tests
that deliberately exercise default resolution must compare against
``Path.home()`` or the ``isolated_*`` fixtures below, never against a
hard-coded user path.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest


@pytest.fixture
def isolated_home(tmp_path: Path) -> Path:
    """Return the temporary ``HOME`` used for the current test."""
    home = tmp_path / "isolated-home"
    home.mkdir()
    return home


@pytest.fixture
def isolated_xdg_data_home(tmp_path: Path) -> Path:
    """Return the temporary ``XDG_DATA_HOME`` used for the current test."""
    root = tmp_path / "isolated-xdg-data"
    root.mkdir()
    return root


@pytest.fixture
def isolated_kala_data_dir(isolated_xdg_data_home: Path) -> Path:
    """Return the temporary core Kala data directory for the current test."""
    data_dir = isolated_xdg_data_home / "kala-agent"
    data_dir.mkdir()
    return data_dir


@pytest.fixture
def isolated_studio_data_dir(isolated_kala_data_dir: Path) -> Path:
    """Return the temporary studio data directory for the current test.

    The location mirrors production layout: the studio keeps its own
    directory inside the core data directory, never in the user's real home.
    """
    studio_dir = isolated_kala_data_dir / "studio"
    studio_dir.mkdir()
    return studio_dir


@pytest.fixture(autouse=True)
def isolate_user_data_dirs(
    isolated_home: Path,
    isolated_xdg_data_home: Path,
    isolated_kala_data_dir: Path,
    isolated_studio_data_dir: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Redirect every supported data-directory source at the test temp tree.

    ``Path.home()`` reads ``HOME`` on POSIX, ``Settings`` reads
    ``KALA_AGENT_DATA_DIR``, and the studio prefers ``KALA_STUDIO_DATA_DIR``
    before falling back to the core settings. Pointing all four at the temp
    tree closes every resolution order a test can exercise, including
    ``create_studio_app(data_dir=None)`` and ``Settings.from_env({})``.
    """
    monkeypatch.setenv("HOME", str(isolated_home))
    if os.name == "nt":
        # ``Path.home()`` resolves ``%USERPROFILE%`` on Windows, not ``HOME``.
        # Setting only HOME leaves the developer's real profile visible, so a
        # test that resolves the default data directory would leak into it.
        monkeypatch.setenv("USERPROFILE", str(isolated_home))
    monkeypatch.setenv("XDG_DATA_HOME", str(isolated_xdg_data_home))
    monkeypatch.setenv("KALA_AGENT_DATA_DIR", str(isolated_kala_data_dir))
    monkeypatch.setenv("KALA_STUDIO_DATA_DIR", str(isolated_studio_data_dir))

    assert Path.home() == isolated_home, (
        "the isolation fixture failed to redirect Path.home(); a test could "
        "resolve the developer's real data directory"
    )
