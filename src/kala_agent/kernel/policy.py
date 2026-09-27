from __future__ import annotations

from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass
from datetime import UTC, datetime
from enum import StrEnum
from threading import RLock
from typing import Final, TypeAlias, assert_never, cast

from kala_agent.contracts import ApprovalKind, ApprovalStatus, SideEffectLevel

ApprovalKey: TypeAlias = tuple[str, str, str]
ToolClassKey: TypeAlias = tuple[str, str | None]

POLICY_STATE_VERSION: Final[int] = 1
DEFAULT_MAX_TRACKED_REQUESTS: Final[int] = 512
MAX_NOTE_LENGTH: Final[int] = 512


class PolicyError(RuntimeError):
    pass


class PolicyTransitionError(PolicyError, ValueError):
    pass


class PolicyRequestError(PolicyError, ValueError):
    pass


class PolicyStateError(PolicyError, ValueError):
    pass


class PolicyDecisionStatus(StrEnum):
    ALLOWED = "allowed"
    DENIED = "denied"
    PENDING = "pending"
    INVALIDATED = "invalidated"
    REJECTED = "rejected"
    ABORTED = "aborted"


class PolicyReason(StrEnum):
    ALLOWED = "allowed"
    APPROVAL_REQUIRED = "approval_required"
    CAPABILITY_NOT_GRANTED = "capability_not_granted"
    CAPABILITY_EXPIRED = "capability_expired"
    SCOPE_MISMATCH = "scope_mismatch"
    STALE_APPROVAL = "stale_approval"
    REJECTED = "rejected"
    ABORTED = "aborted"
    INVALID_REQUEST = "invalid_request"


_APPROVAL_REQUIRED_EFFECTS = frozenset(
    {
        SideEffectLevel.LOCAL_WRITE,
        SideEffectLevel.NETWORK,
        SideEffectLevel.EGRESS,
    }
)

_TERMINAL_APPROVAL_STATUSES = frozenset(
    {
        ApprovalStatus.REJECTED,
        ApprovalStatus.ABORTED,
        ApprovalStatus.INVALIDATED,
        ApprovalStatus.EXPIRED,
    }
)
_REFUSED_APPROVAL_STATUSES = frozenset({ApprovalStatus.REJECTED, ApprovalStatus.ABORTED})


def _unapproved_outcome(status: ApprovalStatus) -> tuple[PolicyDecisionStatus, PolicyReason]:
    match status:
        case ApprovalStatus.PENDING:
            return PolicyDecisionStatus.PENDING, PolicyReason.APPROVAL_REQUIRED
        case ApprovalStatus.REJECTED:
            return PolicyDecisionStatus.REJECTED, PolicyReason.REJECTED
        case ApprovalStatus.ABORTED:
            return PolicyDecisionStatus.ABORTED, PolicyReason.ABORTED
        case ApprovalStatus.INVALIDATED | ApprovalStatus.EXPIRED:
            return PolicyDecisionStatus.INVALIDATED, PolicyReason.STALE_APPROVAL
        case ApprovalStatus.APPROVED:
            return PolicyDecisionStatus.ALLOWED, PolicyReason.ALLOWED
        case _:
            assert_never(status)


_REQUEST_STATE_KEYS = frozenset(
    {
        "request_id",
        "state_digest",
        "action_digest",
        "requested_capabilities",
        "side_effect_level",
        "destructive",
        "egress",
        "scope",
        "checkpoint_type",
    }
)

_APPROVAL_STATE_KEYS = frozenset(
    {
        "request_id",
        "state_digest",
        "action_digest",
        "status",
        "actor_id",
        "decided_at",
        "reason",
    }
)

_GRANT_STATE_KEYS = frozenset({"capability", "scope", "expires_at"})

_TOOL_CLASS_STATE_KEYS = frozenset({"action_digest", "capabilities", "scope"})

_POLICY_STATE_KEYS = frozenset(
    {
        "version",
        "grants",
        "requests",
        "approvals",
        "tool_classes",
        "max_tracked_requests",
    }
)


def _safe_identifier(value: object, field_name: str) -> str:
    if not isinstance(value, str):
        raise PolicyRequestError(f"{field_name} must be a string")
    normalized = value.strip()
    if not normalized or len(normalized) > 256:
        raise PolicyRequestError(f"{field_name} must be nonempty and bounded")
    if any(character.isspace() or ord(character) < 32 for character in normalized):
        raise PolicyRequestError(f"{field_name} contains unsafe characters")
    return normalized


def _digest(value: object, field_name: str) -> str:
    normalized = _safe_identifier(value, field_name)
    if len(normalized) > 256:
        raise PolicyRequestError(f"{field_name} is too long")
    return normalized


