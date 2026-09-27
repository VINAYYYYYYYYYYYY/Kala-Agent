from __future__ import annotations

import json
from dataclasses import replace
from datetime import UTC, datetime, timedelta
from typing import Any

import pytest

from kala_agent.contracts import ApprovalKind, SideEffectLevel
from kala_agent.kernel import (
    ApprovalRecord,
    CapabilityGrant,
    PolicyDecision,
    PolicyDecisionStatus,
    PolicyEngine,
    PolicyReason,
    PolicyRequest,
    PolicyRequestError,
    PolicyStateError,
    PolicyTransitionError,
)

NOW = datetime(2026, 1, 2, 3, 4, 5, tzinfo=UTC)


def _request(
    request_id: str,
    *,
    state_digest: str = "state-1",
    action_digest: str = "action-1",
    capabilities: tuple[str, ...] = ("read",),
    side_effect_level: SideEffectLevel = SideEffectLevel.NONE,
    scope: str | None = None,
    checkpoint_type: ApprovalKind | None = None,
) -> PolicyRequest:
    resolved_checkpoint: ApprovalKind | None = checkpoint_type
    if resolved_checkpoint is None and side_effect_level is SideEffectLevel.EGRESS:
        resolved_checkpoint = ApprovalKind.EGRESS
    return PolicyRequest(
        request_id=request_id,
        state_digest=state_digest,
        action_digest=action_digest,
        requested_capabilities=capabilities,
        side_effect_level=side_effect_level,
        scope=scope,
        checkpoint_type=resolved_checkpoint,
    )


def _egress_request(
    request_id: str,
    *,
    state_digest: str = "state-1",
    action_digest: str = "action-1",
    scope: str | None = None,
) -> PolicyRequest:
    return _request(
        request_id,
        state_digest=state_digest,
        action_digest=action_digest,
        capabilities=("network.egress",),
        side_effect_level=SideEffectLevel.EGRESS,
        scope=scope,
    )


def _assert_not_allowed(decision: PolicyDecision) -> None:
    assert not decision.allowed
    assert not bool(decision)
    assert decision.denied


def test_egress_and_network_are_denied_without_grants() -> None:
    engine = PolicyEngine(now=NOW)

    egress = engine.decide(_egress_request("request-egress"), now=NOW)
    _assert_not_allowed(egress)
    assert egress.status is PolicyDecisionStatus.DENIED
    assert egress.reason is PolicyReason.CAPABILITY_NOT_GRANTED
    assert egress.missing_capabilities == ("network.egress",)
    assert egress.requires_approval

    network = engine.decide(
        _request(
            "request-network",
            capabilities=("network",),
            side_effect_level=SideEffectLevel.NETWORK,
        ),
        now=NOW,
    )
    _assert_not_allowed(network)
    assert network.reason is PolicyReason.CAPABILITY_NOT_GRANTED

    ungranted_read = engine.decide(_request("request-read"), now=NOW)
    _assert_not_allowed(ungranted_read)
    assert ungranted_read.missing_capabilities == ("read",)


def test_egress_without_capabilities_is_never_approvable() -> None:
    engine = PolicyEngine(now=NOW)
    request = _request(
        "request-bare-egress",
        capabilities=(),
        side_effect_level=SideEffectLevel.EGRESS,
    )

    decision = engine.decide(request, now=NOW)

    _assert_not_allowed(decision)
    assert decision.status is PolicyDecisionStatus.DENIED
    assert engine.request_approval(request, now=NOW).status.value == "pending"
    _assert_not_allowed(engine.approve(request, now=NOW))


