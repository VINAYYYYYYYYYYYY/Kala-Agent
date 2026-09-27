"""Security tests for the two-phase, fail-closed revision commit.

A design revision is the one write in this system that changes design history,
so these tests attack that write and nothing else. They are written from the
position of a caller that wants a revision written without authorizing it, or
written on top of a state it never reviewed:

* a revision is only written behind a pre-commit approval bound to the digests
  of the state it commits, and every other outcome writes nothing;
* the parent and head compare-and-swap is transactional, so a lost race leaves
  no orphan revision row and is reported as a conflict;
* a commit must extend the current head, so a design history cannot be forked;
* the approved checkpoint decision is persisted, read back, and is what a
  completed workflow needs before it may write its revision;
* policy approvals and budget usage survive a restart, and state that cannot be
  verified stops the application instead of resetting it.

Every test runs offline against a temporary data directory, and every failure is
asserted on the state left behind rather than on the message alone.
"""

from __future__ import annotations

import hashlib
import sqlite3
import threading
from collections.abc import Callable, Iterator
from functools import partial
from pathlib import Path

import pytest

from kala_agent.application import (
    BUDGET_STATE_AGGREGATE,
    COMMITTED_REVISION_EVENT,
    POLICY_STATE_AGGREGATE,
    ApplicationError,
    CheckpointDecisionDeniedError,
    CheckpointDigestMismatchError,
    CheckpointNotPendingError,
    CorruptKernelStateError,
    KalaApplication,
    ReservedIdentifierError,
    RevisionForkRefusedError,
    kernel_state_aggregate,
)
from kala_agent.config import Settings
from kala_agent.contracts import ApprovalStatus
from kala_agent.persistence.repositories import (
    ApprovalAlreadyDecidedError,
    DesignRevisionRepository,
    HeadMovedError,
)
from kala_agent.tools.protocol import canonical_json_dumps

DESIGN = "commit-security"
_RESERVED_WORKFLOW_CALLS = (
    "get_workflow_state",
    "list_workflow_checkpoints",
    "approve_workflow_checkpoint",
    "reject_workflow_checkpoint",
)
_DEFAULT_GRANTS = (
    "artifact.read",
    "artifact.write",
    "design.read",
    "design.write",
    "tool.execute",
)


@pytest.fixture(autouse=True)
def _forbid_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    def blocked(*args: object, **kwargs: object) -> None:
        raise AssertionError("the composition root must not open network connections")

    monkeypatch.setattr("socket.socket.connect", blocked)
    monkeypatch.setattr("socket.socket.connect_ex", blocked)
    monkeypatch.setattr("socket.create_connection", blocked)
    yield


@pytest.fixture
def data_dir(tmp_path: Path) -> Path:
    return tmp_path / "data"


@pytest.fixture
def application(data_dir: Path) -> Iterator[KalaApplication]:
    resolved = KalaApplication(Settings(data_dir=data_dir))
    try:
        yield resolved
    finally:
        resolved.close()


def _revision_rows(application: KalaApplication) -> list[tuple[str, int]]:
    with application.database.serialized() as connection:
        return [
            (str(row["revision_id"]), int(row["sequence"]))
            for row in connection.execute(
                "SELECT revision_id, sequence FROM design_revisions ORDER BY sequence"
            ).fetchall()
        ]


def test_a_prepared_commit_is_not_a_revision_and_needs_an_approval(
    application: KalaApplication,
) -> None:
    prepared = application.prepare_design_revision(design_id=DESIGN, domain="mechanical")
    assert prepared.status.value == "pending"
    assert prepared.committed is False
    assert prepared.revision_id == f"rev-{DESIGN}-r0"
    assert prepared.sequence == 0
    assert prepared.parent_revision_id is None
    assert len(prepared.state_digest) == 64
    assert len(prepared.action_digest) == 64
    assert prepared.spec_digest != prepared.state_digest
    assert application.list_design_revisions(DESIGN) == ()
    assert _revision_rows(application) == []

    record = application.commit_design_revision(
        prepared.request_id,
        state_digest=prepared.state_digest,
        action_digest=prepared.action_digest,
        actor_id="security-operator",
    )
    assert record.revision.revision_id == f"rev-{DESIGN}-r0"
    assert _revision_rows(application) == [(f"rev-{DESIGN}-r0", 0)]
    events = application.list_events(limit=50).events
    types = [event.event_type for event in events]
    assert types.count("revision.prepared") == 1
    assert types.count("revision.created") == 1
    committed = next(event for event in events if event.event_type == "revision.created")
    prepared_event = next(event for event in events if event.event_type == "revision.prepared")
    assert committed.payload is not None
    assert prepared_event.payload is not None
    assert committed.payload["state_digest"] == prepared_event.payload["state_digest"]
    assert committed.payload["action_digest"] == prepared_event.payload["action_digest"]
    assert committed.payload["actor_id"] == "security-operator"


