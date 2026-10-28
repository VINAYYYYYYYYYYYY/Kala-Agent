from __future__ import annotations

import errno
import hashlib
import os
import secrets
import stat
import sys
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Final, Self

from kala_agent.config import Settings
from kala_agent.persistence.database import _validate_digest

_BUCKET_PREFIX_LENGTH: Final[int] = 2
_DIRECTORY_MODE: Final[int] = 0o700
_OBJECT_MODE: Final[int] = 0o600
_TEMPORARY_PREFIX: Final[str] = ".artifact-"
_TEMPORARY_SUFFIX: Final[str] = ".tmp"
_TEMPORARY_NAME_ATTEMPTS: Final[int] = 8
_TEMPORARY_NAME_BYTES: Final[int] = 16

_CLOEXEC: Final[int] = getattr(os, "O_CLOEXEC", 0)
_NOFOLLOW: Final[int] = getattr(os, "O_NOFOLLOW", 0)
_DIRECTORY_ONLY: Final[int] = getattr(os, "O_DIRECTORY", 0)
_NONBLOCK: Final[int] = getattr(os, "O_NONBLOCK", 0)
_BINARY: Final[int] = getattr(os, "O_BINARY", 0)

_DIRECTORY_OPEN_FLAGS: Final[int] = os.O_RDONLY | _DIRECTORY_ONLY | _NOFOLLOW | _CLOEXEC
_OBJECT_READ_FLAGS: Final[int] = os.O_RDONLY | _NONBLOCK | _NOFOLLOW | _CLOEXEC | _BINARY
_TEMPORARY_WRITE_FLAGS: Final[int] = (
    os.O_WRONLY | os.O_CREAT | os.O_EXCL | _NOFOLLOW | _CLOEXEC | _BINARY
)
_DIRECTORY_SYNC_IGNORED_ERRNOS: Final[frozenset[int]] = frozenset(
    {errno.EINVAL, errno.ENOSYS, errno.EPERM, errno.EACCES, errno.EROFS}
)
#: Every store operation resolves names through an open directory descriptor
#: (``dir_fd``/``O_NOFOLLOW``) and publishes with a same-directory hard link.
#: Windows has neither, so the store refuses to be used there instead of failing
#: later with a bare ``PermissionError`` from ``os.open`` on a directory. The
#: refusal is raised by the first store operation rather than by the constructor:
#: the studio builds a journal (and therefore a store) at startup on every
#: platform, and its health route must stay reachable even where artifact
#: publication is unsupported.
_DIRECTORY_DESCRIPTORS_SUPPORTED: Final[bool] = os.name == "posix"


class ArtifactStoreUnsupportedError(NotImplementedError):
    """Raised when the host cannot provide POSIX directory-descriptor semantics."""


def _entry_is_directory(directory_fd: int, name: str) -> bool:
    try:
        return stat.S_ISDIR(os.stat(name, dir_fd=directory_fd, follow_symlinks=False).st_mode)
    except OSError:
        return False


def _entry_is_symlink(directory_fd: int, name: str) -> bool:
    try:
        return stat.S_ISLNK(os.stat(name, dir_fd=directory_fd, follow_symlinks=False).st_mode)
    except OSError:
        return False


def _entry_is_regular_file(directory_fd: int, name: str) -> bool:
    try:
        return stat.S_ISREG(os.stat(name, dir_fd=directory_fd, follow_symlinks=False).st_mode)
    except OSError:
        return False


def _entry_matches_directory(objects_fd: int, name: str, directory_fd: int) -> bool:
    try:
        reached = os.stat(name, dir_fd=objects_fd, follow_symlinks=False)
        opened = os.fstat(directory_fd)
    except OSError:
        return False
    return reached.st_ino == opened.st_ino and reached.st_dev == opened.st_dev


def _reject_unusable_directory(directory_fd: int, name: str, error: OSError | None) -> None:
    if _entry_is_directory(directory_fd, name):
        return
    if _entry_is_symlink(directory_fd, name):
        raise ValueError("artifact object paths cannot traverse symlinks") from error
    raise ValueError("artifact object parent must be a directory") from error


def _create_directory_tree(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True, mode=_DIRECTORY_MODE)
    if path.is_symlink():
        raise ValueError("artifact root cannot be a symlink")
    if not path.is_dir():
        raise NotADirectoryError(path)


def _create_object_directory(path: Path) -> None:
    try:
        os.mkdir(path, _DIRECTORY_MODE)
    except FileExistsError:
        pass
    if path.is_symlink():
        raise ValueError("artifact object directory cannot be a symlink")
    if not path.is_dir():
        raise NotADirectoryError(path)


