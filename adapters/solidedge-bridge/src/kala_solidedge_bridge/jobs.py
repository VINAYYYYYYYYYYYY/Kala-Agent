"""Durable job ledger, the typed backend seam, and the one-worker supervisor.

HTTP request threads never call COM. They admit or read a job; a single
supervisor thread owns the backend, which in production is one spawned child
process with one STA. The backend interface is typed and separate from HTTP so
tests can inject a scripted backend without ever claiming it is native.

Rules encoded here:

* one active build; admission is refused while another build or an
  unreconciled indeterminate build exists;
* ``request_id`` plus payload digest is idempotent, and the same ID with a
  different payload is a conflict;
* persistence is atomic and a restart marks a never-submitted queued build
  ``interrupted`` while a build that was already running becomes
  ``indeterminate``; nothing is replayed;
* cancellation is cooperative between operations;
* the supervisor deadline, an unconfirmed submission, and a worker crash all
  mark the build ``indeterminate`` and block admission until an explicit
  reconciliation. Stopping the child is not proof that Solid Edge stopped, and
  Solid Edge itself is never killed.
"""

from __future__ import annotations

import json
import os
import queue
import subprocess
import sys
import threading
import time
import uuid
from collections.abc import Callable, Sequence
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Final, Literal, Protocol, cast

from .calibration import CalibrationStore
from .errors import (
    BusyError,
    CalibrationRequiredError,
    ConflictError,
    InputError,
    UnavailableError,
)
from .protocol import (
    MAX_ARTIFACTS,
    MAX_TRACE_STEPS,
    MAX_WARNINGS,
    Artifact,
    ArtifactFormat,
    BuildAccepted,
    BuildRequest,
    BuildResult,
    BuildStatus,
    ErrorCode,
    ErrorResponse,
    Health,
    JobState,
    StepTrace,
)
from .version import BRIDGE_VERSION
from .workspace import ArtifactStore, atomic_write_json, ensure_private_dir, read_json

_TERMINAL_STATES: Final[frozenset[JobState]] = frozenset(
    {
        JobState.SUCCEEDED,
        JobState.FAILED,
        JobState.CANCELLED,
        JobState.INTERRUPTED,
        JobState.INDETERMINATE,
    }
)
_EVENT_KINDS: Final[frozenset[str]] = frozenset(
    {"step", "completed", "failed", "cancelled", "indeterminate", "crashed", "warning"}
)


def _utc_now() -> str:
    return datetime.now(UTC).isoformat()


def _bounded(message: str, limit: int = 500) -> str:
    text = " ".join(message.replace("\r", " ").replace("\n", " ").split())
    return text[:limit] or "bridge failure"


@dataclass(frozen=True)
class BackendHealth:
    """What the backend itself can report without doing COM work on this thread."""

    platform: str
    supported: bool = False
    available: bool = False
    attached: bool = False
    se_version: str | None = None
    detail: str = ""


@dataclass(frozen=True)
class Readiness:
    supported: bool
    available: bool
    calibration_id: str | None
    se_version: str | None = None
    detail: str = ""


@dataclass(frozen=True)
class ArtifactSpec:
    name: str
    format: ArtifactFormat


@dataclass(frozen=True)
class BackendEvent:
    kind: Literal["step", "completed", "failed", "cancelled", "indeterminate", "crashed", "warning"]
    #: Build this event belongs to. An event that names a build is applied only
    #: while that build is the active one, so a stale event can never complete a
    #: later build. ``None`` is accepted for a backend that does not tag events.
    build_id: str | None = None
    trace: StepTrace | None = None
    result: BuildResult | None = None
    error: ErrorResponse | None = None
    message: str = ""
    artifacts: tuple[ArtifactSpec, ...] = ()


@dataclass(frozen=True)
class JobContext:
    build_id: str
    request: BuildRequest
    run_dir: Path
    total: int


class BuildNotStartedError(Exception):
    """The backend proved it did not begin the build.

    Only a backend that can positively distinguish "I never started this" from
    "I sent it and lost the acknowledgement" may raise this. Any other submit
    failure is ambiguous and becomes ``indeterminate``.
    """


class BuildBackend(Protocol):
    """The narrow seam between the supervisor and one owned worker."""

    def start(self) -> None: ...

    def health(self, *, refresh: bool = False) -> BackendHealth: ...

    def submit(self, job: JobContext) -> None: ...

    def poll(self, timeout: float = 0.0) -> Sequence[BackendEvent]: ...

    def cancel(self) -> None: ...

    def abort(self) -> None: ...

    def shutdown(self) -> None: ...


