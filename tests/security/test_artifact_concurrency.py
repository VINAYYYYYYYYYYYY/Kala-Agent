"""Concurrency, atomicity, and confinement of the content-addressed artifact store.

The store holds every design byte that is persisted, and the only reason the
parallel workers described in ADR-0004 may share one root is that each
filesystem step the store takes is individually atomic. These tests hold that
line from several directions: a real multi-threaded and a real multi-process
stress run over one shared root, the byte-level evidence those runs leave
behind, a read path that races publication, and an attacker who replaces a
bucket directory or a stored object with a symlink while writes are in flight.
No writer in this module coordinates with another writer, because nothing in
the store is allowed to require it.
"""

from __future__ import annotations

import errno
import hashlib
import multiprocessing
import multiprocessing.queues
import multiprocessing.synchronize
import os
import shutil
import threading
import time
from collections.abc import Sequence
from pathlib import Path
from typing import Final, TypeAlias

import pytest

from kala_agent.persistence import ContentAddressedArtifactStore, Database

_THREAD_WORKER_COUNT: Final[int] = 12
_PROCESS_WORKER_COUNT: Final[int] = 6
_ROUNDS_PER_WORKER: Final[int] = 60
_SHARED_PAYLOAD_COUNT: Final[int] = 4
_DISTINCT_PAYLOAD_COUNT: Final[int] = 64
_WRITES_PER_ROUND: Final[int] = 3
_MAX_OBJECT_SIZE: Final[int] = 1_000_000
_READER_COUNT: Final[int] = 4
_ATTACKED_BUCKET: Final[str] = "cd"
_ATTACKED_PAYLOAD_COUNT: Final[int] = 96
_ATTACKED_WRITER_COUNT: Final[int] = 4
_ATTACK_BUDGET_SECONDS: Final[float] = 0.75
_GRACE_SECONDS: Final[float] = 0.25
_JOIN_TIMEOUT_SECONDS: Final[float] = 120.0
_PROCESS_JOIN_TIMEOUT_SECONDS: Final[float] = 60.0
_QUEUE_TIMEOUT_SECONDS: Final[float] = 30.0
_BARRIER_TIMEOUT_SECONDS: Final[float] = 30.0

_ProcessReport: TypeAlias = tuple[int, str, tuple[str, ...]]
_Context: Final[multiprocessing.context.ForkContext | None] = (
    multiprocessing.get_context("fork") if hasattr(os, "fork") else None
)


def _shared_payloads() -> tuple[bytes, ...]:
    return tuple(f"shared-artifact-{index}".encode() for index in range(_SHARED_PAYLOAD_COUNT))


def _distinct_payloads() -> tuple[bytes, ...]:
    return tuple(f"distinct-artifact-{index}".encode() for index in range(_DISTINCT_PAYLOAD_COUNT))


def _worker_payload(worker_index: int, round_index: int) -> bytes:
    return f"worker-{worker_index}-{round_index}".encode()


def _round_payloads(round_index: int) -> tuple[bytes, bytes]:
    return (
        _shared_payloads()[round_index % _SHARED_PAYLOAD_COUNT],
        _distinct_payloads()[round_index % _DISTINCT_PAYLOAD_COUNT],
    )


def _written_objects(worker_count: int) -> dict[str, bytes]:
    expected: dict[str, bytes] = {}
    for worker_index in range(worker_count):
        for round_index in range(_ROUNDS_PER_WORKER):
            shared, distinct = _round_payloads(round_index)
            for payload in (shared, distinct, _worker_payload(worker_index, round_index)):
                expected[hashlib.sha256(payload).hexdigest()] = payload
    return expected


def _thread_writer(
    root: Path,
    worker_index: int,
    barrier: threading.Barrier,
    digests: list[list[str]],
    failures: list[tuple[int, BaseException]],
) -> None:
    collected: list[str] = []
    try:
        barrier.wait(timeout=_BARRIER_TIMEOUT_SECONDS)
        store = ContentAddressedArtifactStore(root, max_object_size=_MAX_OBJECT_SIZE)
        for round_index in range(_ROUNDS_PER_WORKER):
            shared, distinct = _round_payloads(round_index)
            collected.append(store.put_bytes(shared))
            collected.append(store.put_bytes(distinct))
            collected.append(store.put_text(_worker_payload(worker_index, round_index).decode()))
    except BaseException as error:
        failures.append((worker_index, error))
    finally:
        digests[worker_index] = collected


