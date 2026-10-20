"""Job ledger, idempotency, cancellation, deadlines, restart, and artifacts."""

from __future__ import annotations

from pathlib import Path

import pytest

from kala_solidedge_bridge.calibration import CalibrationStore
from kala_solidedge_bridge.errors import (
    BusyError,
    CalibrationRequiredError,
    ConflictError,
    UnavailableError,
)
from kala_solidedge_bridge.jobs import (
    ArtifactSpec,
    BackendEvent,
    BackendHealth,
    JobContext,
    JobManager,
    Readiness,
    native_readiness,
)
from kala_solidedge_bridge.protocol import (
    ArtifactFormat,
    ErrorCode,
    ErrorResponse,
    JobState,
    StepTrace,
)
from support import build_request


class FakeClock:
    def __init__(self) -> None:
        self.now = 1_000.0

    def __call__(self) -> float:
        return self.now

    def advance(self, seconds: float) -> None:
        self.now += seconds


class ScriptedBackend:
    def __init__(
        self,
        *,
        script: list[BackendEvent] | None = None,
        artifact: tuple[str, bytes] | None = None,
        health: BackendHealth | None = None,
    ) -> None:
        self.script = list(script or [])
        self.events: list[BackendEvent] = []
        self.artifact = artifact
        self.health_state = health or BackendHealth(
            platform="win32", supported=True, available=True, se_version="2024.0"
        )
        self.started = 0
        self.submitted: list[JobContext] = []
        self.cancelled = 0
        self.aborted = 0
        self.shutdowns = 0

    def start(self) -> None:
        self.started += 1

    def health(self, *, refresh: bool = False) -> BackendHealth:
        return self.health_state

    def submit(self, job: JobContext) -> None:
        self.submitted.append(job)
        if self.artifact is not None:
            name, data = self.artifact
            (job.run_dir / name).write_bytes(data)
        self.events.extend(self.script)

    def poll(self, timeout: float = 0.0) -> list[BackendEvent]:
        if not self.events:
            return []
        return [self.events.pop(0)]

    def cancel(self) -> None:
        self.cancelled += 1
        self.events.append(BackendEvent(kind="cancelled", message="cancelled"))

    def abort(self) -> None:
        self.aborted += 1

    def shutdown(self) -> None:
        self.shutdowns += 1


def ready(_: BackendHealth) -> Readiness:
    return Readiness(
        supported=True,
        available=True,
        calibration_id="cal-test",
        se_version="2024.0",
        detail="test readiness",
    )


def make_manager(
    tmp_path: Path,
    backend: ScriptedBackend,
    *,
    readiness: object | None = None,
    clock: FakeClock | None = None,
) -> JobManager:
    provider = readiness if readiness is not None else ready
    return JobManager(
        tmp_path,
        lambda: backend,
        readiness_provider=provider,  # type: ignore[arg-type]
        start_supervisor=False,
        clock=clock or FakeClock(),
        deadline_grace=0.0,
    )


def test_successful_build_publishes_verified_artifact(tmp_path: Path) -> None:
    backend = ScriptedBackend(
        script=[
            BackendEvent(
                kind="step",
                trace=StepTrace(
                    seq=99, op="part.measure", status="ok", message="measured", duration_ms=1.5
                ),
            ),
            BackendEvent(
                kind="completed",
                artifacts=(ArtifactSpec(name="plate.par", format=ArtifactFormat.PAR),),
            ),
        ],
        artifact=("plate.par", b"PAR-data"),
    )
    manager = make_manager(tmp_path, backend)

    accepted = manager.submit(build_request())
    assert accepted.state is JobState.QUEUED
    manager.run_once()
    manager.run_once()

    status = manager.status(accepted.build_id)
    assert status.state is JobState.SUCCEEDED
    assert status.step_index == status.total
    assert status.result is not None
    assert [entry.seq for entry in status.trace] == [0]
    artifact, data = manager.artifact(accepted.build_id, "plate.par")
    assert artifact.sha256
    assert data == b"PAR-data"
    manager.close()


def test_same_request_id_and_digest_is_idempotent(tmp_path: Path) -> None:
    backend = ScriptedBackend(script=[BackendEvent(kind="completed")])
    manager = make_manager(tmp_path, backend)
    request = build_request("stable-request")

    first = manager.submit(request)
    second = manager.submit(request)

    assert first.build_id == second.build_id
    manager.run_once()
    assert len(backend.submitted) == 1
    manager.close()


def test_same_request_id_with_a_different_payload_conflicts(tmp_path: Path) -> None:
    backend = ScriptedBackend(script=[BackendEvent(kind="completed")])
    manager = make_manager(tmp_path, backend)
    manager.submit(build_request("stable-request", summary="first"))

    with pytest.raises(ConflictError):
        manager.submit(build_request("stable-request", summary="second"))
    manager.close()


