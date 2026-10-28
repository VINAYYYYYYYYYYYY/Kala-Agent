from __future__ import annotations

import errno
import hashlib
import os
import stat
from pathlib import Path
from typing import cast

import pytest

from kala_agent.config import Settings
from kala_agent.persistence import artifacts as artifacts_module
from kala_agent.persistence.artifacts import (
    _BINARY,
    _CLOEXEC,
    _DIRECTORY_ONLY,
    _DIRECTORY_OPEN_FLAGS,
    _NOFOLLOW,
    _NONBLOCK,
    _OBJECT_READ_FLAGS,
    _TEMPORARY_NAME_ATTEMPTS,
    _TEMPORARY_WRITE_FLAGS,
    ContentAddressedArtifactStore,
    _create_bucket_directory,
    _create_directory_tree,
    _create_object_directory,
    _create_temporary_object,
    _entry_is_directory,
    _entry_is_regular_file,
    _entry_is_symlink,
    _open_bucket_directory,
    _open_object_root,
    _reject_unusable_directory,
    _remove_entry,
    _sync_directory,
)


@pytest.fixture
def store(tmp_path: Path) -> ContentAddressedArtifactStore:
    return ContentAddressedArtifactStore(tmp_path / "artifacts", max_object_size=64)


def test_store_refuses_a_host_without_directory_descriptors(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A host without ``dir_fd``/``O_NOFOLLOW`` gets a clear platform refusal.

    Windows cannot open a directory and address its entries through a
    descriptor, so every store operation is unsupported there. The store says
    so instead of failing later with a bare ``PermissionError`` from
    ``os.open``. Construction still succeeds because the studio builds its
    journal — and therefore a store — while creating the app on every
    platform; only the first artifact operation refuses.
    """
    monkeypatch.setattr(artifacts_module, "_DIRECTORY_DESCRIPTORS_SUPPORTED", False)
    store = ContentAddressedArtifactStore(tmp_path / "artifacts")

    with pytest.raises(artifacts_module.ArtifactStoreUnsupportedError) as error:
        store.put_bytes(b"artifact")

    assert "POSIX directory descriptors" in str(error.value)
    with pytest.raises(artifacts_module.ArtifactStoreUnsupportedError):
        store.get_bytes("0" * 64)
    with pytest.raises(artifacts_module.ArtifactStoreUnsupportedError):
        store.contains("0" * 64)


def _objects_root(store: ContentAddressedArtifactStore) -> Path:
    return store.root / "objects"


def test_artifact_store_rejects_bad_max_object_size(tmp_path: Path) -> None:
    with pytest.raises(TypeError, match="max_object_size"):
        ContentAddressedArtifactStore(tmp_path, max_object_size=cast(int, True))
    with pytest.raises(TypeError, match="max_object_size"):
        ContentAddressedArtifactStore(tmp_path, max_object_size=cast(int, "10"))
    with pytest.raises(ValueError, match="max_object_size"):
        ContentAddressedArtifactStore(tmp_path, max_object_size=-1)


def test_artifact_store_rejects_symlinked_root(tmp_path: Path) -> None:
    real = tmp_path / "real"
    real.mkdir()
    linked = tmp_path / "linked"
    linked.symlink_to(real)
    with pytest.raises(ValueError, match="root cannot be a symlink"):
        ContentAddressedArtifactStore(linked)


def test_artifact_store_rejects_symlinked_object_root(tmp_path: Path) -> None:
    root = tmp_path / "artifacts"
    root.mkdir()
    elsewhere = tmp_path / "elsewhere"
    elsewhere.mkdir()
    (root / "objects").symlink_to(elsewhere)
    with pytest.raises(ValueError, match="object directory cannot be a symlink"):
        ContentAddressedArtifactStore(root)


def test_artifact_store_rejects_non_directory_root(tmp_path: Path) -> None:
    plain = tmp_path / "plain"
    plain.write_text("x")
    with pytest.raises(NotADirectoryError):
        ContentAddressedArtifactStore(plain / "nested" / "deeper")


def test_artifact_store_rejects_non_directory_object_root(tmp_path: Path) -> None:
    root = tmp_path / "artifacts"
    root.mkdir()
    (root / "objects").write_text("x")
    with pytest.raises(NotADirectoryError):
        ContentAddressedArtifactStore(root)


def test_artifact_store_exposes_configuration(tmp_path: Path) -> None:
    store = ContentAddressedArtifactStore(tmp_path / "artifacts", max_object_size=32)
    assert store.root == (tmp_path / "artifacts").resolve()
    assert store.max_object_size == 32


def test_artifact_store_from_settings_uses_artifact_dir(tmp_path: Path) -> None:
    settings = Settings(data_dir=tmp_path / "configured")
    store = ContentAddressedArtifactStore.from_settings(settings, max_object_size=16)
    assert store.root == settings.artifact_dir.resolve()
    assert store.max_object_size == 16


def test_artifact_store_expands_user_in_root(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("HOME", str(tmp_path))
    store = ContentAddressedArtifactStore(Path("~/artifacts"))
    assert store.root == (tmp_path / "artifacts").resolve()


def test_key_for_buckets_by_digest_prefix(store: ContentAddressedArtifactStore) -> None:
    digest = hashlib.sha256(b"x").hexdigest()
    assert store.key_for(digest) == f"objects/{digest[:2]}/{digest}"


def test_key_for_rejects_invalid_digest(store: ContentAddressedArtifactStore) -> None:
    with pytest.raises(ValueError):
        store.key_for("not-a-digest")
    with pytest.raises(ValueError):
        store.get_bytes("short")
    with pytest.raises(ValueError):
        store.get_size("z" * 64)
    with pytest.raises(ValueError):
        store.contains("")


def test_put_bytes_roundtrips_and_is_idempotent(
    store: ContentAddressedArtifactStore,
) -> None:
    digest = store.put_bytes(b"payload")
    assert store.get_bytes(digest) == b"payload"
    assert store.get_size(digest) == 7
    assert store.contains(digest) is True
    assert store.put_bytes(b"payload") == digest
    assert len(list(store.root.rglob(digest))) == 1


def test_put_bytes_rejects_wrong_type(store: ContentAddressedArtifactStore) -> None:
    with pytest.raises(TypeError, match="must be bytes"):
        store.put_bytes(cast(bytes, "payload"))


def test_put_bytes_rejects_oversized_payload(store: ContentAddressedArtifactStore) -> None:
    with pytest.raises(ValueError, match="size limit"):
        store.put_bytes(b"x" * 65)


def test_put_text_requires_str(store: ContentAddressedArtifactStore) -> None:
    with pytest.raises(TypeError, match="must be str"):
        store.put_text(cast(str, b"payload"))


def test_put_text_encodes_unicode(store: ContentAddressedArtifactStore) -> None:
    text = "καλός — 漢字 🎯"
    digest = store.put_text(text)
    assert store.get_text(digest) == text


def test_get_text_rejects_invalid_utf8(store: ContentAddressedArtifactStore) -> None:
    digest = store.put_bytes(b"\xff\xfe")
    with pytest.raises(UnicodeDecodeError):
        store.get_text(digest)


def test_contains_reports_missing_object(store: ContentAddressedArtifactStore) -> None:
    assert store.contains(hashlib.sha256(b"absent").hexdigest()) is False


def test_get_bytes_reports_missing_object(store: ContentAddressedArtifactStore) -> None:
    missing = hashlib.sha256(b"absent").hexdigest()
    with pytest.raises(FileNotFoundError) as error:
        store.get_bytes(missing)
    assert str(error.value) == missing


def test_publish_rejects_conflicting_existing_object(
    store: ContentAddressedArtifactStore, monkeypatch: pytest.MonkeyPatch
) -> None:
    data = b"payload"
    digest = store.put_bytes(data)
    original = store._read_bytes

    def corrupt(_bucket_fd: int, normalized: str) -> bytes:
        if normalized == digest:
            return b"tampered"
        return original(normalized)

    monkeypatch.setattr(store, "_read_object", corrupt)
    with pytest.raises(ValueError, match="integrity verification"):
        store.put_bytes(data)


def test_publish_rejects_non_regular_published_object(
    store: ContentAddressedArtifactStore, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(artifacts_module, "_entry_is_regular_file", lambda *_a: False)
    with pytest.raises(OSError, match="publication failed"):
        store.put_bytes(b"payload")


def test_publish_rejects_oversized_stored_object(
    store: ContentAddressedArtifactStore, monkeypatch: pytest.MonkeyPatch
) -> None:
    data = b"payload"
    digest = store.put_bytes(data)
    monkeypatch.setattr(store, "_max_object_size", 1)
    with pytest.raises(ValueError, match="exceeds the configured size limit"):
        store.get_bytes(digest)


def test_read_object_rejects_digest_mismatch(
    store: ContentAddressedArtifactStore, monkeypatch: pytest.MonkeyPatch
) -> None:
    data = b"payload"
    digest = store.put_bytes(data)
    bucket = digest[:2]
    (store.root / "objects" / bucket / digest).write_bytes(b"other")
    with pytest.raises(ValueError, match="digest verification failed"):
        store.get_bytes(digest)


def test_read_object_rejects_non_regular_object(
    store: ContentAddressedArtifactStore,
) -> None:
    digest = "f" * 64
    bucket = _objects_root(store) / "ff"
    bucket.mkdir(parents=True)
    os.mkfifo(bucket / digest)
    with pytest.raises(ValueError, match="regular file"):
        store.get_bytes(digest)


def test_read_object_rejects_symlinked_object(
    store: ContentAddressedArtifactStore, tmp_path: Path
) -> None:
    digest = store.put_bytes(b"payload")
    bucket = _objects_root(store) / digest[:2]
    object_path = bucket / digest
    object_path.unlink()
    outside = tmp_path / "outside.txt"
    outside.write_bytes(b"payload")
    object_path.symlink_to(outside)
    with pytest.raises(ValueError, match="cannot be symlinks"):
        store.get_bytes(digest)


def test_read_object_reports_unopenable_object_for_directory(
    store: ContentAddressedArtifactStore,
) -> None:
    digest = store.put_bytes(b"payload")
    bucket = _objects_root(store) / digest[:2]
    (bucket / digest).unlink()
    (bucket / digest).symlink_to(_objects_root(store))
    with pytest.raises(ValueError, match="cannot be symlinks"):
        store.get_bytes(digest)


@pytest.mark.skipif(
    os.name != "posix",
    reason="temporary-object writes need dir_fd, which only POSIX os.open provides",
)
def test_write_temporary_object_cleans_up_on_failure(
    store: ContentAddressedArtifactStore, monkeypatch: pytest.MonkeyPatch
) -> None:
    def explode(*_args: object, **_kwargs: object) -> None:
        raise OSError(errno.ENOSPC, "full")

    monkeypatch.setattr(os, "fsync", explode)
    with pytest.raises(OSError):
        store.put_bytes(b"payload")
    leaked = [
        entry for entry in _objects_root(store).rglob("*") if entry.name.startswith(".artifact-")
    ]
    assert leaked == []


def test_create_directory_tree_reports_symlink(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(Path, "is_symlink", lambda self: True)
    with pytest.raises(ValueError, match="root cannot be a symlink"):
        _create_directory_tree(tmp_path / "root")


def test_create_directory_tree_reports_non_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(Path, "is_symlink", lambda self: False)
    monkeypatch.setattr(Path, "is_dir", lambda self: False)
    with pytest.raises(NotADirectoryError):
        _create_directory_tree(tmp_path / "root")


def test_create_object_directory_tolerates_existing(
    tmp_path: Path,
) -> None:
    objects = tmp_path / "objects"
    _create_object_directory(objects)
    _create_object_directory(objects)
    assert stat.S_IMODE(objects.stat().st_mode) == 0o700


def test_create_object_directory_reports_symlink(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(Path, "is_symlink", lambda self: True)
    with pytest.raises(ValueError, match="object directory cannot be a symlink"):
        _create_object_directory(tmp_path / "objects")


def test_create_object_directory_reports_non_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(Path, "is_symlink", lambda self: False)
    monkeypatch.setattr(Path, "is_dir", lambda self: False)
    with pytest.raises(NotADirectoryError):
        _create_object_directory(tmp_path / "objects")


def test_open_object_root_reports_symlinked_root(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def explode(_path: Path, _flags: int) -> int:
        raise OSError(errno.ELOOP, "loop")

    monkeypatch.setattr(os, "open", explode)
    monkeypatch.setattr(Path, "is_symlink", lambda self: True)
    with pytest.raises(ValueError, match="object directory cannot be a symlink"):
        _open_object_root(tmp_path / "objects")


def test_open_object_root_reports_non_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def explode(_path: Path, _flags: int) -> int:
        raise OSError(errno.ENOTDIR, "not a directory")

    monkeypatch.setattr(os, "open", explode)
    monkeypatch.setattr(Path, "is_symlink", lambda self: False)
    with pytest.raises(NotADirectoryError):
        _open_object_root(tmp_path / "objects")


def test_open_object_root_reraises_unexpected_errors(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def explode(_path: Path, _flags: int) -> int:
        raise OSError(errno.EACCES, "denied")

    monkeypatch.setattr(os, "open", explode)
    with pytest.raises(OSError) as error:
        _open_object_root(tmp_path / "objects")
    assert error.value.errno == errno.EACCES


def test_open_bucket_directory_reports_missing_bucket(
    tmp_path: Path,
) -> None:
    objects = tmp_path / "objects"
    objects.mkdir()
    objects_fd = os.open(objects, _DIRECTORY_OPEN_FLAGS)
    try:
        with pytest.raises(FileNotFoundError) as error:
            _open_bucket_directory(objects_fd, "zz", "z" * 64)
        assert str(error.value) == "z" * 64
    finally:
        os.close(objects_fd)


def test_open_bucket_directory_reports_unusable_bucket(
    tmp_path: Path,
) -> None:
    objects = tmp_path / "objects"
    objects.mkdir()
    (objects / "aa").write_text("x")
    objects_fd = os.open(objects, _DIRECTORY_OPEN_FLAGS)
    try:
        with pytest.raises(ValueError, match="must be a directory"):
            _open_bucket_directory(objects_fd, "aa", "a" * 64)
    finally:
        os.close(objects_fd)


def test_open_bucket_directory_reraises_unexpected_errors(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def explode(_name: str, _flags: int, *, dir_fd: int) -> int:
        raise OSError(errno.EACCES, "denied")

    monkeypatch.setattr(os, "open", explode)
    with pytest.raises(OSError) as error:
        _open_bucket_directory(3, "aa", "a" * 64)
    assert error.value.errno == errno.EACCES


def test_create_bucket_directory_rejects_symlinked_bucket(
    tmp_path: Path,
) -> None:
    objects = tmp_path / "objects"
    objects.mkdir()
    elsewhere = tmp_path / "elsewhere"
    elsewhere.mkdir()
    (objects / "aa").symlink_to(elsewhere)
    objects_fd = os.open(objects, _DIRECTORY_OPEN_FLAGS)
    try:
        with pytest.raises(ValueError, match="cannot traverse symlinks"):
            _create_bucket_directory(objects_fd, "aa")
    finally:
        os.close(objects_fd)


def test_create_bucket_directory_tolerates_concurrent_creator(tmp_path: Path) -> None:
    objects = tmp_path / "objects"
    objects.mkdir()
    objects_fd = os.open(objects, _DIRECTORY_OPEN_FLAGS)
    try:
        _create_bucket_directory(objects_fd, "aa")
        _create_bucket_directory(objects_fd, "aa")
    finally:
        os.close(objects_fd)
    assert (objects / "aa").is_dir()


def test_create_temporary_object_gives_up_after_attempts(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def collide(_name: str, _flags: int, _mode: int, *, dir_fd: int) -> int:
        raise FileExistsError(errno.EEXIST, "exists")

    monkeypatch.setattr(os, "open", collide)
    with pytest.raises(FileExistsError, match="no temporary artifact name"):
        _create_temporary_object(3)
    assert _TEMPORARY_NAME_ATTEMPTS == 8


def test_create_temporary_object_retries_collision(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    directory_fd = os.open(tmp_path, _DIRECTORY_OPEN_FLAGS)
    real_open = os.open
    attempts: list[str] = []

    def once_then_real(name: str, flags: int, mode: int = 0o600, *, dir_fd: int) -> int:
        if not attempts:
            attempts.append(name)
            raise FileExistsError(errno.EEXIST, "exists")
        return real_open(name, flags, mode, dir_fd=dir_fd)

    monkeypatch.setattr(os, "open", once_then_real)
    name, descriptor = _create_temporary_object(directory_fd)
    os.close(descriptor)
    assert len(attempts) == 1
    assert name.startswith(".artifact-")
    assert name.endswith(".tmp")


def test_remove_entry_tolerates_missing(tmp_path: Path) -> None:
    directory_fd = os.open(tmp_path, _DIRECTORY_OPEN_FLAGS)
    try:
        _remove_entry(directory_fd, "absent")
        (tmp_path / "present").write_text("x")
        _remove_entry(directory_fd, "present")
        assert not (tmp_path / "present").exists()
    finally:
        os.close(directory_fd)


def test_sync_directory_ignores_known_errnos(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    for ignored in sorted(artifacts_module._DIRECTORY_SYNC_IGNORED_ERRNOS):

        def explode(_fd: int, code: int = ignored) -> None:
            raise OSError(code, "ignored")

        monkeypatch.setattr(os, "fsync", explode)
        _sync_directory(1)


def test_sync_directory_reraises_unexpected_errors(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def explode(_fd: int) -> None:
        raise OSError(errno.EIO, "io error")

    monkeypatch.setattr(os, "fsync", explode)
    with pytest.raises(OSError) as error:
        _sync_directory(1)
    assert error.value.errno == errno.EIO


def test_sync_directory_succeeds_silently(monkeypatch: pytest.MonkeyPatch) -> None:
    synced: list[int] = []
    monkeypatch.setattr(os, "fsync", lambda fd: synced.append(fd))
    _sync_directory(7)
    assert synced == [7]


def test_entry_predicates_report_missing_entries(tmp_path: Path) -> None:
    directory_fd = os.open(tmp_path, _DIRECTORY_OPEN_FLAGS)
    try:
        assert _entry_is_directory(directory_fd, "absent") is False
        assert _entry_is_symlink(directory_fd, "absent") is False
        assert _entry_is_regular_file(directory_fd, "absent") is False
    finally:
        os.close(directory_fd)


def test_entry_predicates_classify_entries(tmp_path: Path) -> None:
    (tmp_path / "dir").mkdir()
    (tmp_path / "file").write_text("x")
    (tmp_path / "link").symlink_to(tmp_path / "file")
    directory_fd = os.open(tmp_path, _DIRECTORY_OPEN_FLAGS)
    try:
        assert _entry_is_directory(directory_fd, "dir") is True
        assert _entry_is_symlink(directory_fd, "dir") is False
        assert _entry_is_regular_file(directory_fd, "file") is True
        assert _entry_is_symlink(directory_fd, "link") is True
        assert _entry_is_directory(directory_fd, "link") is False
    finally:
        os.close(directory_fd)


def test_reject_unusable_directory_accepts_real_directory(tmp_path: Path) -> None:
    (tmp_path / "dir").mkdir()
    directory_fd = os.open(tmp_path, _DIRECTORY_OPEN_FLAGS)
    try:
        _reject_unusable_directory(directory_fd, "dir", None)
    finally:
        os.close(directory_fd)


def test_reject_unusable_directory_rejects_symlink(tmp_path: Path) -> None:
    (tmp_path / "file").write_text("x")
    (tmp_path / "link").symlink_to(tmp_path / "file")
    directory_fd = os.open(tmp_path, _DIRECTORY_OPEN_FLAGS)
    try:
        with pytest.raises(ValueError, match="cannot traverse symlinks"):
            _reject_unusable_directory(directory_fd, "link", None)
    finally:
        os.close(directory_fd)


def test_reject_unusable_directory_rejects_regular_file(tmp_path: Path) -> None:
    (tmp_path / "file").write_text("x")
    directory_fd = os.open(tmp_path, _DIRECTORY_OPEN_FLAGS)
    try:
        with pytest.raises(ValueError, match="must be a directory"):
            _reject_unusable_directory(directory_fd, "file", None)
    finally:
        os.close(directory_fd)


def test_directory_and_object_open_flags_are_strict() -> None:
    assert _DIRECTORY_OPEN_FLAGS & _NOFOLLOW
    assert _DIRECTORY_OPEN_FLAGS & _DIRECTORY_ONLY
    assert _OBJECT_READ_FLAGS & _NOFOLLOW
    assert _OBJECT_READ_FLAGS & _NONBLOCK
    assert _TEMPORARY_WRITE_FLAGS & os.O_EXCL
    assert _CLOEXEC >= 0
    assert _BINARY >= 0


def test_read_object_closes_descriptor_when_fdopen_fails(
    store: ContentAddressedArtifactStore, monkeypatch: pytest.MonkeyPatch
) -> None:
    store.put_bytes(b"payload")
    closed: list[int] = []

    def exploding_fdopen(descriptor: int, _mode: str) -> object:
        closed.append(descriptor)
        raise ValueError("cannot wrap")

    monkeypatch.setattr(os, "fdopen", exploding_fdopen)
    with pytest.raises(ValueError, match="cannot wrap"):
        store.get_bytes(hashlib.sha256(b"payload").hexdigest())
    assert closed
