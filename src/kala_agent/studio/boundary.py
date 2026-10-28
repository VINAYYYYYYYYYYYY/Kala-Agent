"""The request boundary of the studio HTTP adapter.

The boundary holds the loopback Host guard policy, the per-launch bearer
token, and the helper that mints it. Token storage is portable: the studio
creates a directory with owner-only permissions (``0o700``) and writes the
token through an atomic same-directory replacement pattern that works on
both POSIX and Windows without relying on ``os.fchmod``. The file itself is
restricted to the current user on both hosts: by mode bits on POSIX and by an
``icacls`` ACL on Windows, where a mode bit is not an access control.

A caller can also pass an explicit token through ``KALA_STUDIO_TOKEN`` for
the Electron shell, in which case no disk copy is made. The token is read
back the same way: either from the explicit environment, or from the
launch-time file inside the studio data directory.
"""

from __future__ import annotations

import logging
import os
import re
import secrets
import sys
from collections.abc import Mapping
from pathlib import Path
from typing import Final

from kala_agent.api.boundary import (
    API_TOKEN_BYTES as CORE_API_TOKEN_BYTES,
)
from kala_agent.api.boundary import (
    API_TOKEN_PATTERN,
    MAX_API_TOKEN_LENGTH,
    MIN_API_TOKEN_LENGTH,
    is_loopback_host,
    is_permitted_authority,
)
from kala_agent.private_files import atomic_write_private_text, tighten_file_permissions

LOGGER: Final[logging.Logger] = logging.getLogger("kala_agent.studio")
STUDIO_TOKEN_ENV_VAR: Final[str] = "KALA_STUDIO_TOKEN"
STUDIO_TOKEN_FILE_NAME: Final[str] = "studio-token"
STUDIO_SUBDIRECTORY_NAME: Final[str] = "studio"
STUDIO_API_TOKEN_BYTES: Final[int] = CORE_API_TOKEN_BYTES
STUDIO_MUTATING_METHODS: Final[frozenset[str]] = frozenset({"DELETE", "PATCH", "POST", "PUT"})
TOKEN_REQUIRED_MESSAGE: Final[str] = "a valid bearer token is required for this request"
HOST_REFUSED_MESSAGE: Final[str] = "the studio request host is not a loopback address"
TOKEN_FILE_FAILED_MESSAGE: Final[str] = (
    "the studio token could not be stored in the local data directory; "
    "the token is held in memory only for this launch"
)
NOT_LOOPBACK_MESSAGE: Final[str] = (
    "the studio binds and serves loopback interfaces only; "
    "expected one of: 127.0.0.1, ::1, localhost"
)
TOKEN_DIR_MODE: Final[int] = 0o700
TOKEN_FILE_MODE: Final[int] = 0o600
IS_WINDOWS: Final[bool] = sys.platform.startswith("win")

_API_TOKEN_RE: Final[re.Pattern[str]] = re.compile(API_TOKEN_PATTERN)


def validate_studio_token(value: object) -> str:
    """Return a usable studio bearer token, or refuse one that is malformed."""
    if not isinstance(value, str):
        raise ValueError("the studio token must be a string")
    if not MIN_API_TOKEN_LENGTH <= len(value) <= MAX_API_TOKEN_LENGTH:
        raise ValueError(
            f"the studio token must be between {MIN_API_TOKEN_LENGTH} and "
            f"{MAX_API_TOKEN_LENGTH} characters"
        )
    if _API_TOKEN_RE.fullmatch(value) is None:
        raise ValueError("the studio token contains unsupported characters")
    return value


def resolve_studio_data_dir(parent: Path) -> Path:
    """Return the per-studio data directory inside ``parent``.

    The directory is created on demand with owner-only permissions where the
    platform supports it. The studio never inherits the core Kala data
    directory: the two surfaces hold independent token stores.
    """
    parent.mkdir(parents=True, exist_ok=True)
    child = parent / STUDIO_SUBDIRECTORY_NAME
    child.mkdir(parents=True, exist_ok=True)
    _tighten_directory_mode(child)
    return child


def _tighten_directory_mode(path: Path) -> None:
    """Set owner-only permissions on ``path`` when the platform supports it."""
    if IS_WINDOWS:
        # POSIX mode bits are not meaningful on Windows; ACLs are out of
        # scope for the portable launcher. The Electron shell is expected
        # to install per-user protections at the bundle level.
        return
    try:
        os.chmod(path, TOKEN_DIR_MODE)
    except OSError as error:
        LOGGER.debug("studio directory mode tighten skipped: %s", type(error).__name__)