def _process_writer(
    root: str,
    worker_index: int,
    barrier: multiprocessing.synchronize.Barrier,
    reports: multiprocessing.queues.Queue[_ProcessReport],
) -> None:
    collected: list[str] = []
    try:
        barrier.wait(timeout=_BARRIER_TIMEOUT_SECONDS)
        store = ContentAddressedArtifactStore(Path(root), max_object_size=_MAX_OBJECT_SIZE)
        for round_index in range(_ROUNDS_PER_WORKER):
            shared, distinct = _round_payloads(round_index)
            collected.append(store.put_bytes(shared))
            collected.append(store.put_bytes(distinct))
            collected.append(store.put_text(_worker_payload(worker_index, round_index).decode()))
    except BaseException as error:
        reports.put((worker_index, f"{type(error).__name__}: {error}", ()))
    else:
        reports.put((worker_index, "ok", tuple(collected)))


def _join_all(threads: Sequence[threading.Thread]) -> None:
    for thread in threads:
        thread.join(timeout=_JOIN_TIMEOUT_SECONDS)
        assert not thread.is_alive(), f"{thread.name} did not finish"


def _assert_exact_store_contents(
    store: ContentAddressedArtifactStore,
    expected: dict[str, bytes],
) -> None:
    objects_root = store.root / "objects"

    assert sorted(path.name for path in objects_root.iterdir()) == sorted(
        {digest[:2] for digest in expected}
    )
    assert not tuple(objects_root.rglob("*.tmp"))
    assert not tuple(path for path in objects_root.rglob("*") if path.is_symlink())

    stored_files = sorted(
        (path for path in objects_root.rglob("*") if path.is_file()),
        key=lambda path: path.name,
    )
    assert [path.name for path in stored_files] == sorted(expected)

    for path in stored_files:
        payload = path.read_bytes()
        assert expected[path.name] == payload
        assert hashlib.sha256(payload).hexdigest() == path.name

    for digest, payload in expected.items():
        assert store.contains(digest)
        assert store.get_bytes(digest) == payload
        assert store.get_size(digest) == len(payload)
        assert store.get_text(digest) == payload.decode()
        assert store.key_for(digest) == f"objects/{digest[:2]}/{digest}"


def _payloads_in_bucket(bucket: str, count: int) -> tuple[bytes, ...]:
    payloads: list[bytes] = []
    candidate = 0
    while len(payloads) < count:
        payload = f"bucket-probe-{candidate}".encode()
        candidate += 1
        if hashlib.sha256(payload).hexdigest().startswith(bucket):
            payloads.append(payload)
    return tuple(payloads)


def test_concurrent_threads_publish_every_object_exactly_once(tmp_path: Path) -> None:
    root = tmp_path / "artifacts"
    ContentAddressedArtifactStore(root, max_object_size=_MAX_OBJECT_SIZE)
    digests: list[list[str]] = [[] for _ in range(_THREAD_WORKER_COUNT)]
    failures: list[tuple[int, BaseException]] = []
    barrier = threading.Barrier(_THREAD_WORKER_COUNT)
    threads = [
        threading.Thread(
            target=_thread_writer,
            args=(root, worker_index, barrier, digests, failures),
            name=f"artifact-writer-{worker_index}",
        )
        for worker_index in range(_THREAD_WORKER_COUNT)
    ]

    for thread in threads:
        thread.start()
    _join_all(threads)

    assert not failures
    reported = [digest for worker_digests in digests for digest in worker_digests]
    expected = _written_objects(_THREAD_WORKER_COUNT)
    assert len(reported) == _THREAD_WORKER_COUNT * _ROUNDS_PER_WORKER * _WRITES_PER_ROUND
    assert set(reported) == set(expected)
    _assert_exact_store_contents(
        ContentAddressedArtifactStore(root, max_object_size=_MAX_OBJECT_SIZE), expected
    )


@pytest.mark.skipif(_Context is None, reason="requires fork start semantics")
def test_concurrent_processes_publish_every_object_exactly_once(tmp_path: Path) -> None:
    context = _Context
    assert context is not None
    root = tmp_path / "artifacts"
    ContentAddressedArtifactStore(root, max_object_size=_MAX_OBJECT_SIZE)
    reports: multiprocessing.queues.Queue[_ProcessReport] = context.Queue()
    barrier = context.Barrier(_PROCESS_WORKER_COUNT)
    processes = [
        context.Process(
            target=_process_writer,
            args=(str(root), worker_index, barrier, reports),
            name=f"artifact-process-{worker_index}",
        )
        for worker_index in range(_PROCESS_WORKER_COUNT)
    ]
    collected: list[_ProcessReport] = []
    drain_failures: list[BaseException] = []

    def drain() -> None:
        try:
            for _ in processes:
                collected.append(reports.get(timeout=_QUEUE_TIMEOUT_SECONDS))
        except BaseException as error:
            drain_failures.append(error)

    try:
        for process in processes:
            process.start()
        collector = threading.Thread(target=drain, name="artifact-report-collector", daemon=True)
        collector.start()
        for process in processes:
            process.join(timeout=_PROCESS_JOIN_TIMEOUT_SECONDS)
        _join_all([collector])
    finally:
        reports.close()
        reports.join_thread()

    assert not drain_failures
    assert [process.exitcode for process in processes] == [0] * _PROCESS_WORKER_COUNT
    assert not [process.exitcode for process in processes if process.exitcode]
    assert len(collected) == _PROCESS_WORKER_COUNT
    reported: list[str] = []
    for worker_index, status, worker_digests in collected:
        assert status == "ok", f"process {worker_index} failed: {status}"
        reported.extend(worker_digests)
    expected = _written_objects(_PROCESS_WORKER_COUNT)
    assert len(reported) == _PROCESS_WORKER_COUNT * _ROUNDS_PER_WORKER * _WRITES_PER_ROUND
    assert set(reported) == set(expected)
    _assert_exact_store_contents(
        ContentAddressedArtifactStore(root, max_object_size=_MAX_OBJECT_SIZE), expected
    )


