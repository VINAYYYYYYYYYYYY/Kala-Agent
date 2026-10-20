"""Regression tests for the autouse data-directory isolation fixture.

These tests pin the contract of ``tests/conftest.py``: every source that can
resolve a data directory must point into the test's temporary tree, and the
default resolution both of ``Settings`` and of the studio app must land under
the temporary ``HOME`` rather than the developer's real home.
"""

from __future__ import annotations

import os
from pathlib import Path

from kala_agent.config import Settings
from kala_agent.studio.server import create_studio_app

TOKEN = "isolation-test-token-0123456789abcdef"


def test_environment_points_every_data_source_at_the_temp_tree(
    tmp_path: Path,
    isolated_home: Path,
    isolated_xdg_data_home: Path,
    isolated_kala_data_dir: Path,
    isolated_studio_data_dir: Path,
) -> None:
    assert Path.home() == isolated_home
    assert Path(os.environ["XDG_DATA_HOME"]) == isolated_xdg_data_home
    assert Path(os.environ["KALA_AGENT_DATA_DIR"]) == isolated_kala_data_dir
    assert Path(os.environ["KALA_STUDIO_DATA_DIR"]) == isolated_studio_data_dir
    for path in (
        isolated_home,
        isolated_xdg_data_home,
        isolated_kala_data_dir,
        isolated_studio_data_dir,
    ):
        assert path.is_relative_to(tmp_path), f"{path} escaped the test temp tree"


def test_settings_default_resolves_under_the_isolated_home(isolated_home: Path) -> None:
    settings = Settings.from_env({})
    expected = (isolated_home / ".local" / "share" / "kala-agent").resolve(strict=False)
    assert settings.data_dir == expected


def test_studio_app_default_resolves_under_the_isolated_home(isolated_home: Path) -> None:
    """``data_dir=None`` must fall back to the temp HOME, not the real one."""
    app = create_studio_app(api_token=TOKEN, max_body_bytes=4096, static_root=None, environ={})
    expected = (isolated_home / ".local" / "share" / "kala-agent" / "studio").resolve(strict=False)
    assert app.state.studio_data_dir == expected
    assert expected.is_dir()
