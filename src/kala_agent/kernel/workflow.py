from __future__ import annotations

import hashlib
import inspect
import json
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, replace
from datetime import UTC, datetime
from threading import RLock
from typing import TypeAlias, cast

from kala_agent.contracts import (
    ApprovalKind,
    ApprovalStatus,
    WorkflowDefinition,
    WorkflowStatus,
    WorkflowTransition,
)


class WorkflowError(RuntimeError):
    pass


class WorkflowTransitionError(WorkflowError, ValueError):
    pass


class WorkflowLimitExceeded(WorkflowTransitionError):
    pass


class WorkflowApprovalError(WorkflowTransitionError):
    pass


WorkflowCheckpointStatus = ApprovalStatus

_STEP_FAILED = "step_failed"
_RETRY_BOUND_EXHAUSTED = "step_retry_bound_exhausted"
_STALE_APPROVAL = "state_or_action_digest_changed"


def _safe_identifier(value: object, field_name: str) -> str:
    if not isinstance(value, str):
        raise WorkflowTransitionError(f"{field_name} must be a string")
    normalized = value.strip()
    if not normalized or len(normalized) > 128:
        raise WorkflowTransitionError(f"{field_name} must be nonempty and bounded")
    if any(character.isspace() or ord(character) < 32 for character in normalized):
        raise WorkflowTransitionError(f"{field_name} contains unsafe characters")
    return normalized


def _status(value: str | WorkflowStatus) -> WorkflowStatus:
    if isinstance(value, WorkflowStatus):
        return value
    try:
        return WorkflowStatus(value)
    except (TypeError, ValueError) as error:
        raise WorkflowTransitionError("workflow status is invalid") from error


def _checkpoint_status(value: str | ApprovalStatus) -> ApprovalStatus:
    if isinstance(value, ApprovalStatus):
        return value
    try:
        return ApprovalStatus(value)
    except (TypeError, ValueError) as error:
        raise WorkflowTransitionError("approval status is invalid") from error


def _checkpoint_kind(value: str | ApprovalKind) -> ApprovalKind:
    if isinstance(value, ApprovalKind):
        return value
    try:
        return ApprovalKind(value)
    except (TypeError, ValueError) as error:
        raise WorkflowTransitionError("checkpoint_type is invalid") from error


def _digest(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _canonical(value: object) -> str:
    return json.dumps(
        value, allow_nan=False, ensure_ascii=True, separators=(",", ":"), sort_keys=True
    )


def _parse_datetime(value: object) -> datetime | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise WorkflowTransitionError("serialized timestamp is invalid")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise WorkflowTransitionError("serialized timestamp is invalid") from error
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        raise WorkflowTransitionError("serialized timestamp must include a timezone")
    return parsed.astimezone(UTC)


@dataclass(frozen=True, slots=True)
class WorkflowApproval:
    approval_id: str
    transition_id: str
    step_id: str
    checkpoint_type: ApprovalKind
    state_digest: str
    action_digest: str
    status: ApprovalStatus = ApprovalStatus.PENDING
    actor_id: str | None = None
    decided_at: datetime | None = None
    reason: str | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "approval_id", _safe_identifier(self.approval_id, "approval_id"))
        object.__setattr__(
            self, "transition_id", _safe_identifier(self.transition_id, "transition_id")
        )
        object.__setattr__(self, "step_id", _safe_identifier(self.step_id, "step_id"))
        object.__setattr__(
            self,
            "checkpoint_type",
            _checkpoint_kind(self.checkpoint_type),
        )
        if not isinstance(self.state_digest, str) or not self.state_digest:
            raise WorkflowTransitionError("state_digest must be nonempty")
        if not isinstance(self.action_digest, str) or not self.action_digest:
            raise WorkflowTransitionError("action_digest must be nonempty")
        object.__setattr__(self, "status", _checkpoint_status(self.status))
        if self.actor_id is not None:
            object.__setattr__(self, "actor_id", _safe_identifier(self.actor_id, "actor_id"))
        if self.decided_at is not None:
            if self.decided_at.tzinfo is None or self.decided_at.utcoffset() is None:
                raise WorkflowTransitionError("decided_at must include a timezone")
            object.__setattr__(self, "decided_at", self.decided_at.astimezone(UTC))
        if self.status is ApprovalStatus.PENDING:
            if self.actor_id is not None or self.decided_at is not None or self.reason is not None:
                raise WorkflowTransitionError("pending approval cannot contain a decision")
        elif self.status in {
            ApprovalStatus.APPROVED,
            ApprovalStatus.REJECTED,
            ApprovalStatus.ABORTED,
        } and (self.actor_id is None or self.decided_at is None):
            raise WorkflowTransitionError("resolved approval requires actor and decision time")

    @property
    def is_pending(self) -> bool:
        return self.status is ApprovalStatus.PENDING

    @property
    def is_approved(self) -> bool:
        return self.status is ApprovalStatus.APPROVED

    def to_dict(self) -> dict[str, object]:
        return {
            "approval_id": self.approval_id,
            "transition_id": self.transition_id,
            "step_id": self.step_id,
            "checkpoint_type": self.checkpoint_type.value,
            "state_digest": self.state_digest,
            "action_digest": self.action_digest,
            "status": self.status.value,
            "actor_id": self.actor_id,
            "decided_at": None if self.decided_at is None else self.decided_at.isoformat(),
            "reason": self.reason,
        }

    @classmethod
    def from_dict(cls, value: Mapping[str, object]) -> WorkflowApproval:
        try:
            return cls(
                approval_id=cast(str, value["approval_id"]),
                transition_id=cast(str, value["transition_id"]),
                step_id=cast(str, value["step_id"]),
                checkpoint_type=_checkpoint_kind(cast(str, value["checkpoint_type"])),
                state_digest=cast(str, value["state_digest"]),
                action_digest=cast(str, value["action_digest"]),
                status=_checkpoint_status(cast(str, value["status"])),
                actor_id=cast(str | None, value.get("actor_id")),
                decided_at=_parse_datetime(value.get("decided_at")),
                reason=cast(str | None, value.get("reason")),
            )
        except KeyError as error:
            raise WorkflowTransitionError("serialized approval is missing a field") from error


