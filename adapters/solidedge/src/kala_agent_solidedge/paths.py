"""Platform path selection for the Windows-local client.

The target runtime is Windows. Linux is a development and test host only, so a
Linux default is deliberately absent: without an explicit ``--token-file`` or
``KALA_SOLIDEDGE_TOKEN_FILE`` there is no token path to guess, and no
``~/.local`` runtime directory is created.
"""

from __future__ import annotations

import os
import re
import sys
from collections.abc import Mapping
from pathlib import Path
from typing import Final

DEFAULT_BRIDGE_URL: Final[str] = "http://127.0.0.1:8797"
BRIDGE_URL_ENVIRONMENT_KEY: Final[str] = "KALA_SOLIDEDGE_BRIDGE_URL"
TOKEN_FILE_ENVIRONMENT_KEY: Final[str] = "KALA_SOLIDEDGE_TOKEN_FILE"
DATA_DIR_PARTS: Final[tuple[str, str]] = ("Kala", "SolidEdgeBridge")
RUNS_DIR_PARTS: Final[tuple[str, ...]] = ("Kala", "SolidEdge", "runs")
TOKEN_FILENAME: Final[str] = "bridge.token"
_WINDOWS_RESERVED: Final[frozenset[str]] = frozenset(
    {"CON", "PRN", "AUX", "NUL"}
    | {f"COM{index}" for index in range(1, 10)}
    | {f"LPT{index}" for index in range(1, 10)}
)


def is_windows(platform: str | None = None) -> bool:
    """Return whether this interpreter is running on Windows."""
    return (sys.platform if platform is None else platform) == "win32"


def local_app_data(
    environment: Mapping[str, str] | None = None,
    *,
    platform: str | None = None,
) -> Path | None:
    """Return ``%LOCALAPPDATA%`` or ``None`` when this is not a Windows host."""
    env = os.environ if environment is None else environment
    base = env.get("LOCALAPPDATA")
    if base:
        return Path(base)
    if is_windows(platform):
        return Path.home() / "AppData" / "Local"
    return None


def default_token_file(
    *,
    environment: Mapping[str, str] | None = None,
    platform: str | None = None,
) -> Path | None:
    """Return the Windows token path shared with the bridge, or ``None``.

    The bridge writes ``bridge.token`` under its data directory. The client
    resolves the same name and directory; it never creates the file.
    """
    if not is_windows(platform):
        return None
    base = local_app_data(environment, platform=platform)
    if base is None:  # pragma: no cover - is_windows implies a base
        return None
    return base.joinpath(*DATA_DIR_PARTS) / TOKEN_FILENAME


def default_output_dir(
    *,
    environment: Mapping[str, str] | None = None,
    platform: str | None = None,
) -> Path | None:
    """Return the Windows default run directory, or ``None`` off Windows."""
    if not is_windows(platform):
        return None
    base = local_app_data(environment, platform=platform)
    if base is None:  # pragma: no cover - is_windows implies a base
        return None
    return base.joinpath(*RUNS_DIR_PARTS)


def resolve_token_file(
    explicit: str | None,
    *,
    environment: Mapping[str, str] | None = None,
    platform: str | None = None,
) -> Path | None:
    """Resolve the token file: flag, then environment, then Windows default."""
    env = os.environ if environment is None else environment
    if explicit:
        return Path(explicit)
    from_environment = env.get(TOKEN_FILE_ENVIRONMENT_KEY)
    if from_environment:
        return Path(from_environment)
    return default_token_file(environment=env, platform=platform)


def resolve_bridge_url(
    explicit: str | None,
    *,
    environment: Mapping[str, str] | None = None,
) -> str:
    """Resolve the bridge URL: flag, then environment, then loopback default."""
    env = os.environ if environment is None else environment
    if explicit:
        return explicit
    return env.get(BRIDGE_URL_ENVIRONMENT_KEY) or DEFAULT_BRIDGE_URL


def resolve_output_dir(
    explicit: str | None,
    *,
    environment: Mapping[str, str] | None = None,
    platform: str | None = None,
) -> Path | None:
    """Resolve the artifact directory: flag, then Windows default."""
    if explicit:
        return Path(explicit)
    return default_output_dir(environment=environment, platform=platform)


_SAFE_BUILD_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}")


def safe_build_id(build_id: str) -> str:
    """Return a build id usable as one Windows directory name.

    The bridge already constrains ids, but the client writes the value to disk,
    so it re-validates: no separators, no drive or ADS colon, no dot-only name,
    no reserved device name, and no trailing dot or space (Windows strips them,
    which would silently collide two builds).
    """
    if _SAFE_BUILD_ID.fullmatch(build_id) is None:
        raise ValueError("the build id is not a safe directory name")
    if build_id in {".", ".."}:
        raise ValueError("the build id is not a safe directory name")
    stem = build_id.split(".", 1)[0].upper()
    if stem in _WINDOWS_RESERVED:
        raise ValueError("the build id is a reserved Windows device name")
    if build_id.endswith((".", " ")):
        raise ValueError("the build id must not end with a dot or space")
    return build_id


def build_output_dir(output_root: Path, build_id: str) -> Path:
    """Return the build-specific artifact directory under the caller's root.

    Each build gets its own directory so two builds that both produce
    ``plate.step`` never overwrite each other, and a re-download of one build is
    idempotent without touching another.
    """
    return output_root / "builds" / safe_build_id(build_id)


__all__ = (
    "BRIDGE_URL_ENVIRONMENT_KEY",
    "DEFAULT_BRIDGE_URL",
    "TOKEN_FILE_ENVIRONMENT_KEY",
    "build_output_dir",
    "default_output_dir",
    "default_token_file",
    "is_windows",
    "local_app_data",
    "resolve_bridge_url",
    "resolve_output_dir",
    "resolve_token_file",
    "safe_build_id",
)
