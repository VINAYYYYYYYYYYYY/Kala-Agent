"""The studio data-directory precedence, row by row.

The native product resolves the same rows in
``backend/crates/kala-studio/src/boundary.rs``
(``default_studio_data_dir``); the Rust unit tests named there assert the
identical order. Each case passes an explicit environment mapping, so the
assertions are independent of the (temporary) process environment the suite
installs.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from kala_agent.studio.settings import (
    AGENT_DATA_DIR_ENV_VAR,
    STUDIO_DATA_DIR_ENV_VAR,
    XDG_DATA_HOME_ENV_VAR,
    resolve_studio_data_dir_from_env,
)


def _environment(home: Path, **values: str) -> dict[str, str]:
    environment = {"HOME": str(home)}
    environment.update(values)
    return environment


def test_studio_data_dir_is_the_studio_directory_itself(tmp_path: Path) -> None:
    """KALA_STUDIO_DATA_DIR is used as-is: no ``studio`` child is appended."""
    studio = tmp_path / "studio-dir"
    resolved = resolve_studio_data_dir_from_env(
        _environment(tmp_path / "home", **{STUDIO_DATA_DIR_ENV_VAR: str(studio)})
    )
    assert resolved == studio
    assert resolved.is_dir()


def test_studio_data_dir_wins_over_every_lower_row(tmp_path: Path) -> None:
    studio = tmp_path / "studio-dir"
    agent = tmp_path / "agent-dir"
    xdg = tmp_path / "xdg"
    resolved = resolve_studio_data_dir_from_env(
        _environment(
            tmp_path / "home",
            **{
                STUDIO_DATA_DIR_ENV_VAR: str(studio),
                AGENT_DATA_DIR_ENV_VAR: str(agent),
                XDG_DATA_HOME_ENV_VAR: str(xdg),
            },
        )
    )
    assert resolved == studio


def test_agent_data_dir_gets_the_studio_child(tmp_path: Path) -> None:
    agent = tmp_path / "agent-dir"
    xdg = tmp_path / "xdg"
    resolved = resolve_studio_data_dir_from_env(
        _environment(
            tmp_path / "home",
            **{AGENT_DATA_DIR_ENV_VAR: str(agent), XDG_DATA_HOME_ENV_VAR: str(xdg)},
        )
    )
    assert resolved == agent / "studio"


def test_agent_data_dir_wins_over_the_xdg_data_home(tmp_path: Path) -> None:
    agent = tmp_path / "agent-dir"
    xdg = tmp_path / "xdg"
    resolved = resolve_studio_data_dir_from_env(
        _environment(
            tmp_path / "home",
            **{AGENT_DATA_DIR_ENV_VAR: str(agent), XDG_DATA_HOME_ENV_VAR: str(xdg)},
        )
    )
    assert resolved == agent / "studio"


def test_xdg_data_home_carries_the_kala_agent_directory(tmp_path: Path) -> None:
    xdg = tmp_path / "xdg"
    resolved = resolve_studio_data_dir_from_env(
        _environment(tmp_path / "home", **{XDG_DATA_HOME_ENV_VAR: str(xdg)})
    )
    assert resolved == xdg / "kala-agent" / "studio"
    assert resolved.is_dir()


def test_no_variable_resolves_the_historical_home_path(tmp_path: Path) -> None:
    home = tmp_path / "home"
    home.mkdir()
    resolved = resolve_studio_data_dir_from_env(_environment(home))
    assert resolved == home / ".local" / "share" / "kala-agent" / "studio"
    assert resolved.is_dir()


@pytest.mark.parametrize("value", ["relative/studio", "/"])
def test_an_unusable_studio_override_is_ignored(tmp_path: Path, value: str) -> None:
    """A relative or root override falls through to the next row."""
    agent = tmp_path / "agent-dir"
    resolved = resolve_studio_data_dir_from_env(
        _environment(
            tmp_path / "home",
            **{STUDIO_DATA_DIR_ENV_VAR: value, AGENT_DATA_DIR_ENV_VAR: str(agent)},
        )
    )
    assert resolved == agent / "studio"


def test_a_file_studio_override_is_ignored(tmp_path: Path) -> None:
    occupied = tmp_path / "a-file"
    occupied.write_text("not a directory", encoding="utf-8")
    agent = tmp_path / "agent-dir"
    resolved = resolve_studio_data_dir_from_env(
        _environment(
            tmp_path / "home",
            **{
                STUDIO_DATA_DIR_ENV_VAR: str(occupied),
                AGENT_DATA_DIR_ENV_VAR: str(agent),
            },
        )
    )
    assert resolved == agent / "studio"


@pytest.mark.parametrize("value", ["relative/path", ""])
def test_an_unusable_agent_data_dir_uses_the_temporary_fallback(
    tmp_path: Path, value: str
) -> None:
    """KALA_AGENT_DATA_DIR keeps its documented Settings failure path."""
    import tempfile

    resolved = resolve_studio_data_dir_from_env(
        _environment(
            tmp_path / "home",
            **{
                AGENT_DATA_DIR_ENV_VAR: value,
                XDG_DATA_HOME_ENV_VAR: str(tmp_path / "xdg"),
            },
        )
    )
    assert resolved == Path(tempfile.gettempdir()) / "kala-studio"


def test_without_a_home_the_temporary_fallback_is_used() -> None:
    import tempfile

    assert resolve_studio_data_dir_from_env({}) == (
        Path(tempfile.gettempdir()) / "kala-studio"
    )