def native_readiness(health: BackendHealth, store: CalibrationStore) -> Readiness:
    """Derive readiness from the backend plus signed probe records.

    No production native executor exists in this milestone, so ``supported``
    stays false even when Solid Edge is attached and calibrated. The detail says
    why instead of implying a capability that was never verified.
    """
    if health.platform != "win32":
        return Readiness(
            supported=False,
            available=False,
            calibration_id=None,
            se_version=health.se_version,
            detail=(
                "non-Windows development host: the authenticated bridge is reachable but "
                "native Solid Edge builds are unavailable"
            ),
        )
    if not health.available or not health.se_version:
        return Readiness(
            supported=False,
            available=False,
            calibration_id=None,
            se_version=health.se_version,
            detail=health.detail
            or "Solid Edge is not attached in this interactive desktop session",
        )
    record = store.find(se_version=health.se_version)
    calibration_id = record.calibration_id if record is not None else None
    if not health.supported:
        detail = (
            "Solid Edge is attached and "
            + ("calibrated" if calibration_id else "not calibrated")
            + "; no live-verified production native build executor exists, so native builds "
            "remain unavailable. Run the probe for diagnostics."
        )
    else:
        detail = health.detail or "backend reports a native executor"
    return Readiness(
        supported=health.supported,
        available=health.available,
        calibration_id=calibration_id,
        se_version=health.se_version,
        detail=detail,
    )


@dataclass
class _Job:
    build_id: str
    request: BuildRequest
    digest: str
    state: JobState
    total: int
    trace: list[StepTrace] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    step_index: int = 0
    cancel_requested: bool = False
    cancel_sent: bool = False
    result: BuildResult | None = None
    message: str = ""
    created_at: str = field(default_factory=_utc_now)
    updated_at: str = field(default_factory=_utc_now)
    started_monotonic: float | None = None
    deadline: float | None = None

    def status(self) -> BuildStatus:
        return BuildStatus(
            build_id=self.build_id,
            state=self.state,
            step_index=min(self.step_index, self.total),
            total=self.total,
            trace=tuple(self.trace),
            result=self.result if self.state in _TERMINAL_STATES else None,
            message=_bounded(self.message)[:500] if self.message else None,
        )