def _note(value: object, field_name: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise PolicyRequestError(f"{field_name} must be a string")
    normalized = value.strip()
    if not normalized or len(normalized) > MAX_NOTE_LENGTH:
        raise PolicyRequestError(f"{field_name} must be nonempty and bounded")
    if any(ord(character) < 32 for character in normalized):
        raise PolicyRequestError(f"{field_name} contains unsafe characters")
    return normalized


def _aware(value: datetime | None, field_name: str) -> datetime | None:
    if value is None:
        return None
    if value.tzinfo is None or value.utcoffset() is None:
        raise PolicyRequestError(f"{field_name} must include a timezone")
    return value.astimezone(UTC)


def _capabilities(values: Iterable[str]) -> tuple[str, ...]:
    result: set[str] = set()
    for value in values:
        result.add(_safe_identifier(value, "capability"))
    return tuple(sorted(result))


def _supplied_action_key(
    request: PolicyRequest,
    state_digest: str | None,
    action_digest: str | None,
) -> tuple[str, str]:
    supplied_state = (
        request.state_digest if state_digest is None else _digest(state_digest, "state_digest")
    )
    supplied_action = (
        request.action_digest if action_digest is None else _digest(action_digest, "action_digest")
    )
    return supplied_state, supplied_action


def _isoformat(value: datetime | None) -> str | None:
    return None if value is None else value.isoformat()


def _state_error(message: str) -> PolicyStateError:
    return PolicyStateError(f"policy state is invalid: {message}")


def _state_mapping(value: object, field_name: str) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise _state_error(f"{field_name} must be a mapping")
    for key in value:
        if not isinstance(key, str):
            raise _state_error(f"{field_name} keys must be strings")
    return value


def _state_sequence(value: object, field_name: str) -> Sequence[object]:
    if isinstance(value, (str, bytes, bytearray)) or not isinstance(value, Sequence):
        raise _state_error(f"{field_name} must be a sequence")
    return value


def _state_str(value: object, field_name: str) -> str:
    if not isinstance(value, str):
        raise _state_error(f"{field_name} must be a string")
    return value


def _state_bool(value: object, field_name: str) -> bool:
    if not isinstance(value, bool):
        raise _state_error(f"{field_name} must be a boolean")
    return value


def _state_int(value: object, field_name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise _state_error(f"{field_name} must be an integer")
    return value


def _state_positive_int(value: object, field_name: str) -> int:
    parsed = _state_int(value, field_name)
    if parsed < 1:
        raise _state_error(f"{field_name} must be positive")
    return parsed


def _state_opt_str(value: object, field_name: str) -> str | None:
    if value is None:
        return None
    return _state_str(value, field_name)


def _state_str_tuple(value: object, field_name: str) -> tuple[str, ...]:
    sequence = _state_sequence(value, field_name)
    return tuple(_state_str(item, field_name) for item in sequence)


def _state_opt_datetime(value: object, field_name: str) -> datetime | None:
    if value is None:
        return None
    return _state_datetime(value, field_name)


def _state_datetime(value: object, field_name: str) -> datetime:
    text = _state_str(value, field_name)
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError as error:
        raise _state_error(f"{field_name} must be an ISO timestamp") from error
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        raise _state_error(f"{field_name} must include a timezone")
    return parsed.astimezone(UTC)


def _require_keys(payload: Mapping[str, object], allowed: frozenset[str], field_name: str) -> None:
    unexpected = sorted(set(payload) - allowed)
    if unexpected:
        raise _state_error(f"{field_name} has unexpected keys: {', '.join(unexpected)}")


@dataclass(frozen=True, slots=True)
class CapabilityGrant:
    capability: str
    scope: str | None = None
    expires_at: datetime | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "capability", _safe_identifier(self.capability, "capability"))
        if self.capability == "*":
            raise PolicyRequestError("wildcard capability grants are not permitted")
        if self.scope is not None:
            object.__setattr__(self, "scope", _safe_identifier(self.scope, "scope"))
        object.__setattr__(self, "expires_at", _aware(self.expires_at, "expires_at"))

    def applies_to(self, capability: str, scope: str | None, now: datetime) -> bool:
        if self.capability != capability:
            return False
        if self.expires_at is not None and self.expires_at <= now:
            return False
        return self.scope is None or self.scope == scope

    def as_dict(self) -> dict[str, object]:
        return {
            "capability": self.capability,
            "scope": self.scope,
            "expires_at": _isoformat(self.expires_at),
        }


@dataclass(frozen=True, slots=True)
class PolicyRequest:
    request_id: str
    state_digest: str
    action_digest: str
    requested_capabilities: tuple[str, ...] = ()
    side_effect_level: SideEffectLevel = SideEffectLevel.NONE
    destructive: bool = False
    egress: bool = False
    scope: str | None = None
    checkpoint_type: ApprovalKind | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "request_id", _safe_identifier(self.request_id, "request_id"))
        object.__setattr__(self, "state_digest", _digest(self.state_digest, "state_digest"))
        object.__setattr__(self, "action_digest", _digest(self.action_digest, "action_digest"))
        object.__setattr__(
            self,
            "requested_capabilities",
            _capabilities(self.requested_capabilities),
        )
        try:
            object.__setattr__(
                self,
                "side_effect_level",
                SideEffectLevel(self.side_effect_level),
            )
        except (TypeError, ValueError) as error:
            raise PolicyRequestError("side_effect_level is invalid") from error
        if not isinstance(self.destructive, bool):
            raise PolicyRequestError("destructive must be a boolean")
        if not isinstance(self.egress, bool):
            raise PolicyRequestError("egress must be a boolean")
        if self.side_effect_level is SideEffectLevel.EGRESS:
            object.__setattr__(self, "egress", True)
        if self.scope is not None:
            object.__setattr__(self, "scope", _safe_identifier(self.scope, "scope"))
        if self.checkpoint_type is not None:
            try:
                object.__setattr__(
                    self,
                    "checkpoint_type",
                    ApprovalKind(self.checkpoint_type),
                )
            except (TypeError, ValueError) as error:
                raise PolicyRequestError("checkpoint_type is invalid") from error

    @property
    def requires_approval(self) -> bool:
        return (
            self.checkpoint_type is not None
            or self.destructive
            or self.egress
            or self.side_effect_level in _APPROVAL_REQUIRED_EFFECTS
        )

    @property
    def tool_class_change(self) -> bool:
        return self.checkpoint_type is ApprovalKind.TOOL_CLASS_CHANGE

    @property
    def capabilities(self) -> tuple[str, ...]:
        return self.requested_capabilities

    @property
    def side_effect(self) -> SideEffectLevel:
        return self.side_effect_level

    def as_dict(self) -> dict[str, object]:
        return {
            "request_id": self.request_id,
            "state_digest": self.state_digest,
            "action_digest": self.action_digest,
            "requested_capabilities": list(self.requested_capabilities),
            "side_effect_level": self.side_effect_level.value,
            "destructive": self.destructive,
            "egress": self.egress,
            "scope": self.scope,
            "checkpoint_type": None if self.checkpoint_type is None else self.checkpoint_type.value,
        }