def _open_object_root(path: Path) -> int:
    try:
        return os.open(path, _DIRECTORY_OPEN_FLAGS)
    except OSError as error:
        if error.errno in (errno.ELOOP, errno.ENOTDIR):
            if path.is_symlink():
                raise ValueError("artifact object directory cannot be a symlink") from error
            raise NotADirectoryError(path) from error
        raise


def _create_bucket_directory(objects_fd: int, bucket_name: str) -> None:
    try:
        os.mkdir(bucket_name, _DIRECTORY_MODE, dir_fd=objects_fd)
    except FileExistsError:
        _reject_unusable_directory(objects_fd, bucket_name, None)


def _open_bucket_directory(objects_fd: int, bucket_name: str, digest: str) -> int:
    try:
        return os.open(bucket_name, _DIRECTORY_OPEN_FLAGS, dir_fd=objects_fd)
    except FileNotFoundError as error:
        raise FileNotFoundError(digest) from error
    except OSError as error:
        if error.errno in (errno.ELOOP, errno.ENOTDIR):
            _reject_unusable_directory(objects_fd, bucket_name, error)
        raise


def _create_temporary_object(bucket_fd: int) -> tuple[str, int]:
    for _ in range(_TEMPORARY_NAME_ATTEMPTS):
        name = f"{_TEMPORARY_PREFIX}{secrets.token_hex(_TEMPORARY_NAME_BYTES)}{_TEMPORARY_SUFFIX}"
        try:
            return name, os.open(name, _TEMPORARY_WRITE_FLAGS, _OBJECT_MODE, dir_fd=bucket_fd)
        except FileExistsError:
            continue
    raise FileExistsError("no temporary artifact name was available")


def _remove_entry(directory_fd: int, name: str) -> None:
    try:
        os.unlink(name, dir_fd=directory_fd)
    except FileNotFoundError:
        pass


def _sync_directory(directory_fd: int) -> None:
    try:
        os.fsync(directory_fd)
    except OSError as error:
        if error.errno not in _DIRECTORY_SYNC_IGNORED_ERRNOS:
            raise


@dataclass(frozen=True, slots=True)
class _Bucket:
    name: str
    objects_fd: int
    directory_fd: int