class JobManager:
    """Single active build, durable state, and one owned backend process."""

    def __init__(
        self,
        data_dir: Path,
        backend_factory: Callable[[], BuildBackend],
        *,
        calibration_store: CalibrationStore | None = None,
        readiness_provider: Callable[[BackendHealth], Readiness] | None = None,
        start_supervisor: bool = True,
        clock: Callable[[], float] = time.monotonic,
        poll_interval: float = 0.05,
        deadline_grace: float = 2.0,
        supervisor_deadline: float | None = None,
    ) -> None:
        self.data_dir = data_dir
        self.jobs_dir = data_dir / "jobs"
        ensure_private_dir(self.jobs_dir)
        self._backend_factory = backend_factory
        self._calibration_store = calibration_store or CalibrationStore(data_dir)
        self._readiness_provider = readiness_provider or (
            lambda health: native_readiness(health, self._calibration_store)
        )
        self._clock = clock
        self._poll_interval = poll_interval
        self._deadline_grace = deadline_grace
        self._supervisor_deadline = supervisor_deadline
        self._lock = threading.RLock()
        self._jobs: dict[str, _Job] = {}
        self._by_request: dict[str, str] = {}
        self._active: str | None = None
        self._blocked: str | None = None
        self._pending: queue.Queue[str] = queue.Queue()
        self._wake = threading.Event()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._backend: BuildBackend | None = None
        self._backend_failure: str | None = None
        self._cached_health = BackendHealth(
            platform=sys.platform,
            detail="backend has not started",
        )
        self._recover()
        if start_supervisor:
            self._thread = threading.Thread(
                target=self._supervisor_loop, name="kala-se-supervisor", daemon=True
            )
            self._thread.start()

    # ------------------------------------------------------------------ persistence

    def _job_dir(self, build_id: str) -> Path:
        return self.jobs_dir / build_id

    def _persist(self, job: _Job) -> None:
        job.updated_at = _utc_now()
        payload: dict[str, Any] = {
            "schema": 1,
            "build_id": job.build_id,
            "request_id": job.request.request_id,
            "digest": job.digest,
            "state": job.state.value,
            "step_index": job.step_index,
            "total": job.total,
            "trace": [entry.model_dump(mode="json") for entry in job.trace],
            "warnings": list(job.warnings),
            "cancel_requested": job.cancel_requested,
            "result": job.result.model_dump(mode="json") if job.result is not None else None,
            "message": job.message,
            "created_at": job.created_at,
            "updated_at": job.updated_at,
        }
        atomic_write_json(self._job_dir(job.build_id) / "job.json", payload)

    def _recover(self) -> None:
        if not self.jobs_dir.is_dir():
            return
        for job_dir in sorted(path for path in self.jobs_dir.iterdir() if path.is_dir()):
            request_payload = read_json(job_dir / "request.json")
            job_payload = read_json(job_dir / "job.json")
            if not isinstance(request_payload, dict) or not isinstance(job_payload, dict):
                continue
            try:
                request = BuildRequest.model_validate(request_payload)
                job = _Job(
                    build_id=str(job_payload["build_id"]),
                    request=request,
                    digest=str(job_payload["digest"]),
                    state=JobState(str(job_payload["state"])),
                    total=int(job_payload["total"]),
                    trace=[
                        StepTrace.model_validate(entry)
                        for entry in cast(list[Any], job_payload.get("trace", []))
                    ],
                    warnings=[
                        str(item) for item in cast(list[Any], job_payload.get("warnings", []))
                    ][:MAX_WARNINGS],
                    step_index=int(job_payload.get("step_index", 0)),
                    cancel_requested=bool(job_payload.get("cancel_requested", False)),
                    result=(
                        BuildResult.model_validate(job_payload["result"])
                        if job_payload.get("result") is not None
                        else None
                    ),
                    message=str(job_payload.get("message", "")),
                    created_at=str(job_payload.get("created_at", _utc_now())),
                    updated_at=str(job_payload.get("updated_at", _utc_now())),
                )
            except Exception:
                continue
            with self._lock:
                self._jobs[job.build_id] = job
                self._by_request[request.request_id] = job.build_id
                if job.state is JobState.QUEUED:
                    # Never handed to the backend: no COM operation can exist,
                    # so this is definite and must never be replayed.
                    job.state = JobState.INTERRUPTED
                    job.message = (
                        "bridge restarted while this build was queued and not yet submitted; "
                        "it was not replayed"
                    )
                    job.cancel_requested = False
                    self._persist(job)
                elif job.state is JobState.RUNNING:
                    # A running build may still have an external COM operation in
                    # flight. Its outcome is unknown, so it blocks admission until
                    # an operator reconciles it.
                    job.state = JobState.INDETERMINATE
                    job.message = (
                        "bridge restarted while this build was running; whether Solid Edge "
                        "completed the operation is unknown, so it is blocked until "
                        "reconciliation and was not replayed"
                    )
                    job.result = BuildResult(
                        build_id=job.build_id,
                        state=JobState.INDETERMINATE,
                        trace=tuple(job.trace),
                        warnings=tuple(job.warnings),
                        error=ErrorResponse(
                            code=ErrorCode.TIMEOUT_INDETERMINATE,
                            message=_bounded(job.message),
                        ),
                    )
                    job.cancel_requested = False
                    self._blocked = job.build_id
                    self._persist(job)
                elif job.state is JobState.INDETERMINATE:
                    self._blocked = job.build_id

    # ------------------------------------------------------------------ admission

    def _readiness(self) -> Readiness:
        with self._lock:
            backend = self._backend
        health = backend.health(refresh=False) if backend is not None else self._cached_health
        return self._readiness_provider(health)

    def submit(self, request: BuildRequest) -> BuildAccepted:
        """Admit one build or return the existing idempotent result."""
        digest = request.request_digest()
        with self._lock:
            existing = self._by_request.get(request.request_id)
            if existing is not None:
                job = self._jobs[existing]
                if job.digest != digest:
                    raise ConflictError("request_id already exists with a different payload digest")
                return BuildAccepted(build_id=job.build_id, state=job.state)
            if self._blocked is not None:
                raise BusyError(
                    "an indeterminate build requires reconciliation before new admission"
                )
            if self._active is not None:
                raise BusyError("another build owns the single Solid Edge worker")

        readiness = self._readiness()
        if not readiness.supported:
            raise UnavailableError(
                readiness.detail
                or "no live-verified native build executor is available on this host"
            )
        if not readiness.available:
            raise UnavailableError(
                readiness.detail or "the Solid Edge worker is not attached or available"
            )
        current_calibration = readiness.calibration_id
        if not current_calibration:
            raise CalibrationRequiredError(
                "no signed probe calibration matches this Solid Edge version, template, and binding"
            )
        requested_calibration = request.calibration_id
        if not requested_calibration:
            raise CalibrationRequiredError(
                "the build request carries no calibration id; run the probe and pass the id "
                "reported by doctor"
            )
        if requested_calibration != current_calibration:
            raise ConflictError(
                "the request calibration id does not match the host's current calibration; "
                "run doctor again and rebuild with the reported id"
            )

        with self._lock:
            existing = self._by_request.get(request.request_id)
            if existing is not None:
                job = self._jobs[existing]
                if job.digest != digest:
                    raise ConflictError("request_id already exists with a different payload digest")
                return BuildAccepted(build_id=job.build_id, state=job.state)
            if self._blocked is not None or self._active is not None:
                raise BusyError("another build owns the single Solid Edge worker")
            build_id = uuid.uuid4().hex
            job = _Job(
                build_id=build_id,
                request=request,
                digest=digest,
                state=JobState.QUEUED,
                total=len(request.procedure),
            )
            self._jobs[build_id] = job
            self._by_request[request.request_id] = build_id
            self._active = build_id
            ensure_private_dir(self._job_dir(build_id))
            atomic_write_json(
                self._job_dir(build_id) / "request.json",
                request.model_dump(mode="json"),
            )
            self._persist(job)
            self._pending.put(build_id)
        self._wake.set()
        return BuildAccepted(build_id=build_id, state=JobState.QUEUED)

    # ------------------------------------------------------------------ reads

    def _job(self, build_id: str) -> _Job:
        job = self._jobs.get(build_id)
        if job is None:
            raise KeyError(build_id)
        return job

    def status(self, build_id: str) -> BuildStatus:
        with self._lock:
            return self._job(build_id).status()

    def health(self) -> Health:
        with self._lock:
            backend = self._backend
            busy = self._active is not None or self._blocked is not None
        health = backend.health(refresh=False) if backend is not None else self._cached_health
        readiness = self._readiness_provider(health)
        detail = readiness.detail
        if self._backend_failure:
            detail = f"{detail} backend: {self._backend_failure}"
        if busy:
            detail = f"{detail} A build is active or requires reconciliation."
        capabilities = ["protocol.v1", "probe.diagnostic", "jobs.persistence", "artifacts.verified"]
        if health.attached:
            capabilities.append("com.attached")
        return Health(
            bridge_version=BRIDGE_VERSION,
            platform=(health.platform or sys.platform)[:32],
            supported=readiness.supported,
            available=readiness.available,
            busy=busy,
            se_version=(readiness.se_version or health.se_version or None),
            calibration_id=readiness.calibration_id,
            capabilities=tuple(capabilities),
            detail=_bounded(detail),
        )

    def cancel(self, build_id: str) -> BuildStatus:
        with self._lock:
            job = self._job(build_id)
            if job.state in _TERMINAL_STATES:
                return job.status()
            job.cancel_requested = True
            if job.state is JobState.QUEUED:
                self._cancel_queued(job)
            else:
                job.message = "cancellation requested; the worker checks between operations"
                self._persist(job)
            status = job.status()
        self._wake.set()
        return status

    def _cancel_queued(self, job: _Job) -> None:
        """Cancel a build that has not been handed to the backend.

        The caller holds ``self._lock``; the state change and the ``_active``
        release happen in that one critical section so the supervisor can never
        start a build a cancel already completed.
        """
        job.state = JobState.CANCELLED
        job.message = "cancelled before the worker started"
        job.result = BuildResult(
            build_id=job.build_id,
            state=JobState.CANCELLED,
            trace=tuple(job.trace),
            warnings=tuple(job.warnings),
        )
        if self._active == job.build_id:
            self._active = None
        self._persist(job)

    def reconcile(
        self, build_id: str, note: str = "operator reconciled indeterminate build"
    ) -> BuildStatus:
        """Resolve an indeterminate build without replaying it."""
        with self._lock:
            job = self._job(build_id)
            if job.state is not JobState.INDETERMINATE:
                raise InputError("only an indeterminate build can be reconciled")
            job.state = JobState.INTERRUPTED
            job.message = _bounded(note)
            job.result = BuildResult(
                build_id=job.build_id,
                state=JobState.INTERRUPTED,
                trace=tuple(job.trace),
                warnings=tuple(job.warnings),
            )
            if self._blocked == build_id:
                self._blocked = None
            self._persist(job)
            return job.status()

    def artifact(self, build_id: str, name: str) -> tuple[Artifact, bytes]:
        with self._lock:
            self._job(build_id)
            run_dir = self._job_dir(build_id)
        store = ArtifactStore(run_dir)
        return store.read_verified(name)

    # ------------------------------------------------------------------ supervisor

    def _ensure_backend(self) -> BuildBackend | None:
        with self._lock:
            if self._backend is not None:
                return self._backend
        try:
            backend = self._backend_factory()
            backend.start()
        except Exception as exc:
            with self._lock:
                self._backend_failure = _bounded(str(exc), 200)
            return None
        with self._lock:
            self._backend = backend
            self._backend_failure = None
        return backend

    def _supervisor_loop(self) -> None:
        while not self._stop.is_set():
            try:
                self.run_once(timeout=self._poll_interval)
            except Exception:
                time.sleep(self._poll_interval)
            self._wake.wait(self._poll_interval)
            self._wake.clear()

    def run_once(self, timeout: float = 0.0) -> None:
        """One supervisor iteration. Tests call this instead of racing a thread."""
        backend = self._ensure_backend()
        if backend is None:
            with self._lock:
                job = self._jobs.get(self._active) if self._active is not None else None
            if job is not None and job.state is JobState.RUNNING:
                # A running job whose backend vanished may still have a native
                # operation in flight; its outcome is unknown.
                self._mark_indeterminate(
                    job,
                    "the worker backend became unavailable while this build was running; "
                    "whether Solid Edge completed the operation is unknown",
                )
            elif job is not None and job.state is JobState.QUEUED:
                self._finish_without_result(
                    job,
                    JobState.FAILED,
                    ErrorCode.UNAVAILABLE,
                    self._backend_failure or "the worker process is unavailable",
                )
            return

        started: _Job | None = None
        with self._lock:
            active_job = self._jobs.get(self._active) if self._active is not None else None
            can_start = active_job is None or active_job.state is JobState.QUEUED
            if can_start and self._blocked is None and not self._pending.empty():
                build_id = self._pending.get_nowait()
                candidate = self._jobs.get(build_id)
                if (
                    candidate is not None
                    and candidate.state is JobState.QUEUED
                    and not candidate.cancel_requested
                ):
                    # Dequeue, the QUEUED check, the transition to RUNNING, the
                    # _active assignment, and the durable write are one critical
                    # section. A cancel therefore either wins (and this build is
                    # skipped above) or observes RUNNING and stays cooperative;
                    # it can never complete a build the supervisor then submits.
                    candidate.state = JobState.RUNNING
                    candidate.started_monotonic = self._clock()
                    candidate.deadline = (
                        candidate.started_monotonic
                        + candidate.request.timeout_seconds
                        + self._deadline_grace
                    )
                    self._active = candidate.build_id
                    self._persist(candidate)
                    started = candidate
        if started is not None:
            with self._lock:
                if started.state is JobState.RUNNING and started.cancel_requested:
                    # Cancelled between the transition and submission: no COM
                    # work exists yet, so the build is definitely cancelled.
                    self._cancel_queued(started)
                    return
                if started.state is not JobState.RUNNING:
                    return
            context = JobContext(
                build_id=started.build_id,
                request=started.request,
                run_dir=self._job_dir(started.build_id),
                total=started.total,
            )
            try:
                backend.submit(context)
            except BuildNotStartedError as exc:
                # Only a positively-pre-start refusal may free admission.
                self._finish_without_result(
                    started,
                    JobState.FAILED,
                    ErrorCode.UNAVAILABLE,
                    f"worker refused the build before starting: {_bounded(str(exc), 200)}",
                )
                return
            except Exception as exc:
                # The acknowledgement was lost or the worker failed while the
                # child may already have started a native operation. Never claim
                # a definite failure and never replay it.
                self._mark_indeterminate(
                    started,
                    "the submission acknowledgement was lost; whether the worker started "
                    f"the build is unknown ({_bounded(str(exc), 200)})",
                )
                self._abort_backend(backend)
                return

        try:
            events = backend.poll(timeout)
        except Exception as exc:
            # A backend that fails while the build is running may have an
            # external COM operation in flight. Its outcome is unknown, so block
            # admission and stop only the child rather than reporting a result.
            with self._lock:
                job = self._jobs.get(self._active) if self._active else None
            if job is not None and job.state is JobState.RUNNING:
                self._mark_indeterminate(
                    job,
                    "the worker backend failed while the build was running; whether Solid "
                    f"Edge completed the operation is unknown ({_bounded(str(exc), 200)})",
                )
                self._abort_backend(backend)
            return
        with self._lock:
            for event in events:
                self._apply_event(event)
            job = self._jobs.get(self._active) if self._active else None
            if job is None or job.state is not JobState.RUNNING:
                return
            if job.cancel_requested and not job.cancel_sent:
                job.cancel_sent = True
                send_cancel = True
            else:
                send_cancel = False
            if job.deadline is not None and self._clock() > job.deadline:
                timed_out = job
            else:
                timed_out = None
        if send_cancel:
            backend.cancel()
        if timed_out is not None:
            self._mark_indeterminate(
                timed_out,
                "the build exceeded its supervisor deadline; whether the COM call completed is unknown",
            )
            self._abort_backend(backend)

    @staticmethod
    def _abort_backend(backend: BuildBackend) -> None:
        """Stop only the child process. This is not proof COM stopped."""
        try:
            backend.abort()
        except Exception:
            pass

    def _finish_without_result(
        self, job: _Job, state: JobState, code: ErrorCode, message: str
    ) -> None:
        with self._lock:
            error = ErrorResponse(code=code, message=_bounded(message))
            job.state = state
            job.message = error.message
            artifacts = self._publish_artifacts(job, ())
            job.result = BuildResult(
                build_id=job.build_id,
                state=state,
                artifacts=artifacts,
                trace=tuple(job.trace),
                warnings=tuple(job.warnings),
                error=error,
            )
            if self._active == job.build_id:
                self._active = None
            self._persist(job)

    def _mark_indeterminate(self, job: _Job, message: str) -> None:
        with self._lock:
            if job.state is not JobState.RUNNING:
                return
            error = ErrorResponse(code=ErrorCode.TIMEOUT_INDETERMINATE, message=_bounded(message))
            job.state = JobState.INDETERMINATE
            job.message = error.message
            artifacts = self._publish_artifacts(job, ())
            job.result = BuildResult(
                build_id=job.build_id,
                state=JobState.INDETERMINATE,
                artifacts=artifacts,
                trace=tuple(job.trace),
                warnings=tuple(job.warnings),
                error=error,
            )
            if self._active == job.build_id:
                self._active = None
            self._blocked = job.build_id
            self._persist(job)

    def _apply_event(self, event: BackendEvent) -> None:
        if event.kind not in _EVENT_KINDS:
            return
        job = self._jobs.get(self._active) if self._active else None
        if job is None or job.state is not JobState.RUNNING:
            return
        if event.build_id is not None and event.build_id != job.build_id:
            # A tagged event that belongs to another build must never be applied
            # to the active one (its trace, result, or artifacts would be wrong).
            return
        if event.kind == "step":
            if event.trace is not None:
                self._append_trace(job, event.trace)
            job.step_index = min(job.step_index + 1, job.total)
            self._persist(job)
            return
        if event.kind == "warning":
            self._append_warning(job, event.message)
            self._persist(job)
            return
        if event.kind == "cancelled":
            self._complete(job, JobState.CANCELLED, event)
            return
        if event.kind == "completed":
            self._complete(job, JobState.SUCCEEDED, event)
            return
        if event.kind == "failed":
            self._complete(job, JobState.FAILED, event)
            return
        if event.kind == "indeterminate":
            self._mark_indeterminate(
                job, event.message or "the worker reported an indeterminate native operation"
            )
            return
        if event.kind == "crashed":
            self._mark_indeterminate(
                job,
                event.message
                or "the worker process exited; whether Solid Edge completed the operation is unknown",
            )

    def _append_trace(self, job: _Job, entry: StepTrace) -> None:
        if len(job.trace) >= MAX_TRACE_STEPS:
            self._append_warning(job, f"trace truncated at {MAX_TRACE_STEPS} steps")
            return
        next_seq = job.trace[-1].seq + 1 if job.trace else 0
        job.trace.append(entry.model_copy(update={"seq": next_seq}))

    def _append_warning(self, job: _Job, message: str) -> None:
        if len(job.warnings) >= MAX_WARNINGS:
            return
        job.warnings.append(_bounded(message))

    def _publish_artifacts(self, job: _Job, specs: Sequence[ArtifactSpec]) -> tuple[Artifact, ...]:
        store = ArtifactStore(self._job_dir(job.build_id))
        artifacts: list[Artifact] = []
        for spec in specs:
            if len(artifacts) >= MAX_ARTIFACTS:
                self._append_warning(
                    job, "artifact cap reached; remaining artifacts were not published"
                )
                break
            try:
                artifacts.append(store.register(spec.name, spec.format))
            except Exception as exc:
                self._append_warning(job, f"artifact {spec.name!r} was not published: {exc}")
        store.write_manifest(tuple(artifacts))
        return tuple(artifacts)

    def _complete(self, job: _Job, state: JobState, event: BackendEvent) -> None:
        artifacts = self._publish_artifacts(job, event.artifacts)
        base = event.result
        error = event.error
        if base is None:
            result = BuildResult(
                build_id=job.build_id,
                state=state,
                artifacts=artifacts,
                trace=tuple(job.trace),
                warnings=tuple(job.warnings),
                error=error,
            )
        else:
            result = base.model_copy(
                update={
                    "build_id": job.build_id,
                    "state": state,
                    "artifacts": artifacts,
                    "trace": tuple(job.trace),
                    "warnings": tuple(job.warnings),
                    "error": error if error is not None else base.error,
                }
            )
        job.state = state
        job.result = result
        job.message = event.message or (error.message if error is not None else "")
        job.step_index = job.total
        if self._active == job.build_id:
            self._active = None
        self._persist(job)

    def close(self) -> None:
        """Stop the supervisor; preserve uncertainty for a running build."""
        self._stop.set()
        self._wake.set()
        if self._thread is not None:
            self._thread.join(self._supervisor_deadline or 5.0)
        with self._lock:
            job = self._jobs.get(self._active) if self._active else None
            if job is not None and job.state is JobState.RUNNING:
                # Shutting the child down is not proof the COM call stopped. Persist
                # the uncertainty so a restart blocks admission instead of
                # reporting a definite interrupted build.
                self._mark_indeterminate(
                    job,
                    "the bridge shut down while this build was running; whether Solid Edge "
                    "completed the operation is unknown, so it is blocked until reconciliation "
                    "and was not replayed",
                )
            elif job is not None and job.state is JobState.QUEUED:
                job.state = JobState.INTERRUPTED
                job.message = (
                    "bridge shut down while this build was queued and not yet submitted; "
                    "it was not replayed"
                )
                job.result = BuildResult(
                    build_id=job.build_id,
                    state=JobState.INTERRUPTED,
                    trace=tuple(job.trace),
                    warnings=tuple(job.warnings),
                )
                if self._active == job.build_id:
                    self._active = None
                self._persist(job)
            backend = self._backend
            self._backend = None
        if backend is not None:
            backend.shutdown()