@dataclass(frozen=True, slots=True)
class PolicyDecision:
    request_id: str
    status: PolicyDecisionStatus
    reason: PolicyReason
    state_digest: str
    action_digest: str
    allowed: bool
    requires_approval: bool
    missing_capabilities: tuple[str, ...] = ()
    actor_id: str | None = None
    decided_at: datetime | None = None

    @property
    def approved(self) -> bool:
        return self.allowed

    @property
    def denied(self) -> bool:
        return not self.allowed

    def __bool__(self) -> bool:
        return self.allowed

    def as_dict(self) -> dict[str, object]:
        return {
            "request_id": self.request_id,
            "status": self.status.value,
            "reason": self.reason.value,
            "state_digest": self.state_digest,
            "action_digest": self.action_digest,
            "allowed": self.allowed,
            "requires_approval": self.requires_approval,
            "missing_capabilities": list(self.missing_capabilities),
            "actor_id": self.actor_id,
            "decided_at": None if self.decided_at is None else self.decided_at.isoformat(),
        }


@dataclass(frozen=True, slots=True)
class ApprovalRecord:
    request_id: str
    state_digest: str
    action_digest: str
    status: ApprovalStatus
    actor_id: str | None = None
    decided_at: datetime | None = None
    reason: str | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "request_id", _safe_identifier(self.request_id, "request_id"))
        object.__setattr__(self, "state_digest", _digest(self.state_digest, "state_digest"))
        object.__setattr__(self, "action_digest", _digest(self.action_digest, "action_digest"))
        try:
            object.__setattr__(self, "status", ApprovalStatus(self.status))
        except (TypeError, ValueError) as error:
            raise PolicyRequestError("approval status is invalid") from error
        if self.actor_id is not None:
            object.__setattr__(self, "actor_id", _safe_identifier(self.actor_id, "actor_id"))
        object.__setattr__(self, "decided_at", _aware(self.decided_at, "decided_at"))
        object.__setattr__(self, "reason", _note(self.reason, "reason"))
        if self.status is ApprovalStatus.PENDING:
            if self.actor_id is not None or self.decided_at is not None or self.reason is not None:
                raise PolicyRequestError("pending approvals cannot contain a decision")
        elif self.actor_id is None or self.decided_at is None:
            raise PolicyRequestError("resolved approvals require actor and decision time")

    @property
    def action_key(self) -> tuple[str, str]:
        return self.state_digest, self.action_digest

    @property
    def key(self) -> ApprovalKey:
        return self.request_id, self.state_digest, self.action_digest

    @property
    def resolved(self) -> bool:
        return self.status is not ApprovalStatus.PENDING

    def as_dict(self) -> dict[str, object]:
        return {
            "request_id": self.request_id,
            "state_digest": self.state_digest,
            "action_digest": self.action_digest,
            "status": self.status.value,
            "actor_id": self.actor_id,
            "decided_at": _isoformat(self.decided_at),
            "reason": self.reason,
        }


PolicyApprovalRequest = PolicyRequest
PolicyApproval = ApprovalRecord
ApprovalRequest = PolicyRequest
PolicyRequestModel = PolicyRequest
DecisionStatus = PolicyDecisionStatus
PolicyStatus = PolicyDecisionStatus
ApprovalDecision = ApprovalRecord


def _request_from_state(value: object) -> PolicyRequest:
    payload = _state_mapping(value, "request")
    _require_keys(payload, _REQUEST_STATE_KEYS, "request")
    checkpoint = _state_opt_str(payload.get("checkpoint_type"), "request.checkpoint_type")
    try:
        return PolicyRequest(
            request_id=_state_str(payload["request_id"], "request.request_id"),
            state_digest=_state_str(payload["state_digest"], "request.state_digest"),
            action_digest=_state_str(payload["action_digest"], "request.action_digest"),
            requested_capabilities=_state_str_tuple(
                payload.get("requested_capabilities", ()),
                "request.requested_capabilities",
            ),
            side_effect_level=cast(
                SideEffectLevel,
                _state_str(
                    payload.get("side_effect_level", SideEffectLevel.NONE.value),
                    "request.side_effect_level",
                ),
            ),
            destructive=_state_bool(payload.get("destructive", False), "request.destructive"),
            egress=_state_bool(payload.get("egress", False), "request.egress"),
            scope=_state_opt_str(payload.get("scope"), "request.scope"),
            checkpoint_type=cast(ApprovalKind, checkpoint) if checkpoint is not None else None,
        )
    except PolicyRequestError as error:
        raise _state_error(f"request entry is invalid: {error}") from error