def test_egress_requires_explicit_approval_even_with_a_valid_grant() -> None:
    engine = PolicyEngine({"network.egress"}, now=NOW)
    request = _egress_request("request-approved")

    pending = engine.decide(request, now=NOW)
    _assert_not_allowed(pending)
    assert pending.status is PolicyDecisionStatus.PENDING
    assert pending.reason is PolicyReason.APPROVAL_REQUIRED

    approval = engine.request_approval(request, now=NOW)
    assert approval.status.value == "pending"
    assert approval.actor_id is None

    allowed = engine.approve(request, actor_id="reviewer-1", now=NOW)
    assert allowed.status is PolicyDecisionStatus.ALLOWED
    assert allowed.reason is PolicyReason.ALLOWED
    assert allowed.allowed
    assert bool(allowed)
    assert allowed.actor_id == "reviewer-1"
    assert allowed.decided_at == NOW

    repeated = engine.decide(request, now=NOW)
    assert repeated.allowed
    assert repeated.status is PolicyDecisionStatus.ALLOWED


def test_read_only_requests_are_allowed_within_the_active_grant() -> None:
    engine = PolicyEngine({"read"}, now=NOW)

    decision = engine.decide(_request("request-allowed-read"), now=NOW)

    assert decision.allowed
    assert decision.status is PolicyDecisionStatus.ALLOWED
    assert decision.reason is PolicyReason.ALLOWED
    assert not decision.requires_approval
    assert decision.as_dict()["allowed"] is True


def test_expired_grants_deny_even_after_approval() -> None:
    engine = PolicyEngine(
        {
            "network.egress": CapabilityGrant(
                "network.egress",
                expires_at=NOW + timedelta(seconds=30),
            )
        },
        now=NOW,
    )
    request = _egress_request("request-expiring")

    assert engine.approve(request, now=NOW).allowed
    later = engine.decide(request, now=NOW + timedelta(minutes=5))

    _assert_not_allowed(later)
    assert later.status is PolicyDecisionStatus.DENIED
    assert later.reason is PolicyReason.CAPABILITY_EXPIRED
    assert later.missing_capabilities == ("network.egress",)

    expired_at_construction = PolicyEngine(
        {"network.egress": CapabilityGrant("network.egress", expires_at=NOW)},
        now=NOW,
    )
    boundary = expired_at_construction.decide(
        _egress_request("request-expiry-boundary"),
        now=NOW,
    )
    _assert_not_allowed(boundary)
    assert boundary.reason is PolicyReason.CAPABILITY_EXPIRED


def test_stale_state_digest_invalidates_an_existing_approval() -> None:
    engine = PolicyEngine({"network.egress"}, now=NOW)
    request = _egress_request("request-stale")

    assert engine.approve(request, now=NOW).allowed

    changed = replace(request, state_digest="state-2")
    invalidated = engine.decide(changed, now=NOW)

    _assert_not_allowed(invalidated)
    assert invalidated.status is PolicyDecisionStatus.INVALIDATED
    assert invalidated.reason is PolicyReason.STALE_APPROVAL
    record = engine.get_approval(request.request_id)
    assert isinstance(record, ApprovalRecord)
    assert record.status.value == "invalidated"
    assert record.state_digest == request.state_digest
    assert record.action_digest == request.action_digest

    reapproved = engine.approve(changed, now=NOW)
    assert reapproved.allowed
    assert engine.decide(changed, now=NOW).allowed


def test_approval_with_mismatched_digests_is_invalidated() -> None:
    engine = PolicyEngine({"network.egress"}, now=NOW)
    request = _egress_request("request-mismatched")
    engine.request_approval(request, now=NOW)

    decision = engine.approve(
        request,
        state_digest="state-other",
        action_digest=request.action_digest,
        now=NOW,
    )

    _assert_not_allowed(decision)
    assert decision.status is PolicyDecisionStatus.INVALIDATED
    assert decision.reason is PolicyReason.STALE_APPROVAL
    record = engine.get_approval(request.request_id)
    assert isinstance(record, ApprovalRecord)
    assert record.status.value == "invalidated"

    other_action = _egress_request("request-mismatched-action")
    engine.request_approval(other_action, now=NOW)
    action_mismatch = engine.approve(
        other_action,
        state_digest=other_action.state_digest,
        action_digest="action-other",
        now=NOW,
    )
    assert action_mismatch.status is PolicyDecisionStatus.INVALIDATED