class ChildProcessBackend:
    """Production backend: one spawned child process owns COM.

    The parent never calls COM. It speaks newline-delimited JSON over the
    child's stdin/stdout and keeps a bounded reader thread, so an HTTP thread is
    never blocked by a COM call. ``abort`` stops the child; it never kills or
    quits Solid Edge.
    """

    def __init__(
        self,
        data_dir: Path,
        *,
        command: Sequence[str] | None = None,
        request_timeout: float = 15.0,
    ) -> None:
        self.data_dir = data_dir
        self._command = (
            list(command)
            if command is not None
            else [
                sys.executable,
                "-m",
                "kala_solidedge_bridge.worker",
                "--data-dir",
                str(data_dir),
            ]
        )
        self._request_timeout = request_timeout
        self._process: subprocess.Popen[str] | None = None
        self._reader: threading.Thread | None = None
        self._messages: queue.Queue[dict[str, Any] | None] = queue.Queue()
        self._events: list[BackendEvent] = []
        self._health = BackendHealth(platform=sys.platform, detail="worker has not started")
        self._lock = threading.Lock()
        self._stderr: list[str] = []

    def start(self) -> None:
        with self._lock:
            if self._process is not None and self._process.poll() is None:
                return
            creationflags = getattr(subprocess, "CREATE_NO_WINDOW", 0) if os.name == "nt" else 0
            environment = dict(os.environ)
            package_root = str(Path(__file__).resolve().parent.parent)
            existing_pythonpath = environment.get("PYTHONPATH")
            environment["PYTHONPATH"] = (
                package_root
                if not existing_pythonpath
                else package_root + os.pathsep + existing_pythonpath
            )
            self._process = subprocess.Popen(
                self._command,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                encoding="utf-8",
                bufsize=1,
                creationflags=creationflags,
                env=environment,
            )
            # Each reader thread owns the queue it was started with. A restarted
            # worker therefore never receives a buffered line from the previous
            # child, even while the old reader thread is still draining.
            messages: queue.Queue[dict[str, Any] | None] = queue.Queue()
            self._messages = messages
            self._reader = threading.Thread(
                target=self._read_messages,
                args=(messages,),
                name="kala-se-worker-reader",
                daemon=True,
            )
            self._reader.start()
            if self._process.stderr is not None:
                threading.Thread(
                    target=self._drain_stderr, args=(self._process.stderr,), daemon=True
                ).start()
        self._health = self._parse_health(self._request({"cmd": "health"}, "health"))

    def _drain_stderr(self, stream: Any) -> None:  # noqa: ANN401
        try:
            for line in stream:
                if len(self._stderr) < 50:
                    self._stderr.append(line.strip()[:300])
        except Exception:
            pass

    def _read_messages(self, messages: queue.Queue[dict[str, Any] | None]) -> None:
        process = self._process
        if process is None or process.stdout is None:
            return
        try:
            for line in process.stdout:
                try:
                    message = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if isinstance(message, dict):
                    messages.put(message)
        except Exception:
            pass
        finally:
            messages.put(None)

    def _send(self, payload: dict[str, Any]) -> None:
        process = self._process
        if process is None or process.poll() is not None or process.stdin is None:
            raise UnavailableError("the Solid Edge worker process is not running")
        try:
            process.stdin.write(
                json.dumps(payload, ensure_ascii=True, separators=(",", ":")) + "\n"
            )
            process.stdin.flush()
        except (BrokenPipeError, OSError) as exc:
            raise UnavailableError("the Solid Edge worker process closed its input") from exc

    def _request(self, payload: dict[str, Any], expected: str) -> dict[str, Any]:
        self._send(payload)
        deadline = time.monotonic() + self._request_timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise UnavailableError(f"the worker did not answer {expected} before the deadline")
            try:
                message = self._messages.get(timeout=remaining)
            except queue.Empty as exc:
                raise UnavailableError(
                    f"the worker did not answer {expected} before the deadline"
                ) from exc
            if message is None:
                raise UnavailableError("the Solid Edge worker process exited unexpectedly")
            kind = message.get("type")
            if kind == "event":
                event = _event_from_wire(message.get("event"))
                if event is not None:
                    self._events.append(event)
                continue
            if kind == expected:
                return message
            if kind == "error":
                raise UnavailableError(_bounded(str(message.get("message", "worker error"))))

    @staticmethod
    def _parse_health(message: dict[str, Any]) -> BackendHealth:
        return BackendHealth(
            platform=str(message.get("platform", sys.platform))[:32],
            supported=bool(message.get("supported", False)),
            available=bool(message.get("available", False)),
            attached=bool(message.get("attached", False)),
            se_version=(str(message["se_version"]) if message.get("se_version") else None),
            detail=_bounded(str(message.get("detail", ""))),
        )

    def health(self, *, refresh: bool = False) -> BackendHealth:
        with self._lock:
            if refresh:
                try:
                    self._health = self._parse_health(self._request({"cmd": "health"}, "health"))
                except Exception as exc:
                    self._health = BackendHealth(
                        platform=sys.platform,
                        detail=f"worker health failed: {_bounded(str(exc), 200)}",
                    )
            return self._health

    def submit(self, job: JobContext) -> None:
        with self._lock:
            response = self._request(
                {
                    "cmd": "build",
                    "build_id": job.build_id,
                    "request": job.request.model_dump(mode="json"),
                    "run_dir": str(job.run_dir),
                },
                "build_started",
            )
            if response.get("accepted") is False:
                raise UnavailableError(
                    _bounded(str(response.get("message", "worker refused the build")))
                )

    def poll(self, timeout: float = 0.0) -> Sequence[BackendEvent]:
        if timeout > 0 and not self._events:
            try:
                message = self._messages.get(timeout=timeout)
            except queue.Empty:
                return ()
            if message is None:
                self._events.append(
                    BackendEvent(kind="crashed", message="the Solid Edge worker process exited")
                )
            elif message.get("type") == "event":
                event = _event_from_wire(message.get("event"))
                if event is not None:
                    self._events.append(event)
        events = self._events
        self._events = []
        return events

    def cancel(self) -> None:
        try:
            self._send({"cmd": "cancel"})
        except UnavailableError:
            pass

    def abort(self) -> None:
        with self._lock:
            process = self._process
            if process is None:
                return
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=3.0)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=2.0)
            self._process = None

    def shutdown(self) -> None:
        try:
            self._send({"cmd": "shutdown"})
        except UnavailableError:
            pass
        process = self._process
        if process is not None:
            try:
                process.wait(timeout=5.0)
            except subprocess.TimeoutExpired:
                self.abort()