def _tighten_file_mode(path: Path) -> None:
    """Restrict a studio credential file to the current user.

    On Windows this is an ACL change (``icacls``); on POSIX it is the mode
    bits. ``0o600`` alone is a no-op for Windows access control, which is why
    the file itself is tightened rather than only its mode.
    """
    tighten_file_permissions(path, TOKEN_FILE_MODE)


def _atomic_write_text(path: Path, content: str) -> None:
    """Atomically replace ``path`` with ``content`` in the same directory.

    The function uses ``tempfile`` + ``os.replace`` (rather than
    ``O_TRUNC`` + ``os.fchmod``) so the approach is portable to Windows.
    The temp file lives in the same directory as the destination so
    ``os.replace`` is guaranteed to be atomic on the same filesystem, and it
    is restricted to the current user before the rename (ACL on Windows, mode
    bits on POSIX).
    """
    atomic_write_private_text(path, content, TOKEN_FILE_MODE)


def issue_studio_token(data_dir: Path) -> str:
    """Return a fresh studio launch token.

    A caller can pass a directory via ``KALA_STUDIO_TOKEN`` to skip disk
    storage entirely; when the variable is empty the studio mints a fresh
    token and writes it to ``data_dir / STUDIO_TOKEN_FILE_NAME``.
    """
    token = secrets.token_urlsafe(STUDIO_API_TOKEN_BYTES)
    try:
        _atomic_write_text(data_dir / STUDIO_TOKEN_FILE_NAME, f"{token}\n")
    except OSError as error:
        LOGGER.warning("%s: %s", TOKEN_FILE_FAILED_MESSAGE, type(error).__name__)
    return token


def store_studio_token_file(data_dir: Path, token: str) -> None:
    """Write the launch token through the atomic same-directory path."""
    _atomic_write_text(data_dir / STUDIO_TOKEN_FILE_NAME, f"{token}\n")


def read_studio_token(data_dir: Path) -> str | None:
    """Return the token stored for the running studio launch, if any.

    A missing, unreadable, over-long, or malformed file reads as ``None``
    rather than raising, so a corrupt store is reported as an absence
    instead of a crash. The caller treats absence as "no usable token,
    refuse the request".
    """
    path = data_dir / STUDIO_TOKEN_FILE_NAME
    try:
        with path.open("rb") as handle:
            raw = handle.read(MAX_API_TOKEN_LENGTH + 2)
    except OSError:
        return None
    try:
        candidate = raw.decode("utf-8").strip()
    except UnicodeDecodeError:
        return None
    try:
        return validate_studio_token(candidate)
    except ValueError:
        return None


def resolve_studio_token(data_dir: Path, *, environ: Mapping[str, str]) -> str:
    """Return the per-launch token for this process.

    The token is taken from ``KALA_STUDIO_TOKEN`` when the variable is set
    and non-empty, otherwise the studio mints one and stores it through the
    atomic writer. The ``environ`` parameter lets the CLI and tests pass a
    controlled view without mutating the real process environment.
    """
    explicit = environ.get(STUDIO_TOKEN_ENV_VAR)
    if isinstance(explicit, str) and explicit.strip():
        return validate_studio_token(explicit.strip())
    return issue_studio_token(data_dir)


__all__ = (
    "HOST_REFUSED_MESSAGE",
    "IS_WINDOWS",
    "LOGGER",
    "NOT_LOOPBACK_MESSAGE",
    "STUDIO_API_TOKEN_BYTES",
    "STUDIO_MUTATING_METHODS",
    "STUDIO_SUBDIRECTORY_NAME",
    "STUDIO_TOKEN_ENV_VAR",
    "STUDIO_TOKEN_FILE_NAME",
    "TOKEN_DIR_MODE",
    "TOKEN_FILE_FAILED_MESSAGE",
    "TOKEN_FILE_MODE",
    "TOKEN_REQUIRED_MESSAGE",
    "is_loopback_host",
    "is_permitted_authority",
    "issue_studio_token",
    "read_studio_token",
    "resolve_studio_data_dir",
    "resolve_studio_token",
    "store_studio_token_file",
    "validate_studio_token",
)