def _approval_from_state(value: object) -> ApprovalRecord:
    payload = _state_mapping(value, "approval")
    _require_keys(payload, _APPROVAL_STATE_KEYS, "approval")
    try:
        return ApprovalRecord(
            request_id=_state_str(payload["request_id"], "approval.request_id"),
            state_digest=_state_str(payload["state_digest"], "approval.state_digest"),
            action_digest=_state_str(payload["action_digest"], "approval.action_digest"),
            status=cast(
                ApprovalStatus,
                _state_str(payload.get("status", ApprovalStatus.PENDING.value), "approval.status"),
            ),
            actor_id=_state_opt_str(payload.get("actor_id"), "approval.actor_id"),
            decided_at=_state_opt_datetime(payload.get("decided_at"), "approval.decided_at"),
            reason=_state_opt_str(payload.get("reason"), "approval.reason"),
        )
    except PolicyRequestError as error:
        raise _state_error(f"approval entry is invalid: {error}") from error


def _grant_from_state(value: object) -> CapabilityGrant:
    payload = _state_mapping(value, "grant")
    _require_keys(payload, _GRANT_STATE_KEYS, "grant")
    try:
        return CapabilityGrant(
            capability=_state_str(payload.get("capability"), "grant.capability"),
            scope=_state_opt_str(payload.get("scope"), "grant.scope"),
            expires_at=_state_opt_datetime(payload.get("expires_at"), "grant.expires_at"),
        )
    except PolicyRequestError as error:
        raise _state_error(f"grant entry is invalid: {error}") from error


def _tool_class_from_state(value: object) -> tuple[ToolClassKey, tuple[str, ...]]:
    payload = _state_mapping(value, "tool_class")
    _require_keys(payload, _TOOL_CLASS_STATE_KEYS, "tool_class")
    action_digest = _state_str(payload.get("action_digest"), "tool_class.action_digest")
    capabilities = _state_str_tuple(payload.get("capabilities", ()), "tool_class.capabilities")
    scope = _state_opt_str(payload.get("scope"), "tool_class.scope")
    try:
        key: ToolClassKey = (
            _digest(action_digest, "action_digest"),
            None if scope is None else _safe_identifier(scope, "scope"),
        )
    except PolicyRequestError as error:
        raise _state_error(f"tool class entry is invalid: {error}") from error
    return key, _capabilities(capabilities)


