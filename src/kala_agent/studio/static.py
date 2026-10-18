"""Static-asset root resolution for the studio server.

The studio serves its UI from a server-controlled directory. The directory
is resolved in a fixed order:

1. The environment variable ``KALA_STUDIO_DIST`` when the operator set
   one; the directory must contain an ``index.html`` for the studio to
   consider it usable. The check fails closed if the path is unsafe.
2. The frozen executable layout (``sys._MEIPASS`` / ``studio_dist``)
   when the studio was launched from a PyInstaller bundle.
3. The repo-relative ``<repo>/studio/dist`` directory when running from a
   source checkout. The check walks the package's parent directories
   until it locates a ``studio/dist`` directory that contains
   ``index.html``.

The function never creates directories or falls back to the empty
string: the server needs an honest answer so it can refuse to advertise
a static root that does not exist.
"""

from __future__ import annotations

import sys
from collections.abc import Callable, Mapping
from pathlib import Path
from typing import Final

STUDIO_DIST_ENV_VAR: Final[str] = "KALA_STUDIO_DIST"
STUDIO_DIST_INDEX: Final[str] = "index.html"
STUDIO_DIST_SUBDIR: Final[str] = "studio_dist"
REPO_DIST_DIR_PARTS: Final[tuple[str, ...]] = ("studio", "dist")


def _candidate_from_meipass(environ: Mapping[str, str]) -> Path | None:
    del environ  # the frozen layout does not consult the environment
    meipass = getattr(sys, "_MEIPASS", None)
    if not meipass or not isinstance(meipass, str):
        return None
    meipass_path = Path(meipass)
    nested = meipass_path / STUDIO_DIST_SUBDIR
    if (nested / STUDIO_DIST_INDEX).is_file():
        return nested
    direct = meipass_path / STUDIO_DIST_INDEX
    if direct.is_file():
        return meipass_path
    return None


def _candidate_from_repo(environ: Mapping[str, str]) -> Path | None:
    del environ  # the repo layout is discovered from this module's path
    # The studio package lives at <repo>/src/kala_agent/studio; the dist
    # directory is the sibling <repo>/studio/dist.
    try:
        here = Path(__file__).resolve()
    except (OSError, RuntimeError):
        return None
    for parent in here.parents:
        candidate = parent.joinpath(*REPO_DIST_DIR_PARTS)
        try:
            index = candidate / STUDIO_DIST_INDEX
        except (OSError, ValueError):
            continue
        if index.is_file():
            return candidate
    return None


def _candidate_from_env(environ: Mapping[str, str]) -> Path | None:
    raw = environ.get(STUDIO_DIST_ENV_VAR)
    if not raw or not isinstance(raw, str):
        return None
    try:
        resolved = Path(raw).expanduser().resolve(strict=False)
    except (OSError, RuntimeError):
        return None
    if not (resolved / STUDIO_DIST_INDEX).is_file():
        return None
    return resolved


Resolver = Callable[[Mapping[str, str]], Path | None]

_RESOLVERS: Final[tuple[Resolver, ...]] = (
    _candidate_from_env,
    _candidate_from_meipass,
    _candidate_from_repo,
)


def resolve_static_root(environ: Mapping[str, str]) -> Path | None:
    """Return the static-asset root for the studio, or ``None``."""
    for resolver in _RESOLVERS:
        try:
            candidate = resolver(environ)
        except Exception:
            candidate = None
        if candidate is not None:
            return candidate
    return None


def list_candidate_sources() -> tuple[str, ...]:
    """Return the names of the resolvers used by :func:`resolve_static_root`."""
    return tuple(resolver.__name__ for resolver in _RESOLVERS)


__all__ = (
    "REPO_DIST_DIR_PARTS",
    "STUDIO_DIST_ENV_VAR",
    "STUDIO_DIST_INDEX",
    "STUDIO_DIST_SUBDIR",
    "list_candidate_sources",
    "resolve_static_root",
)