def test_second_build_is_busy_while_one_owns_the_worker(tmp_path: Path) -> None:
    backend = ScriptedBackend(script=[BackendEvent(kind="completed")])
    manager = make_manager(tmp_path, backend)
    manager.submit(build_request("one"))

    with pytest.raises(BusyError):
        manager.submit(build_request("two"))
    manager.close()


def test_cancel_before_the_worker_starts_never_submits(tmp_path: Path) -> None:
    backend = ScriptedBackend()
    manager = make_manager(tmp_path, backend)
    accepted = manager.submit(build_request())

    status = manager.cancel(accepted.build_id)
    manager.run_once()

    assert status.state is JobState.CANCELLED
    assert status.result is not None
    assert status.result.state is JobState.CANCELLED
    assert backend.submitted == []
    manager.close()


def test_cancel_is_cooperative_while_running(tmp_path: Path) -> None:
    backend = ScriptedBackend()
    manager = make_manager(tmp_path, backend)
    accepted = manager.submit(build_request())
    manager.run_once()

    requested = manager.cancel(accepted.build_id)
    assert requested.state is JobState.RUNNING
    manager.run_once()
    manager.run_once()

    assert manager.status(accepted.build_id).state is JobState.CANCELLED
    assert backend.cancelled == 1
    manager.close()


def test_deadline_marks_indeterminate_and_blocks_admission_until_reconciled(
    tmp_path: Path,
) -> None:
    clock = FakeClock()
    backend = ScriptedBackend()
    manager = make_manager(tmp_path, backend, clock=clock)
    accepted = manager.submit(build_request(timeout_seconds=10.0))
    manager.run_once()

    clock.advance(11.0)
    manager.run_once()

    status = manager.status(accepted.build_id)
    assert status.state is JobState.INDETERMINATE
    assert status.result is not None
    assert status.result.error is not None
    assert status.result.error.code is ErrorCode.TIMEOUT_INDETERMINATE
    assert backend.aborted == 1
    with pytest.raises(BusyError):
        manager.submit(build_request("blocked", timeout_seconds=10.0))

    reconciled = manager.reconcile(accepted.build_id)
    assert reconciled.state is JobState.INTERRUPTED
    assert manager.submit(build_request("allowed", timeout_seconds=10.0)).build_id
    manager.close()


def test_restart_marks_a_running_job_indeterminate_and_never_replays(tmp_path: Path) -> None:
    backend = ScriptedBackend()
    manager = make_manager(tmp_path, backend)
    accepted = manager.submit(build_request())
    manager.run_once()
    assert manager.status(accepted.build_id).state is JobState.RUNNING
    manager.close()

    second_backend = ScriptedBackend()
    restarted = JobManager(
        tmp_path,
        lambda: second_backend,
        readiness_provider=ready,
        start_supervisor=False,
        clock=FakeClock(),
    )

    status = restarted.status(accepted.build_id)
    assert status.state is JobState.INDETERMINATE
    assert "not replayed" in (status.message or "")
    assert status.result is not None
    assert status.result.error is not None
    assert status.result.error.code is ErrorCode.TIMEOUT_INDETERMINATE
    with pytest.raises(BusyError):
        restarted.submit(build_request("after-restart"))
    restarted.run_once()
    assert second_backend.submitted == []
    restarted.close()


def test_restart_marks_a_never_submitted_queued_job_interrupted(tmp_path: Path) -> None:
    backend = ScriptedBackend()
    manager = make_manager(tmp_path, backend)
    accepted = manager.submit(build_request())
    assert manager.status(accepted.build_id).state is JobState.QUEUED
    manager.close()

    second_backend = ScriptedBackend()
    restarted = JobManager(
        tmp_path,
        lambda: second_backend,
        readiness_provider=ready,
        start_supervisor=False,
        clock=FakeClock(),
    )

    status = restarted.status(accepted.build_id)
    assert status.state is JobState.INTERRUPTED
    assert "not replayed" in (status.message or "")
    restarted.run_once()
    assert second_backend.submitted == []
    # A queued build never reached COM, so it does not block new admission.
    restarted.submit(build_request("allowed-after-queued-restart"))
    restarted.close()


