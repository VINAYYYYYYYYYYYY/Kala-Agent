"""Windows path defaults are selected without inventing a Linux runtime."""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from kala_agent_solidedge import paths


def test_windows_defaults_use_local_app_data() -> None:
    environment = {"LOCALAPPDATA": r"C:\Users\kala\AppData\Local"}

    token = paths.default_token_file(environment=environment, platform="win32")
    runs = paths.default_output_dir(environment=environment, platform="win32")

    assert token is not None
    assert runs is not None
    assert str(token).replace("\\", "/").endswith("AppData/Local/Kala/SolidEdgeBridge/bridge.token")
    assert str(runs).replace("\\", "/").endswith("AppData/Local/Kala/SolidEdge/runs")


def test_no_linux_token_or_run_default_exists() -> None:
    assert paths.default_token_file(environment={}, platform="linux") is None
    assert paths.default_output_dir(environment={}, platform="linux") is None


def test_a_linux_home_is_never_used_as_a_runtime_default() -> None:
    resolved = paths.resolve_token_file(None, environment={"HOME": "/home/kala"}, platform="linux")
    assert resolved is None
    assert paths.resolve_output_dir(None, environment={}, platform="linux") is None


def test_explicit_and_environment_values_win() -> None:
    environment = {
        paths.TOKEN_FILE_ENVIRONMENT_KEY: "/tmp/env.token",
        paths.BRIDGE_URL_ENVIRONMENT_KEY: "http://127.0.0.1:9000",
        "LOCALAPPDATA": "/tmp/local",
    }

    assert paths.resolve_token_file("/tmp/flag.token", environment=environment) == Path(
        "/tmp/flag.token"
    )
    assert paths.resolve_token_file(None, environment=environment) == Path("/tmp/env.token")
    assert paths.resolve_bridge_url(None, environment=environment) == "http://127.0.0.1:9000"
    assert paths.resolve_bridge_url(None, environment={}) == paths.DEFAULT_BRIDGE_URL
    assert paths.resolve_output_dir("/tmp/out", environment=environment) == Path("/tmp/out")


def test_the_default_bridge_url_is_loopback() -> None:
    assert paths.DEFAULT_BRIDGE_URL == "http://127.0.0.1:8797"


def test_the_token_filename_matches_the_bridge_contract() -> None:
    # The bridge writes ``bridge.token`` under %LOCALAPPDATA%/Kala/SolidEdgeBridge.
    assert paths.TOKEN_FILENAME == "bridge.token"
    assert paths.DATA_DIR_PARTS == ("Kala", "SolidEdgeBridge")


def test_windows_without_local_app_data_falls_back_to_the_user_profile() -> None:
    token = paths.default_token_file(environment={}, platform="win32")
    assert token is not None
    assert token.parts[-3:] == ("Kala", "SolidEdgeBridge", "bridge.token")


@pytest.mark.skipif(os.name == "nt", reason="POSIX permission check")
def test_no_linux_directory_is_created_by_resolution(tmp_path: Path) -> None:
    before = set(tmp_path.iterdir())
    paths.resolve_token_file(None, environment={"XDG_DATA_HOME": str(tmp_path)}, platform="linux")
    assert set(tmp_path.iterdir()) == before
