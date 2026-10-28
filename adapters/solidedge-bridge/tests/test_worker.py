"""The spawned worker and the parent-side child-process backend."""

from __future__ import annotations

import queue
import sys
import threading
from pathlib import Path
from types import SimpleNamespace
from typing import Any, cast

import pytest

from kala_solidedge_bridge.errors import UnavailableError
from kala_solidedge_bridge.jobs import BackendEvent, ChildProcessBackend, JobContext
from kala_solidedge_bridge.protocol import ErrorCode
from kala_solidedge_bridge.worker import Worker
from support import build_request


def test_worker_health_on_a_non_windows_host_is_honest(tmp_path: Path) -> None:
    worker = Worker(tmp_path)
    health = worker.health()

    if sys.platform == "win32":
        pytest.skip("this assertion describes the development host")
    assert health["supported"] is False
    assert health["available"] is False
    assert "non-Windows" in health["detail"]


def test_child_backend_build_is_acknowledged_then_fails_unavailable(tmp_path: Path) -> None:
    backend = ChildProcessBackend(tmp_path)
    backend.start()
    try:
        health = backend.health()
        if sys.platform == "win32":
            pytest.skip("this assertion describes the development host")
        assert health.platform == sys.platform
        assert health.supported is False
        assert health.available is False

        run_dir = tmp_path / "jobs" / "build-1"
        run_dir.mkdir(parents=True)
        request = build_request("worker-request")
        job = JobContext(
            build_id="build-1",
            request=request,
            run_dir=run_dir,
            total=len(request.procedure),
        )
        backend.submit(job)

        events: list[BackendEvent] = []
        for _ in range(10):
            events.extend(backend.poll(timeout=1.0))
            if any(event.kind in {"failed", "completed", "cancelled"} for event in events):
                break
        failed = [event for event in events if event.kind == "failed"]
        assert failed, "the worker must report the missing native executor"
        assert failed[0].error is not None
        assert failed[0].error.code is ErrorCode.UNAVAILABLE
        assert (run_dir / "plate.par").exists() is False
    finally:
        backend.shutdown()


def test_child_backend_reports_a_dead_process_usefully(tmp_path: Path) -> None:
    backend = ChildProcessBackend(
        tmp_path,
        command=[sys.executable, "-c", "import sys; sys.exit(3)"],
    )
    with pytest.raises(UnavailableError):
        backend.start()
    backend.shutdown()


def test_a_restarted_reader_never_delivers_into_the_new_queue(tmp_path: Path) -> None:
    backend = ChildProcessBackend(tmp_path)
    stale: queue.Queue[dict[str, object] | None] = queue.Queue()
    backend._messages = queue.Queue()
    backend._process = cast(Any, SimpleNamespace(stdout=iter(['{"type": "event"}\n'])))

    reader = threading.Thread(target=backend._read_messages, args=(stale,))
    reader.start()
    reader.join(5.0)

    assert stale.get_nowait() == {"type": "event"}
    assert stale.get_nowait() is None
    assert backend._messages.empty()


def test_child_backend_cancel_is_best_effort(tmp_path: Path) -> None:
    backend = ChildProcessBackend(tmp_path)
    backend.start()
    try:
        backend.cancel()
    finally:
        backend.shutdown()