def test_lost_submission_acknowledgement_is_indeterminate_and_blocks(
    tmp_path: Path,
) -> None:
    class AckLostBackend(ScriptedBackend):
        def submit(self, job: JobContext) -> None:
            self.submitted.append(job)
            raise UnavailableError("the worker closed its input after the build started")

    backend = AckLostBackend()
    manager = make_manager(tmp_path, backend)
    accepted = manager.submit(build_request())

    manager.run_once()

    status = manager.status(accepted.build_id)
    assert status.state is JobState.INDETERMINATE
    assert status.result is not None
    assert status.result.error is not None
    assert status.result.error.code is ErrorCode.TIMEOUT_INDETERMINATE
    assert "acknowledgement was lost" in (status.message or "")
    assert backend.aborted == 1
    with pytest.raises(BusyError):
        manager.submit(build_request("blocked-after-ambiguous"))
    manager.close()

    # A restart must not clear the block or replay the build.
    second_backend = ScriptedBackend()
    restarted = JobManager(
        tmp_path,
        lambda: second_backend,
        readiness_provider=ready,
        start_supervisor=False,
        clock=FakeClock(),
    )
    restarted_status = restarted.status(accepted.build_id)
    assert restarted_status.state is JobState.INDETERMINATE
    with pytest.raises(BusyError):
        restarted.submit(build_request("still-blocked"))
    for _ in range(3):
        restarted.run_once()
    assert second_backend.submitted == []
    restarted.close()


def test_a_known_prestart_refusal_may_fail_definitely(tmp_path: Path) -> None:
    from kala_solidedge_bridge.jobs import BuildNotStartedError

    class RefusingBackend(ScriptedBackend):
        def submit(self, job: JobContext) -> None:
            raise BuildNotStartedError("worker is not accepting builds")

    backend = RefusingBackend()
    manager = make_manager(tmp_path, backend)
    accepted = manager.submit(build_request())
    manager.run_once()

    status = manager.status(accepted.build_id)
    assert status.state is JobState.FAILED
    assert status.result is not None
    assert status.result.error is not None
    assert status.result.error.code is ErrorCode.UNAVAILABLE
    # A definite pre-start refusal frees the worker for the next build.
    manager.submit(build_request("next-request"))
    manager.close()


def test_a_crash_event_while_running_is_indeterminate(tmp_path: Path) -> None:
    backend = ScriptedBackend(
        script=[BackendEvent(kind="crashed", message="the worker process exited")]
    )
    manager = make_manager(tmp_path, backend)
    accepted = manager.submit(build_request())
    manager.run_once()
    manager.run_once()

    status = manager.status(accepted.build_id)
    assert status.state is JobState.INDETERMINATE
    assert backend.aborted == 0
    with pytest.raises(BusyError):
        manager.submit(build_request("blocked-after-crash"))
    manager.close()


def test_a_poll_failure_while_running_is_indeterminate(tmp_path: Path) -> None:
    class PollCrashBackend(ScriptedBackend):
        def poll(self, timeout: float = 0.0) -> list[BackendEvent]:
            raise RuntimeError("the worker reader died")

    backend = PollCrashBackend()
    manager = make_manager(tmp_path, backend)
    accepted = manager.submit(build_request())
    manager.run_once()

    status = manager.status(accepted.build_id)
    assert status.state is JobState.INDETERMINATE
    assert status.result is not None
    assert status.result.error is not None
    assert status.result.error.code is ErrorCode.TIMEOUT_INDETERMINATE
    assert backend.aborted == 1
    with pytest.raises(BusyError):
        manager.submit(build_request("blocked-after-poll-crash"))
    manager.close()


def test_readiness_is_enforced_before_admission(tmp_path: Path) -> None:
    backend = ScriptedBackend()
    unsupported = make_manager(
        tmp_path,
        backend,
        readiness=lambda _: Readiness(
            supported=False, available=False, calibration_id=None, detail="unsupported"
        ),
    )
    with pytest.raises(UnavailableError):
        unsupported.submit(build_request("unsupported"))
    unsupported.close()

    uncalibrated = make_manager(
        tmp_path / "second",
        backend,
        readiness=lambda _: Readiness(
            supported=True, available=True, calibration_id=None, detail="uncalibrated"
        ),
    )
    with pytest.raises(CalibrationRequiredError):
        uncalibrated.submit(build_request("uncalibrated"))
    uncalibrated.close()


def test_a_missing_request_calibration_is_refused_without_admission(tmp_path: Path) -> None:
    backend = ScriptedBackend()
    manager = make_manager(tmp_path, backend)

    with pytest.raises(CalibrationRequiredError):
        manager.submit(build_request("no-calibration", calibration_id=None))

    assert manager.health().busy is False
    assert list((tmp_path / "jobs").iterdir()) == []
    manager.run_once()
    assert backend.submitted == []
    manager.close()