def _supplied_action_key(
    approval: WorkflowApproval,
    state_digest: str | None,
    action_digest: str | None,
) -> tuple[str, str]:
    supplied_state = (
        approval.state_digest
        if state_digest is None
        else _safe_identifier(state_digest, "state_digest")
    )
    supplied_action = (
        approval.action_digest
        if action_digest is None
        else _safe_identifier(action_digest, "action_digest")
    )
    return supplied_state, supplied_action


@dataclass(frozen=True, slots=True)
class StepAttempt:
    step_id: str
    attempt: int
    succeeded: bool
    error_code: str | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "step_id", _safe_identifier(self.step_id, "step_id"))
        if isinstance(self.attempt, bool) or not isinstance(self.attempt, int) or self.attempt < 1:
            raise WorkflowTransitionError("step attempt must be a positive integer")
        if not isinstance(self.succeeded, bool):
            raise WorkflowTransitionError("step succeeded must be a boolean")
        if self.error_code is not None:
            object.__setattr__(self, "error_code", _safe_identifier(self.error_code, "error_code"))

    def to_dict(self) -> dict[str, object]:
        return {
            "step_id": self.step_id,
            "attempt": self.attempt,
            "succeeded": self.succeeded,
            "error_code": self.error_code,
        }

    @classmethod
    def from_dict(cls, value: Mapping[str, object]) -> StepAttempt:
        try:
            return cls(
                step_id=cast(str, value["step_id"]),
                attempt=cast(int, value["attempt"]),
                succeeded=cast(bool, value["succeeded"]),
                error_code=cast(str | None, value.get("error_code")),
            )
        except KeyError as error:
            raise WorkflowTransitionError("serialized step attempt is missing a field") from error


@dataclass(frozen=True, slots=True)
class WorkflowTransitionRecord:
    sequence: int
    transition_id: str
    from_state: str
    to_state: str
    step_id: str
    attempt: int

    def __post_init__(self) -> None:
        if (
            isinstance(self.sequence, bool)
            or not isinstance(self.sequence, int)
            or self.sequence < 1
        ):
            raise WorkflowTransitionError("transition sequence must be positive")
        for field_name, value in (
            ("transition_id", self.transition_id),
            ("from_state", self.from_state),
            ("to_state", self.to_state),
            ("step_id", self.step_id),
        ):
            object.__setattr__(self, field_name, _safe_identifier(value, field_name))
        if isinstance(self.attempt, bool) or not isinstance(self.attempt, int) or self.attempt < 1:
            raise WorkflowTransitionError("transition attempt must be positive")

    def to_dict(self) -> dict[str, object]:
        return {
            "sequence": self.sequence,
            "transition_id": self.transition_id,
            "from_state": self.from_state,
            "to_state": self.to_state,
            "step_id": self.step_id,
            "attempt": self.attempt,
        }

    @classmethod
    def from_dict(cls, value: Mapping[str, object]) -> WorkflowTransitionRecord:
        try:
            return cls(
                sequence=cast(int, value["sequence"]),
                transition_id=cast(str, value["transition_id"]),
                from_state=cast(str, value["from_state"]),
                to_state=cast(str, value["to_state"]),
                step_id=cast(str, value["step_id"]),
                attempt=cast(int, value["attempt"]),
            )
        except KeyError as error:
            raise WorkflowTransitionError("serialized transition is missing a field") from error


