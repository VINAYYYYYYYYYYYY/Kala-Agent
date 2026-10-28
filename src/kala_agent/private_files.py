"""Owner-only local files.

The operator data directory and the credential files inside it are written by
more than one surface (the core API's launch token, the studio's launch token
and the studio settings file). This module holds the one portable writer they
share: a same-directory ``tempfile`` + ``os.replace`` replacement instead of
``O_TRUNC`` + ``os.fchmod``, and a Windows ACL step for the files that hold a
secret.

``os.fchmod`` does not exist on Windows, and on POSIX the mode of a truncated
file is the mode it already had. Both problems disappear when the new content
is written to a fresh temporary file, tightened, and renamed over the old one.

On Windows, POSIX mode bits are not an access-control mechanism: ``os.chmod``
changes a read-only attribute and nothing else, so a credential file keeps
whatever ACL it inherited from its directory. ``restrict_windows_acl`` grants
the file to the current user only (via the system ``icacls`` tool) so a
shared, roaming, or permissive parent directory cannot expose it.
"""

from __future__ import annotations

import logging
import os
import subprocess
import tempfile
from pathlib import Path
from typing import Final

LOGGER: Final[logging.Logger] = logging.getLogger("kala_agent.private_files")
IS_WINDOWS: Final[bool] = os.name == "nt"
PRIVATE_FILE_MODE: Final[int] = 0o600
PRIVATE_DIRECTORY_MODE: Final[int] = 0o700


def restrict_windows_acl(path: Path) -> None:
    """Grant ``path`` to the current Windows user only.

    Inherited access entries are removed and a single full-control entry for
    ``%USERNAME%`` is added, which is the same approach the Solid Edge bridge
    uses for its credential files. A non-Windows host is a no-op. A host that
    cannot be tightened raises :class:`OSError` so the caller can report it
    instead of leaving a secret looking private when it is not.
    """
    if not IS_WINDOWS:
        return
    user = os.environ.get("USERNAME")
    if not user:
        raise OSError("USERNAME is unavailable; the file ACL cannot be restricted")
    completed = subprocess.run(
        ["icacls", str(path), "/inheritance:r", "/grant:r", f"{user}:F"],
        check=False,
        capture_output=True,
        text=True,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )
    if completed.returncode != 0:
        raise OSError(f"icacls failed to restrict {path.name}: {completed.returncode}")


def tighten_file_permissions(path: Path, mode: int = PRIVATE_FILE_MODE) -> None:
    """Restrict an existing file to its owner, by ACL on Windows and by mode on POSIX.

    Failure never raises: a token or settings file that could not be tightened
    is still usable for this launch, and the failure is reported at warning
    level so a broken ACL does not look like a private file.
    """
    try:
        if IS_WINDOWS:
            restrict_windows_acl(path)
        else:
            os.chmod(path, mode)
    except OSError as error:
        LOGGER.warning(
            "could not restrict %s to the current user: %s", path.name, type(error).__name__
        )


def atomic_write_private_text(path: Path, content: str, mode: int = PRIVATE_FILE_MODE) -> None:
    """Atomically replace ``path`` with ``content`` and restrict it to the owner.

    The temporary file lives in the destination directory so ``os.replace`` is
    atomic on the same filesystem. It is tightened before the rename, which
    means the readable name never points at a file with looser access than the
    final one, and the rename itself removes the previous inode.
    """
    directory = path.parent
    directory.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=path.name + ".", suffix=".tmp", dir=str(directory)
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            handle.write(content)
            handle.flush()
            try:
                os.fsync(handle.fileno())
            except (OSError, AttributeError):
                pass
        # Tighten before the rename: the readable name then never points at a
        # file with looser access than the final one, and the rename replaces
        # the previous inode (and with it the previous file's access entries).
        _tighten_or_raise(temporary, mode)
        os.replace(temporary, path)
    except BaseException:
        try:
            temporary.unlink()
        except OSError:
            pass
        raise


def _tighten_or_raise(path: Path, mode: int) -> None:
    """Tighten a file that has not been published yet, propagating a failure."""
    if IS_WINDOWS:
        restrict_windows_acl(path)
    else:
        os.chmod(path, mode)


__all__ = (
    "IS_WINDOWS",
    "PRIVATE_DIRECTORY_MODE",
    "PRIVATE_FILE_MODE",
    "atomic_write_private_text",
    "restrict_windows_acl",
    "tighten_file_permissions",
)