def test_explicit_invalidation_requires_matching_digests() -> None:
    engine = PolicyEngine({"network.egress"}, now=NOW)
    request = _egress_request("request-invalidated")
    assert engine.approve(request, now=NOW).allowed

    decision = engine.invalidate(
        request,
        state_digest=request.state_digest,
        action_digest=request.action_digest,
        actor_id="reviewer-1",
        now=NOW,
    )

    _assert_not_allowed(decision)
    assert decision.status is PolicyDecisionStatus.INVALIDATED
    assert decision.reason is PolicyReason.STALE_APPROVAL
    assert decision.actor_id == "reviewer-1"
    _assert_not_allowed(engine.decide(request, now=NOW))

    with pytest.raises(PolicyTransitionError):
        engine.invalidate(
            request,
            state_digest="state-other",
            action_digest=request.action_digest,
            now=NOW,
        )


def test_rejection_and_abort_are_terminal_and_block_later_approval() -> None:
    engine = PolicyEngine({"network.egress"}, now=NOW)
    rejected_request = _egress_request("request-rejected")

    rejected = engine.reject(rejected_request, actor_id="reviewer-1", now=NOW)

    assert rejected.status is PolicyDecisionStatus.REJECTED
    assert rejected.reason is PolicyReason.REJECTED
    _assert_not_allowed(rejected)
    assert rejected.actor_id == "reviewer-1"
    repeat = engine.decide(rejected_request, now=NOW)
    assert repeat.status is PolicyDecisionStatus.REJECTED
    _assert_not_allowed(repeat)
    with pytest.raises(PolicyTransitionError):
        engine.approve(rejected_request, now=NOW)

    aborted_request = _egress_request("request-aborted")
    aborted = engine.abort(aborted_request, actor_id="reviewer-1", now=NOW)

    assert aborted.status is PolicyDecisionStatus.ABORTED
    assert aborted.reason is PolicyReason.ABORTED
    _assert_not_allowed(aborted)
    assert engine.decide(aborted_request, now=NOW).status is PolicyDecisionStatus.ABORTED
    with pytest.raises(PolicyTransitionError):
        engine.abort(aborted_request, now=NOW)


def test_a_rejection_survives_invalidation_and_stale_digests() -> None:
    engine = PolicyEngine({"network.egress"}, now=NOW)
    request = _egress_request("request-reject-then-invalidate")
    engine.reject(request, actor_id="reviewer-1", now=NOW)

    invalidated = engine.invalidate(request, now=NOW)
    assert invalidated.status is PolicyDecisionStatus.REJECTED
    stale = engine.approve(request, state_digest="state-bogus", action_digest="action-1", now=NOW)
    assert stale.status is PolicyDecisionStatus.REJECTED

    record = engine.request_approval(request, now=NOW)
    assert record.status.value == "rejected"
    with pytest.raises(PolicyTransitionError):
        engine.approve(request, now=NOW)
    assert engine.decide(request, now=NOW).status is PolicyDecisionStatus.REJECTED


def test_eviction_forgets_approvals_before_rejections() -> None:
    engine = PolicyEngine({"network.egress"}, now=NOW, max_tracked_requests=2)
    rejected = _egress_request("request-evicted-rejection")
    engine.reject(rejected, now=NOW)
    for index in range(3):
        engine.approve(_egress_request(f"request-evicted-approved-{index}"), now=NOW)

    assert engine.decide(rejected, now=NOW).status is PolicyDecisionStatus.REJECTED
    with pytest.raises(PolicyTransitionError):
        engine.approve(rejected, now=NOW)


def test_a_pending_approval_is_reported_as_pending() -> None:
    engine = PolicyEngine({"network.egress"}, now=NOW)
    request = _egress_request("request-pending-reason")
    engine.request_approval(request, now=NOW)
    decision = engine.decide(request, now=NOW)
    assert decision.status is PolicyDecisionStatus.PENDING
    assert decision.reason is PolicyReason.APPROVAL_REQUIRED