def test_a_stale_request_calibration_is_refused_without_admission(tmp_path: Path) -> None:
    backend = ScriptedBackend()
    manager = make_manager(tmp_path, backend)

    with pytest.raises(ConflictError):
        manager.submit(build_request("stale-calibration", calibration_id="cal-old"))

    assert manager.health().busy is False
    assert list((tmp_path / "jobs").iterdir()) == []
    manager.run_once()
    assert backend.submitted == []
    manager.close()


def test_a_changed_host_calibration_does_not_replay_an_existing_job(tmp_path: Path) -> None:
    backend = ScriptedBackend(script=[BackendEvent(kind="completed")])
    current = {"value": "cal-test"}
    manager = JobManager(
        tmp_path,
        lambda: backend,
        readiness_provider=lambda _: Readiness(
            supported=True,
            available=True,
            calibration_id=current["value"],
            se_version="2024.0",
        ),
        start_supervisor=False,
        clock=FakeClock(),
    )
    request = build_request("stable-request", calibration_id="cal-test")
    first = manager.submit(request)

    # The host re-probes while the job is still pending; the identical request
    # is still the same job and is not re-executed.
    current["value"] = "cal-new"
    second = manager.submit(request)
    assert second.build_id == first.build_id
    manager.run_once()
    assert len(backend.submitted) == 1

    # A *new* semantic request must carry the current calibration.
    with pytest.raises(ConflictError):
        manager.submit(build_request("new-request", calibration_id="cal-test"))
    manager.close()


def test_trace_is_monotonic_and_bounded(tmp_path: Path) -> None:
    steps = [
        BackendEvent(
            kind="step",
            trace=StepTrace(seq=index, op="part.measure", status="ok", duration_ms=0.1),
        )
        for index in range(605)
    ]
    backend = ScriptedBackend(script=steps)
    manager = make_manager(tmp_path, backend)
    accepted = manager.submit(build_request())
    for _ in range(605):
        manager.run_once()

    status = manager.status(accepted.build_id)
    assert len(status.trace) == 600
    assert [entry.seq for entry in status.trace] == list(range(600))
    assert status.result is None
    job_payload = (tmp_path / "jobs" / accepted.build_id / "job.json").read_text(encoding="utf-8")
    assert "trace truncated" in job_payload
    manager.close()


def test_status_is_durable_and_tampered_artifact_is_refused(tmp_path: Path) -> None:
    backend = ScriptedBackend(
        script=[
            BackendEvent(
                kind="completed",
                artifacts=(ArtifactSpec(name="plate.par", format=ArtifactFormat.PAR),),
            )
        ],
        artifact=("plate.par", b"PAR-data"),
    )
    manager = make_manager(tmp_path, backend)
    accepted = manager.submit(build_request())
    manager.run_once()
    manager.run_once()
    assert manager.status(accepted.build_id).state is JobState.SUCCEEDED

    artifact_path = tmp_path / "jobs" / accepted.build_id / "plate.par"
    artifact_path.write_bytes(b"PAR-DATA")
    with pytest.raises(UnavailableError):
        manager.artifact(accepted.build_id, "plate.par")
    with pytest.raises(KeyError):
        manager.artifact(accepted.build_id, "other.par")
    manager.close()


def test_failed_event_is_terminal_and_keeps_the_typed_error(tmp_path: Path) -> None:
    backend = ScriptedBackend(
        script=[
            BackendEvent(
                kind="failed",
                error=make_error(ErrorCode.UNAVAILABLE, "no executor"),
            )
        ]
    )
    manager = make_manager(tmp_path, backend)
    accepted = manager.submit(build_request())
    manager.run_once()
    manager.run_once()

    status = manager.status(accepted.build_id)
    assert status.state is JobState.FAILED
    assert status.result is not None
    assert status.result.error is not None
    assert status.result.error.code is ErrorCode.UNAVAILABLE
    manager.close()


def make_error(code: ErrorCode, message: str) -> ErrorResponse:
    return ErrorResponse(code=code, message=message)


def test_health_reports_busy_and_never_claims_native_support(tmp_path: Path) -> None:
    attached = BackendHealth(
        platform="win32",
        supported=False,
        available=True,
        attached=True,
        se_version="2024.0",
        detail="attached",
    )
    direct = native_readiness(attached, CalibrationStore(tmp_path))
    assert direct.supported is False
    assert direct.available is True
    assert "probe" in direct.detail.lower()

    backend = ScriptedBackend(health=attached)
    manager = make_manager(tmp_path, backend)
    manager.run_once()
    manager.submit(build_request())
    manager.run_once()

    health = manager.health()
    assert health.supported is True
    assert health.available is True
    assert health.busy is True
    assert health.se_version == "2024.0"
    manager.close()