def test_the_state_digest_covers_the_head_the_spec_the_packages_and_the_artifacts(
    application: KalaApplication,
) -> None:
    first = application.prepare_design_revision(design_id=DESIGN, domain="mechanical")
    application.commit_design_revision(first.request_id)
    second = application.prepare_design_revision(design_id=DESIGN, domain="mechanical")
    assert second.state_digest != first.state_digest
    assert second.action_digest != first.action_digest
    assert second.parent_revision_id == f"rev-{DESIGN}-r0"
    assert second.sequence == 1
    assert second.artifact_digests == first.artifact_digests
    assert second.package_digests == first.package_digests

    overridden = application.prepare_design_revision(
        design_id=DESIGN,
        domain="mechanical",
        parameters={"load_kn": 9.0},
    )
    assert overridden.parameters_digest != first.parameters_digest
    assert overridden.artifact_digests != first.artifact_digests
    assert overridden.spec_digest != first.spec_digest
    assert overridden.state_digest != first.state_digest

    summarized = application.prepare_design_revision(
        design_id=DESIGN,
        domain="mechanical",
        parameters={"load_kn": 9.0},
        summary="a different summary",
    )
    assert summarized.spec_digest != overridden.spec_digest
    assert summarized.state_digest != overridden.state_digest
    assert summarized.action_digest != overridden.action_digest

    other_domain = application.prepare_design_revision(
        design_id=DESIGN,
        domain="wing",
        parameters={"span_m": 9.0},
        summary="a different summary",
    )
    assert other_domain.package_digests != summarized.package_digests
    assert other_domain.artifact_digests != summarized.artifact_digests
    assert other_domain.state_digest != summarized.state_digest
    assert application.list_design_revisions(DESIGN)[0].revision.revision_id == f"rev-{DESIGN}-r0"


def test_a_stale_approval_writes_nothing(application: KalaApplication) -> None:
    reviewed = application.prepare_design_revision(
        design_id=DESIGN,
        domain="mechanical",
        summary="the configuration under review",
    )
    other = application.prepare_design_revision(
        design_id=DESIGN,
        domain="mechanical",
        summary="a different configuration",
    )
    application.commit_design_revision(other.request_id)
    assert application.list_design_revisions(DESIGN)[0].revision.revision_id == f"rev-{DESIGN}-r0"

    with pytest.raises(CheckpointDigestMismatchError):
        application.commit_design_revision(reviewed.request_id)
    assert _revision_rows(application) == [(f"rev-{DESIGN}-r0", 0)]
    assert application.get_revision_commit(reviewed.request_id).status.value == "pending"

    supplied = application.prepare_design_revision(design_id="supplied", domain="mechanical")
    with pytest.raises(CheckpointDigestMismatchError):
        application.commit_design_revision(supplied.request_id, state_digest="0" * 64)
    with pytest.raises(CheckpointDigestMismatchError):
        application.commit_design_revision(supplied.request_id, action_digest="0" * 64)
    assert application.list_design_revisions("supplied") == ()

    assert (
        application.commit_design_revision(
            supplied.request_id,
            state_digest=supplied.state_digest,
            action_digest=supplied.action_digest,
        ).revision.revision_id
        == "rev-supplied-r0"
    )


def test_a_rejected_approval_writes_nothing_and_cannot_be_reused(
    application: KalaApplication,
) -> None:
    prepared = application.prepare_design_revision(design_id=DESIGN, domain="mechanical")
    rejected = application.decide_revision_commit(
        prepared.request_id,
        "reject",
        actor_id="security-operator",
        reason="the surrogate result is not what the operator reviewed",
    )
    assert rejected.status.value == "rejected"
    assert rejected.committed is False
    assert application.list_design_revisions(DESIGN) == ()
    assert _revision_rows(application) == []
    types = [event.event_type for event in application.list_events(limit=50).events]
    assert "revision.commit_rejected" in types
    assert "revision.created" not in types

    with pytest.raises(CheckpointNotPendingError):
        application.commit_design_revision(prepared.request_id)
    assert application.list_design_revisions(DESIGN) == ()

    read_back = application.get_revision_commit(prepared.request_id)
    assert read_back.status.value == "rejected"
    assert read_back.committed is False
    assert read_back.actor_id == "security-operator"
    assert read_back.reason == "the surrogate result is not what the operator reviewed"
    stored = application.approval_requests.get(prepared.request_id)
    assert stored is not None
    assert stored.status.value == "rejected"
    assert stored.selected_option_id == "reject"
    assert stored.actor_id == "security-operator"
    assert stored.checkpoint_type.value == "pre_commit"