def test_revoked_grants_deny_previously_allowed_requests() -> None:
    engine = PolicyEngine({"network.egress"}, now=NOW)
    request = _egress_request("request-revoked")
    assert engine.approve(request, now=NOW).allowed

    assert engine.revoke("network.egress") is True
    assert engine.revoke("network.egress") is False

    denied = engine.decide(request, now=NOW)
    _assert_not_allowed(denied)
    assert denied.reason is PolicyReason.CAPABILITY_NOT_GRANTED


def test_scoped_grants_deny_requests_outside_their_scope() -> None:
    engine = PolicyEngine(
        {"read": CapabilityGrant("read", scope="project-1")},
        now=NOW,
    )

    unscoped = engine.decide(_request("request-unscoped"), now=NOW)
    _assert_not_allowed(unscoped)
    assert unscoped.reason is PolicyReason.SCOPE_MISMATCH
    assert unscoped.missing_capabilities == ("read",)

    other_scope = engine.decide(
        _request("request-other-scope", scope="project-2"),
        now=NOW,
    )
    _assert_not_allowed(other_scope)
    assert other_scope.reason is PolicyReason.SCOPE_MISMATCH

    matching = engine.decide(
        _request("request-matching-scope", scope="project-1"),
        now=NOW,
    )
    assert matching.allowed
    assert matching.status is PolicyDecisionStatus.ALLOWED


def test_scoped_side_effect_grants_need_approval_within_scope() -> None:
    engine = PolicyEngine(
        {"local.write": CapabilityGrant("local.write", scope="project-1")},
        now=NOW,
    )
    request = _request(
        "request-scoped-write",
        capabilities=("local.write",),
        side_effect_level=SideEffectLevel.LOCAL_WRITE,
        scope="project-1",
    )

    pending = engine.decide(request, now=NOW)
    _assert_not_allowed(pending)
    assert pending.status is PolicyDecisionStatus.PENDING

    allowed = engine.approve(request, actor_id="reviewer-1", now=NOW)
    assert allowed.allowed
    assert allowed.actor_id == "reviewer-1"

    unscoped = replace(request, request_id="request-scoped-write-unscoped", scope=None)
    _assert_not_allowed(engine.decide(unscoped, now=NOW))


def test_wildcard_grants_and_naive_timestamps_are_rejected() -> None:
    with pytest.raises(PolicyRequestError):
        CapabilityGrant("*")

    engine = PolicyEngine({"read"}, now=NOW)
    naive = datetime(2026, 1, 2, 3, 4, 5)
    with pytest.raises(PolicyRequestError):
        engine.decide(_request("request-naive-now"), now=naive)

    with pytest.raises(PolicyRequestError):
        engine.decide(
            replace(_request("request-unsafe-id"), request_id="request id"),
            now=NOW,
        )


def test_every_checkpoint_type_requires_approval_without_a_side_effect_level() -> None:
    engine = PolicyEngine({"read"}, now=NOW)

    for index, checkpoint in enumerate(ApprovalKind):
        request = _request(
            f"request-checkpoint-{index}",
            capabilities=("read",),
            side_effect_level=SideEffectLevel.READ,
            checkpoint_type=checkpoint,
        )
        assert request.requires_approval

        pending = engine.decide(request, now=NOW)
        _assert_not_allowed(pending)
        assert pending.status is PolicyDecisionStatus.PENDING
        assert pending.reason is PolicyReason.APPROVAL_REQUIRED
        assert pending.requires_approval

        approved = engine.approve(request, actor_id="reviewer-1", now=NOW)
        assert approved.allowed
        assert approved.actor_id == "reviewer-1"
        assert engine.decide(request, now=NOW).allowed

    ungranted = _request(
        "request-checkpoint-ungranted",
        capabilities=("simulate",),
        checkpoint_type=ApprovalKind.TOOL_CLASS_CHANGE,
    )
    denied = engine.decide(ungranted, now=NOW)
    _assert_not_allowed(denied)
    assert denied.reason is PolicyReason.CAPABILITY_NOT_GRANTED