@dataclass(frozen=True, slots=True)
class WorkflowState:
    workflow_id: str
    definition_id: str
    definition_version: str
    status: WorkflowStatus
    current_state: str
    completed_step_ids: tuple[str, ...] = ()
    attempts: tuple[StepAttempt, ...] = ()
    pending_approval: WorkflowApproval | None = None
    replanning_attempts: int = 0
    history: tuple[WorkflowTransitionRecord, ...] = ()
    last_error: str | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "workflow_id", _safe_identifier(self.workflow_id, "workflow_id"))
        object.__setattr__(
            self, "definition_id", _safe_identifier(self.definition_id, "definition_id")
        )
        object.__setattr__(
            self,
            "definition_version",
            _safe_identifier(self.definition_version, "definition_version"),
        )
        object.__setattr__(self, "status", _status(self.status))
        object.__setattr__(
            self, "current_state", _safe_identifier(self.current_state, "current_state")
        )
        object.__setattr__(
            self,
            "completed_step_ids",
            tuple(
                _safe_identifier(value, "completed_step_id") for value in self.completed_step_ids
            ),
        )
        object.__setattr__(self, "attempts", tuple(self.attempts))
        if isinstance(self.replanning_attempts, bool) or not isinstance(
            self.replanning_attempts, int
        ):
            raise WorkflowTransitionError("replanning_attempts must be an integer")
        if self.replanning_attempts < 0:
            raise WorkflowTransitionError("replanning_attempts must be nonnegative")
        object.__setattr__(self, "history", tuple(self.history))
        if self.pending_approval is not None and not isinstance(
            self.pending_approval, WorkflowApproval
        ):
            raise WorkflowTransitionError("pending_approval must be a WorkflowApproval")
        if self.status is WorkflowStatus.WAITING_FOR_APPROVAL and self.pending_approval is None:
            raise WorkflowTransitionError("waiting workflows require a pending approval")

    @property
    def is_terminal(self) -> bool:
        return self.status in {
            WorkflowStatus.COMPLETED,
            WorkflowStatus.FAILED,
            WorkflowStatus.CANCELLED,
        }

    @property
    def is_cancelled(self) -> bool:
        return self.status is WorkflowStatus.CANCELLED

    def to_dict(self) -> dict[str, object]:
        return {
            "workflow_id": self.workflow_id,
            "definition_id": self.definition_id,
            "definition_version": self.definition_version,
            "status": self.status.value,
            "current_state": self.current_state,
            "completed_step_ids": list(self.completed_step_ids),
            "attempts": [attempt.to_dict() for attempt in self.attempts],
            "pending_approval": (
                None if self.pending_approval is None else self.pending_approval.to_dict()
            ),
            "replanning_attempts": self.replanning_attempts,
            "history": [record.to_dict() for record in self.history],
            "last_error": self.last_error,
        }

    def to_json(self) -> str:
        return _canonical(self.to_dict())

    @classmethod
    def from_dict(cls, value: Mapping[str, object]) -> WorkflowState:
        try:
            pending_value = value.get("pending_approval")
            attempts_value = value.get("attempts", ())
            history_value = value.get("history", ())
            if pending_value is not None and not isinstance(pending_value, Mapping):
                raise WorkflowTransitionError("serialized approval is invalid")
            if not isinstance(attempts_value, Sequence) or isinstance(attempts_value, (str, bytes)):
                raise WorkflowTransitionError("serialized attempts are invalid")
            if not isinstance(history_value, Sequence) or isinstance(history_value, (str, bytes)):
                raise WorkflowTransitionError("serialized history is invalid")
            attempts = tuple(
                StepAttempt.from_dict(cast(Mapping[str, object], item)) for item in attempts_value
            )
            history = tuple(
                WorkflowTransitionRecord.from_dict(cast(Mapping[str, object], item))
                for item in history_value
            )
            pending = (
                None
                if pending_value is None
                else WorkflowApproval.from_dict(cast(Mapping[str, object], pending_value))
            )
            return cls(
                workflow_id=cast(str, value["workflow_id"]),
                definition_id=cast(str, value["definition_id"]),
                definition_version=cast(str, value["definition_version"]),
                status=_status(cast(str, value["status"])),
                current_state=cast(str, value["current_state"]),
                completed_step_ids=tuple(cast(list[str], value.get("completed_step_ids", ()))),
                attempts=attempts,
                pending_approval=pending,
                replanning_attempts=cast(int, value.get("replanning_attempts", 0)),
                history=history,
                last_error=cast(str | None, value.get("last_error")),
            )
        except KeyError as error:
            raise WorkflowTransitionError("serialized workflow state is missing a field") from error

    @classmethod
    def from_json(cls, value: str) -> WorkflowState:
        if not isinstance(value, str):
            raise TypeError("serialized workflow state must be a string")
        try:
            decoded = json.loads(value)
        except json.JSONDecodeError as error:
            raise WorkflowTransitionError("serialized workflow state is invalid") from error
        if not isinstance(decoded, dict):
            raise WorkflowTransitionError("serialized workflow state must be an object")
        return cls.from_dict(cast(Mapping[str, object], decoded))


@dataclass(frozen=True, slots=True)
class WorkflowEvent:
    event_type: str
    workflow_id: str
    from_status: WorkflowStatus | None
    to_status: WorkflowStatus
    current_state: str
    transition_id: str | None = None
    step_id: str | None = None
    attempt: int | None = None
    reason: str | None = None
    approval: WorkflowApproval | None = None

    @property
    def payload(self) -> dict[str, object]:
        return {
            "workflow_id": self.workflow_id,
            "from_status": None if self.from_status is None else self.from_status.value,
            "to_status": self.to_status.value,
            "current_state": self.current_state,
            "transition_id": self.transition_id,
            "step_id": self.step_id,
            "attempt": self.attempt,
            "reason": self.reason,
            "approval": None if self.approval is None else self.approval.to_dict(),
        }

    def to_dict(self) -> dict[str, object]:
        return {"event_type": self.event_type, **self.payload}


EventJournalCallback: TypeAlias = Callable[[WorkflowEvent], None]