def test_a_revoked_capability_prevents_the_write(application: KalaApplication) -> None:
    prepared = application.prepare_design_revision(design_id=DESIGN, domain="mechanical")
    assert application.policy.revoke("design.write") is True
    with pytest.raises(ApplicationError) as excinfo:
        application.commit_design_revision(prepared.request_id)
    assert "capability_not_granted" in str(excinfo.value)
    assert application.list_design_revisions(DESIGN) == ()
    assert _revision_rows(application) == []

    application.policy.grant("design.write")
    application.commit_design_revision(prepared.request_id)
    assert _revision_rows(application) == [(f"rev-{DESIGN}-r0", 0)]


def test_a_commit_cannot_fork_the_design_history(application: KalaApplication) -> None:

    first = application.create_design_revision(design_id=DESIGN, domain="mechanical")
    second = application.create_design_revision(design_id=DESIGN, domain="mechanical")
    with pytest.raises(RevisionForkRefusedError):
        application.create_design_revision(
            design_id=DESIGN,
            domain="mechanical",
            parent_revision_id=first.revision.revision_id,
        )
    with pytest.raises(RevisionForkRefusedError):
        application.prepare_design_revision(
            design_id=DESIGN,
            domain="mechanical",
            parent_revision_id=first.revision.revision_id,
        )
    with pytest.raises(RevisionForkRefusedError):
        application.create_design_revision(
            design_id="unborn",
            domain="mechanical",
            parent_revision_id=second.revision.revision_id,
        )
    assert _revision_rows(application) == [
        (first.revision.revision_id, 0),
        (second.revision.revision_id, 1),
    ]


def test_the_head_compare_and_swap_is_transactional_and_leaves_no_orphan(
    data_dir: Path,
) -> None:
    writer = KalaApplication(Settings(data_dir=data_dir))
    try:
        prepared = writer.prepare_design_revision(design_id=DESIGN, domain="mechanical")
        winner = KalaApplication(Settings(data_dir=data_dir))
        try:
            winner.create_design_revision(
                design_id=DESIGN,
                domain="mechanical",
                summary="committed while the review was open",
            )
        finally:
            winner.close()
        with pytest.raises(CheckpointDigestMismatchError):
            writer.commit_design_revision(prepared.request_id)
        assert _revision_rows(writer) == [(f"rev-{DESIGN}-r0", 0)]
    finally:
        writer.close()

    database = Settings(data_dir=data_dir).database_path
    with sqlite3.connect(database) as connection:
        rows = connection.execute(
            "SELECT revision_id FROM design_revisions ORDER BY sequence"
        ).fetchall()
    assert [row[0] for row in rows] == [f"rev-{DESIGN}-r0"]


def test_a_lost_race_reports_a_head_moved_conflict_and_writes_no_row(
    data_dir: Path,
) -> None:
    first = KalaApplication(Settings(data_dir=data_dir))
    try:
        prepared = first.prepare_design_revision(design_id=DESIGN, domain="mechanical")
        second = KalaApplication(Settings(data_dir=data_dir))
        try:
            resolved = second.prepare_design_revision(design_id=DESIGN, domain="mechanical")
            assert resolved.request_id == prepared.request_id
        finally:
            second.close()
        assert first.commit_design_revision(prepared.request_id).revision.sequence == 0
        assert _revision_rows(first) == [(f"rev-{DESIGN}-r0", 0)]
    finally:
        first.close()

    observer = KalaApplication(Settings(data_dir=data_dir))
    try:
        repository = DesignRevisionRepository(observer.database)
        head = repository.latest_for_design(DESIGN)
        assert head is not None
        for revision_id, sequence, expected_head in (
            (f"rev-{DESIGN}-r2", 2, f"rev-{DESIGN}-r0"),
            (f"rev-{DESIGN}-r1", 1, f"rev-{DESIGN}-r7"),
        ):
            with pytest.raises(HeadMovedError) as excinfo:
                repository.commit_revision(
                    head.model_copy(update={"revision_id": revision_id, "sequence": sequence}),
                    expected_head_revision_id=expected_head,
                )
            assert excinfo.value.design_id == DESIGN
            assert excinfo.value.actual_head_revision_id == f"rev-{DESIGN}-r0"
            assert excinfo.value.actual_sequence == 0
            assert excinfo.value.expected_sequence == sequence
        assert repository.latest_for_design(DESIGN) == head
    finally:
        observer.close()
    with sqlite3.connect(Settings(data_dir=data_dir).database_path) as connection:
        sequences = [
            int(row[0])
            for row in connection.execute("SELECT sequence FROM design_revisions").fetchall()
        ]
    assert sequences == [0]