def test_first_tool_class_use_requires_approval_and_repeats_stay_inside_the_class() -> None:
    engine = PolicyEngine({"read", "simulate"}, now=NOW)
    first = _request(
        "request-tool-class-first",
        capabilities=("simulate",),
        checkpoint_type=ApprovalKind.TOOL_CLASS_CHANGE,
    )
    assert first.tool_class_change

    pending = engine.decide(first, now=NOW)
    _assert_not_allowed(pending)
    assert pending.status is PolicyDecisionStatus.PENDING
    assert list(engine.approved_tool_classes) == []

    assert engine.approve(first, actor_id="reviewer-1", now=NOW).allowed
    assert engine.approved_tool_classes == ("action-1",)

    repeat = _request(
        "request-tool-class-repeat",
        capabilities=("simulate",),
        checkpoint_type=ApprovalKind.TOOL_CLASS_CHANGE,
    )
    assert engine.decide(repeat, now=NOW).allowed

    wider = _request(
        "request-tool-class-wider",
        capabilities=("simulate", "read"),
        checkpoint_type=ApprovalKind.TOOL_CLASS_CHANGE,
    )
    broadened = engine.decide(wider, now=NOW)
    _assert_not_allowed(broadened)
    assert broadened.status is PolicyDecisionStatus.PENDING

    renamed = _request(
        "request-tool-class-renamed",
        action_digest="action-2",
        capabilities=("simulate",),
        checkpoint_type=ApprovalKind.TOOL_CLASS_CHANGE,
    )
    assert engine.decide(renamed, now=NOW).status is PolicyDecisionStatus.PENDING

    assert engine.forget_tool_class("action-1") is True
    assert engine.forget_tool_class("action-1") is False
    assert list(engine.approved_tool_classes) == []
    assert engine.decide(repeat, now=NOW).status is PolicyDecisionStatus.PENDING

    rebound = replace(first, action_digest="action-2")
    assert engine.decide(rebound, now=NOW).status is PolicyDecisionStatus.INVALIDATED


def test_an_approval_cannot_be_replayed_onto_a_different_action() -> None:
    engine = PolicyEngine({"network.egress"}, now=NOW)
    request = _egress_request("request-replay")
    assert engine.approve(request, actor_id="reviewer-1", now=NOW).allowed

    captured = engine.get_approval(
        request.request_id,
        state_digest=request.state_digest,
        action_digest=request.action_digest,
    )
    assert isinstance(captured, ApprovalRecord)
    assert captured.status.value == "approved"

    rebound = replace(request, action_digest="action-2")
    replayed = engine.approve(rebound, actor_id="attacker", now=NOW)
    _assert_not_allowed(replayed)
    assert replayed.status is PolicyDecisionStatus.INVALIDATED
    assert replayed.reason is PolicyReason.STALE_APPROVAL

    _assert_not_allowed(engine.decide(rebound, now=NOW))
    _assert_not_allowed(engine.decide(request, now=NOW))
    with pytest.raises(PolicyTransitionError):
        engine.approve(
            request,
            state_digest=request.state_digest,
            action_digest=request.action_digest,
            now=NOW,
        )

    replayed_state = engine.decide(rebound, now=NOW)
    assert replayed_state.status is PolicyDecisionStatus.PENDING
    assert engine.approve(rebound, actor_id="reviewer-1", now=NOW).allowed