def _event_from_wire(payload: object) -> BackendEvent | None:
    if not isinstance(payload, dict):
        return None
    kind = str(payload.get("kind", ""))
    if kind not in _EVENT_KINDS:
        return None
    trace_payload = payload.get("trace")
    trace = None
    if isinstance(trace_payload, dict):
        try:
            trace = StepTrace.model_validate(trace_payload)
        except Exception:
            trace = None
    result_payload = payload.get("result")
    result = None
    if isinstance(result_payload, dict):
        try:
            result = BuildResult.model_validate(result_payload)
        except Exception:
            result = None
    error_payload = payload.get("error")
    error = None
    if isinstance(error_payload, dict):
        try:
            error = ErrorResponse.model_validate(error_payload)
        except Exception:
            error = None
    artifacts: list[ArtifactSpec] = []
    raw_artifacts = payload.get("artifacts")
    if isinstance(raw_artifacts, list):
        for item in raw_artifacts:
            if not isinstance(item, dict):
                continue
            try:
                artifacts.append(
                    ArtifactSpec(
                        name=str(item["name"]),
                        format=ArtifactFormat(str(item["format"])),
                    )
                )
            except (KeyError, ValueError):
                continue
    return BackendEvent(
        kind=cast(Any, kind),
        build_id=(str(payload["build_id"]) if payload.get("build_id") else None),
        trace=trace,
        result=result,
        error=error,
        message=_bounded(str(payload.get("message", ""))),
        artifacts=tuple(artifacts),
    )


__all__ = (
    "ArtifactSpec",
    "BackendEvent",
    "BackendHealth",
    "BuildBackend",
    "BuildNotStartedError",
    "ChildProcessBackend",
    "JobContext",
    "JobManager",
    "Readiness",
    "native_readiness",
)