def test_two_concurrent_commits_produce_one_revision(
    application: KalaApplication,
    data_dir: Path,
) -> None:
    first = KalaApplication(Settings(data_dir=data_dir))
    second = KalaApplication(Settings(data_dir=data_dir))
    try:
        results: list[str] = []
        errors: list[Exception] = []
        barrier = threading.Barrier(2)

        def commit(resolved: KalaApplication, load_kn: float) -> None:
            try:
                barrier.wait(timeout=30)
                record = resolved.create_design_revision(
                    design_id=DESIGN,
                    domain="mechanical",
                    parameters={"load_kn": load_kn},
                )
                results.append(record.revision.revision_id)
            except Exception as error:
                errors.append(error)

        threads = [
            threading.Thread(target=commit, args=(first, 8.0)),
            threading.Thread(target=commit, args=(second, 9.0)),
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=60)
        assert all(isinstance(error, CheckpointDecisionDeniedError) for error in errors)
        assert len(results) == 1
        assert _revision_rows(application) == [(f"rev-{DESIGN}-r0", 0)]
    finally:
        first.close()
        second.close()


def test_the_workflow_revision_needs_its_own_approved_checkpoint(
    application: KalaApplication,
) -> None:
    run = application.run_mechanical_reference_workflow(design_id=DESIGN)
    prepared = application.prepare_design_revision(
        design_id=DESIGN,
        domain="mechanical",
        workflow_id=run.workflow_id,
    )
    with pytest.raises(ApplicationError) as excinfo:
        application.commit_design_revision(prepared.request_id, workflow_id=run.workflow_id)
    assert "has not completed" in str(excinfo.value)
    assert application.list_design_revisions(DESIGN) == ()

    decided = application.approve_workflow_checkpoint(run.workflow_id, actor_id="operator-a")
    assert decided.status.value == "completed"
    assert decided.revision is not None
    assert decided.revision.revision.revision_id == f"rev-{DESIGN}-r0"
    approvals = application.list_workflow_checkpoints(run.workflow_id)
    assert [record.status.value for record in approvals] == ["approved"]
    assert approvals[0].actor_id == "operator-a"
    assert approvals[0].selected_option_id == "approve"
    assert approvals[0].decided_at is not None
    assert approvals[0].checkpoint_type.value == "pre_commit"

    created = next(
        event
        for event in application.list_events(limit=200).events
        if event.event_type == "revision.created"
    )
    assert created.payload is not None
    request_id = str(created.payload["request_id"])
    commit = application.get_revision_commit(request_id)
    assert commit.status.value == "approved"
    assert commit.committed is True
    assert commit.actor_id == "operator-a"
    assert commit.workflow_id == run.workflow_id
    assert commit.sequence == 0
    stored_commit = application.approval_requests.get(commit.request_id)
    assert stored_commit is not None
    assert stored_commit.workflow_id == commit.request_id
    assert stored_commit.checkpoint_type.value == "pre_commit"
    assert stored_commit.selected_option_id == "approve"


def test_a_rejected_workflow_leaves_no_revision_and_no_authorization(
    application: KalaApplication,
) -> None:
    run = application.run_mechanical_reference_workflow(design_id=DESIGN)
    decided = application.reject_workflow_checkpoint(run.workflow_id, reason="declined")
    assert decided.status.value == "failed"
    assert decided.revision is None
    assert application.list_design_revisions(DESIGN) == ()
    assert _revision_rows(application) == []
    prepared = application.prepare_design_revision(
        design_id=DESIGN,
        domain="mechanical",
        workflow_id=run.workflow_id,
    )
    with pytest.raises(ApplicationError) as excinfo:
        application.commit_design_revision(prepared.request_id, workflow_id=run.workflow_id)
    assert "authorizes no revision commit" in str(excinfo.value)
    assert application.list_design_revisions(DESIGN) == ()