def test_policy_state_round_trip_restores_grants_and_decisions() -> None:
    engine = PolicyEngine({"network.egress"}, now=NOW)
    request = _egress_request("request-persisted")
    assert engine.approve(request, actor_id="reviewer-1", now=NOW).allowed
    rejected = _egress_request("request-persisted-rejected")
    assert engine.reject(rejected, actor_id="reviewer-1", reason="no", now=NOW).denied

    state = engine.export_state()
    assert json.loads(json.dumps(state)) == state

    restored = PolicyEngine.from_state(state, now=NOW)
    assert restored.grants == engine.grants
    assert restored.max_tracked_requests == engine.max_tracked_requests
    assert restored.decide(request, now=NOW).allowed
    assert restored.decide(rejected, now=NOW).status is PolicyDecisionStatus.REJECTED
    assert restored.get_approval(
        request.request_id,
        state_digest=request.state_digest,
        action_digest=request.action_digest,
    ) == engine.get_approval(request.request_id)
    with pytest.raises(PolicyTransitionError):
        restored.approve(
            request,
            state_digest=request.state_digest,
            action_digest=request.action_digest,
            now=NOW,
        )
    assert restored.decide(request, now=NOW).allowed

    rebound = replace(request, action_digest="action-2")
    _assert_not_allowed(restored.approve(rebound, actor_id="attacker", now=NOW))
    _assert_not_allowed(restored.decide(request, now=NOW))


def test_expired_grants_are_denied_after_policy_state_restore() -> None:
    engine = PolicyEngine(
        {
            "read": CapabilityGrant("read", expires_at=NOW + timedelta(seconds=30)),
            "network.egress": CapabilityGrant(
                "network.egress",
                scope="project-1",
                expires_at=NOW + timedelta(hours=1),
            ),
        },
        now=NOW,
    )
    read_request = _request("request-expiring-read")
    assert engine.decide(read_request, now=NOW).allowed

    restored = PolicyEngine.from_state(engine.export_state(), now=NOW)
    assert restored.decide(read_request, now=NOW).allowed

    expired = restored.decide(read_request, now=NOW + timedelta(minutes=5))
    _assert_not_allowed(expired)
    assert expired.status is PolicyDecisionStatus.DENIED
    assert expired.reason is PolicyReason.CAPABILITY_EXPIRED
    assert expired.missing_capabilities == ("read",)

    scoped = _egress_request("request-expiring-egress", scope="project-1")
    assert engine.approve(scoped, now=NOW).allowed
    scoped_restored = PolicyEngine.from_state(engine.export_state(), now=NOW)
    assert scoped_restored.decide(scoped, now=NOW).allowed
    after_expiry = scoped_restored.decide(scoped, now=NOW + timedelta(hours=2))
    _assert_not_allowed(after_expiry)
    assert after_expiry.reason is PolicyReason.CAPABILITY_EXPIRED


def test_policy_state_import_rejects_malformed_and_inconsistent_state() -> None:
    engine = PolicyEngine({"read"}, now=NOW)
    valid: dict[str, Any] = engine.export_state()
    approved_engine = PolicyEngine({"read"}, now=NOW)
    approved_engine.approve(
        _request("request-valid", checkpoint_type=ApprovalKind.PLAN_APPROVAL),
        now=NOW,
    )
    approved: dict[str, Any] = approved_engine.export_state()

    malformed: tuple[tuple[str, Any], ...] = (
        ("version", {**approved, "version": 99}),
        ("version-type", {**approved, "version": "1"}),
        ("unknown-key", {**approved, "unexpected": True}),
        ("budget-shape", {**approved, "grants": "none"}),
        ("grant-key", {**approved, "grants": [{"capability": "read", "scope": None, "extra": 1}]}),
        ("grant-wildcard", {**approved, "grants": [{"capability": "*"}]}),
        (
            "request-key",
            {
                **approved,
                "requests": [{**approved["requests"][0], "extra": 1}],
            },
        ),
        (
            "request-unsafe",
            {
                **approved,
                "requests": [{**approved["requests"][0], "request_id": "request id"}],
            },
        ),
        (
            "approval-status",
            {
                **approved,
                "approvals": [{**approved["approvals"][0], "status": "unknown"}],
            },
        ),
        (
            "approval-naive-time",
            {
                **approved,
                "approvals": [{**approved["approvals"][0], "decided_at": "2026-01-02T03:04:05"}],
            },
        ),
        ("orphan-approval", {**approved, "requests": []}),
        (
            "duplicate-approval",
            {**approved, "approvals": [approved["approvals"][0], approved["approvals"][0]]},
        ),
        (
            "tool-class",
            {
                **approved,
                "tool_classes": [{"action_digest": "action 1", "capabilities": ["read"]}],
            },
        ),
        (
            "tool-class-duplicate",
            {
                **approved,
                "tool_classes": [
                    {"action_digest": "action-1", "capabilities": ["read"], "scope": None},
                    {"action_digest": "action-1", "capabilities": ["read"], "scope": None},
                ],
            },
        ),
        ("not-a-mapping", []),
    )
    for label, payload in malformed:
        with pytest.raises(PolicyStateError):
            engine.import_state(payload)
        assert engine.decide(_request("request-valid"), now=NOW).allowed, label

    assert PolicyEngine.from_state(valid, now=NOW).grants == frozenset({"read"})
    with pytest.raises(ValueError):
        PolicyEngine({"read"}, now=NOW, max_tracked_requests=0)
    with pytest.raises(TypeError):
        PolicyEngine({"read"}, now=NOW, max_tracked_requests=True)