def test_bucket_creation_survives_a_writer_that_lost_the_creation_race(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store = ContentAddressedArtifactStore(tmp_path / "artifacts", max_object_size=_MAX_OBJECT_SIZE)
    data = _shared_payloads()[0]
    digest = hashlib.sha256(data).hexdigest()
    attempts: list[str] = []
    real_mkdir = os.mkdir

    def racing_mkdir(path: str, mode: int = 0o777, *, dir_fd: int | None = None) -> None:
        attempts.append(path)
        real_mkdir(path, mode, dir_fd=dir_fd)
        raise FileExistsError(errno.EEXIST, os.strerror(errno.EEXIST), path)

    monkeypatch.setattr(os, "mkdir", racing_mkdir)
    assert store.put_bytes(data) == digest
    assert attempts == [digest[:2]]

    monkeypatch.undo()
    later = b"written after the lost race"
    assert store.get_bytes(digest) == data
    assert store.put_bytes(data) == digest
    assert store.put_bytes(later) == hashlib.sha256(later).hexdigest()
    assert (store.root / "objects" / digest[:2]).is_dir()


def test_readers_never_observe_a_partially_published_object(tmp_path: Path) -> None:
    store = ContentAddressedArtifactStore(tmp_path / "artifacts", max_object_size=_MAX_OBJECT_SIZE)
    payloads = _distinct_payloads()
    digests = tuple(hashlib.sha256(payload).hexdigest() for payload in payloads)
    stop = threading.Event()
    observations: list[bytes] = []
    failures: list[BaseException] = []
    lock = threading.Lock()

    def reader() -> None:
        seen = 0
        while not stop.is_set():
            for payload, digest in zip(payloads, digests, strict=True):
                try:
                    data = store.get_bytes(digest)
                except FileNotFoundError:
                    continue
                with lock:
                    observations.append(data)
                if data != payload:
                    failures.append(
                        AssertionError("a reader observed bytes outside a published object")
                    )
                seen += 1
            if seen == 0:
                time.sleep(0)

    def writer() -> None:
        for payload in payloads:
            store.put_bytes(payload)

    readers = [
        threading.Thread(target=reader, name=f"artifact-reader-{index}", daemon=True)
        for index in range(_READER_COUNT)
    ]
    publishing = threading.Thread(target=writer, name="artifact-publisher", daemon=True)
    for thread in readers:
        thread.start()
    publishing.start()
    _join_all([publishing])
    time.sleep(_GRACE_SECONDS)
    stop.set()
    _join_all(readers)

    assert not failures
    assert observations
    known = set(digests)
    assert all(hashlib.sha256(data).hexdigest() in known for data in observations)
    _assert_exact_store_contents(
        ContentAddressedArtifactStore(store.root, max_object_size=_MAX_OBJECT_SIZE),
        dict(zip(digests, payloads, strict=True)),
    )


def test_a_symlinked_bucket_never_redirects_a_concurrent_write(tmp_path: Path) -> None:
    root = tmp_path / "artifacts"
    outside = tmp_path / "outside"
    outside.mkdir()
    store = ContentAddressedArtifactStore(root, max_object_size=_MAX_OBJECT_SIZE)
    bucket = store.root / "objects" / _ATTACKED_BUCKET
    payloads = _payloads_in_bucket(_ATTACKED_BUCKET, _ATTACKED_PAYLOAD_COUNT)
    stop = threading.Event()
    refusals: list[str] = []
    published: list[str] = []
    lock = threading.Lock()
    deadline = time.monotonic() + _ATTACK_BUDGET_SECONDS

    def attacker() -> None:
        while time.monotonic() < deadline and not stop.is_set():
            try:
                if bucket.is_symlink():
                    bucket.unlink()
                elif bucket.exists():
                    shutil.rmtree(bucket)
            except OSError:
                pass
            try:
                bucket.symlink_to(outside, target_is_directory=True)
            except OSError:
                pass
            time.sleep(0)

    def writer() -> None:
        collected: list[str] = []
        for payload in payloads:
            digest = hashlib.sha256(payload).hexdigest()
            try:
                stored = store.put_bytes(payload)
            except (OSError, ValueError) as error:
                with lock:
                    refusals.append(str(error))
            else:
                assert stored == digest
                collected.append(digest)
        with lock:
            published.extend(collected)

    attack = threading.Thread(target=attacker, name="bucket-attacker", daemon=True)
    writers = [
        threading.Thread(target=writer, name=f"attacked-writer-{index}", daemon=True)
        for index in range(_ATTACKED_WRITER_COUNT)
    ]
    attack.start()
    for thread in writers:
        thread.start()
    _join_all(writers)
    stop.set()
    _join_all([attack])

    assert not tuple(outside.iterdir())
    assert refusals
    assert set(published) <= {hashlib.sha256(payload).hexdigest() for payload in payloads}
    for digest in published:
        try:
            data = store.get_bytes(digest)
        except (FileNotFoundError, ValueError):
            continue
        assert hashlib.sha256(data).hexdigest() == digest

    if bucket.is_symlink():
        bucket.unlink()
    elif bucket.exists():
        shutil.rmtree(bucket)
    bucket.symlink_to(outside, target_is_directory=True)
    with pytest.raises(ValueError, match="symlink"):
        store.get_bytes(hashlib.sha256(payloads[0]).hexdigest())
    with pytest.raises(ValueError, match="symlink"):
        store.put_bytes(payloads[0])
    assert not tuple(outside.iterdir())


def test_a_symlinked_object_is_never_read_as_a_stored_artifact(tmp_path: Path) -> None:
    store = ContentAddressedArtifactStore(tmp_path / "artifacts", max_object_size=_MAX_OBJECT_SIZE)
    outside = tmp_path / "outside.bin"
    outside.write_bytes(b"attacker controlled bytes")
    data = _shared_payloads()[0]
    digest = hashlib.sha256(data).hexdigest()
    store.put_bytes(data)
    target = store.root / "objects" / digest[:2] / digest
    stop = threading.Event()
    failures: list[str] = []
    deadline = time.monotonic() + _ATTACK_BUDGET_SECONDS

    def swapper() -> None:
        while time.monotonic() < deadline and not stop.is_set():
            try:
                target.unlink()
            except OSError:
                pass
            try:
                target.symlink_to(outside)
            except OSError:
                pass
            try:
                store.put_bytes(data)
            except ValueError:
                pass
            time.sleep(0)

    def reader() -> None:
        while time.monotonic() < deadline:
            try:
                observed = store.get_bytes(digest)
            except FileNotFoundError:
                continue
            except ValueError as error:
                if "symlink" not in str(error):
                    failures.append(f"object read failed for an unrelated reason: {error}")
                continue
            if observed != data:
                failures.append("a symlinked object was read as a stored artifact")

    attack = threading.Thread(target=swapper, name="object-swapper", daemon=True)
    readers = [
        threading.Thread(target=reader, name=f"swapped-reader-{index}", daemon=True)
        for index in range(_READER_COUNT)
    ]
    attack.start()
    for thread in readers:
        thread.start()
    _join_all(readers)
    stop.set()
    _join_all([attack])

    assert not failures
    if target.is_symlink():
        target.unlink()
    assert store.put_bytes(data) == digest
    assert store.get_bytes(digest) == data
    _assert_exact_store_contents(
        ContentAddressedArtifactStore(store.root, max_object_size=_MAX_OBJECT_SIZE),
        {digest: data},
    )


def test_artifact_publication_never_waits_for_the_database_writer_lock(tmp_path: Path) -> None:
    with Database(tmp_path / "journal.sqlite3") as database:
        store = ContentAddressedArtifactStore(
            tmp_path / "artifacts", max_object_size=_MAX_OBJECT_SIZE
        )
        locked = threading.Event()
        release = threading.Event()
        released = threading.Event()
        payloads = _distinct_payloads()

        def holder() -> None:
            try:
                with database.serialized() as connection:
                    connection.execute("SELECT COUNT(*) FROM events").fetchone()
                    locked.set()
                    release.wait(timeout=_BARRIER_TIMEOUT_SECONDS)
            finally:
                released.set()

        thread = threading.Thread(target=holder, name="database-lock-holder", daemon=True)
        thread.start()
        assert locked.wait(timeout=_BARRIER_TIMEOUT_SECONDS)
        try:
            for payload in payloads:
                assert store.put_bytes(payload) == hashlib.sha256(payload).hexdigest()
        finally:
            release.set()
            _join_all([thread])
        assert released.is_set()

        assert database.schema_version == 2
        for payload in payloads:
            assert store.get_bytes(hashlib.sha256(payload).hexdigest()) == payload