def test_a_workflow_cannot_authorize_another_designs_commit(
    application: KalaApplication,
) -> None:
    run = application.run_mechanical_reference_workflow(design_id=DESIGN)
    application.approve_workflow_checkpoint(run.workflow_id)
    prepared = application.prepare_design_revision(design_id="unrelated", domain="mechanical")
    with pytest.raises(ApplicationError) as excinfo:
        application.commit_design_revision(prepared.request_id, workflow_id=run.workflow_id)
    assert "was not prepared for workflow" in str(excinfo.value)
    assert application.list_design_revisions("unrelated") == ()


def test_a_completed_workflow_without_a_stored_approval_authorizes_nothing(
    application: KalaApplication,
) -> None:
    run = application.run_mechanical_reference_workflow(design_id=DESIGN)
    application.approve_workflow_checkpoint(run.workflow_id)
    stored = application.approval_requests.get(
        application.list_workflow_checkpoints(run.workflow_id)[0].request_id
    )
    assert stored is not None
    tampered = stored.model_copy(
        update={"status": ApprovalStatus.REJECTED, "selected_option_id": "reject"}
    )
    with pytest.raises(ApprovalAlreadyDecidedError):
        application.approval_requests.save(tampered)
    with application.database.transaction() as connection:
        connection.execute(
            "UPDATE approval_requests SET status = ?, model_json = ? WHERE request_id = ?",
            (tampered.status.value, tampered.model_dump_json(), tampered.request_id),
        )
    prepared = application.prepare_design_revision(
        design_id=DESIGN,
        domain="mechanical",
        summary="a second configuration of the same design",
        workflow_id=run.workflow_id,
    )
    with pytest.raises(ApplicationError) as excinfo:
        application.commit_design_revision(prepared.request_id, workflow_id=run.workflow_id)
    assert "no single approved pre-commit checkpoint" in str(excinfo.value)
    assert [
        record.revision.revision_id for record in application.list_design_revisions(DESIGN)
    ] == [f"rev-{DESIGN}-r0"]