def test_policy_maps_are_bounded_and_evict_the_oldest_resolved_entries() -> None:
    limit = 4
    engine = PolicyEngine({"read"}, now=NOW, max_tracked_requests=limit)
    assert engine.max_tracked_requests == limit

    for index in range(limit * 5):
        assert engine.decide(_request(f"request-bounded-{index}"), now=NOW).allowed

    assert len(engine.tracked_request_ids) == limit
    assert engine.tracked_request_ids == tuple(
        f"request-bounded-{index}" for index in range(16, 20)
    )

    for index in range(limit * 5):
        engine.request_approval(
            _egress_request(f"request-bounded-egress-{index}"),
            now=NOW,
        )
    assert len(engine.approvals) == limit
    assert len(engine.tracked_request_ids) <= limit

    resolved = PolicyEngine({"network.egress"}, now=NOW, max_tracked_requests=limit)
    for index in range(limit * 3):
        engine_request = _egress_request(f"request-bounded-resolved-{index}")
        assert resolved.approve(engine_request, now=NOW).allowed
    assert len(resolved.approvals) == limit
    assert len(resolved.tracked_request_ids) == limit

    kept = _egress_request("request-bounded-kept")
    resolved.request_approval(kept, now=NOW)
    for index in range(limit * 3):
        resolved.decide(_egress_request(f"request-bounded-later-{index}"), now=NOW)
    assert len(resolved.approvals) <= limit
    assert len(resolved.tracked_request_ids) <= limit

    classes = PolicyEngine({"network.egress"}, now=NOW, max_tracked_requests=limit)
    for index in range(limit * 3):
        assert classes.approve(
            _request(
                f"request-bounded-class-{index}",
                action_digest=f"action-{index}",
                capabilities=("network.egress",),
                side_effect_level=SideEffectLevel.EGRESS,
                checkpoint_type=ApprovalKind.TOOL_CLASS_CHANGE,
            ),
            now=NOW,
        ).allowed
    assert len(classes.approved_tool_classes) <= limit
    assert len(classes.approvals) <= limit
    assert len(classes.tracked_request_ids) <= limit

    source = PolicyEngine({"network.egress"}, now=NOW)
    assert source.approve(_egress_request("request-bounded-shared"), now=NOW).allowed
    exported: dict[str, Any] = source.export_state()
    approval = exported["approvals"][0]
    exported["max_tracked_requests"] = limit
    exported["approvals"] = [
        {**approval, "action_digest": f"action-{index}"} for index in range(limit * 3)
    ]
    restored = PolicyEngine.from_state(exported, now=NOW)
    assert len(restored.approvals) <= limit
    assert len(restored.tracked_request_ids) <= limit
    assert restored.max_tracked_requests == limit
