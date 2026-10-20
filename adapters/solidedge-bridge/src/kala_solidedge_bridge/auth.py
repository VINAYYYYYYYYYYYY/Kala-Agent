"""Bearer-token creation, storage, and constant-time verification.

The token file is named ``bridge.token`` under the operator data directory so a
Windows-local client resolves exactly the same file the service used. The only
command that prints the token is the explicit ``token`` command; this module
never logs it.
"""

from __future__ import annotations

import hmac
import os
import secrets
from pathlib import Path

from .errors import UnavailableError
from .workspace import (
    atomic_write_bytes,
    default_data_dir,
    ensure_private_dir,
    restrict_windows_acl,
)

TOKEN_FILENAME = "bridge.token"
TOKEN_BYTES = 32
_MIN_TOKEN_CHARS = 16
_MAX_TOKEN_CHARS = 256


def token_path(data_dir: Path) -> Path:
    """Return the one token path shared by the service and local clients."""
    return data_dir / TOKEN_FILENAME


def _validate_token_text(token: str) -> str:
    candidate = token.strip()
    if not (_MIN_TOKEN_CHARS <= len(candidate) <= _MAX_TOKEN_CHARS):
        raise UnavailableError("bridge token file is empty or malformed")
    return candidate


def ensure_token(data_dir: Path) -> str:
    """Create a random token once and return the current token.

    Creation uses an exclusive create so two concurrent commands cannot install
    different tokens. Existing files are read, not rewritten.
    """
    path = token_path(data_dir)
    ensure_private_dir(path.parent)
    if path.exists():
        return read_token(data_dir)

    token = secrets.token_urlsafe(TOKEN_BYTES)
    try:
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        return read_token(data_dir)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(token.encode("ascii"))
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(path, 0o600)
    except BaseException:
        try:
            path.unlink()
        except OSError:
            pass
        raise
    restrict_windows_acl(path)
    return token


def read_token(data_dir: Path) -> str:
    """Read and validate the token file without ever logging its contents."""
    path = token_path(data_dir)
    try:
        raw = path.read_text(encoding="ascii")
    except FileNotFoundError as exc:
        raise UnavailableError(
            "the bridge token file does not exist; run the token command"
        ) from exc
    except (OSError, UnicodeError) as exc:
        raise UnavailableError("the bridge token file could not be read") from exc
    return _validate_token_text(raw)


def token_matches(expected: str, provided: str) -> bool:
    """Compare a presented bearer token in constant time."""
    if not provided:
        return False
    return hmac.compare_digest(expected.encode("utf-8"), provided.encode("utf-8"))


def resolve_data_dir(explicit: Path | None) -> Path:
    """Return an explicit data directory or the platform default."""
    return default_data_dir() if explicit is None else explicit


def secret_mode() -> int:
    """The POSIX mode used for secret files (0600 for tests too)."""
    return 0o600


def write_secret(path: Path, value: str) -> None:
    """Write one secret atomically with private permissions."""
    atomic_write_bytes(path, value.encode("utf-8"), mode=secret_mode())
    restrict_windows_acl(path)


__all__ = (
    "TOKEN_FILENAME",
    "ensure_token",
    "read_token",
    "resolve_data_dir",
    "token_matches",
    "token_path",
)