class PolicyEngine:
    def __init__(
        self,
        grants: Iterable[str | CapabilityGrant] | Mapping[str, CapabilityGrant | str | None] = (),
        *,
        capability_grants: Iterable[str | CapabilityGrant] | None = None,
        granted_capabilities: Iterable[str | CapabilityGrant] | None = None,
        capabilities: Iterable[str | CapabilityGrant] | None = None,
        allowed_capabilities: Iterable[str | CapabilityGrant] | None = None,
        now: datetime | None = None,
        max_tracked_requests: int = DEFAULT_MAX_TRACKED_REQUESTS,
    ) -> None:
        supplied_grants = [
            value
            for value in (
                capability_grants,
                granted_capabilities,
                capabilities,
                allowed_capabilities,
            )
            if value is not None
        ]
        if len(supplied_grants) > 1:
            raise TypeError("provide only one capability grant collection")
        if supplied_grants:
            grants = supplied_grants[0]
        if isinstance(max_tracked_requests, bool) or not isinstance(max_tracked_requests, int):
            raise TypeError("max_tracked_requests must be an integer")
        if max_tracked_requests < 1:
            raise ValueError("max_tracked_requests must be positive")
        self._grants: dict[str, tuple[CapabilityGrant, ...]] = {}
        self._requests: dict[str, PolicyRequest] = {}
        self._approvals: dict[ApprovalKey, ApprovalRecord] = {}
        self._tool_classes: dict[ToolClassKey, tuple[str, ...]] = {}
        self._max_tracked_requests = max_tracked_requests
        self._lock = RLock()
        self._clock = _aware(now, "now")
        for item in self._grant_items(grants):
            existing = self._grants.get(item.capability, ())
            if item not in existing:
                self._grants[item.capability] = (*existing, item)

    @property
    def clock(self) -> datetime | None:
        with self._lock:
            return self._clock

    @property
    def grants(self) -> frozenset[str]:
        with self._lock:
            return frozenset(self._grants)

    @property
    def capability_grants(self) -> frozenset[str]:
        return self.grants

    @property
    def max_tracked_requests(self) -> int:
        with self._lock:
            return self._max_tracked_requests

    @property
    def tracked_request_ids(self) -> tuple[str, ...]:
        with self._lock:
            return tuple(self._requests)

    @property
    def approvals(self) -> tuple[ApprovalRecord, ...]:
        with self._lock:
            return tuple(self._approvals.values())

    @property
    def approved_tool_classes(self) -> tuple[str, ...]:
        with self._lock:
            return tuple(sorted({action_digest for action_digest, _ in self._tool_classes}))

    def grant(self, capability: str | CapabilityGrant) -> CapabilityGrant:
        item = (
            capability if isinstance(capability, CapabilityGrant) else CapabilityGrant(capability)
        )
        with self._lock:
            existing = self._grants.get(item.capability, ())
            if item not in existing:
                self._grants[item.capability] = (*existing, item)
            return item

    grant_capability = grant

    def revoke(self, capability: str) -> bool:
        normalized = _safe_identifier(capability, "capability")
        with self._lock:
            return self._grants.pop(normalized, None) is not None

    revoke_capability = revoke

    def decide(self, request: PolicyRequest, *, now: datetime | None = None) -> PolicyDecision:
        normalized = self._request(request)
        current_time = self._resolve_now(now)
        with self._lock:
            self._track_request_locked(normalized)
            if normalized.requires_approval and not normalized.requested_capabilities:
                self._prune_locked()
                return self._decision(
                    normalized,
                    PolicyDecisionStatus.DENIED,
                    PolicyReason.CAPABILITY_NOT_GRANTED,
                    allowed=False,
                )
            missing, expired, scope_mismatch = self._capability_state(normalized, current_time)
            if missing or expired or scope_mismatch:
                if missing:
                    reason = PolicyReason.CAPABILITY_NOT_GRANTED
                elif expired:
                    reason = PolicyReason.CAPABILITY_EXPIRED
                else:
                    reason = PolicyReason.SCOPE_MISMATCH
                self._prune_locked()
                return self._decision(
                    normalized,
                    PolicyDecisionStatus.DENIED,
                    reason,
                    allowed=False,
                    missing_capabilities=tuple(sorted(expired or scope_mismatch or missing)),
                )
            if not self._requires_approval_locked(normalized):
                self._remember_tool_class_locked(normalized)
                self._prune_locked()
                return self._decision(
                    normalized,
                    PolicyDecisionStatus.ALLOWED,
                    PolicyReason.ALLOWED,
                    allowed=True,
                )
            key = _approval_key(normalized)
            approval = self._approvals.get(key)
            if approval is None:
                conflict = self._conflicting_approval_locked(normalized)
                if conflict is not None:
                    return self._invalidate_record(
                        normalized,
                        conflict,
                        conflict.actor_id,
                        conflict.decided_at,
                    )
                self._prune_locked()
                return self._decision(
                    normalized,
                    PolicyDecisionStatus.PENDING,
                    PolicyReason.APPROVAL_REQUIRED,
                    allowed=False,
                )
            if approval.status is not ApprovalStatus.APPROVED:
                status, reason = _unapproved_outcome(approval.status)
                self._prune_locked()
                return self._decision(
                    normalized,
                    status,
                    reason,
                    allowed=False,
                    actor_id=approval.actor_id,
                )
            if approval.action_key != (normalized.state_digest, normalized.action_digest):
                return self._invalidate_record(
                    normalized,
                    approval,
                    approval.actor_id,
                    approval.decided_at,
                )
            self._remember_tool_class_locked(normalized)
            self._prune_locked()
            return self._decision(
                normalized,
                PolicyDecisionStatus.ALLOWED,
                PolicyReason.ALLOWED,
                allowed=True,
                actor_id=approval.actor_id,
                decided_at=approval.decided_at,
            )

    evaluate = decide
    check = decide
    authorize = decide
    request = decide
    evaluate_request = decide

    def request_approval(
        self,
        request: PolicyRequest,
        *,
        now: datetime | None = None,
    ) -> ApprovalRecord:
        normalized = self._request(request)
        with self._lock:
            self._track_request_locked(normalized)
            self._resolve_now(now)
            current = self._approvals.get(_approval_key(normalized))
            if current is not None and current.status is not ApprovalStatus.INVALIDATED:
                self._prune_locked()
                return current
            record = self._store_pending_locked(
                normalized.request_id,
                normalized.state_digest,
                normalized.action_digest,
            )
            self._prune_locked()
            return record

    create_approval = request_approval

    def approve(
        self,
        request_or_id: PolicyRequest | str,
        *,
        state_digest: str | None = None,
        action_digest: str | None = None,
        actor_id: str = "local-user",
        now: datetime | None = None,
    ) -> PolicyDecision:
        request = self._resolve_request(request_or_id)
        action_key = _supplied_action_key(request, state_digest, action_digest)
        actor = _safe_identifier(actor_id, "actor_id")
        with self._lock:
            self._track_request_locked(request)
            approval = self._approvals.get((request.request_id, *action_key))
            if approval is None:
                stale = self._stale_approval_locked(request, action_key)
                if stale is not None:
                    return self._invalidate_record(request, stale, actor, self._resolve_now(now))
                approval = self._store_pending_locked(request.request_id, *action_key)
            if approval.action_key != action_key:
                return self._invalidate_record(request, approval, actor, self._resolve_now(now))
            if approval.status is not ApprovalStatus.PENDING:
                raise PolicyTransitionError(
                    f"approval for {request.request_id} is already {approval.status.value}"
                )
            self._store_approval_locked(
                ApprovalRecord(
                    request_id=approval.request_id,
                    state_digest=approval.state_digest,
                    action_digest=approval.action_digest,
                    status=ApprovalStatus.APPROVED,
                    actor_id=actor,
                    decided_at=self._resolve_now(now),
                )
            )
            self._prune_locked()
            return self.decide(request, now=now)

    def reject(
        self,
        request_or_id: PolicyRequest | str,
        *,
        state_digest: str | None = None,
        action_digest: str | None = None,
        actor_id: str = "local-user",
        reason: str | None = None,
        now: datetime | None = None,
    ) -> PolicyDecision:
        return self._resolve_non_approval(
            request_or_id,
            ApprovalStatus.REJECTED,
            state_digest=state_digest,
            action_digest=action_digest,
            actor_id=actor_id,
            reason=reason,
            now=now,
        )

    def abort(
        self,
        request_or_id: PolicyRequest | str,
        *,
        state_digest: str | None = None,
        action_digest: str | None = None,
        actor_id: str = "local-user",
        reason: str | None = None,
        now: datetime | None = None,
    ) -> PolicyDecision:
        return self._resolve_non_approval(
            request_or_id,
            ApprovalStatus.ABORTED,
            state_digest=state_digest,
            action_digest=action_digest,
            actor_id=actor_id,
            reason=reason,
            now=now,
        )

    def invalidate(
        self,
        request_or_id: PolicyRequest | str,
        *,
        state_digest: str | None = None,
        action_digest: str | None = None,
        actor_id: str = "local-user",
        now: datetime | None = None,
    ) -> PolicyDecision:
        request = self._resolve_request(request_or_id)
        action_key = _supplied_action_key(request, state_digest, action_digest)
        actor = _safe_identifier(actor_id, "actor_id")
        with self._lock:
            self._track_request_locked(request)
            approval = self._approvals.get((request.request_id, *action_key))
            if approval is None or approval.action_key != action_key:
                raise PolicyTransitionError(
                    f"approval for {request.request_id} does not match the supplied digests"
                )
            self._prune_locked()
            return self._invalidate_record(request, approval, actor, self._resolve_now(now))

    def get_approval(
        self,
        request_id: str,
        *,
        state_digest: str | None = None,
        action_digest: str | None = None,
    ) -> ApprovalRecord | None:
        normalized = _safe_identifier(request_id, "request_id")
        with self._lock:
            if (state_digest is None) != (action_digest is None):
                raise PolicyRequestError("supply both state_digest and action_digest or neither")
            if state_digest is not None and action_digest is not None:
                key = (
                    normalized,
                    _digest(state_digest, "state_digest"),
                    _digest(action_digest, "action_digest"),
                )
                return self._approvals.get(key)
            for key, record in reversed(self._approvals.items()):
                if key[0] == normalized:
                    return record
            return None

    def forget_tool_class(self, action_digest: str, *, scope: str | None = None) -> bool:
        normalized = _digest(action_digest, "action_digest")
        wanted_scope = None if scope is None else _safe_identifier(scope, "scope")
        with self._lock:
            removed = False
            for key in [key for key in self._tool_classes if key[0] == normalized]:
                if wanted_scope is not None and key[1] != wanted_scope:
                    continue
                del self._tool_classes[key]
                removed = True
            return removed

    def export_state(self) -> dict[str, object]:
        with self._lock:
            return {
                "version": POLICY_STATE_VERSION,
                "grants": [grant.as_dict() for grants in self._grants.values() for grant in grants],
                "requests": [request.as_dict() for request in self._requests.values()],
                "approvals": [record.as_dict() for record in self._approvals.values()],
                "tool_classes": [
                    {
                        "action_digest": action_digest,
                        "capabilities": list(capabilities),
                        "scope": scope,
                    }
                    for (action_digest, scope), capabilities in self._tool_classes.items()
                ],
                "max_tracked_requests": self._max_tracked_requests,
            }

    def import_state(self, state: Mapping[str, object]) -> None:
        payload = _state_mapping(state, "state")
        _require_keys(payload, _POLICY_STATE_KEYS, "state")
        version = _state_int(payload.get("version"), "state.version")
        if version != POLICY_STATE_VERSION:
            raise _state_error(f"unsupported version {version}")
        limit = self._max_tracked_requests
        if "max_tracked_requests" in payload:
            limit = _state_positive_int(
                payload["max_tracked_requests"],
                "state.max_tracked_requests",
            )
        grants = tuple(
            _grant_from_state(item) for item in _state_sequence(payload.get("grants", ()), "grants")
        )
        requests: dict[str, PolicyRequest] = {}
        for item in _state_sequence(payload.get("requests", ()), "requests"):
            request = _request_from_state(item)
            if request.request_id in requests:
                raise _state_error(f"duplicate request {request.request_id}")
            requests[request.request_id] = request
        approvals: dict[ApprovalKey, ApprovalRecord] = {}
        for item in _state_sequence(payload.get("approvals", ()), "approvals"):
            record = _approval_from_state(item)
            if record.key in approvals:
                raise _state_error(f"duplicate approval for {record.request_id}")
            approvals[record.key] = record
        orphaned = sorted({key[0] for key in approvals} - set(requests))
        if orphaned:
            raise _state_error(f"approvals without a tracked request: {', '.join(orphaned)}")
        tool_classes: dict[ToolClassKey, tuple[str, ...]] = {}
        for item in _state_sequence(payload.get("tool_classes", ()), "tool_classes"):
            key, capabilities = _tool_class_from_state(item)
            if key in tool_classes:
                raise _state_error(f"duplicate tool class {key[0]}")
            tool_classes[key] = capabilities
        with self._lock:
            self._grants.clear()
            for grant in grants:
                existing = self._grants.get(grant.capability, ())
                if grant not in existing:
                    self._grants[grant.capability] = (*existing, grant)
            self._requests = requests
            self._approvals = approvals
            self._tool_classes = tool_classes
            self._max_tracked_requests = limit
            self._prune_locked()

    @classmethod
    def from_state(
        cls,
        state: Mapping[str, object],
        *,
        now: datetime | None = None,
        max_tracked_requests: int = DEFAULT_MAX_TRACKED_REQUESTS,
    ) -> PolicyEngine:
        engine = cls(now=now, max_tracked_requests=max_tracked_requests)
        engine.import_state(state)
        return engine

    def _invalidate_record(
        self,
        request: PolicyRequest,
        approval: ApprovalRecord,
        actor_id: str | None,
        decided_at: datetime | None,
    ) -> PolicyDecision:
        resolved_actor = "local-user" if actor_id is None else actor_id
        resolved_time = self._resolve_now(None) if decided_at is None else decided_at
        self._tool_classes.pop((approval.action_digest, request.scope), None)
        if approval.status in _REFUSED_APPROVAL_STATUSES:
            status, reason = _unapproved_outcome(approval.status)
            self._prune_locked()
            return self._decision(
                request,
                status,
                reason,
                allowed=False,
                actor_id=approval.actor_id,
                decided_at=approval.decided_at,
            )
        self._store_approval_locked(
            ApprovalRecord(
                request_id=approval.request_id,
                state_digest=approval.state_digest,
                action_digest=approval.action_digest,
                status=ApprovalStatus.INVALIDATED,
                actor_id=resolved_actor,
                decided_at=resolved_time,
                reason="state or action digest changed",
            )
        )
        self._prune_locked()
        return self._decision(
            request,
            PolicyDecisionStatus.INVALIDATED,
            PolicyReason.STALE_APPROVAL,
            allowed=False,
            actor_id=resolved_actor,
            decided_at=resolved_time,
        )

    def _resolve_non_approval(
        self,
        request_or_id: PolicyRequest | str,
        status: ApprovalStatus,
        *,
        state_digest: str | None,
        action_digest: str | None,
        actor_id: str,
        reason: str | None,
        now: datetime | None,
    ) -> PolicyDecision:
        request = self._resolve_request(request_or_id)
        action_key = _supplied_action_key(request, state_digest, action_digest)
        actor = _safe_identifier(actor_id, "actor_id")
        with self._lock:
            self._track_request_locked(request)
            approval = self._approvals.get((request.request_id, *action_key))
            if approval is None:
                stale = self._stale_approval_locked(request, action_key)
                if stale is not None:
                    return self._invalidate_record(request, stale, actor, self._resolve_now(now))
                approval = self._store_pending_locked(request.request_id, *action_key)
            if approval.action_key != action_key:
                return self._invalidate_record(request, approval, actor, self._resolve_now(now))
            if approval.status is not ApprovalStatus.PENDING:
                raise PolicyTransitionError(
                    f"approval for {request.request_id} is already {approval.status.value}"
                )
            self._tool_classes.pop((approval.action_digest, request.scope), None)
            self._store_approval_locked(
                ApprovalRecord(
                    request_id=approval.request_id,
                    state_digest=approval.state_digest,
                    action_digest=approval.action_digest,
                    status=status,
                    actor_id=actor,
                    decided_at=self._resolve_now(now),
                    reason=reason,
                )
            )
            self._prune_locked()
            return self.decide(request, now=now)

    def _store_pending_locked(
        self,
        request_id: str,
        state_digest: str,
        action_digest: str,
    ) -> ApprovalRecord:
        record = ApprovalRecord(
            request_id=request_id,
            state_digest=state_digest,
            action_digest=action_digest,
            status=ApprovalStatus.PENDING,
        )
        self._store_approval_locked(record)
        return record

    def _store_approval_locked(self, record: ApprovalRecord) -> None:
        self._approvals.pop(record.key, None)
        self._approvals[record.key] = record

    def _conflicting_approval_locked(self, request: PolicyRequest) -> ApprovalRecord | None:
        target = (request.state_digest, request.action_digest)
        for key, record in reversed(self._approvals.items()):
            if key[0] != request.request_id:
                continue
            if (key[1], key[2]) == target:
                continue
            if record.status is ApprovalStatus.PENDING or record.status is ApprovalStatus.APPROVED:
                return record
        return None

    def _stale_approval_locked(
        self,
        request: PolicyRequest,
        action_key: tuple[str, str],
    ) -> ApprovalRecord | None:
        current = self._approvals.get(_approval_key(request))
        if current is not None and current.action_key != action_key:
            return current
        conflict = self._conflicting_approval_locked(request)
        if conflict is not None and conflict.action_key != action_key:
            return conflict
        return None

    def _requires_approval_locked(self, request: PolicyRequest) -> bool:
        if not request.requires_approval:
            return False
        return not self._tool_class_approved_locked(request)

    def _tool_class_approved_locked(self, request: PolicyRequest) -> bool:
        if not request.tool_class_change:
            return False
        remembered = self._tool_classes.get((request.action_digest, request.scope))
        return remembered is not None and remembered == request.requested_capabilities

    def _remember_tool_class_locked(self, request: PolicyRequest) -> None:
        if not request.tool_class_change:
            return
        key = (request.action_digest, request.scope)
        if self._tool_classes.get(key) == request.requested_capabilities:
            return
        self._tool_classes[key] = request.requested_capabilities
        while len(self._tool_classes) > self._max_tracked_requests:
            del self._tool_classes[next(iter(self._tool_classes))]

    def _prune_locked(self) -> None:
        limit = self._max_tracked_requests
        if len(self._approvals) <= limit and len(self._requests) <= limit:
            return
        tracked = set(self._requests)
        for key in tuple(self._approvals):
            if key[0] not in tracked:
                del self._approvals[key]
        if len(self._approvals) <= limit and len(self._requests) <= limit:
            return
        recency = {request_id: index for index, request_id in enumerate(self._requests)}
        ranked_ids = sorted(
            self._requests,
            key=lambda request_id: (
                self._eviction_tier_locked(request_id),
                recency[request_id],
            ),
        )
        for request_id in ranked_ids:
            if len(self._requests) <= limit and len(self._approvals) <= limit:
                break
            self._discard_request_locked(request_id)

    def _eviction_tier_locked(self, request_id: str) -> int:
        """Rank a request for eviction; forgetting a lower tier grants no authority.

        A forgotten approval must be requested again, so approvals go before
        refusals: forgetting a refusal would let the same request be approved.
        """
        records = [record for key, record in self._approvals.items() if key[0] == request_id]
        if any(not record.resolved for record in records):
            return 3
        if any(record.status in _REFUSED_APPROVAL_STATUSES for record in records):
            return 2
        if any(record.status not in _TERMINAL_APPROVAL_STATUSES for record in records):
            return 1
        return 0

    def _discard_request_locked(self, request_id: str) -> None:
        self._requests.pop(request_id, None)
        for key in [key for key in self._approvals if key[0] == request_id]:
            del self._approvals[key]

    def _track_request_locked(self, request: PolicyRequest) -> None:
        self._requests.pop(request.request_id, None)
        self._requests[request.request_id] = request

    def _request(self, request: object) -> PolicyRequest:
        if not isinstance(request, PolicyRequest):
            raise PolicyRequestError("request must be a PolicyRequest")
        return PolicyRequest(
            request_id=request.request_id,
            state_digest=request.state_digest,
            action_digest=request.action_digest,
            requested_capabilities=request.requested_capabilities,
            side_effect_level=request.side_effect_level,
            destructive=request.destructive,
            egress=request.egress,
            scope=request.scope,
            checkpoint_type=request.checkpoint_type,
        )

    def _resolve_request(self, request_or_id: PolicyRequest | str) -> PolicyRequest:
        if isinstance(request_or_id, PolicyRequest):
            return self._request(request_or_id)
        request_id = _safe_identifier(request_or_id, "request_id")
        with self._lock:
            request = self._requests.get(request_id)
        if request is None:
            raise PolicyRequestError(f"unknown request: {request_id}")
        return request

    def _capability_state(
        self,
        request: PolicyRequest,
        now: datetime,
    ) -> tuple[tuple[str, ...], tuple[str, ...], tuple[str, ...]]:
        missing: list[str] = []
        expired: list[str] = []
        scope_mismatch: list[str] = []
        for capability in request.requested_capabilities:
            grants = self._grants.get(capability, ())
            if not grants:
                missing.append(capability)
                continue
            active = [
                grant for grant in grants if grant.expires_at is None or grant.expires_at > now
            ]
            if not active:
                expired.append(capability)
                continue
            if not any(grant.applies_to(capability, request.scope, now) for grant in active):
                scope_mismatch.append(capability)
        return tuple(missing), tuple(expired), tuple(scope_mismatch)

    @staticmethod
    def _decision(
        request: PolicyRequest,
        status: PolicyDecisionStatus,
        reason: PolicyReason,
        *,
        allowed: bool,
        missing_capabilities: tuple[str, ...] = (),
        actor_id: str | None = None,
        decided_at: datetime | None = None,
    ) -> PolicyDecision:
        return PolicyDecision(
            request_id=request.request_id,
            status=status,
            reason=reason,
            state_digest=request.state_digest,
            action_digest=request.action_digest,
            allowed=allowed,
            requires_approval=request.requires_approval,
            missing_capabilities=missing_capabilities,
            actor_id=actor_id,
            decided_at=decided_at,
        )

    def _resolve_now(self, value: datetime | None) -> datetime:
        resolved = _aware(value, "now")
        if resolved is not None:
            return resolved
        if self._clock is not None:
            return self._clock
        return datetime.now(UTC)

    @staticmethod
    def _grant_items(
        grants: Iterable[str | CapabilityGrant] | Mapping[str, CapabilityGrant | str | None],
    ) -> tuple[CapabilityGrant, ...]:
        if isinstance(grants, Mapping):
            result: list[CapabilityGrant] = []
            for capability, value in grants.items():
                if value is None:
                    result.append(CapabilityGrant(capability))
                elif isinstance(value, CapabilityGrant):
                    if value.capability != capability:
                        raise PolicyRequestError("grant mapping key must match capability")
                    result.append(value)
                else:
                    result.append(CapabilityGrant(capability, scope=value))
            return tuple(result)
        return tuple(
            value if isinstance(value, CapabilityGrant) else CapabilityGrant(value)
            for value in grants
        )


def _approval_key(request: PolicyRequest) -> ApprovalKey:
    return request.request_id, request.state_digest, request.action_digest


__all__ = (
    "DEFAULT_MAX_TRACKED_REQUESTS",
    "POLICY_STATE_VERSION",
    "ApprovalDecision",
    "ApprovalKey",
    "ApprovalRecord",
    "ApprovalRequest",
    "CapabilityGrant",
    "DecisionStatus",
    "PolicyApproval",
    "PolicyApprovalRequest",
    "PolicyDecision",
    "PolicyDecisionStatus",
    "PolicyEngine",
    "PolicyError",
    "PolicyReason",
    "PolicyRequest",
    "PolicyRequestError",
    "PolicyRequestModel",
    "PolicyStateError",
    "PolicyStatus",
    "PolicyTransitionError",
    "ToolClassKey",
)