class WorkflowEngine:
    def __init__(
        self,
        definition: WorkflowDefinition,
        event_journal_callback: object | None = None,
        *,
        journal_callback: object | None = None,
        event_callback: object | None = None,
        max_attempts: Mapping[str, int] | None = None,
    ) -> None:
        if not isinstance(definition, WorkflowDefinition):
            raise TypeError("definition must be a WorkflowDefinition")
        callbacks = [
            callback
            for callback in (event_journal_callback, journal_callback, event_callback)
            if callback is not None
        ]
        if len(callbacks) > 1:
            raise TypeError("provide only one event journal callback")
        self._definition = definition
        self._callback = callbacks[0] if callbacks else None
        self._max_attempts = self._validate_max_attempts(max_attempts or {})
        self._state: WorkflowState | None = None
        self._lock = RLock()

    @property
    def definition(self) -> WorkflowDefinition:
        return self._definition

    @property
    def state(self) -> WorkflowState | None:
        with self._lock:
            return self._state

    @property
    def current_state(self) -> WorkflowState | None:
        return self.state

    def start(
        self,
        workflow_id: str = "workflow-1",
        *,
        replanning_attempts: int = 0,
    ) -> WorkflowState:
        normalized_workflow_id = _safe_identifier(workflow_id, "workflow_id")
        with self._lock:
            if self._state is not None and self._state.workflow_id == normalized_workflow_id:
                return self._state
            if isinstance(replanning_attempts, bool) or not isinstance(replanning_attempts, int):
                raise WorkflowTransitionError("replanning_attempts must be an integer")
            if (
                replanning_attempts < 0
                or replanning_attempts > self._definition.max_replanning_attempts
            ):
                raise WorkflowLimitExceeded("replanning attempts exceed the workflow limit")
            state = WorkflowState(
                workflow_id=normalized_workflow_id,
                definition_id=self._definition.definition_id,
                definition_version=self._definition.version,
                status=WorkflowStatus.CREATED,
                current_state=self._definition.initial_state,
                replanning_attempts=replanning_attempts,
            )
            self._state = state
            self._emit(
                WorkflowEvent(
                    event_type="workflow.created",
                    workflow_id=state.workflow_id,
                    from_status=None,
                    to_status=state.status,
                    current_state=state.current_state,
                )
            )
            return state

    def run(
        self,
        state: WorkflowState | None = None,
        *,
        transition_id: str | None = None,
        step_id: str | None = None,
        succeeded: bool | None = None,
    ) -> WorkflowState:
        current = self._require_state(state)
        if succeeded is not None or step_id is not None:
            selected_step = step_id
            if selected_step is None:
                transition = self._select_transition(current, transition_id)
                if transition is None:
                    return self._complete_if_terminal(current)
                selected_step = transition.step_id
            if succeeded is None:
                raise WorkflowTransitionError("succeeded must be supplied with step_id")
            current = self.record_step_result(
                current,
                selected_step,
                succeeded,
                transition_id=transition_id,
            )
            return self._store(current)
        return self._store(self._advance(current, transition_id))

    advance = run
    execute = run
    step = run

    def resume(self, state: WorkflowState | None = None) -> WorkflowState:
        current = self._require_state(state)
        if current.pending_approval is not None:
            if not current.pending_approval.is_approved:
                return current
            if not self._approval_matches(current, current.pending_approval):
                return self._invalidate_approval(current, _STALE_APPROVAL)
            pending = current.pending_approval
            transition = self._get_transition(pending.transition_id)
            current = replace(current, pending_approval=None, status=WorkflowStatus.RUNNING)
            return self._store(self._apply_transition(current, transition))
        return self.run(current)

    def record_step_result(
        self,
        state: WorkflowState,
        step_id: str,
        succeeded: bool,
        *,
        transition_id: str | None = None,
        error_code: str | None = None,
    ) -> WorkflowState:
        current = self._validated_state(state)
        if current.is_terminal:
            return current
        approved_transition_id: str | None = None
        if current.pending_approval is not None:
            pending = current.pending_approval
            if not pending.is_approved:
                raise WorkflowApprovalError("step cannot run before checkpoint approval")
            if not self._approval_matches(current, pending):
                return self._invalidate_approval(current, _STALE_APPROVAL)
            approved_transition_id = pending.transition_id
            current = replace(current, pending_approval=None, status=WorkflowStatus.RUNNING)
        if not isinstance(succeeded, bool):
            raise WorkflowTransitionError("succeeded must be a boolean")
        transition = self._select_transition(current, transition_id)
        if transition is None:
            return self._complete_if_terminal(current)
        if transition.step_id != step_id:
            raise WorkflowTransitionError("step result does not match the declared transition")
        if (
            succeeded
            and transition.checkpoint_type is not None
            and approved_transition_id != transition.transition_id
        ):
            raise WorkflowApprovalError("step cannot run before checkpoint approval")
        if succeeded:
            if self._attempt_count(current, step_id) >= self._attempt_limit(step_id):
                return self._fail_state(current, _RETRY_BOUND_EXHAUSTED, step_id)
            return self._apply_transition(current, transition)
        return self._record_failure(current, transition, error_code)

    def record_step_success(
        self,
        state: WorkflowState,
        step_id: str,
        *,
        transition_id: str | None = None,
    ) -> WorkflowState:
        return self.record_step_result(
            state,
            step_id,
            True,
            transition_id=transition_id,
        )

    def record_step_failure(
        self,
        state: WorkflowState,
        step_id: str,
        *,
        error_code: str = "step_failed",
        transition_id: str | None = None,
    ) -> WorkflowState:
        return self.record_step_result(
            state,
            step_id,
            False,
            transition_id=transition_id,
            error_code=error_code,
        )

    retry_step = record_step_failure

    def approve_checkpoint(
        self,
        state: WorkflowState | None = None,
        approval_id: str | None = None,
        *,
        state_digest: str | None = None,
        action_digest: str | None = None,
        actor_id: str = "local-user",
        now: datetime | None = None,
    ) -> WorkflowState:
        current = self._require_state(state)
        pending = current.pending_approval
        if pending is None:
            raise WorkflowApprovalError("workflow has no pending checkpoint")
        if approval_id is not None and approval_id != pending.approval_id:
            raise WorkflowApprovalError("approval_id does not match the pending checkpoint")
        if pending.status is not ApprovalStatus.PENDING:
            if pending.status is ApprovalStatus.APPROVED:
                return current
            raise WorkflowApprovalError(f"checkpoint is already {pending.status.value}")
        if _supplied_action_key(pending, state_digest, action_digest) != (
            pending.state_digest,
            pending.action_digest,
        ):
            return self._invalidate_approval(current, _STALE_APPROVAL)
        decision_time = self._normalize_now(now)
        approved = replace(
            pending,
            status=ApprovalStatus.APPROVED,
            actor_id=_safe_identifier(actor_id, "actor_id"),
            decided_at=decision_time,
        )
        current = replace(current, pending_approval=approved)
        self._emit(
            WorkflowEvent(
                event_type="workflow.approval_resolved",
                workflow_id=current.workflow_id,
                from_status=current.status,
                to_status=current.status,
                current_state=current.current_state,
                transition_id=approved.transition_id,
                step_id=approved.step_id,
                approval=approved,
            )
        )
        return self._store(current)

    def approve(
        self,
        state: WorkflowState | None = None,
        approval_id: str | None = None,
        *,
        state_digest: str | None = None,
        action_digest: str | None = None,
        actor_id: str = "local-user",
        now: datetime | None = None,
    ) -> WorkflowState:
        return self.approve_checkpoint(
            state,
            approval_id,
            state_digest=state_digest,
            action_digest=action_digest,
            actor_id=actor_id,
            now=now,
        )

    def reject_checkpoint(
        self,
        state: WorkflowState | None = None,
        approval_id: str | None = None,
        *,
        state_digest: str | None = None,
        action_digest: str | None = None,
        actor_id: str = "local-user",
        reason: str = "checkpoint rejected",
        now: datetime | None = None,
    ) -> WorkflowState:
        return self._resolve_checkpoint(
            state,
            approval_id,
            ApprovalStatus.REJECTED,
            state_digest=state_digest,
            action_digest=action_digest,
            actor_id=actor_id,
            reason=reason,
            now=now,
        )

    reject = reject_checkpoint

    def abort_checkpoint(
        self,
        state: WorkflowState | None = None,
        approval_id: str | None = None,
        *,
        state_digest: str | None = None,
        action_digest: str | None = None,
        actor_id: str = "local-user",
        reason: str = "checkpoint aborted",
        now: datetime | None = None,
    ) -> WorkflowState:
        return self._resolve_checkpoint(
            state,
            approval_id,
            ApprovalStatus.ABORTED,
            state_digest=state_digest,
            action_digest=action_digest,
            actor_id=actor_id,
            reason=reason,
            now=now,
        )

    abort = abort_checkpoint

    def resolve_checkpoint(
        self,
        state: WorkflowState | None = None,
        approval_id: str | None = None,
        decision: str | ApprovalStatus = ApprovalStatus.APPROVED,
        *,
        state_digest: str | None = None,
        action_digest: str | None = None,
        actor_id: str = "local-user",
        reason: str = "checkpoint rejected",
        now: datetime | None = None,
    ) -> WorkflowState:
        return self.decide_checkpoint(
            state,
            approval_id,
            decision,
            state_digest=state_digest,
            action_digest=action_digest,
            actor_id=actor_id,
            reason=reason,
            now=now,
        )

    def decide_checkpoint(
        self,
        state: WorkflowState | None = None,
        approval_id: str | None = None,
        decision: str | ApprovalStatus = ApprovalStatus.APPROVED,
        *,
        state_digest: str | None = None,
        action_digest: str | None = None,
        actor_id: str = "local-user",
        reason: str = "checkpoint rejected",
        now: datetime | None = None,
    ) -> WorkflowState:
        decision_values = {
            "approve": ApprovalStatus.APPROVED,
            "approved": ApprovalStatus.APPROVED,
            "reject": ApprovalStatus.REJECTED,
            "rejected": ApprovalStatus.REJECTED,
            "abort": ApprovalStatus.ABORTED,
            "aborted": ApprovalStatus.ABORTED,
        }
        selected = (
            decision_values.get(decision, decision) if isinstance(decision, str) else decision
        )
        selected = _checkpoint_status(selected)
        if selected is ApprovalStatus.APPROVED:
            return self.approve_checkpoint(
                state,
                approval_id,
                state_digest=state_digest,
                action_digest=action_digest,
                actor_id=actor_id,
                now=now,
            )
        if selected is ApprovalStatus.REJECTED:
            return self.reject_checkpoint(
                state,
                approval_id,
                state_digest=state_digest,
                action_digest=action_digest,
                actor_id=actor_id,
                reason=reason,
                now=now,
            )
        if selected is ApprovalStatus.ABORTED:
            return self.abort_checkpoint(
                state,
                approval_id,
                state_digest=state_digest,
                action_digest=action_digest,
                actor_id=actor_id,
                reason=reason,
                now=now,
            )
        raise WorkflowApprovalError("checkpoint decision must be approve, reject, or abort")

    def cancel(
        self,
        state: WorkflowState | None = None,
        *,
        reason: str = "workflow cancelled",
    ) -> WorkflowState:
        current = self._require_state(state)
        if current.status is WorkflowStatus.CANCELLED:
            return current
        if current.status in {WorkflowStatus.COMPLETED, WorkflowStatus.FAILED}:
            return current
        approval = current.pending_approval
        if approval is not None:
            approval = replace(
                approval,
                status=ApprovalStatus.ABORTED,
                actor_id="local-user",
                decided_at=self._normalize_now(None),
                reason=reason,
            )
        cancelled = replace(
            current,
            status=WorkflowStatus.CANCELLED,
            pending_approval=approval,
            last_error=reason,
        )
        self._emit(
            WorkflowEvent(
                event_type="workflow.cancelled",
                workflow_id=cancelled.workflow_id,
                from_status=current.status,
                to_status=WorkflowStatus.CANCELLED,
                current_state=cancelled.current_state,
                reason=reason,
                approval=approval,
            )
        )
        return self._store(cancelled)

    def request_replan(
        self,
        state: WorkflowState | None = None,
        *,
        reason: str = "replan requested",
    ) -> WorkflowState:
        current = self._require_state(state)
        if current.is_terminal:
            return current
        if current.replanning_attempts >= self._definition.max_replanning_attempts:
            raise WorkflowLimitExceeded("replanning attempts exhausted")
        updated = replace(
            current,
            replanning_attempts=current.replanning_attempts + 1,
            last_error=None,
        )
        self._emit(
            WorkflowEvent(
                event_type="workflow.replanning_requested",
                workflow_id=updated.workflow_id,
                from_status=current.status,
                to_status=updated.status,
                current_state=updated.current_state,
                reason=reason,
            )
        )
        return self._store(updated)

    replan = request_replan

    def serialize(self, state: WorkflowState | None = None) -> str:
        return self._require_state(state).to_json()

    def snapshot(self, state: WorkflowState | None = None) -> WorkflowState:
        return self._require_state(state)

    def restore(
        self,
        state: WorkflowState | Mapping[str, object] | str,
    ) -> WorkflowState:
        if isinstance(state, WorkflowState):
            restored = state
        elif isinstance(state, str):
            restored = WorkflowState.from_json(state)
        elif isinstance(state, Mapping):
            restored = WorkflowState.from_dict(state)
        else:
            raise TypeError("state must be a WorkflowState, mapping, or JSON string")
        if restored.definition_id != self._definition.definition_id:
            raise WorkflowTransitionError("state definition_id does not match the engine")
        if restored.definition_version != self._definition.version:
            raise WorkflowTransitionError("state definition version does not match the engine")
        if restored.current_state not in self._definition.states:
            raise WorkflowTransitionError("state current_state is not declared by the definition")
        with self._lock:
            self._state = restored
        return restored

    restore_state = restore
    resume_from = restore

    def state_digest(self, state: WorkflowState | None = None) -> str:
        current = self._require_state(state)
        return self._state_digest(current)

    def _advance(
        self,
        state: WorkflowState,
        transition_id: str | None = None,
    ) -> WorkflowState:
        current = self._validated_state(state)
        if current.is_terminal:
            return current
        if current.pending_approval is not None:
            if not current.pending_approval.is_approved:
                return current
            if not self._approval_matches(current, current.pending_approval):
                return self._invalidate_approval(current, _STALE_APPROVAL)
            pending = current.pending_approval
            transition = self._get_transition(pending.transition_id)
            current = replace(current, pending_approval=None, status=WorkflowStatus.RUNNING)
            return self._apply_transition(current, transition)
        if current.status is WorkflowStatus.CREATED:
            previous = current.status
            current = replace(current, status=WorkflowStatus.RUNNING)
            self._emit(
                WorkflowEvent(
                    event_type="workflow.status_changed",
                    workflow_id=current.workflow_id,
                    from_status=previous,
                    to_status=current.status,
                    current_state=current.current_state,
                )
            )
        selected = self._select_transition(current, transition_id)
        if selected is None:
            return self._complete_if_terminal(current)
        if selected.checkpoint_type is not None:
            approval = self._make_approval(current, selected)
            waiting = replace(
                current,
                status=WorkflowStatus.WAITING_FOR_APPROVAL,
                pending_approval=approval,
            )
            self._emit(
                WorkflowEvent(
                    event_type="workflow.checkpoint",
                    workflow_id=waiting.workflow_id,
                    from_status=current.status,
                    to_status=waiting.status,
                    current_state=waiting.current_state,
                    transition_id=selected.transition_id,
                    step_id=selected.step_id,
                    approval=approval,
                )
            )
            return waiting
        return self._apply_transition(current, selected)

    def _apply_transition(
        self,
        state: WorkflowState,
        transition: WorkflowTransition,
    ) -> WorkflowState:
        current = self._validated_state(state)
        if current.current_state != transition.from_state:
            raise WorkflowTransitionError("transition is not legal from the current state")
        if transition.step_id not in self._definition.available_step_ids:
            raise WorkflowTransitionError("transition step is not declared by the workflow")
        attempt = self._attempt_count(current, transition.step_id) + 1
        if attempt > self._attempt_limit(transition.step_id):
            return self._fail_state(current, _RETRY_BOUND_EXHAUSTED, transition.step_id)
        attempt_record = StepAttempt(
            step_id=transition.step_id,
            attempt=attempt,
            succeeded=True,
        )
        history_record = WorkflowTransitionRecord(
            sequence=len(current.history) + 1,
            transition_id=transition.transition_id,
            from_state=transition.from_state,
            to_state=transition.to_state,
            step_id=transition.step_id,
            attempt=attempt,
        )
        next_status = (
            WorkflowStatus.COMPLETED
            if not self._outgoing(transition.to_state)
            else WorkflowStatus.RUNNING
        )
        updated = replace(
            current,
            status=next_status,
            current_state=transition.to_state,
            completed_step_ids=(*current.completed_step_ids, transition.step_id),
            attempts=(*current.attempts, attempt_record),
            history=(*current.history, history_record),
            last_error=None,
        )
        self._emit(
            WorkflowEvent(
                event_type="workflow.transition",
                workflow_id=updated.workflow_id,
                from_status=current.status,
                to_status=updated.status,
                current_state=updated.current_state,
                transition_id=transition.transition_id,
                step_id=transition.step_id,
                attempt=attempt,
            )
        )
        return updated

    def _record_failure(
        self,
        state: WorkflowState,
        transition: WorkflowTransition,
        error_code: str | None,
    ) -> WorkflowState:
        current = self._validated_state(state)
        attempt = self._attempt_count(current, transition.step_id) + 1
        if attempt > self._attempt_limit(transition.step_id):
            return self._fail_state(current, _RETRY_BOUND_EXHAUSTED, transition.step_id)
        attempt_record = StepAttempt(
            step_id=transition.step_id,
            attempt=attempt,
            succeeded=False,
            error_code=error_code,
        )
        updated = replace(
            current,
            status=WorkflowStatus.RUNNING,
            attempts=(*current.attempts, attempt_record),
            last_error=error_code,
        )
        self._emit(
            WorkflowEvent(
                event_type="workflow.step_retry",
                workflow_id=updated.workflow_id,
                from_status=current.status,
                to_status=updated.status,
                current_state=updated.current_state,
                transition_id=transition.transition_id,
                step_id=transition.step_id,
                attempt=attempt,
                reason=error_code,
            )
        )
        if attempt >= self._attempt_limit(transition.step_id):
            return self._fail_state(updated, error_code or _STEP_FAILED, transition.step_id)
        return updated

    def _fail_state(
        self,
        state: WorkflowState,
        reason: str,
        step_id: str,
    ) -> WorkflowState:
        failed = replace(
            self._validated_state(state),
            status=WorkflowStatus.FAILED,
            last_error=reason,
        )
        self._emit(
            WorkflowEvent(
                event_type="workflow.failed",
                workflow_id=failed.workflow_id,
                from_status=state.status,
                to_status=WorkflowStatus.FAILED,
                current_state=failed.current_state,
                step_id=step_id,
                reason=reason,
            )
        )
        return failed

    def _complete_if_terminal(self, state: WorkflowState) -> WorkflowState:
        current = self._validated_state(state)
        if current.status in {
            WorkflowStatus.COMPLETED,
            WorkflowStatus.FAILED,
            WorkflowStatus.CANCELLED,
        }:
            return current
        completed = replace(current, status=WorkflowStatus.COMPLETED, last_error=None)
        self._emit(
            WorkflowEvent(
                event_type="workflow.completed",
                workflow_id=completed.workflow_id,
                from_status=current.status,
                to_status=WorkflowStatus.COMPLETED,
                current_state=completed.current_state,
            )
        )
        return completed

    def _resolve_checkpoint(
        self,
        state: WorkflowState | None,
        approval_id: str | None,
        status: ApprovalStatus,
        *,
        state_digest: str | None,
        action_digest: str | None,
        actor_id: str,
        reason: str,
        now: datetime | None,
    ) -> WorkflowState:
        current = self._require_state(state)
        pending = current.pending_approval
        if pending is None:
            raise WorkflowApprovalError("workflow has no pending checkpoint")
        if approval_id is not None and approval_id != pending.approval_id:
            raise WorkflowApprovalError("approval_id does not match the pending checkpoint")
        if pending.status is not ApprovalStatus.PENDING:
            raise WorkflowApprovalError(f"checkpoint is already {pending.status.value}")
        if _supplied_action_key(pending, state_digest, action_digest) != (
            pending.state_digest,
            pending.action_digest,
        ):
            return self._invalidate_approval(current, _STALE_APPROVAL)
        resolved = replace(
            pending,
            status=status,
            actor_id=_safe_identifier(actor_id, "actor_id"),
            decided_at=self._normalize_now(now),
            reason=reason,
        )
        next_status = (
            WorkflowStatus.CANCELLED if status is ApprovalStatus.ABORTED else WorkflowStatus.FAILED
        )
        updated = replace(
            current,
            status=next_status,
            pending_approval=None,
            last_error=reason,
        )
        self._emit(
            WorkflowEvent(
                event_type="workflow.approval_resolved",
                workflow_id=updated.workflow_id,
                from_status=current.status,
                to_status=updated.status,
                current_state=updated.current_state,
                transition_id=pending.transition_id,
                step_id=pending.step_id,
                reason=reason,
                approval=resolved,
            )
        )
        return self._store(updated)

    def _invalidate_approval(self, state: WorkflowState, reason: str) -> WorkflowState:
        pending = state.pending_approval
        if pending is None:
            return state
        invalidated = replace(
            pending,
            status=ApprovalStatus.INVALIDATED,
            actor_id=pending.actor_id or "local-user",
            decided_at=pending.decided_at or self._normalize_now(None),
            reason=reason,
        )
        updated = replace(
            state,
            status=WorkflowStatus.WAITING_FOR_APPROVAL,
            pending_approval=invalidated,
            last_error=reason,
        )
        self._emit(
            WorkflowEvent(
                event_type="workflow.approval_invalidated",
                workflow_id=updated.workflow_id,
                from_status=state.status,
                to_status=updated.status,
                current_state=updated.current_state,
                transition_id=pending.transition_id,
                step_id=pending.step_id,
                reason=reason,
                approval=invalidated,
            )
        )
        return self._store(updated)

    def _make_approval(
        self,
        state: WorkflowState,
        transition: WorkflowTransition,
    ) -> WorkflowApproval:
        state_digest = self._state_digest(state)
        action_digest = self._action_digest(transition, state_digest)
        approval_id = f"approval-{state.workflow_id}-{transition.transition_id}"
        checkpoint_type = transition.checkpoint_type
        if checkpoint_type is None:
            raise WorkflowTransitionError("transition does not declare a checkpoint type")
        return WorkflowApproval(
            approval_id=approval_id,
            transition_id=transition.transition_id,
            step_id=transition.step_id,
            checkpoint_type=checkpoint_type,
            state_digest=state_digest,
            action_digest=action_digest,
        )

    def _approval_matches(self, state: WorkflowState, approval: WorkflowApproval) -> bool:
        try:
            transition = self._get_transition(approval.transition_id)
        except WorkflowTransitionError:
            return False
        if transition.from_state != state.current_state:
            return False
        current_state_digest = self._state_digest(state)
        expected_action_digest = self._action_digest(transition, current_state_digest)
        return (
            approval.state_digest == current_state_digest
            and approval.action_digest == expected_action_digest
        )

    def _action_digest(self, transition: WorkflowTransition, state_digest: str) -> str:
        action_payload = {
            "definition_id": self._definition.definition_id,
            "definition_version": self._definition.version,
            "from_state": transition.from_state,
            "to_state": transition.to_state,
            "step_id": transition.step_id,
            "state_digest": state_digest,
        }
        return _digest(_canonical(action_payload))

    def _state_digest(self, state: WorkflowState) -> str:
        payload = {
            "workflow_id": state.workflow_id,
            "definition_id": state.definition_id,
            "definition_version": state.definition_version,
            "current_state": state.current_state,
            "completed_step_ids": list(state.completed_step_ids),
            "attempts": [attempt.to_dict() for attempt in state.attempts],
            "replanning_attempts": state.replanning_attempts,
        }
        return _digest(_canonical(payload))

    def _select_transition(
        self,
        state: WorkflowState,
        transition_id: str | None,
    ) -> WorkflowTransition | None:
        outgoing = self._outgoing(state.current_state)
        if transition_id is not None:
            normalized = _safe_identifier(transition_id, "transition_id")
            for transition in outgoing:
                if transition.transition_id == normalized:
                    return transition
            raise WorkflowTransitionError("transition is not declared from the current state")
        return outgoing[0] if outgoing else None

    def _get_transition(self, transition_id: str) -> WorkflowTransition:
        normalized = _safe_identifier(transition_id, "transition_id")
        for transition in self._definition.transitions:
            if transition.transition_id == normalized:
                return transition
        raise WorkflowTransitionError("transition is not declared by the workflow")

    def _outgoing(self, state: str) -> tuple[WorkflowTransition, ...]:
        return tuple(
            transition
            for transition in self._definition.transitions
            if transition.from_state == state
        )

    def _attempt_count(self, state: WorkflowState, step_id: str) -> int:
        return sum(attempt.step_id == step_id for attempt in state.attempts)

    def _attempt_limit(self, step_id: str) -> int:
        return self._max_attempts.get(step_id, 1)

    def _validated_state(self, state: WorkflowState) -> WorkflowState:
        if not isinstance(state, WorkflowState):
            raise TypeError("state must be a WorkflowState")
        if state.definition_id != self._definition.definition_id:
            raise WorkflowTransitionError("state definition_id does not match the engine")
        if state.definition_version != self._definition.version:
            raise WorkflowTransitionError("state definition version does not match the engine")
        if state.current_state not in self._definition.states:
            raise WorkflowTransitionError("state current_state is not declared by the definition")
        return state

    def _require_state(self, state: WorkflowState | None) -> WorkflowState:
        if state is not None:
            return self._validated_state(state)
        with self._lock:
            if self._state is None:
                raise WorkflowTransitionError("workflow has not been started")
            return self._state

    def _store(self, state: WorkflowState) -> WorkflowState:
        with self._lock:
            self._state = state
        return state

    def _normalize_now(self, value: datetime | None) -> datetime:
        if value is None:
            return datetime.now(UTC)
        if value.tzinfo is None or value.utcoffset() is None:
            raise WorkflowTransitionError("checkpoint time must include a timezone")
        return value.astimezone(UTC)

    def _emit(self, event: WorkflowEvent) -> None:
        callback = self._callback
        if callback is None:
            return
        target: object
        if callable(callback):
            target = callback
        else:
            append = getattr(callback, "append", None)
            if not callable(append):
                raise TypeError("event journal callback must be callable or expose append")
            target = append
        try:
            signature = inspect.signature(cast(Callable[..., object], target))
        except (TypeError, ValueError):
            cast(Callable[[WorkflowEvent], None], target)(event)
            return
        parameters = [
            parameter
            for parameter in signature.parameters.values()
            if parameter.kind
            in (inspect.Parameter.POSITIONAL_ONLY, inspect.Parameter.POSITIONAL_OR_KEYWORD)
        ]
        has_varargs = any(
            parameter.kind is inspect.Parameter.VAR_POSITIONAL
            for parameter in signature.parameters.values()
        )
        if has_varargs or len(parameters) >= 3:
            cast(Callable[[str, str, dict[str, object]], None], target)(
                event.event_type,
                event.workflow_id,
                event.payload,
            )
        elif len(parameters) == 2:
            cast(Callable[[str, dict[str, object]], None], target)(
                event.event_type,
                event.payload,
            )
        else:
            cast(Callable[[WorkflowEvent], None], target)(event)

    @staticmethod
    def _validate_max_attempts(values: Mapping[str, int]) -> dict[str, int]:
        result: dict[str, int] = {}
        for step_id, value in values.items():
            normalized_step_id = _safe_identifier(step_id, "step_id")
            if isinstance(value, bool) or not isinstance(value, int) or value < 1:
                raise WorkflowTransitionError("step retry bounds must be positive integers")
            result[normalized_step_id] = value
        return result


WorkflowCheckpoint = WorkflowApproval
WorkflowStateStore = WorkflowState
WorkflowEventCallback: TypeAlias = EventJournalCallback


__all__ = (
    "EventJournalCallback",
    "StepAttempt",
    "WorkflowApproval",
    "WorkflowApprovalError",
    "WorkflowCheckpoint",
    "WorkflowCheckpointStatus",
    "WorkflowEngine",
    "WorkflowError",
    "WorkflowEvent",
    "WorkflowEventCallback",
    "WorkflowLimitExceeded",
    "WorkflowState",
    "WorkflowStateStore",
    "WorkflowTransitionError",
)