class ContentAddressedArtifactStore:
    """Immutable content-addressed objects addressed by the digest of their bytes.

    Every operation resolves its path components from an open directory
    descriptor with ``O_NOFOLLOW``, so a bucket or object replaced by a symlink
    between two calls can neither redirect a read nor redirect a write out of the
    configured root, and a directory that stops being the one at its own path is
    reported as a failed publication instead of a lost artifact. A bucket
    directory is created with an exclusive ``mkdir`` that tolerates a concurrent
    creator, publication is a same-directory hard link of a fully written and
    fsynced temporary file, and reads verify the digest of the bytes they return.
    Those steps are the only operations that mutate the store, so any number of
    threads or processes may share one root without coordinating with each other.
    """

    def __init__(self, root: Path, *, max_object_size: int = 1_000_000_000) -> None:
        if isinstance(max_object_size, bool) or not isinstance(max_object_size, int):
            raise TypeError("max_object_size must be an integer")
        if max_object_size < 0:
            raise ValueError("max_object_size must be nonnegative")
        expanded_root = root.expanduser()
        if expanded_root.is_symlink():
            raise ValueError("artifact root cannot be a symlink")
        resolved_root = expanded_root.resolve(strict=False)
        _create_directory_tree(resolved_root)
        objects_root = resolved_root / "objects"
        _create_object_directory(objects_root)
        self._root = resolved_root
        self._objects_root = objects_root
        self._max_object_size = max_object_size

    @classmethod
    def from_settings(
        cls,
        settings: Settings,
        *,
        max_object_size: int = 1_000_000_000,
    ) -> Self:
        return cls(settings.artifact_dir, max_object_size=max_object_size)

    @property
    def root(self) -> Path:
        return self._root

    @property
    def max_object_size(self) -> int:
        return self._max_object_size

    def key_for(self, digest: str) -> str:
        normalized_digest = _validate_digest(digest)
        return f"objects/{normalized_digest[:_BUCKET_PREFIX_LENGTH]}/{normalized_digest}"

    def put_bytes(self, data: bytes) -> str:
        if not isinstance(data, bytes):
            raise TypeError("artifact data must be bytes")
        if len(data) > self._max_object_size:
            raise ValueError("artifact exceeds the configured size limit")
        digest = hashlib.sha256(data).hexdigest()
        with self._bucket(digest, create=True) as bucket:
            self._publish(bucket, digest, data)
        return digest

    def put_text(self, text: str) -> str:
        if not isinstance(text, str):
            raise TypeError("artifact text must be str")
        return self.put_bytes(text.encode("utf-8"))

    def get_bytes(self, digest: str) -> bytes:
        normalized_digest = _validate_digest(digest)
        return self._read_bytes(normalized_digest)

    def get_text(self, digest: str) -> str:
        return self.get_bytes(digest).decode("utf-8")

    def contains(self, digest: str) -> bool:
        normalized_digest = _validate_digest(digest)
        try:
            with self._bucket(normalized_digest, create=False) as bucket:
                self._read_object(bucket, normalized_digest)
        except FileNotFoundError:
            return False
        return True

    def get_size(self, digest: str) -> int:
        normalized_digest = _validate_digest(digest)
        return len(self._read_bytes(normalized_digest))

    def _ensure_supported(self) -> None:
        if not _DIRECTORY_DESCRIPTORS_SUPPORTED:
            raise ArtifactStoreUnsupportedError(
                "the content-addressed artifact store requires POSIX directory descriptors "
                f"(dir_fd, O_NOFOLLOW, hard links) and is unsupported on {sys.platform}"
            )

    @contextmanager
    def _bucket(self, digest: str, *, create: bool) -> Iterator[_Bucket]:
        self._ensure_supported()
        bucket_name = digest[:_BUCKET_PREFIX_LENGTH]
        objects_fd = _open_object_root(self._objects_root)
        try:
            if create:
                _create_bucket_directory(objects_fd, bucket_name)
            directory_fd = _open_bucket_directory(objects_fd, bucket_name, digest)
            try:
                yield _Bucket(bucket_name, objects_fd, directory_fd)
            finally:
                os.close(directory_fd)
        finally:
            os.close(objects_fd)

    def _publish(self, bucket: _Bucket, digest: str, data: bytes) -> None:
        temporary_name = self._write_temporary_object(bucket.directory_fd, data)
        try:
            try:
                os.link(
                    temporary_name,
                    digest,
                    src_dir_fd=bucket.directory_fd,
                    dst_dir_fd=bucket.directory_fd,
                    follow_symlinks=False,
                )
            except FileExistsError as error:
                try:
                    existing = self._read_object(bucket, digest)
                except FileNotFoundError:
                    raise OSError("artifact publication failed") from error
                if existing != data:
                    raise ValueError("existing artifact failed integrity verification") from error
            except FileNotFoundError as error:
                raise OSError("artifact publication failed") from error
            else:
                _sync_directory(bucket.directory_fd)
                if not _entry_is_regular_file(bucket.directory_fd, digest):
                    raise OSError("artifact publication failed")
        finally:
            _remove_entry(bucket.directory_fd, temporary_name)
        if not _entry_matches_directory(bucket.objects_fd, bucket.name, bucket.directory_fd):
            raise OSError("artifact publication failed")

    def _write_temporary_object(self, directory_fd: int, data: bytes) -> str:
        name, file_descriptor = _create_temporary_object(directory_fd)
        try:
            temporary_file = os.fdopen(file_descriptor, "wb")
        except BaseException:
            os.close(file_descriptor)
            _remove_entry(directory_fd, name)
            raise
        try:
            with temporary_file:
                temporary_file.write(data)
                temporary_file.flush()
                os.fsync(temporary_file.fileno())
        except BaseException:
            _remove_entry(directory_fd, name)
            raise
        return name

    def _read_bytes(self, digest: str) -> bytes:
        with self._bucket(digest, create=False) as bucket:
            return self._read_object(bucket, digest)

    def _read_object(self, bucket: _Bucket, digest: str) -> bytes:
        try:
            file_descriptor = os.open(digest, _OBJECT_READ_FLAGS, dir_fd=bucket.directory_fd)
        except FileNotFoundError as error:
            raise FileNotFoundError(digest) from error
        except OSError as error:
            if error.errno == errno.ELOOP:
                raise ValueError("artifact objects cannot be symlinks") from error
            raise ValueError("artifact object could not be opened safely") from error
        try:
            artifact_file = os.fdopen(file_descriptor, "rb")
        except BaseException:
            os.close(file_descriptor)
            raise
        with artifact_file:
            if not stat.S_ISREG(os.fstat(artifact_file.fileno()).st_mode):
                raise ValueError("artifact object must be a regular file")
            data = artifact_file.read(self._max_object_size + 1)
        if len(data) > self._max_object_size:
            raise ValueError("stored artifact exceeds the configured size limit")
        if hashlib.sha256(data).hexdigest() != digest:
            raise ValueError("artifact digest verification failed")
        return data