def test_a_commit_that_fails_midway_leaves_nothing_and_can_be_retried(
    data_dir: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    first = KalaApplication(Settings(data_dir=data_dir))
    try:
        prepared = first.prepare_design_revision(design_id=DESIGN, domain="mechanical")
        events_before = first.journal.verify_chain().event_count
        original_append = first.journal.append

        def failing_append(event_type: str, *args: object, **kwargs: object) -> object:
            if event_type == COMMITTED_REVISION_EVENT:
                raise sqlite3.OperationalError("database is locked")
            return original_append(event_type, *args, **kwargs)  # type: ignore[arg-type]

        monkeypatch.setattr(first.journal, "append", failing_append)
        with pytest.raises(sqlite3.OperationalError):
            first.commit_design_revision(prepared.request_id)
        monkeypatch.undo()

        assert _revision_rows(first) == []
        assert first.list_design_revisions(DESIGN) == ()
        assert first.journal.verify_chain().event_count == events_before
        stored = first.approval_requests.get(prepared.request_id)
        assert stored is not None
        assert stored.status is ApprovalStatus.PENDING
    finally:
        first.close()

    second = KalaApplication(Settings(data_dir=data_dir))
    try:
        record = second.commit_design_revision(prepared.request_id)
        assert record.revision.revision_id == f"rev-{DESIGN}-r0"
        assert record.provenance is not None
    finally:
        second.close()


def test_an_operation_that_is_never_closed_does_not_block_the_next_launch(
    data_dir: Path,
) -> None:
    first = KalaApplication(Settings(data_dir=data_dir))
    first.evaluate_domain_design("mechanical")
    assert first.journal.verify_chain().event_count > 0

    second = KalaApplication(Settings(data_dir=data_dir))
    try:
        assert second.budget.usage == first.budget.usage
    finally:
        second.close()
        first.close()


def test_a_decided_revision_commit_cannot_be_prepared_again_from_the_store(
    application: KalaApplication,
) -> None:
    prepared = application.prepare_design_revision(design_id=DESIGN, domain="mechanical")
    application.decide_revision_commit(prepared.request_id, "reject")
    stored = application.approval_requests.get(prepared.request_id)
    assert stored is not None
    with pytest.raises(ApprovalAlreadyDecidedError):
        application.approval_requests.save(
            stored.model_copy(update={"status": ApprovalStatus.PENDING})
        )
    application.approval_requests.save(stored)
    with pytest.raises(CheckpointNotPendingError):
        application.prepare_design_revision(design_id=DESIGN, domain="mechanical")


def test_policy_approvals_and_budget_usage_survive_a_restart(data_dir: Path) -> None:
    first = KalaApplication(Settings(data_dir=data_dir))
    try:
        run = first.run_mechanical_reference_workflow(design_id=DESIGN)
        prepared = first.prepare_design_revision(design_id="durable", domain="mechanical")
        usage = first.budget.usage
        assert usage.artifact_bytes > 0
        approvals = {record.request_id for record in first.policy.approvals}
        assert prepared.request_id in approvals
        pending = first.policy.get_approval(prepared.request_id)
        assert pending is not None
        assert pending.status.value == "pending"
    finally:
        first.close()

    second = KalaApplication(Settings(data_dir=data_dir))
    try:
        assert second.budget.usage == usage
        assert {record.request_id for record in second.policy.approvals} >= approvals
        assert second.policy.grants >= first.policy.grants
        restored = second.policy.get_approval(prepared.request_id)
        assert restored is not None
        assert restored.status.value == "pending"
        assert restored.state_digest == prepared.state_digest
        assert restored.action_digest == prepared.action_digest
        assert second.get_revision_commit(prepared.request_id).status.value == "pending"

        decided = second.decide_revision_commit(
            prepared.request_id, "approve", actor_id="restarted-operator"
        )
        assert decided.committed is True
        assert second.list_design_revisions("durable")[0].revision.revision_id == "rev-durable-r0"
        assert second.run_mechanical_reference_workflow(design_id=DESIGN).started is False
        assert second.approve_workflow_checkpoint(run.workflow_id).status.value == "completed"
    finally:
        second.close()

    third = KalaApplication(Settings(data_dir=data_dir))
    try:
        assert third.budget.usage.artifact_bytes > usage.artifact_bytes
        approved = third.policy.get_approval(prepared.request_id)
        assert approved is not None
        assert approved.status.value == "approved"
        assert approved.actor_id == "restarted-operator"
    finally:
        third.close()


def test_a_revoked_capability_stays_revoked_across_a_restart(data_dir: Path) -> None:
    first = KalaApplication(Settings(data_dir=data_dir))
    try:
        prepared = first.prepare_design_revision(design_id=DESIGN, domain="mechanical")
        first.policy.revoke("design.write")
    finally:
        first.close()

    second = KalaApplication(Settings(data_dir=data_dir))
    try:
        assert "design.write" not in second.policy.grants
        with pytest.raises(ApplicationError):
            second.commit_design_revision(prepared.request_id)
        assert second.list_design_revisions(DESIGN) == ()
    finally:
        second.close()


def test_a_tampered_kernel_state_record_stops_the_launch(data_dir: Path) -> None:
    first = KalaApplication(Settings(data_dir=data_dir))
    try:
        first.create_design_revision(design_id=DESIGN, domain="mechanical")
    finally:
        first.close()

    database = Settings(data_dir=data_dir).database_path
    with sqlite3.connect(database) as connection:
        connection.execute(
            "UPDATE workflow_snapshots SET digest = 'tampered' WHERE workflow_id = ?",
            (kernel_state_aggregate(POLICY_STATE_AGGREGATE),),
        )
    with pytest.raises(CorruptKernelStateError) as excinfo:
        KalaApplication(Settings(data_dir=data_dir))
    assert excinfo.value.code == "corrupt_kernel_state"
    assert "verification" in str(excinfo.value)

    with sqlite3.connect(database) as connection:
        connection.execute(
            "UPDATE workflow_snapshots SET digest = 'tampered' WHERE workflow_id = ?",
            (kernel_state_aggregate(BUDGET_STATE_AGGREGATE),),
        )
    with pytest.raises(CorruptKernelStateError):
        KalaApplication(Settings(data_dir=data_dir))


def test_a_kernel_state_record_that_does_not_validate_stops_the_launch(
    data_dir: Path,
) -> None:
    first = KalaApplication(Settings(data_dir=data_dir))
    try:
        first.create_design_revision(design_id=DESIGN, domain="mechanical")
    finally:
        first.close()

    database = Settings(data_dir=data_dir).database_path
    with sqlite3.connect(database) as connection:
        row = connection.execute(
            "SELECT state_json FROM workflow_snapshots WHERE workflow_id = ?",
            (kernel_state_aggregate(POLICY_STATE_AGGREGATE),),
        ).fetchone()
        assert row is not None
        corrupted = canonical_json_dumps({"version": 1, "grants": "not-a-collection"})
        connection.execute(
            "UPDATE workflow_snapshots SET state_json = ?, digest = ? WHERE workflow_id = ?",
            (
                corrupted,
                hashlib.sha256(corrupted.encode("utf-8")).hexdigest(),
                kernel_state_aggregate(POLICY_STATE_AGGREGATE),
            ),
        )
    with pytest.raises(CorruptKernelStateError) as excinfo:
        KalaApplication(Settings(data_dir=data_dir))
    assert "policy state" in str(excinfo.value)


def test_a_clean_restart_restores_usage_grants_and_approvals(data_dir: Path) -> None:
    first = KalaApplication(Settings(data_dir=data_dir))
    try:
        first.create_design_revision(design_id=DESIGN, domain="mechanical")
        granted = first.policy.grants
        usage = first.budget.usage
    finally:
        first.close()

    reopened = KalaApplication(Settings(data_dir=data_dir))
    try:
        assert reopened.list_design_revisions(DESIGN)[0].revision.revision_id == f"rev-{DESIGN}-r0"
        assert reopened.budget.usage == usage
        assert reopened.budget.usage.artifact_bytes > 0
        assert reopened.policy.grants == granted
    finally:
        reopened.close()


def _reserved_workflow_calls(
    application: KalaApplication,
    workflow_id: str,
) -> tuple[Callable[[], object], ...]:
    calls: list[Callable[[], object]] = [
        partial(getattr(application, name), workflow_id) for name in _RESERVED_WORKFLOW_CALLS
    ]
    calls.append(partial(application.list_events, workflow_id=workflow_id))
    calls.append(
        partial(
            application.prepare_design_revision,
            design_id=DESIGN,
            domain="mechanical",
            workflow_id=workflow_id,
        )
    )
    return tuple(calls)


def test_a_workflow_identifier_may_not_name_a_reserved_kernel_aggregate(
    application: KalaApplication,
    data_dir: Path,
) -> None:
    reserved = (
        POLICY_STATE_AGGREGATE,
        BUDGET_STATE_AGGREGATE,
        kernel_state_aggregate(POLICY_STATE_AGGREGATE),
        f"{kernel_state_aggregate(BUDGET_STATE_AGGREGATE)}-0123456789ab",
        "kernel-state:anything",
    )
    for workflow_id in reserved:
        with pytest.raises(ReservedIdentifierError) as excinfo:
            application.run_mechanical_reference_workflow(
                workflow_id=workflow_id,
                design_id=DESIGN,
            )
        assert excinfo.value.code == "reserved_identifier"
        for call in _reserved_workflow_calls(application, workflow_id):
            with pytest.raises(ReservedIdentifierError) as failure:
                call()
            assert failure.value.code == "reserved_identifier"
    assert application.list_workflow_checkpoints(f"workflow-{DESIGN}") == ()
    with sqlite3.connect(Settings(data_dir=data_dir).database_path) as connection:
        aggregates = {
            str(row[0]) for row in connection.execute("SELECT workflow_id FROM workflow_snapshots")
        }
    assert aggregates <= {
        kernel_state_aggregate(POLICY_STATE_AGGREGATE),
        kernel_state_aggregate(BUDGET_STATE_AGGREGATE),
    }


def test_kernel_state_is_stored_under_the_reserved_namespace(data_dir: Path) -> None:
    first = KalaApplication(Settings(data_dir=data_dir))
    try:
        first.create_design_revision(design_id=DESIGN, domain="mechanical")
    finally:
        first.close()

    with sqlite3.connect(Settings(data_dir=data_dir).database_path) as connection:
        aggregates = {
            str(row[0]) for row in connection.execute("SELECT workflow_id FROM workflow_snapshots")
        }
    assert aggregates == {
        kernel_state_aggregate(POLICY_STATE_AGGREGATE),
        kernel_state_aggregate(BUDGET_STATE_AGGREGATE),
    }
    for aggregate in (POLICY_STATE_AGGREGATE, BUDGET_STATE_AGGREGATE):
        assert aggregate not in aggregates

    reopened = KalaApplication(Settings(data_dir=data_dir))
    try:
        assert reopened.list_design_revisions(DESIGN)
        assert reopened.budget.usage.artifact_bytes > 0
    finally:
        reopened.close()


def test_a_legacy_kernel_state_record_is_still_restored(data_dir: Path) -> None:
    first = KalaApplication(Settings(data_dir=data_dir))
    try:
        first.create_design_revision(design_id=DESIGN, domain="mechanical")
        usage = first.budget.usage
        grants = first.policy.grants
        state = first.policy.export_state()
    finally:
        first.close()

    with sqlite3.connect(Settings(data_dir=data_dir).database_path) as connection:
        connection.execute(
            "DELETE FROM workflow_snapshots WHERE workflow_id = ?",
            (kernel_state_aggregate(POLICY_STATE_AGGREGATE),),
        )
        connection.execute(
            "DELETE FROM workflow_snapshots WHERE workflow_id = ?",
            (kernel_state_aggregate(BUDGET_STATE_AGGREGATE),),
        )
        connection.execute(
            """
            INSERT INTO workflow_snapshots(
                snapshot_id, workflow_id, state_json, digest, created_at
            ) VALUES (?, ?, ?, ?, ?)
            """,
            (
                f"{POLICY_STATE_AGGREGATE}-legacy",
                POLICY_STATE_AGGREGATE,
                canonical_json_dumps(state),
                hashlib.sha256(canonical_json_dumps(state).encode("utf-8")).hexdigest(),
                "2026-01-02T03:04:05.000000Z",
            ),
        )
        connection.execute(
            """
            INSERT INTO workflow_snapshots(
                snapshot_id, workflow_id, state_json, digest, created_at
            ) VALUES (?, ?, ?, ?, ?)
            """,
            (
                f"{BUDGET_STATE_AGGREGATE}-legacy",
                BUDGET_STATE_AGGREGATE,
                canonical_json_dumps(first.budget.export_state()),
                hashlib.sha256(
                    canonical_json_dumps(first.budget.export_state()).encode("utf-8")
                ).hexdigest(),
                "2026-01-02T03:04:05.000000Z",
            ),
        )

    reopened = KalaApplication(Settings(data_dir=data_dir))
    try:
        assert reopened.policy.grants == grants
        assert reopened.budget.usage == usage
        assert reopened.budget.usage.artifact_bytes > 0
    finally:
        reopened.close()


def test_missing_kernel_state_in_a_used_directory_stops_the_launch(data_dir: Path) -> None:
    first = KalaApplication(Settings(data_dir=data_dir))
    try:
        first.create_design_revision(design_id=DESIGN, domain="mechanical")
    finally:
        first.close()

    database = Settings(data_dir=data_dir).database_path
    for removed, subject in (
        (kernel_state_aggregate(BUDGET_STATE_AGGREGATE), "budget"),
        (kernel_state_aggregate(POLICY_STATE_AGGREGATE), "policy"),
    ):
        with sqlite3.connect(database) as connection:
            connection.execute(
                "DELETE FROM workflow_snapshots WHERE workflow_id = ?",
                (removed,),
            )
        with pytest.raises(CorruptKernelStateError) as excinfo:
            KalaApplication(Settings(data_dir=data_dir))
        assert excinfo.value.code == "corrupt_kernel_state"
        assert f"{subject} state" in str(excinfo.value)
        assert "missing" in str(excinfo.value)
        with sqlite3.connect(database) as connection:
            connection.execute(
                """
                INSERT INTO workflow_snapshots(
                    snapshot_id, workflow_id, state_json, digest, created_at
                )
                SELECT snapshot_id || '-restored',
                       workflow_id,
                       state_json,
                       digest,
                       created_at
                FROM workflow_snapshots
                WHERE workflow_id = ?
                """,
                (
                    kernel_state_aggregate(
                        POLICY_STATE_AGGREGATE if subject == "budget" else BUDGET_STATE_AGGREGATE
                    ),
                ),
            )
    with sqlite3.connect(database) as connection:
        connection.execute("DELETE FROM workflow_snapshots WHERE workflow_id LIKE 'kernel-state:%'")
    with pytest.raises(CorruptKernelStateError):
        KalaApplication(Settings(data_dir=data_dir))


def test_a_fresh_directory_restores_default_kernel_state_without_a_record(
    data_dir: Path,
) -> None:
    fresh = KalaApplication(Settings(data_dir=data_dir))
    try:
        assert set(fresh.policy.grants) == set(_DEFAULT_GRANTS)
        assert fresh.budget.usage.artifact_bytes == 0
        with sqlite3.connect(Settings(data_dir=data_dir).database_path) as connection:
            assert connection.execute("SELECT COUNT(*) FROM events").fetchone()[0] == 0
            assert connection.execute("SELECT COUNT(*) FROM workflow_snapshots").fetchone()[0] == 0
    finally:
        fresh.close()
    reopened = KalaApplication(Settings(data_dir=data_dir))
    try:
        assert set(reopened.policy.grants) == set(_DEFAULT_GRANTS)
        assert reopened.budget.usage.artifact_bytes == 0
    finally:
        reopened.close()
