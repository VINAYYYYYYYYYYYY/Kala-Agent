"""Local composition root for the Kala agent.

This module is the only place that knows how the field-independent kernel, the
persistence layer, the bounded tool plane, the provider-neutral model gateway,
and the declarative domain packages are wired together. It owns composition and
lifecycle, nothing else: every rule it applies is a rule of a component it
already holds.

The root is deliberately local and offline. It builds a deterministic in-process
model client, grants only the capabilities the declared domain packages need,
never opens a socket, and never substitutes a provider. Every consequential step
is journalled, every revision is immutable and content addressed, and the
mechanical reference workflow pauses at its declared pre-commit checkpoint until
a policy decision resolves it. Egress is reported from the client this root
actually holds, never from a declared setting: a provider-neutral endpoint in the
settings is configuration for a future adapter, and only a gateway the caller
constructs itself can make this process reach a hosted model.

A revision is written in two phases and never in one. Preparation derives the
commit, reduces it to a state digest over the design head, the parent, the
specification, the domain package digests, and the artifact digests, plus an
action digest over the write those digests authorize, and opens a pre-commit
checkpoint bound to both. The commit then resolves that checkpoint against the
digests recomputed from the state that exists at that moment and writes under a
transactional head compare-and-swap. A rejection, a stale approval, a revoked
capability, a fork, and a lost race all leave the design exactly as it was. The
policy engine and the budget ledger are recorded at the end of every operation
and restored at startup, so a restart resumes the approvals and the usage that
were already in force. Those records live under an aggregate the kernel reserves
for itself, a workflow identifier that names one of them is refused before any
work is done, and durable state that is missing, unverifiable, or invalid stops
the launch rather than restoring defaults.

Declared limitations, stated plainly: nothing here is CAD or CAE, not a solver,
not a building-code implementation, and not a certification of fitness. The
domain packages compute bounded arithmetic surrogates only.
"""

from __future__ import annotations

import json
import re
from collections.abc import Callable, Iterator, Mapping, Sequence
from contextlib import contextmanager
from dataclasses import dataclass
from dataclasses import replace as dataclass_replace
from datetime import UTC, datetime
from types import TracebackType
from typing import ClassVar, Final, Self, cast
from uuid import uuid4

from pydantic import AwareDatetime, BaseModel, ConfigDict, ValidationError

from kala_agent.config import Settings
from kala_agent.contracts import (
    ApprovalKind,
    ApprovalRequest,
    ApprovalStatus,
    ArtifactClassification,
    ArtifactRef,
    DesignRevision,
    DesignSpec,
    DomainPackageManifest,
    EvaluationResult,
    Provenance,
    RunBudget,
    SideEffectLevel,
    WorkflowStatus,
)
from kala_agent.domains import (
    NON_AUTHORITATIVE_FINDING,
    DomainPackage,
    EvaluationContext,
    ReferenceWorkflowRun,
    enforce_capability_request,
    reference_context,
    run_reference_workflow,
)
from kala_agent.domains.building import (
    BUILDING_PACKAGE,
)
from kala_agent.domains.building import (
    REFERENCE_PARAMETERS as BUILDING_REFERENCE_PARAMETERS,
)
from kala_agent.domains.building import (
    REFERENCE_SUBJECT_ID as BUILDING_REFERENCE_SUBJECT_ID,
)
from kala_agent.domains.building import (
    REFERENCE_WORKFLOW_STEP_IDS as BUILDING_REFERENCE_STEP_IDS,
)
from kala_agent.domains.mechanical import (
    MECHANICAL_PACKAGE,
    MECHANICAL_WORKFLOW,
)
from kala_agent.domains.mechanical import (
    REFERENCE_PARAMETERS as MECHANICAL_REFERENCE_PARAMETERS,
)
from kala_agent.domains.mechanical import (
    REFERENCE_SUBJECT_ID as MECHANICAL_REFERENCE_SUBJECT_ID,
)
from kala_agent.domains.mechanical import (
    REFERENCE_WORKFLOW_STEP_IDS as MECHANICAL_REFERENCE_STEP_IDS,
)
from kala_agent.domains.wing import (
    REFERENCE_PARAMETERS as WING_REFERENCE_PARAMETERS,
)
from kala_agent.domains.wing import (
    REFERENCE_SUBJECT_ID as WING_REFERENCE_SUBJECT_ID,
)
from kala_agent.domains.wing import (
    REFERENCE_WORKFLOW_STEP_IDS as WING_REFERENCE_STEP_IDS,
)
from kala_agent.domains.wing import (
    WING_PACKAGE,
)
from kala_agent.kernel.budget import BudgetLedger
from kala_agent.kernel.policy import (
    PolicyDecision,
    PolicyDecisionStatus,
    PolicyEngine,
    PolicyReason,
    PolicyRequest,
    PolicyRequestError,
    PolicyTransitionError,
)
from kala_agent.kernel.workflow import (
    WorkflowApproval,
    WorkflowEngine,
    WorkflowEvent,
    WorkflowState,
)
from kala_agent.persistence.artifacts import ContentAddressedArtifactStore
from kala_agent.persistence.database import Database, SnapshotStore, WorkflowSnapshot
from kala_agent.persistence.journal import Event, EventJournal
from kala_agent.persistence.repositories import (
    ApprovalRequestRepository,
    DesignRevisionRepository,
    DesignSpecRepository,
    HeadMovedError,
)
from kala_agent.runtime.agent import CatalogStep, OfflinePlanner, StepCatalog
from kala_agent.runtime.model_gateway import (
    LOCAL_PROVIDER_ID,
    LocalModelClient,
    ModelGateway,
)
from kala_agent.tools import (
    SIMULATOR_CAPABILITIES,
    SIMULATOR_TOOL_ID,
    SIMULATOR_TOOL_VERSION,
    SandboxProfile,
    ToolExecutor,
    ToolRegistry,
    register_simulator,
)
from kala_agent.tools.protocol import canonical_json_digest, canonical_json_dumps, sha256_hex

LOCAL_ACTOR_ID: Final[str] = "local-user"
DOMAIN_NAMES: Final[tuple[str, ...]] = ("building", "mechanical", "wing")
MECHANICAL_DOMAIN: Final[str] = "mechanical"
DEFAULT_MECHANICAL_DESIGN_ID: Final[str] = "mechanical-reference"
PRE_COMMIT_CAPABILITIES: Final[tuple[str, ...]] = ("artifact.write", "design.write")
APPROVAL_OPTIONS: Final[tuple[str, ...]] = ("approve", "reject")
CHECKPOINT_DECISIONS: Final[tuple[str, ...]] = ("approve", "reject")
REVISION_DECISIONS: Final[tuple[str, ...]] = ("approve", "reject")
HARD_CONSTRAINT_CODE: Final[str] = "hard_constraint_violation"
MECHANICAL_PLAN_OBJECTIVE: Final[str] = "section plate material pattern load stress margin safety"
DEFAULT_REJECT_REASON: Final[str] = "checkpoint rejected by the local operator"
MAX_IDENTIFIER_LENGTH: Final[int] = 64
MAX_DESIGN_ID_LENGTH: Final[int] = 64
MAX_SUMMARY_LENGTH: Final[int] = 512
MAX_EVENT_PAYLOAD_BYTES: Final[int] = 65_536
MAX_SPEC_ATTRIBUTES_BYTES: Final[int] = 8_192
MAX_REPORT_BYTES: Final[int] = 524_288
MAX_REFERENCE_STEPS: Final[int] = 64
MAX_ATTRIBUTE_COUNT: Final[int] = 4_096
MAX_REVISION_LIMIT: Final[int] = 1_000
MAX_EVENT_LIMIT: Final[int] = 1_000
REPORT_MEDIA_TYPE: Final[str] = "application/json"
REPORT_CLASSIFICATION: Final[ArtifactClassification] = ArtifactClassification.TEXT
ARTIFACT_SCHEME_PREFIX: Final[str] = "artifact://sha256/"
WORKFLOW_ID_PREFIX: Final[str] = "workflow-"
REVISION_COMMIT_OPERATION: Final[str] = "design-revision-commit"
REVISION_COMMIT_CHECKPOINT: Final[ApprovalKind] = ApprovalKind.PRE_COMMIT
REVISION_COMMIT_PREFIX: Final[str] = "commit-"
DIGEST_PREFIX_LENGTH: Final[int] = 16
POLICY_STATE_AGGREGATE: Final[str] = "kernel-policy-state"
BUDGET_STATE_AGGREGATE: Final[str] = "kernel-budget-state"
KERNEL_STATE_NAMESPACE: Final[str] = "kernel-state:"
RESERVED_WORKFLOW_IDS: Final[frozenset[str]] = frozenset(
    {POLICY_STATE_AGGREGATE, BUDGET_STATE_AGGREGATE}
)
PREPARED_REVISION_EVENT: Final[str] = "revision.prepared"
DECIDED_REVISION_EVENT: Final[str] = "revision.commit_decided"
COMMITTED_REVISION_EVENT: Final[str] = "revision.created"
REJECTED_REVISION_EVENT: Final[str] = "revision.commit_rejected"
CONFLICTED_REVISION_EVENT: Final[str] = "revision.commit_conflicted"
_UNDECIDABLE_POLICY_REASONS: Final[frozenset[PolicyReason]] = frozenset(
    {
        PolicyReason.ABORTED,
        PolicyReason.CAPABILITY_EXPIRED,
        PolicyReason.CAPABILITY_NOT_GRANTED,
        PolicyReason.INVALID_REQUEST,
        PolicyReason.REJECTED,
        PolicyReason.SCOPE_MISMATCH,
    }
)
_IDENTIFIER_PATTERN: Final[re.Pattern[str]] = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$")
_DIGEST_PATTERN: Final[re.Pattern[str]] = re.compile(r"^[0-9a-f]{64}$")
DEFAULT_LOCAL_MODEL_ID: Final[str] = "local-deterministic"
TRANSFORM_REFERENCE_EVALUATION: Final[str] = "reference-workflow-evaluation"
TRANSFORM_MECHANICAL_RUN: Final[str] = "mechanical-reference-workflow"


class ApplicationError(RuntimeError):
    """Base class for every failure the composition root reports."""

    code: ClassVar[str] = "application_error"

    def __init__(self, message: str) -> None:
        super().__init__(message)
        self.message = message


class UnknownDomainError(ApplicationError, ValueError):
    code: ClassVar[str] = "unknown_domain"


class InvalidIdentifierError(ApplicationError, ValueError):
    code: ClassVar[str] = "invalid_identifier"


class ReservedIdentifierError(InvalidIdentifierError):
    """Raised when an identifier names a record this process reserves for itself.

    The kernel state aggregates are the aggregates durable policy and budget state
    is stored under, and a user-supplied workflow identifier that reached one of
    them would write over the very record the next launch restores. The error is
    a distinct code so a caller can tell a reserved name from a malformed one,
    and it is raised before any work is derived, resolved, or written.
    """

    code: ClassVar[str] = "reserved_identifier"


class InvalidParametersError(ApplicationError, ValueError):
    code: ClassVar[str] = "invalid_parameters"


class UnknownRevisionError(ApplicationError, LookupError):
    code: ClassVar[str] = "unknown_revision"


class UnknownWorkflowError(ApplicationError, LookupError):
    code: ClassVar[str] = "unknown_workflow"


class ProvenanceUnavailableError(ApplicationError, LookupError):
    code: ClassVar[str] = "provenance_unavailable"


class CheckpointNotPendingError(ApplicationError, ValueError):
    code: ClassVar[str] = "checkpoint_not_pending"


class CheckpointDecisionDeniedError(ApplicationError, ValueError):
    code: ClassVar[str] = "checkpoint_denied"


class CheckpointDigestMismatchError(CheckpointDecisionDeniedError):
    code: ClassVar[str] = "checkpoint_digest_mismatch"


class RevisionCommitRejectedError(CheckpointDecisionDeniedError):
    """Raised when a revision commit was rejected; nothing was written."""

    code: ClassVar[str] = "revision_commit_rejected"


class RevisionForkRefusedError(CheckpointDecisionDeniedError):
    """Raised when a prepared commit would not extend the current design head."""

    code: ClassVar[str] = "revision_fork_refused"


class RevisionHeadMovedError(CheckpointDecisionDeniedError):
    """Raised when the design head moved between a prepared commit and its write."""

    code: ClassVar[str] = "revision_head_moved"


class UnknownPreparedRevisionError(ApplicationError, LookupError):
    code: ClassVar[str] = "unknown_prepared_revision"


class CorruptKernelStateError(ApplicationError, RuntimeError):
    """Raised when durable policy or budget state cannot be trusted."""

    code: ClassVar[str] = "corrupt_kernel_state"


class WorkflowFinishedError(ApplicationError, ValueError):
    code: ClassVar[str] = "workflow_finished"


class ClosedApplicationError(ApplicationError, RuntimeError):
    code: ClassVar[str] = "application_closed"


class _ViewModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)


class DomainPackageView(_ViewModel):
    """An inspectable summary of one registered domain package."""

    domain: str
    package_id: str
    version: str
    digest: str
    description: str
    disclaimer: str
    workflow_definition_id: str
    reference_subject_id: str
    schema_digests: tuple[str, ...]
    required_capabilities: tuple[str, ...]
    catalog_step_ids: tuple[str, ...]


class CatalogStepView(_ViewModel):
    """One declared catalog step of a domain package."""

    step_id: str
    name: str
    description: str
    input_names: tuple[str, ...]
    metric_names: tuple[str, ...]
    required_capabilities: tuple[str, ...]
    checkpoint_kind: ApprovalKind | None
    max_attempts: int


class WorkflowTransitionView(_ViewModel):
    transition_id: str
    from_state: str
    to_state: str
    step_id: str
    checkpoint_type: ApprovalKind | None


class WorkflowDefinitionView(_ViewModel):
    """A summary of one declared workflow definition."""

    domain: str
    definition_id: str
    version: str
    name: str
    description: str | None
    initial_state: str
    states: tuple[str, ...]
    available_step_ids: tuple[str, ...]
    transitions: tuple[WorkflowTransitionView, ...]
    max_replanning_attempts: int


class DomainDetail(_ViewModel):
    """One domain package with its workflow and declared catalog steps."""

    package: DomainPackageView
    workflow: WorkflowDefinitionView
    catalog_steps: tuple[CatalogStepView, ...]


class DomainEvaluationReport(_ViewModel):
    """The deterministic outcome of one reference design evaluation."""

    domain: str
    package_id: str
    package_version: str
    package_digest: str
    subject_id: str
    status: WorkflowStatus
    passed: bool
    score: float | None
    parameters_digest: str
    completed_step_ids: tuple[str, ...]
    blocked_step_id: str | None
    evaluations: tuple[EvaluationResult, ...]
    findings: tuple[str, ...]
    disclaimer: str


class StepAttemptView(_ViewModel):
    step_id: str
    attempt: int
    succeeded: bool
    error_code: str | None


class WorkflowTransitionRecordView(_ViewModel):
    sequence: int
    transition_id: str
    from_state: str
    to_state: str
    step_id: str
    attempt: int


class CheckpointView(_ViewModel):
    """A policy checkpoint awaiting or carrying a local decision."""

    approval_id: str
    transition_id: str
    step_id: str
    checkpoint_type: ApprovalKind
    state_digest: str
    action_digest: str
    status: ApprovalStatus
    actor_id: str | None
    decided_at: AwareDatetime | None
    reason: str | None


class WorkflowStateView(_ViewModel):
    """A bounded, serializable mirror of the kernel workflow state."""

    workflow_id: str
    definition_id: str
    definition_version: str
    status: WorkflowStatus
    current_state: str
    completed_step_ids: tuple[str, ...]
    attempts: tuple[StepAttemptView, ...]
    pending_approval: CheckpointView | None
    replanning_attempts: int
    history: tuple[WorkflowTransitionRecordView, ...]
    last_error: str | None


class BudgetUsageView(_ViewModel):
    model_calls: int
    tool_calls: int
    total_tokens: int
    artifact_bytes: int
    duration_seconds: float
    cost: str
    replanning_attempts: int


class RevisionRecord(_ViewModel):
    """An immutable revision together with its provenance record."""

    revision: DesignRevision
    provenance: Provenance
    artifact_digests: tuple[str, ...]


class RevisionCommitView(_ViewModel):
    """One prepared revision commit and the decision recorded against it.

    The two digests are what an approval is bound to: ``state_digest`` covers the
    design head, the parent, the specification, the domain package digests, and
    the artifact digests, and ``action_digest`` covers the write those digests
    authorize. A commit is refused unless the digests recomputed at commit time
    still equal these, so a stale approval can never write.
    """

    request_id: str
    design_id: str
    domain: str
    revision_id: str
    sequence: int
    parent_revision_id: str | None
    state_digest: str
    action_digest: str
    spec_digest: str
    parameters_digest: str
    package_digests: tuple[str, ...]
    artifact_digests: tuple[str, ...]
    workflow_id: str | None
    status: ApprovalStatus
    actor_id: str | None
    decided_at: AwareDatetime | None
    reason: str | None
    created_at: AwareDatetime
    committed: bool


class MechanicalRunResult(_ViewModel):
    """The outcome of one mechanical reference workflow run or decision."""

    workflow_id: str
    design_id: str
    status: WorkflowStatus
    state: WorkflowStateView
    evaluations: tuple[EvaluationResult, ...]
    checkpoint: CheckpointView | None
    policy_status: PolicyDecisionStatus | None
    policy_reason: str | None
    planned_task_ids: tuple[str, ...]
    started: bool
    revision: RevisionRecord | None
    budget: BudgetUsageView


class EventView(_ViewModel):
    sequence: int
    event_type: str
    aggregate_id: str
    workflow_id: str | None
    run_id: str | None
    digest: str
    occurred_at: AwareDatetime
    payload: Mapping[str, object] | None
    payload_omitted: bool


class EventPage(_ViewModel):
    events: tuple[EventView, ...]
    count: int
    after_sequence: int
    next_after_sequence: int
    payload_truncated: bool


@dataclass(frozen=True, slots=True)
class DomainBinding:
    """Everything the composition root needs to evaluate one domain package."""

    domain: str
    package: DomainPackage
    parameter_model: type[BaseModel]
    reference_parameters: BaseModel
    reference_step_ids: tuple[str, ...]
    subject_id: str

    def parameters(self, overrides: Mapping[str, object] | None) -> BaseModel:
        if overrides is None:
            return self.reference_parameters
        if not isinstance(overrides, Mapping):
            raise InvalidParametersError(f"{self.domain} parameters must be a JSON object")
        if len(overrides) > MAX_ATTRIBUTE_COUNT:
            raise InvalidParametersError(f"{self.domain} parameters exceed the bounded entry count")
        try:
            return self.parameter_model.model_validate(dict(overrides))
        except ValidationError as error:
            raise InvalidParametersError(_summarize_validation(error)) from error

    def context(self, subject_id: str, evaluated_at: datetime | None) -> EvaluationContext:
        return reference_context(subject_id, evaluated_at)


@dataclass(frozen=True, slots=True)
class _DerivedCommit:
    """One fully derived, not yet written revision commit.

    Everything an approval must be bound to is here and nothing in it depends on
    the moment the write happens, which is what lets the same derivation be
    recomputed at commit time and compared digest for digest.
    """

    design_id: str
    domain: str
    revision_id: str
    provenance_id: str
    sequence: int
    parent_revision_id: str | None
    expected_head_revision_id: str | None
    spec: DesignSpec
    artifact: ArtifactRef
    domain_package: DomainPackageManifest
    provenance_inputs: Provenance
    parameters: Mapping[str, object]
    spec_digest: str
    parameters_digest: str
    package_digests: tuple[str, ...]
    parent_provenance_ids: tuple[str, ...]
    transformation_ids: tuple[str, ...]
    workflow_id: str | None
    state_digest: str
    action_digest: str
    request_id: str

    def revision(self, created_at: datetime) -> DesignRevision:
        return DesignRevision(
            revision_id=self.revision_id,
            design_id=self.design_id,
            sequence=self.sequence,
            spec=self.spec,
            parent_revision_id=self.parent_revision_id,
            artifacts=(self.artifact,),
            domain_packages=(self.domain_package,),
            provenance_id=self.provenance_id,
            created_at=created_at,
        )

    def provenance(self, recorded_at: datetime) -> Provenance:
        return self.provenance_inputs.model_copy(update={"recorded_at": recorded_at})


def _summarize_validation(error: ValidationError) -> str:
    parts: list[str] = []
    for failure in error.errors()[:8]:
        location = ".".join(str(item) for item in failure.get("loc", ())) or "parameters"
        parts.append(f"{location}: {failure.get('msg', 'invalid value')}")
    return "; ".join(parts) or "parameters are invalid"


def _require_identifier(value: object, field_name: str, maximum: int) -> str:
    if not isinstance(value, str):
        raise InvalidIdentifierError(f"{field_name} must be a string")
    normalized = value.strip()
    if _IDENTIFIER_PATTERN.fullmatch(normalized) is None:
        raise InvalidIdentifierError(
            f"{field_name} must start with a letter or digit and contain only safe characters"
        )
    if len(normalized) > maximum:
        raise InvalidIdentifierError(f"{field_name} must be at most {maximum} characters")
    return normalized


def _require_text(value: str, field_name: str, maximum: int) -> str:
    normalized = value.strip()
    if not normalized:
        raise InvalidParametersError(f"{field_name} cannot be empty")
    if len(normalized) > maximum:
        raise InvalidParametersError(f"{field_name} must be at most {maximum} characters")
    return normalized


def _require_limit(value: int, field_name: str, maximum: int) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise InvalidParametersError(f"{field_name} must be an integer")
    if not 1 <= value <= maximum:
        raise InvalidParametersError(f"{field_name} must be between 1 and {maximum}")
    return value


def _optional_text(value: object, field_name: str, maximum: int) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise ApplicationError(f"the recorded {field_name} is not text")
    return _require_text(value, field_name, maximum)


def _identifier_sequence(value: object, field_name: str) -> tuple[str, ...]:
    if value is None:
        return ()
    if isinstance(value, (str, bytes, bytearray)) or not isinstance(value, Sequence):
        raise ApplicationError(f"the recorded {field_name} collection is unreadable")
    return tuple(_require_identifier(item, field_name, MAX_IDENTIFIER_LENGTH) for item in value)


def _require_digest(value: object, field_name: str) -> str:
    if not isinstance(value, str) or not _DIGEST_PATTERN.fullmatch(value):
        raise ApplicationError(f"the recorded {field_name} is not a digest")
    return value


def kernel_state_aggregate(aggregate: str) -> str:
    """Return the internal snapshot aggregate a kernel state record lives under.

    Kernel state is namespaced so that no user-supplied identifier can name the
    aggregate it is stored under, now or after a future aggregate is added: the
    whole ``kernel-state:`` prefix is reserved, and the bare aggregate names
    remain reserved because records written by an earlier version are still read
    back from them.
    """
    if not isinstance(aggregate, str) or not aggregate:
        raise ApplicationError("a kernel state aggregate must be a nonempty string")
    return f"{KERNEL_STATE_NAMESPACE}{aggregate}"


def _is_reserved_identifier(value: str) -> bool:
    return value in RESERVED_WORKFLOW_IDS or value.startswith(KERNEL_STATE_NAMESPACE)


def _require_workflow_identifier(value: object) -> str:
    """Return a workflow identifier that cannot name a reserved kernel record."""
    normalized = _require_identifier(value, "workflow_id", MAX_IDENTIFIER_LENGTH)
    if _is_reserved_identifier(normalized):
        raise ReservedIdentifierError(
            "workflow_id is reserved for durable kernel state; it cannot be one of "
            f"{', '.join(sorted(RESERVED_WORKFLOW_IDS))} or begin with {KERNEL_STATE_NAMESPACE}"
        )
    return normalized


def _commit_view_from_prepared(
    prepared: Mapping[str, object],
    *,
    request_id: str,
    status: ApprovalStatus,
    created_at: datetime,
    actor_id: str | None,
    decided_at: datetime | None,
    reason: str | None,
    committed: bool,
) -> RevisionCommitView:
    """Build the read view of a commit from what its preparation recorded."""
    sequence = prepared.get("sequence")
    if isinstance(sequence, bool) or not isinstance(sequence, int) or sequence < 0:
        raise ApplicationError("the recorded sequence is not a revision sequence")
    parent = prepared.get("parent_revision_id")
    workflow = prepared.get("workflow_id")
    return RevisionCommitView(
        request_id=request_id,
        design_id=_require_identifier(prepared.get("design_id"), "design_id", MAX_DESIGN_ID_LENGTH),
        domain=_require_identifier(prepared.get("domain"), "domain", MAX_IDENTIFIER_LENGTH),
        revision_id=_require_identifier(
            prepared.get("revision_id"), "revision_id", MAX_IDENTIFIER_LENGTH
        ),
        sequence=sequence,
        parent_revision_id=None if parent is None else str(parent),
        state_digest=_require_digest(prepared.get("state_digest"), "state digest"),
        action_digest=_require_digest(prepared.get("action_digest"), "action digest"),
        spec_digest=_require_digest(prepared.get("spec_digest"), "spec digest"),
        parameters_digest=_require_digest(prepared.get("parameters_digest"), "parameters digest"),
        package_digests=tuple(
            _require_digest(item, "package digest")
            for item in _identifier_sequence(prepared.get("package_digests"), "package_digest")
        ),
        artifact_digests=tuple(
            _require_digest(item, "artifact digest")
            for item in _identifier_sequence(prepared.get("artifact_digests"), "artifact_digest")
        ),
        workflow_id=None if workflow is None else str(workflow),
        status=status,
        actor_id=actor_id,
        decided_at=decided_at,
        reason=reason,
        created_at=created_at,
        committed=committed,
    )


def _commit_view(
    derived: _DerivedCommit,
    prepared: Mapping[str, object],
    *,
    status: ApprovalStatus,
    created_at: datetime,
    actor_id: str | None = None,
    decided_at: datetime | None = None,
    reason: str | None = None,
    committed: bool = False,
) -> RevisionCommitView:
    return _commit_view_from_prepared(
        prepared,
        request_id=derived.request_id,
        status=status,
        created_at=created_at,
        actor_id=actor_id,
        decided_at=decided_at,
        reason=reason,
        committed=committed,
    )


def _approval_view(approval: WorkflowApproval) -> CheckpointView:
    return CheckpointView(
        approval_id=approval.approval_id,
        transition_id=approval.transition_id,
        step_id=approval.step_id,
        checkpoint_type=approval.checkpoint_type,
        state_digest=approval.state_digest,
        action_digest=approval.action_digest,
        status=approval.status,
        actor_id=approval.actor_id,
        decided_at=approval.decided_at,
        reason=approval.reason,
    )


def _state_view(state: WorkflowState) -> WorkflowStateView:
    return WorkflowStateView(
        workflow_id=state.workflow_id,
        definition_id=state.definition_id,
        definition_version=state.definition_version,
        status=state.status,
        current_state=state.current_state,
        completed_step_ids=state.completed_step_ids,
        attempts=tuple(
            StepAttemptView(
                step_id=attempt.step_id,
                attempt=attempt.attempt,
                succeeded=attempt.succeeded,
                error_code=attempt.error_code,
            )
            for attempt in state.attempts
        ),
        pending_approval=(
            None if state.pending_approval is None else _approval_view(state.pending_approval)
        ),
        replanning_attempts=state.replanning_attempts,
        history=tuple(
            WorkflowTransitionRecordView(
                sequence=record.sequence,
                transition_id=record.transition_id,
                from_state=record.from_state,
                to_state=record.to_state,
                step_id=record.step_id,
                attempt=record.attempt,
            )
            for record in state.history
        ),
        last_error=state.last_error,
    )


def _budget_view(ledger: BudgetLedger) -> BudgetUsageView:
    usage = ledger.usage
    return BudgetUsageView(
        model_calls=usage.model_calls,
        tool_calls=usage.tool_calls,
        total_tokens=usage.total_tokens,
        artifact_bytes=usage.artifact_bytes,
        duration_seconds=usage.duration_seconds,
        cost=str(usage.cost),
        replanning_attempts=usage.replanning_attempts,
    )


def _default_capabilities() -> tuple[str, ...]:
    return tuple(sorted({*PRE_COMMIT_CAPABILITIES, *SIMULATOR_CAPABILITIES}))


def _default_domains() -> Mapping[str, DomainBinding]:
    return {
        "building": DomainBinding(
            domain="building",
            package=BUILDING_PACKAGE,
            parameter_model=BUILDING_REFERENCE_PARAMETERS.__class__,
            reference_parameters=BUILDING_REFERENCE_PARAMETERS,
            reference_step_ids=BUILDING_REFERENCE_STEP_IDS,
            subject_id=BUILDING_REFERENCE_SUBJECT_ID,
        ),
        "mechanical": DomainBinding(
            domain=MECHANICAL_DOMAIN,
            package=MECHANICAL_PACKAGE,
            parameter_model=MECHANICAL_REFERENCE_PARAMETERS.__class__,
            reference_parameters=MECHANICAL_REFERENCE_PARAMETERS,
            reference_step_ids=MECHANICAL_REFERENCE_STEP_IDS,
            subject_id=MECHANICAL_REFERENCE_SUBJECT_ID,
        ),
        "wing": DomainBinding(
            domain="wing",
            package=WING_PACKAGE,
            parameter_model=WING_REFERENCE_PARAMETERS.__class__,
            reference_parameters=WING_REFERENCE_PARAMETERS,
            reference_step_ids=WING_REFERENCE_STEP_IDS,
            subject_id=WING_REFERENCE_SUBJECT_ID,
        ),
    }


def mechanical_step_catalog() -> StepCatalog:
    """Return the declared mechanical step catalog the planner selects from."""
    return StepCatalog(
        [
            CatalogStep(
                catalog_step_id=step.step_id,
                tool_id=SIMULATOR_TOOL_ID,
                tool_version=SIMULATOR_TOOL_VERSION,
                default_arguments={"operation": "beam_volume", "write_report": False},
                required_capabilities=SIMULATOR_CAPABILITIES,
                keywords=(*step.input_names, *step.metric_names),
                intent=f"{step.name}: {step.description}",
            )
            for step in MECHANICAL_PACKAGE.steps
        ]
    )


class KalaApplication:
    """The local composition root: components, domain packages, and lifecycle.

    Every component may be supplied by a test. Anything the application builds
    itself, it also closes, unless the component was injected.
    """

    def __init__(
        self,
        settings: Settings | None = None,
        *,
        database: Database | None = None,
        artifact_store: ContentAddressedArtifactStore | None = None,
        journal: EventJournal | None = None,
        budget: BudgetLedger | None = None,
        policy: PolicyEngine | None = None,
        tool_registry: ToolRegistry | None = None,
        tool_executor: ToolExecutor | None = None,
        model_gateway: ModelGateway | None = None,
        planner: OfflinePlanner | None = None,
        domains: Mapping[str, DomainBinding] | None = None,
        clock: Callable[[], datetime] | None = None,
    ) -> None:
        resolved_settings = Settings() if settings is None else settings
        if not isinstance(resolved_settings, Settings):
            raise InvalidParametersError("settings must be a Settings instance")
        self._closed = False
        self._settings = resolved_settings
        self._clock: Callable[[], datetime] = (
            (lambda: datetime.now(UTC)) if clock is None else clock
        )
        if not callable(self._clock):
            raise InvalidParametersError("clock must be callable")

        if database is None:
            self._owns_database = True
            resolved_database = Database.from_settings(resolved_settings)
        else:
            self._owns_database = False
            resolved_database = database
        self._database: Database = resolved_database
        self._journal = EventJournal(resolved_database) if journal is None else journal
        self._artifact_store = (
            ContentAddressedArtifactStore.from_settings(resolved_settings)
            if artifact_store is None
            else artifact_store
        )
        self._snapshots = SnapshotStore(resolved_database)
        self._specs = DesignSpecRepository(resolved_database)
        self._revisions = DesignRevisionRepository(resolved_database)
        self._approvals = ApprovalRequestRepository(resolved_database)
        self._budget = BudgetLedger(RunBudget()) if budget is None else budget
        self._owns_budget = budget is None
        self._policy = PolicyEngine(_default_capabilities()) if policy is None else policy
        self._owns_policy = policy is None
        if self._owns_policy or self._owns_budget:
            self._restore_kernel_state()

        if tool_registry is None:
            self._registry = ToolRegistry()
            register_simulator(self._registry)
        else:
            self._registry = tool_registry
        if tool_executor is None:
            resolved_executor = ToolExecutor(
                self._registry,
                run_directory=resolved_settings.data_dir / "runs",
                granted_capabilities=SIMULATOR_CAPABILITIES,
                profile=SandboxProfile(
                    max_workers=resolved_settings.worker_count,
                    default_timeout_seconds=max(1.0, resolved_settings.tool_timeout_seconds),
                ),
                ledger=self._budget,
                policy=self._policy,
            )
        else:
            resolved_executor = tool_executor
        self._executor: ToolExecutor = resolved_executor

        model_id = (
            resolved_settings.model_id
            if _IDENTIFIER_PATTERN.fullmatch(resolved_settings.model_id) is not None
            else DEFAULT_LOCAL_MODEL_ID
        )
        if model_gateway is None:
            self._gateway = ModelGateway(
                LocalModelClient(model_id=model_id, provider_id=LOCAL_PROVIDER_ID),
                ledger=self._budget,
                policy=self._policy,
                clock=self._now,
            )
        else:
            self._gateway = model_gateway
        self._owns_gateway = model_gateway is None
        self._catalog = mechanical_step_catalog()
        if planner is None:
            self._planner = OfflinePlanner(
                self._catalog, fallback_step_id=MECHANICAL_REFERENCE_STEP_IDS[0]
            )
        else:
            self._planner = planner
        self._domains: dict[str, DomainBinding] = dict(
            _default_domains() if domains is None else domains
        )
        if not self._domains:
            raise InvalidParametersError("at least one domain package must be registered")

    @property
    def settings(self) -> Settings:
        """Return the validated settings this application was built from."""
        return self._settings

    @property
    def database(self) -> Database:
        """Return the open persistence database."""
        return self._database

    @property
    def journal(self) -> EventJournal:
        """Return the append-only event journal."""
        return self._journal

    @property
    def artifact_store(self) -> ContentAddressedArtifactStore:
        """Return the content-addressed artifact store."""
        return self._artifact_store

    @property
    def snapshots(self) -> SnapshotStore:
        """Return the workflow snapshot store."""
        return self._snapshots

    @property
    def design_specs(self) -> DesignSpecRepository:
        """Return the design specification repository."""
        return self._specs

    @property
    def design_revisions(self) -> DesignRevisionRepository:
        """Return the immutable design revision repository."""
        return self._revisions

    @property
    def approval_requests(self) -> ApprovalRequestRepository:
        """Return the policy checkpoint repository."""
        return self._approvals

    @property
    def budget(self) -> BudgetLedger:
        """Return the shared budget ledger."""
        return self._budget

    @property
    def policy(self) -> PolicyEngine:
        """Return the shared capability policy engine."""
        return self._policy

    @property
    def tool_registry(self) -> ToolRegistry:
        """Return the tool registry holding the declared reference tools."""
        return self._registry

    @property
    def tool_executor(self) -> ToolExecutor:
        """Return the bounded tool executor."""
        return self._executor

    @property
    def model_gateway(self) -> ModelGateway:
        """Return the provider-neutral model gateway bound to the local client."""
        return self._gateway

    @property
    def planner(self) -> OfflinePlanner:
        """Return the deterministic offline planner."""
        return self._planner

    @property
    def step_catalog(self) -> StepCatalog:
        """Return the mechanical step catalog the planner selects from."""
        return self._catalog

    @property
    def domains(self) -> tuple[str, ...]:
        """Return the registered domain names in a stable order."""
        return tuple(sorted(self._domains))

    @property
    def closed(self) -> bool:
        """Return whether this application has released its own resources."""
        return self._closed

    @property
    def offline(self) -> bool:
        """Return whether the configured model gateway can reach the network."""
        return not self._gateway.requires_egress

    @property
    def egress_configured(self) -> bool:
        """Return whether the wired model client can reach a hosted provider.

        This is a fact about the client this application actually holds, never
        about a setting. The composition root builds the deterministic local
        client unless a gateway is supplied explicitly, so a hosted path is
        reported here only when a supplied gateway is bound to a client that
        declares it needs egress.
        """
        return self._gateway.requires_egress

    @property
    def provider_endpoint_declared(self) -> bool:
        """Return whether the settings declare a provider-neutral model endpoint.

        A declared endpoint is configuration for a future adapter, not a wired
        one. It is reported as declared and never as egress, so nothing here can
        be read as a claim that this process can reach a hosted model.
        """
        return self._settings.model_endpoint is not None

    @property
    def model_gateway_supplied(self) -> bool:
        """Return whether the model gateway was constructed by the caller.

        A gateway this application built is the local client by construction, so
        this says whether the provider-neutral gateway is something the operator
        wired in rather than what the default path chose.
        """
        return not self._owns_gateway

    @property
    def provider_id(self) -> str:
        """Return the identity of the configured model provider."""
        return self._gateway.provider_id

    @property
    def model_id(self) -> str:
        """Return the identity of the configured model."""
        return self._gateway.model_id

    def list_domain_packages(self) -> tuple[DomainPackageView, ...]:
        """Return every registered domain package declaration."""
        return tuple(
            DomainPackageView(
                domain=binding.domain,
                package_id=binding.package.package_id,
                version=binding.package.version,
                digest=binding.package.digest(),
                description=binding.package.description,
                disclaimer=binding.package.disclaimer,
                workflow_definition_id=binding.package.workflow.definition_id,
                reference_subject_id=binding.subject_id,
                schema_digests=binding.package.schema_digests,
                required_capabilities=binding.package.required_capabilities,
                catalog_step_ids=binding.reference_step_ids,
            )
            for binding in self._sorted_bindings()
        )

    def list_workflow_definitions(self) -> tuple[WorkflowDefinitionView, ...]:
        """Return every registered workflow definition declaration."""
        return tuple(
            WorkflowDefinitionView(
                domain=binding.domain,
                definition_id=definition.definition_id,
                version=definition.version,
                name=definition.name,
                description=definition.description,
                initial_state=definition.initial_state,
                states=definition.states,
                available_step_ids=definition.available_step_ids,
                transitions=tuple(
                    WorkflowTransitionView(
                        transition_id=transition.transition_id,
                        from_state=transition.from_state,
                        to_state=transition.to_state,
                        step_id=transition.step_id,
                        checkpoint_type=transition.checkpoint_type,
                    )
                    for transition in definition.transitions
                ),
                max_replanning_attempts=definition.max_replanning_attempts,
            )
            for binding in self._sorted_bindings()
            for definition in (binding.package.workflow,)
        )

    def list_catalog_steps(self, domain: str) -> tuple[CatalogStepView, ...]:
        """Return the declared catalog steps of one domain package."""
        return self.describe_domain(domain).catalog_steps

    def describe_domain(self, domain: str) -> DomainDetail:
        """Return one domain package with its workflow and catalog steps."""
        binding = self._binding(domain)
        workflows = self.list_workflow_definitions()
        workflow = next(
            (
                item
                for item in workflows
                if item.definition_id == binding.package.workflow.definition_id
            ),
            None,
        )
        if workflow is None:
            raise ApplicationError("the declared workflow definition is not registered")
        return DomainDetail(
            package=DomainPackageView(
                domain=binding.domain,
                package_id=binding.package.package_id,
                version=binding.package.version,
                digest=binding.package.digest(),
                description=binding.package.description,
                disclaimer=binding.package.disclaimer,
                workflow_definition_id=binding.package.workflow.definition_id,
                reference_subject_id=binding.subject_id,
                schema_digests=binding.package.schema_digests,
                required_capabilities=binding.package.required_capabilities,
                catalog_step_ids=binding.reference_step_ids,
            ),
            workflow=workflow,
            catalog_steps=tuple(
                CatalogStepView(
                    step_id=step.step_id,
                    name=step.name,
                    description=step.description,
                    input_names=step.input_names,
                    metric_names=step.metric_names,
                    required_capabilities=step.required_capabilities,
                    checkpoint_kind=step.checkpoint_kind,
                    max_attempts=step.max_attempts,
                )
                for step in binding.package.steps
            ),
        )

    def evaluate_domain_design(
        self,
        domain: str,
        *,
        parameters: Mapping[str, object] | None = None,
        design_id: str | None = None,
    ) -> DomainEvaluationReport:
        """Evaluate one reference design of a domain package deterministically."""
        self._require_open()
        binding = self._binding(domain)
        subject_id = (
            _require_identifier(design_id, "design_id", MAX_DESIGN_ID_LENGTH)
            if design_id is not None
            else binding.subject_id
        )
        model = binding.parameters(parameters)
        report = self._evaluate(binding, subject_id, model)
        with self._unit_of_work():
            self._journal.append(
                "domain.evaluated",
                subject_id,
                {
                    "blocked_step_id": report.blocked_step_id,
                    "completed_step_ids": list(report.completed_step_ids),
                    "domain": report.domain,
                    "evaluation_count": len(report.evaluations),
                    "package_digest": report.package_digest,
                    "parameters_digest": report.parameters_digest,
                    "passed": report.passed,
                    "status": report.status.value,
                },
                workflow_id=None,
            )
        return report

    def create_design_revision(
        self,
        *,
        design_id: str,
        domain: str,
        summary: str | None = None,
        parameters: Mapping[str, object] | None = None,
        parent_revision_id: str | None = None,
        parent_provenance_ids: Sequence[str] = (),
        transformation_ids: Sequence[str] = (),
        workflow_id: str | None = None,
        actor_id: str = LOCAL_ACTOR_ID,
        state_digest: str | None = None,
        action_digest: str | None = None,
    ) -> RevisionRecord:
        """Prepare and commit one revision in a single local operator call.

        This is the local single-user flow, and it is exactly
        :meth:`prepare_design_revision` followed by
        :meth:`commit_design_revision` with this call's own approve decision:
        the digests are computed first, a pre-commit policy checkpoint is
        registered against them, the approval is resolved against those same
        digests, and only then does the write happen under a transactional
        head compare-and-swap. Nothing about the write is unconditional: a
        refusal, a revoked capability, a design head that moved, or a fork all
        leave the design untouched. A caller that wants to read the digests
        before approving calls the two phases separately, and may pass the
        digests it reviewed here to bind this call to that review.
        """
        self._require_open()
        prepared = self.prepare_design_revision(
            design_id=design_id,
            domain=domain,
            summary=summary,
            parameters=parameters,
            parent_revision_id=parent_revision_id,
            parent_provenance_ids=parent_provenance_ids,
            transformation_ids=transformation_ids,
            workflow_id=workflow_id,
        )
        return self.commit_design_revision(
            prepared.request_id,
            actor_id=actor_id,
            state_digest=state_digest,
            action_digest=action_digest,
        )

    def prepare_design_revision(
        self,
        *,
        design_id: str,
        domain: str,
        summary: str | None = None,
        parameters: Mapping[str, object] | None = None,
        parent_revision_id: str | None = None,
        parent_provenance_ids: Sequence[str] = (),
        transformation_ids: Sequence[str] = (),
        workflow_id: str | None = None,
    ) -> RevisionCommitView:
        """Derive one revision commit and open its pre-commit policy checkpoint.

        Nothing is written to the design here. The evaluation is recomputed, the
        report bytes are stored content-addressed, and the derived commit is
        reduced to two strong digests: ``state_digest`` over the design head, the
        parent, the specification, the domain package digests, and the artifact
        digests, and ``action_digest`` over the write those digests authorize.
        A pre-commit checkpoint is registered against both, and the derivation
        itself is journalled so the second phase can recompute it later.

        The returned commit is a single-use authorization: the same design, head,
        and inputs always produce the same request, and a request that already
        carries a decision is not reopened.
        """
        self._require_open()
        subject = _require_identifier(design_id, "design_id", MAX_DESIGN_ID_LENGTH)
        binding = self._binding(domain)
        workflow = None if workflow_id is None else _require_workflow_identifier(workflow_id)
        derived = self._derive_commit(
            binding,
            subject,
            summary=summary,
            parameters=parameters,
            parent_revision_id=parent_revision_id,
            parent_provenance_ids=parent_provenance_ids,
            transformation_ids=transformation_ids,
            workflow_id=workflow,
            publish=True,
        )
        request = self._commit_policy_request(derived)
        with self._unit_of_work():
            self._require_undecided_commit(derived.request_id)
            record = self._policy.get_approval(derived.request_id)
            if record is None:
                record = self._policy.request_approval(request)
            if record.status is not ApprovalStatus.PENDING:
                raise CheckpointNotPendingError(
                    f"revision commit {derived.request_id} is already {record.status.value}"
                )
            created_at = self._now()
            self._approvals.save(
                ApprovalRequest(
                    request_id=derived.request_id,
                    checkpoint_type=REVISION_COMMIT_CHECKPOINT,
                    status=ApprovalStatus.PENDING,
                    workflow_id=derived.request_id,
                    step_id=None,
                    action_digest=derived.action_digest,
                    state_digest=derived.state_digest,
                    options=APPROVAL_OPTIONS,
                    selected_option_id=None,
                    requested_capabilities=request.requested_capabilities,
                    created_at=created_at,
                )
            )
            payload = self._prepared_payload(derived, created_at=created_at)
            self._journal.append(
                PREPARED_REVISION_EVENT,
                derived.request_id,
                payload,
                workflow_id=derived.workflow_id,
            )
        return _commit_view(
            derived,
            payload,
            status=ApprovalStatus.PENDING,
            created_at=created_at,
        )

    def decide_revision_commit(
        self,
        request_id: str,
        decision: str,
        *,
        actor_id: str = LOCAL_ACTOR_ID,
        state_digest: str | None = None,
        action_digest: str | None = None,
        reason: str | None = None,
        workflow_id: str | None = None,
    ) -> RevisionCommitView:
        """Record an approve or reject decision for one prepared revision commit.

        An approval is only ever honoured against the digests recomputed here,
        so a decision prepared against a different design head, a different
        specification, or different artifact bytes is stale and writes nothing.
        A rejection is recorded and also writes nothing. Either way the decision
        is persisted, read back from the store, and journalled before the
        result is returned.

        ``workflow_id`` names the completed workflow whose approved pre-commit
        checkpoint authorizes this commit, which is how the mechanical reference
        run writes its revision. The authorizing approval is then read back from
        the store, and the actor recorded here is the actor that checkpoint named.
        """
        self._require_open()
        identifier = _require_identifier(request_id, "request_id", MAX_IDENTIFIER_LENGTH)
        normalized = _require_identifier(decision, "decision", MAX_IDENTIFIER_LENGTH)
        if normalized not in REVISION_DECISIONS:
            raise InvalidParametersError(f"decision must be one of {', '.join(REVISION_DECISIONS)}")
        actor = _require_identifier(actor_id, "actor_id", MAX_IDENTIFIER_LENGTH)
        prepared = self._prepared_commit(identifier)
        self._require_undecided_commit(identifier)
        derived = self._rederive_commit(prepared)
        if state_digest is not None and state_digest != derived.state_digest:
            raise CheckpointDigestMismatchError("the supplied state digest is stale")
        if action_digest is not None and action_digest != derived.action_digest:
            raise CheckpointDigestMismatchError("the supplied action digest is stale")
        if workflow_id is not None:
            actor = self._workflow_commit_actor(
                _require_workflow_identifier(workflow_id),
                derived,
            )
        request = self._commit_policy_request(derived)
        created_at = self._prepared_created_at(identifier)
        decision_note = (
            None if reason is None else _require_text(reason, "reason", MAX_SUMMARY_LENGTH)
        )
        with self._unit_of_work():
            self._require_undecided_commit(identifier)
            self._policy.request_approval(request)
            precheck = self._policy.decide(request)
            if precheck.reason in _UNDECIDABLE_POLICY_REASONS:
                raise CheckpointDecisionDeniedError(
                    f"the policy engine will not accept a revision commit decision: "
                    f"{precheck.reason.value}"
                )
            if normalized == "approve":
                self._approve_policy(request, (derived.state_digest, derived.action_digest), actor)
                selected_option = "approve"
                status = ApprovalStatus.APPROVED
                note = decision_note
            else:
                self._reject_policy(
                    request, (derived.state_digest, derived.action_digest), actor, decision_note
                )
                selected_option = "reject"
                status = ApprovalStatus.REJECTED
                note = decision_note or DEFAULT_REJECT_REASON
            decided_at = max(self._now(), created_at)
            self._approvals.save(
                ApprovalRequest(
                    request_id=derived.request_id,
                    checkpoint_type=REVISION_COMMIT_CHECKPOINT,
                    status=status,
                    workflow_id=derived.request_id,
                    step_id=None,
                    action_digest=derived.action_digest,
                    state_digest=derived.state_digest,
                    options=APPROVAL_OPTIONS,
                    selected_option_id=selected_option,
                    requested_capabilities=request.requested_capabilities,
                    created_at=created_at,
                    decided_at=decided_at,
                    actor_id=actor,
                )
            )
            self._require_stored_decision(derived.request_id, status, actor)
            self._journal.append(
                DECIDED_REVISION_EVENT,
                derived.request_id,
                {
                    "action_digest": derived.action_digest,
                    "actor_id": actor,
                    "design_id": derived.design_id,
                    "revision_id": derived.revision_id,
                    "selected_option": selected_option,
                    "state_digest": derived.state_digest,
                    "status": status.value,
                    "workflow_id": derived.workflow_id,
                },
                workflow_id=derived.workflow_id,
            )
            committed = False
            if status is ApprovalStatus.APPROVED:
                committed = self._write_revision(derived, actor=actor, reason=note)
            else:
                self._journal.append(
                    REJECTED_REVISION_EVENT,
                    derived.request_id,
                    {
                        "actor_id": actor,
                        "design_id": derived.design_id,
                        "reason": note,
                        "revision_id": derived.revision_id,
                    },
                    workflow_id=derived.workflow_id,
                )
        if status is ApprovalStatus.APPROVED and not committed:
            raise RevisionHeadMovedError(
                f"the head of design {derived.design_id} moved before the revision was written"
            )
        return _commit_view(
            derived,
            prepared,
            status=status,
            created_at=created_at,
            actor_id=actor,
            decided_at=decided_at,
            reason=note,
            committed=committed,
        )

    def commit_design_revision(
        self,
        request_id: str,
        *,
        actor_id: str = LOCAL_ACTOR_ID,
        state_digest: str | None = None,
        action_digest: str | None = None,
        reason: str | None = None,
        workflow_id: str | None = None,
    ) -> RevisionRecord:
        """Approve one prepared revision commit and return the stored revision.

        The record is read back out of the design store after the write, so what
        a caller receives is the persisted revision and its journalled
        provenance rather than the in-memory derivation.
        """
        decided = self.decide_revision_commit(
            request_id,
            "approve",
            actor_id=actor_id,
            state_digest=state_digest,
            action_digest=action_digest,
            reason=reason,
            workflow_id=workflow_id,
        )
        if not decided.committed:
            raise RevisionCommitRejectedError(
                f"revision commit {decided.request_id} was not approved"
            )
        return self.get_design_revision(decided.revision_id)

    def get_revision_commit(self, request_id: str) -> RevisionCommitView:
        """Return one prepared or decided revision commit as it is stored.

        The digests reported here are the ones the preparation recorded, not
        ones recomputed against the present head: a commit that has already been
        written has itself moved the head, and re-deriving it would report a
        stale preparation rather than the decision that was recorded for it.
        """
        self._require_open()
        identifier = _require_identifier(request_id, "request_id", MAX_IDENTIFIER_LENGTH)
        prepared = self._prepared_commit(identifier)
        stored = self._approvals.get(identifier)
        decided = self._policy.get_approval(identifier)
        status = ApprovalStatus.PENDING if stored is None else stored.status
        return _commit_view_from_prepared(
            prepared,
            request_id=identifier,
            status=status,
            created_at=self._prepared_created_at(identifier),
            actor_id=None if stored is None else stored.actor_id,
            decided_at=None if stored is None else stored.decided_at,
            reason=None if decided is None else decided.reason,
            committed=(
                stored is not None
                and stored.status is ApprovalStatus.APPROVED
                and self._revision_from_prepared(prepared) is not None
            ),
        )

    def list_design_revisions(
        self,
        design_id: str,
        *,
        limit: int = MAX_REVISION_LIMIT,
    ) -> tuple[RevisionRecord, ...]:
        """Return every stored revision of one design in sequence order."""
        self._require_open()
        subject = _require_identifier(design_id, "design_id", MAX_DESIGN_ID_LENGTH)
        bounded = _require_limit(limit, "limit", MAX_REVISION_LIMIT)
        return tuple(
            self._record_for(revision)
            for revision in self._revisions.list_for_design(subject, limit=bounded)
        )

    def get_design_revision(self, revision_id: str) -> RevisionRecord:
        """Return one stored revision together with its provenance record."""
        self._require_open()
        normalized = _require_identifier(revision_id, "revision_id", MAX_IDENTIFIER_LENGTH)
        revision = self._revisions.get(normalized)
        if revision is None:
            raise UnknownRevisionError(f"revision {normalized} does not exist")
        return self._record_for(revision)

    def run_mechanical_reference_workflow(
        self,
        *,
        workflow_id: str | None = None,
        design_id: str = DEFAULT_MECHANICAL_DESIGN_ID,
        actor_id: str = LOCAL_ACTOR_ID,
    ) -> MechanicalRunResult:
        """Run the mechanical reference workflow up to its policy checkpoint."""
        self._require_open()
        subject = _require_identifier(design_id, "design_id", MAX_DESIGN_ID_LENGTH)
        workflow = self._workflow_identifier(workflow_id, subject)
        actor = _require_identifier(actor_id, "actor_id", MAX_IDENTIFIER_LENGTH)
        engine = self._mechanical_engine(workflow)
        state = engine.state
        if state is not None and state.is_terminal:
            raise WorkflowFinishedError(f"workflow {workflow} already reached {state.status.value}")
        started = state is None
        checkpoint: CheckpointView | None = None
        policy_status: PolicyDecisionStatus | None = None
        policy_reason: str | None = None

        with self._unit_of_work():
            if state is None:
                self._record_plan(workflow)
                self._journal.append(
                    "workflow.started",
                    workflow,
                    {
                        "definition_id": MECHANICAL_WORKFLOW.definition_id,
                        "definition_version": MECHANICAL_WORKFLOW.version,
                        "design_id": subject,
                        "model": self.model_id,
                        "offline": self.offline,
                        "provider": self.provider_id,
                        "step_ids": list(MECHANICAL_REFERENCE_STEP_IDS),
                    },
                    workflow_id=workflow,
                )
                state = engine.start(workflow)
                self._snapshot(engine, state)

            evaluations = list(self._load_evaluations(workflow))
            executed = 0
            while not state.is_terminal and state.pending_approval is None:
                executed += 1
                if executed > MAX_REFERENCE_STEPS:
                    raise ApplicationError("the reference workflow exceeded its bounded step count")
                step_id = self._next_step_id(state)
                evaluation = self._evaluate_step(step_id, subject)
                self._journal.append(
                    "workflow.step_evaluated",
                    workflow,
                    {
                        "evaluation": evaluation.model_dump(mode="json"),
                        "passed": evaluation.passed,
                        "step_id": step_id,
                    },
                    workflow_id=workflow,
                )
                evaluations.append(evaluation)
                if evaluation.passed:
                    state = engine.run(state)
                else:
                    state = engine.record_step_failure(
                        state,
                        step_id,
                        error_code=HARD_CONSTRAINT_CODE,
                    )
                self._snapshot(engine, state)

            if state.pending_approval is not None:
                checkpoint, policy_status, policy_reason = self._register_checkpoint(
                    state, workflow, actor=actor
                )

        return MechanicalRunResult(
            workflow_id=workflow,
            design_id=subject,
            status=state.status,
            state=_state_view(state),
            evaluations=tuple(evaluations),
            checkpoint=checkpoint,
            policy_status=policy_status,
            policy_reason=policy_reason,
            planned_task_ids=self._planned_task_ids(workflow),
            started=started,
            revision=None,
            budget=_budget_view(self._budget),
        )

    def get_workflow_state(self, workflow_id: str) -> WorkflowStateView:
        """Return the current state of one stored workflow."""
        self._require_open()
        workflow = _require_workflow_identifier(workflow_id)
        state = self._load_state(workflow)
        return _state_view(state)

    def list_workflow_checkpoints(self, workflow_id: str) -> tuple[ApprovalRequest, ...]:
        """Return every recorded policy checkpoint of one workflow."""
        self._require_open()
        workflow = _require_workflow_identifier(workflow_id)
        return self._approvals.list_for_workflow(workflow)

    def approve_workflow_checkpoint(
        self,
        workflow_id: str,
        *,
        actor_id: str = LOCAL_ACTOR_ID,
        state_digest: str | None = None,
        action_digest: str | None = None,
        reason: str | None = None,
    ) -> MechanicalRunResult:
        """Approve the pending checkpoint and drive the workflow to completion."""
        return self.decide_workflow_checkpoint(
            workflow_id,
            "approve",
            actor_id=actor_id,
            state_digest=state_digest,
            action_digest=action_digest,
            reason=reason,
        )

    def reject_workflow_checkpoint(
        self,
        workflow_id: str,
        *,
        actor_id: str = LOCAL_ACTOR_ID,
        state_digest: str | None = None,
        action_digest: str | None = None,
        reason: str | None = None,
    ) -> MechanicalRunResult:
        """Reject the pending checkpoint and end the workflow as failed."""
        return self.decide_workflow_checkpoint(
            workflow_id,
            "reject",
            actor_id=actor_id,
            state_digest=state_digest,
            action_digest=action_digest,
            reason=reason,
        )

    def decide_workflow_checkpoint(
        self,
        workflow_id: str,
        decision: str,
        *,
        actor_id: str = LOCAL_ACTOR_ID,
        state_digest: str | None = None,
        action_digest: str | None = None,
        reason: str | None = None,
    ) -> MechanicalRunResult:
        """Record an approve or reject decision for the pending checkpoint.

        The mechanical reference workflow declares exactly one checkpoint, so one
        decision resolves it. The decision is written to the store, read back
        from it, and journalled before the workflow moves, so an approved
        checkpoint is never an in-memory fact only. A revision is then committed
        only when the workflow completes, and only through the two-phase
        pre-commit flow that checkpoint authorized.
        """
        self._require_open()
        workflow = _require_workflow_identifier(workflow_id)
        normalized_decision = _require_identifier(decision, "decision", MAX_IDENTIFIER_LENGTH)
        if normalized_decision not in CHECKPOINT_DECISIONS:
            raise InvalidParametersError(
                f"decision must be one of {', '.join(CHECKPOINT_DECISIONS)}"
            )
        actor = _require_identifier(actor_id, "actor_id", MAX_IDENTIFIER_LENGTH)
        engine = self._mechanical_engine(workflow)
        state = engine.state
        if state is None:
            raise UnknownWorkflowError(f"workflow {workflow} has not been started")
        if state.is_terminal:
            raise WorkflowFinishedError(f"workflow {workflow} already reached {state.status.value}")
        approval = state.pending_approval
        if approval is None or approval.status is not ApprovalStatus.PENDING:
            raise CheckpointNotPendingError(f"workflow {workflow} has no pending checkpoint")
        if state_digest is not None and state_digest != approval.state_digest:
            raise CheckpointDigestMismatchError("the supplied state digest is stale")
        if action_digest is not None and action_digest != approval.action_digest:
            raise CheckpointDigestMismatchError("the supplied action digest is stale")

        request = self._policy_request(approval, workflow)
        created_at = self._checkpoint_created_at(request.request_id)
        decision_note = (
            None if reason is None else _require_text(reason, "reason", MAX_SUMMARY_LENGTH)
        )
        with self._unit_of_work():
            record = self._policy.get_approval(request.request_id)
            if record is None:
                record = self._policy.request_approval(request)
            if record.status is not ApprovalStatus.PENDING:
                raise CheckpointDecisionDeniedError(
                    "a checkpoint decision was already recorded for this workflow"
                )
            precheck = self._policy.decide(request)
            if precheck.reason in _UNDECIDABLE_POLICY_REASONS:
                raise CheckpointDecisionDeniedError(
                    "the policy engine will not accept a checkpoint decision: "
                    f"{precheck.reason.value}"
                )
            if normalized_decision == "approve":
                policy_decision = self._approve_policy(
                    request, (approval.state_digest, approval.action_digest), actor
                )
                state = engine.approve_checkpoint(
                    state,
                    approval.approval_id,
                    state_digest=approval.state_digest,
                    action_digest=approval.action_digest,
                    actor_id=actor,
                )
                state = engine.resume(state)
                resolved_status = ApprovalStatus.APPROVED
                selected_option = "approve"
                resolved_reason = decision_note
            else:
                policy_decision = self._reject_policy(
                    request, (approval.state_digest, approval.action_digest), actor, decision_note
                )
                reject_reason = decision_note or DEFAULT_REJECT_REASON
                state = engine.reject_checkpoint(
                    state,
                    approval.approval_id,
                    state_digest=approval.state_digest,
                    action_digest=approval.action_digest,
                    actor_id=actor,
                    reason=reject_reason,
                )
                resolved_status = ApprovalStatus.REJECTED
                selected_option = "reject"
                resolved_reason = reject_reason
            self._snapshot(engine, state)

            decided_at = max(self._now(), created_at)
            resolved = dataclass_replace(
                approval,
                status=resolved_status,
                actor_id=actor,
                decided_at=decided_at,
                reason=resolved_reason,
            )
            self._approvals.save(
                ApprovalRequest(
                    request_id=request.request_id,
                    checkpoint_type=approval.checkpoint_type,
                    status=resolved_status,
                    workflow_id=workflow,
                    step_id=approval.step_id,
                    action_digest=approval.action_digest,
                    state_digest=approval.state_digest,
                    options=APPROVAL_OPTIONS,
                    selected_option_id=selected_option,
                    requested_capabilities=request.requested_capabilities,
                    created_at=created_at,
                    decided_at=decided_at,
                    actor_id=actor,
                )
            )
            self._require_stored_decision(request.request_id, resolved_status, actor)
            self._journal.append(
                "workflow.checkpoint_decided",
                workflow,
                {
                    "actor_id": actor,
                    "approval_id": approval.approval_id,
                    "policy_reason": policy_decision.reason.value,
                    "policy_status": policy_decision.status.value,
                    "selected_option": selected_option,
                    "state_digest": approval.state_digest,
                    "action_digest": approval.action_digest,
                    "status": state.status.value,
                    "step_id": approval.step_id,
                },
                workflow_id=workflow,
            )
        design_id = self._design_id_for(workflow)
        revision: RevisionRecord | None = None
        if state.status is WorkflowStatus.COMPLETED:
            revision = self._commit_workflow_revision(design_id, workflow, actor=actor)
        return MechanicalRunResult(
            workflow_id=workflow,
            design_id=design_id,
            status=state.status,
            state=_state_view(state),
            evaluations=tuple(self._load_evaluations(workflow)),
            checkpoint=_approval_view(resolved),
            policy_status=policy_decision.status,
            policy_reason=policy_decision.reason.value,
            planned_task_ids=self._planned_task_ids(workflow),
            started=False,
            revision=revision,
            budget=_budget_view(self._budget),
        )

    def list_events(
        self,
        *,
        after_sequence: int = 0,
        limit: int = 50,
        workflow_id: str | None = None,
        event_type: str | None = None,
        aggregate_id: str | None = None,
    ) -> EventPage:
        """Return a bounded page of journal events in sequence order."""
        self._require_open()
        if isinstance(after_sequence, bool) or not isinstance(after_sequence, int):
            raise InvalidParametersError("after_sequence must be an integer")
        if after_sequence < 0:
            raise InvalidParametersError("after_sequence must be nonnegative")
        bounded = _require_limit(limit, "limit", MAX_EVENT_LIMIT)
        workflow = None if workflow_id is None else _require_workflow_identifier(workflow_id)
        normalized_type = (
            None
            if event_type is None
            else _require_identifier(event_type, "event_type", MAX_IDENTIFIER_LENGTH)
        )
        aggregate = (
            None
            if aggregate_id is None
            else _require_identifier(aggregate_id, "aggregate_id", MAX_IDENTIFIER_LENGTH)
        )
        events = self._journal.read(
            event_type=normalized_type,
            aggregate_id=aggregate,
            workflow_id=workflow,
            after_sequence=after_sequence,
            limit=bounded,
        )
        views: list[EventView] = []
        truncated = False
        for event in events:
            view, omitted = _event_view(event)
            truncated = truncated or omitted
            views.append(view)
        next_sequence = views[-1].sequence if views else after_sequence
        return EventPage(
            events=tuple(views),
            count=len(views),
            after_sequence=after_sequence,
            next_after_sequence=next_sequence,
            payload_truncated=truncated,
        )

    def close(self) -> None:
        """Release every resource this application created itself.

        Every operation already recorded its kernel state in its own
        transaction; the flush here only adds usage charged outside one, such as
        tool and model calls, and the database is released even when it fails.
        """
        if self._closed:
            return
        try:
            with self._database.transaction():
                self._persist_kernel_state()
        finally:
            self._closed = True
            if (
                getattr(self, "_owns_database", False)
                and getattr(self, "_database", None) is not None
            ):
                self._database.close()

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.close()

    def _binding(self, domain: str) -> DomainBinding:
        self._require_open()
        normalized = _require_identifier(domain, "domain", MAX_IDENTIFIER_LENGTH)
        binding = self._domains.get(normalized)
        if binding is None:
            raise UnknownDomainError(
                f"domain {normalized} is not registered; declared domains are "
                f"{', '.join(self.domains)}"
            )
        return binding

    def _sorted_bindings(self) -> tuple[DomainBinding, ...]:
        return tuple(self._domains[name] for name in self.domains)

    def _require_open(self) -> None:
        if self._closed:
            raise ClosedApplicationError("the application is closed")

    def _now(self) -> datetime:
        value = self._clock()
        if value.tzinfo is None or value.utcoffset() is None:
            raise InvalidParametersError("the application clock must return an aware datetime")
        return value.astimezone(UTC)

    def _evaluate(
        self,
        binding: DomainBinding,
        subject_id: str,
        model: BaseModel,
    ) -> DomainEvaluationReport:
        return _report_from_run(
            binding,
            run_reference_workflow(
                binding.package,
                binding.context(subject_id, None),
                model,
                binding.reference_step_ids,
            ),
            model,
            subject_id,
        )

    def _evaluate_step(self, step_id: str, subject_id: str) -> EvaluationResult:
        binding = self._binding(MECHANICAL_DOMAIN)
        enforce_capability_request(binding.package, (step_id,))
        return binding.package.evaluate(
            step_id,
            binding.context(subject_id, None),
            binding.reference_parameters,
        )

    def _next_step_id(self, state: WorkflowState) -> str:
        for transition in MECHANICAL_WORKFLOW.transitions:
            if transition.from_state == state.current_state:
                return transition.step_id
        raise WorkflowFinishedError(
            f"workflow state {state.current_state} declares no outgoing transition"
        )

    def _report_payload(
        self,
        report: DomainEvaluationReport,
        model: BaseModel,
    ) -> Mapping[str, object]:
        return {
            "blocked_step_id": report.blocked_step_id,
            "completed_step_ids": list(report.completed_step_ids),
            "disclaimer": report.disclaimer,
            "domain": report.domain,
            "evaluations": [
                evaluation.model_dump(mode="json") for evaluation in report.evaluations
            ],
            "findings": list(report.findings),
            "non_authoritative_finding": NON_AUTHORITATIVE_FINDING,
            "package_digest": report.package_digest,
            "package_id": report.package_id,
            "package_version": report.package_version,
            "parameters": model.model_dump(mode="json"),
            "passed": report.passed,
            "status": report.status.value,
            "subject_id": report.subject_id,
        }

    def _design_spec(
        self,
        binding: DomainBinding,
        subject_id: str,
        report: DomainEvaluationReport,
        model: BaseModel,
        *,
        summary: str | None,
    ) -> DesignSpec:
        attributes = {
            "domain": binding.domain,
            "package_digest": binding.package.digest(),
            "parameters_digest": report.parameters_digest,
            "passed": report.passed,
            "status": report.status.value,
            "step_count": len(report.evaluations),
        }
        attributes_json = canonical_json_dumps(attributes, max_bytes=MAX_SPEC_ATTRIBUTES_BYTES)
        resolved_summary = (
            f"{binding.package.description} Bounded surrogate reference evaluation of {subject_id}."
            if summary is None
            else _require_text(summary, "summary", MAX_SUMMARY_LENGTH)
        )
        schema_digest = (
            binding.package.schema_digests[0] if binding.package.schema_digests else None
        )
        return DesignSpec(
            id=subject_id,
            name=f"{binding.domain} reference design",
            summary=resolved_summary,
            parameter_schema_digest=schema_digest,
            attributes_json=attributes_json,
            requirement_ids=(),
            constraints=(
                NON_AUTHORITATIVE_FINDING,
                f"{binding.package.disclaimer}",
            ),
        )

    def _derive_commit(
        self,
        binding: DomainBinding,
        subject: str,
        *,
        summary: str | None,
        parameters: Mapping[str, object] | None,
        parent_revision_id: str | None,
        parent_provenance_ids: Sequence[str],
        transformation_ids: Sequence[str],
        workflow_id: str | None,
        publish: bool,
    ) -> _DerivedCommit:
        """Derive one revision commit from the current head, without writing it.

        The head is read once and every digest is computed against it, so the
        derivation is a complete description of the state the write expects to
        extend. A parent that is not that head is a fork and is refused here,
        before a checkpoint is opened for it.

        ``publish`` stores the report bytes and charges them to the budget, which
        only a preparation does. A later re-derivation recomputes the same bytes,
        takes their digest locally, and verifies that the store still holds them,
        so re-deriving a preparation costs nothing and re-writes nothing.
        """
        head = self._revisions.latest_for_design(subject)
        head_revision_id = None if head is None else head.revision_id
        if parent_revision_id is None:
            parent_revision_id = head_revision_id
        else:
            parent_revision_id = _require_identifier(
                parent_revision_id, "parent_revision_id", MAX_IDENTIFIER_LENGTH
            )
        parent: DesignRevision | None = None
        if parent_revision_id is not None:
            parent = self._revisions.get(parent_revision_id)
            if parent is None:
                raise UnknownRevisionError(f"parent revision {parent_revision_id} does not exist")
        if parent_revision_id != head_revision_id:
            raise RevisionForkRefusedError(
                f"design {subject} has a head of {head_revision_id or 'none'}; a commit must "
                f"extend that head, not {parent_revision_id or 'none'}"
            )
        parents = tuple(
            _require_identifier(value, "parent_provenance_id", MAX_IDENTIFIER_LENGTH)
            for value in parent_provenance_ids
        )
        if not parents and parent is not None and parent.provenance_id is not None:
            parents = (parent.provenance_id,)
        transformations = tuple(
            sorted(
                {
                    _require_identifier(value, "transformation_id", MAX_IDENTIFIER_LENGTH)
                    for value in (*transformation_ids, TRANSFORM_REFERENCE_EVALUATION)
                }
            )
        )
        model = binding.parameters(parameters)
        report = self._evaluate(binding, subject, model)
        sequence = 0 if head is None else head.sequence + 1
        revision_id = f"rev-{subject}-r{sequence}"
        provenance_id = f"prov-{subject}-r{sequence}"
        report_json = canonical_json_dumps(
            self._report_payload(report, model), max_bytes=MAX_REPORT_BYTES
        )
        encoded = (report_json + "\n").encode("utf-8")
        if publish:
            digest = self._artifact_store.put_bytes(encoded)
            self._budget.record_artifact_bytes(len(encoded))
        else:
            digest = sha256_hex(encoded)
            if not self._artifact_store.contains(digest):
                raise ApplicationError(
                    f"the report artifact a revision commit was prepared against is not in the "
                    f"store: {digest}"
                )
        artifact = ArtifactRef(
            artifact_id=f"artifact-{binding.domain}-report",
            uri=f"{ARTIFACT_SCHEME_PREFIX}{digest}",
            digest=digest,
            media_type=REPORT_MEDIA_TYPE,
            size_bytes=len(encoded),
            classification=REPORT_CLASSIFICATION,
            provenance_id=provenance_id,
        )
        spec = self._design_spec(binding, subject, report, model, summary=summary)
        package_digest = binding.package.digest()
        spec_digest = canonical_json_digest(spec.model_dump(mode="json"))
        provenance_inputs = Provenance(
            provenance_id=provenance_id,
            subject_ids=(subject,),
            output_artifact_ids=(artifact.artifact_id,),
            domain_package_digests=(package_digest,),
            parent_provenance_ids=parents,
            transformation_ids=transformations,
            recorded_at=datetime(1970, 1, 1, tzinfo=UTC),
        )
        state_digest = canonical_json_digest(
            {
                "artifact_digests": [artifact.digest],
                "artifact_metadata": [
                    {
                        "artifact_id": artifact.artifact_id,
                        "classification": artifact.classification.value,
                        "digest": artifact.digest,
                        "media_type": artifact.media_type,
                        "size_bytes": artifact.size_bytes,
                    }
                ],
                "design_head": (
                    None
                    if head is None
                    else {
                        "revision_id": head.revision_id,
                        "sequence": head.sequence,
                        "spec_digest": canonical_json_digest(head.spec.model_dump(mode="json")),
                    }
                ),
                "design_id": subject,
                "domain": binding.domain,
                "package_digests": [package_digest],
                "parameters_digest": report.parameters_digest,
                "parent_provenance_ids": list(parents),
                "parent_revision_id": parent_revision_id,
                "provenance_id": provenance_id,
                "spec_digest": spec_digest,
                "transformation_ids": list(transformations),
            }
        )
        action_digest = canonical_json_digest(
            {
                "capabilities": list(PRE_COMMIT_CAPABILITIES),
                "checkpoint_type": REVISION_COMMIT_CHECKPOINT.value,
                "design_id": subject,
                "domain": binding.domain,
                "expected_head_revision_id": head_revision_id,
                "operation": REVISION_COMMIT_OPERATION,
                "planned_revision_id": revision_id,
                "planned_sequence": sequence,
                "spec_digest": spec_digest,
                "state_digest": state_digest,
                "workflow_id": workflow_id,
            }
        )
        return _DerivedCommit(
            design_id=subject,
            domain=binding.domain,
            revision_id=revision_id,
            provenance_id=provenance_id,
            sequence=sequence,
            parent_revision_id=parent_revision_id,
            expected_head_revision_id=head_revision_id,
            spec=spec,
            artifact=artifact,
            domain_package=binding.package.manifest(),
            provenance_inputs=provenance_inputs,
            parameters=model.model_dump(mode="json"),
            spec_digest=spec_digest,
            parameters_digest=report.parameters_digest,
            package_digests=(package_digest,),
            parent_provenance_ids=parents,
            transformation_ids=transformations,
            workflow_id=workflow_id,
            state_digest=state_digest,
            action_digest=action_digest,
            request_id=(
                f"{REVISION_COMMIT_PREFIX}{subject}-{state_digest[:DIGEST_PREFIX_LENGTH]}"
                f"-{action_digest[:DIGEST_PREFIX_LENGTH]}"
            ),
        )

    def _commit_policy_request(self, derived: _DerivedCommit) -> PolicyRequest:
        return PolicyRequest(
            request_id=derived.request_id,
            state_digest=derived.state_digest,
            action_digest=derived.action_digest,
            requested_capabilities=PRE_COMMIT_CAPABILITIES,
            side_effect_level=SideEffectLevel.LOCAL_WRITE,
            checkpoint_type=REVISION_COMMIT_CHECKPOINT,
        )

    def _prepared_payload(
        self,
        derived: _DerivedCommit,
        *,
        created_at: datetime,
    ) -> Mapping[str, object]:
        return {
            "action_digest": derived.action_digest,
            "artifact_digests": [derived.artifact.digest],
            "created_at": created_at.isoformat(),
            "design_id": derived.design_id,
            "domain": derived.domain,
            "expected_head_revision_id": derived.expected_head_revision_id,
            "package_digests": list(derived.package_digests),
            "parameters": dict(derived.parameters),
            "parameters_digest": derived.parameters_digest,
            "parent_provenance_ids": list(derived.parent_provenance_ids),
            "parent_revision_id": derived.parent_revision_id,
            "request_id": derived.request_id,
            "revision_id": derived.revision_id,
            "sequence": derived.sequence,
            "spec": derived.spec.model_dump(mode="json"),
            "spec_digest": derived.spec_digest,
            "state_digest": derived.state_digest,
            "summary": derived.spec.summary,
            "transformation_ids": list(derived.transformation_ids),
            "workflow_id": derived.workflow_id,
        }

    def _prepared_commit(self, request_id: str) -> Mapping[str, object]:
        events = self._journal.read(
            event_type=PREPARED_REVISION_EVENT,
            aggregate_id=request_id,
            limit=1,
        )
        for event in events:
            return event.payload
        raise UnknownPreparedRevisionError(
            f"revision commit {request_id} has no recorded preparation"
        )

    def _prepared_created_at(self, request_id: str) -> datetime:
        prepared = self._prepared_commit(request_id)
        stored = self._approvals.get(request_id)
        if stored is not None:
            return stored.created_at
        recorded = prepared.get("created_at")
        if isinstance(recorded, str):
            return self._parse_timestamp(recorded, "created_at")
        raise UnknownPreparedRevisionError(
            f"revision commit {request_id} has no recorded creation time"
        )

    def _parse_timestamp(self, value: str, field_name: str) -> datetime:
        try:
            parsed = datetime.fromisoformat(value)
        except ValueError as error:
            raise ApplicationError(f"the recorded {field_name} is not a timestamp") from error
        if parsed.tzinfo is None or parsed.utcoffset() is None:
            raise ApplicationError(f"the recorded {field_name} has no timezone")
        return parsed.astimezone(UTC)

    def _rederive_commit(self, prepared: Mapping[str, object]) -> _DerivedCommit:
        """Recompute a prepared commit from the state that exists right now.

        Every input is taken from the journalled preparation and every digest is
        recomputed from scratch, so a preparation that no longer matches the
        stored head, specification, packages, or artifact bytes produces a
        different request identity and the caller is told the approval is stale
        instead of committing what it reviewed something else.
        """
        design_id = _require_identifier(
            prepared.get("design_id"), "design_id", MAX_DESIGN_ID_LENGTH
        )
        domain = _require_identifier(prepared.get("domain"), "domain", MAX_IDENTIFIER_LENGTH)
        binding = self._binding(domain)
        parameters = prepared.get("parameters")
        try:
            derived = self._derive_commit(
                binding,
                design_id,
                summary=_optional_text(prepared.get("summary"), "summary", MAX_SUMMARY_LENGTH),
                parameters=parameters if isinstance(parameters, Mapping) else None,
                parent_revision_id=(
                    None
                    if prepared.get("parent_revision_id") is None
                    else str(prepared["parent_revision_id"])
                ),
                parent_provenance_ids=_identifier_sequence(
                    prepared.get("parent_provenance_ids"), "parent_provenance_id"
                ),
                transformation_ids=_identifier_sequence(
                    prepared.get("transformation_ids"), "transformation_id"
                ),
                workflow_id=(
                    None if prepared.get("workflow_id") is None else str(prepared["workflow_id"])
                ),
                publish=False,
            )
        except RevisionForkRefusedError as error:
            raise CheckpointDigestMismatchError(
                "the design head moved after the revision commit was prepared"
            ) from error
        if derived.request_id != prepared.get("request_id"):
            raise CheckpointDigestMismatchError(
                f"the state prepared for revision commit {prepared.get('request_id')} has changed"
            )
        return derived

    def _require_stored_decision(
        self,
        request_id: str,
        status: ApprovalStatus,
        actor: str,
    ) -> ApprovalRequest:
        stored = self._approvals.get(request_id)
        if stored is None or stored.status is not status or stored.actor_id != actor:
            raise ApplicationError(f"the decision recorded for {request_id} could not be read back")
        return stored

    def _revision_from_prepared(self, prepared: Mapping[str, object]) -> DesignRevision | None:
        revision_id = prepared.get("revision_id")
        if not isinstance(revision_id, str):
            return None
        return self._revisions.get(revision_id)

    def _require_undecided_commit(self, request_id: str) -> None:
        """Refuse a preparation that already carries a decision.

        A preparation is a single-use authorization, so a request that already
        carries a decision, in the policy engine or in the persisted store, is
        refused before anything is derived, resolved, or written.
        """
        for record in (self._policy.get_approval(request_id), self._approvals.get(request_id)):
            if record is not None and record.status is not ApprovalStatus.PENDING:
                raise CheckpointNotPendingError(
                    f"revision commit {request_id} is already {record.status.value}"
                )

    def _workflow_commit_actor(self, workflow_id: str, derived: _DerivedCommit) -> str:
        """Return the actor of the approved workflow checkpoint authorizing a commit."""
        if derived.workflow_id != workflow_id:
            raise InvalidParametersError(
                f"revision commit {derived.request_id} was not prepared for workflow {workflow_id}"
            )
        state = self._load_state(workflow_id)
        if state.status is not WorkflowStatus.COMPLETED:
            raise CheckpointDecisionDeniedError(
                f"workflow {workflow_id} has not completed, so it authorizes no revision commit"
            )
        approved = [
            record
            for record in self._approvals.list_for_workflow(workflow_id)
            if record.checkpoint_type is ApprovalKind.PRE_COMMIT
            and record.status is ApprovalStatus.APPROVED
            and record.actor_id is not None
        ]
        if len(approved) != 1:
            raise CheckpointDecisionDeniedError(
                f"workflow {workflow_id} has no single approved pre-commit checkpoint"
            )
        return approved[0].actor_id or LOCAL_ACTOR_ID

    def _commit_workflow_revision(
        self,
        design_id: str,
        workflow_id: str,
        *,
        actor: str,
    ) -> RevisionRecord:
        """Commit the revision a completed mechanical run is authorized to write.

        The run's approved checkpoint is the authorization, and it is read back
        from the store rather than taken from this call. Both the workflow
        decision and this commit pass the same policy gate, so a run whose
        checkpoint was approved is a run whose commit is approvable; a refusal
        here is therefore a conflict with another writer that moved the design
        head, and the design history stays linear either way.
        """
        prepared = self.prepare_design_revision(
            design_id=design_id,
            domain=MECHANICAL_DOMAIN,
            summary=(
                "Committed mechanical reference evaluation after the declared pre-commit "
                "checkpoint was approved."
            ),
            transformation_ids=(TRANSFORM_MECHANICAL_RUN,),
            workflow_id=workflow_id,
        )
        return self.commit_design_revision(
            prepared.request_id,
            state_digest=prepared.state_digest,
            action_digest=prepared.action_digest,
            workflow_id=workflow_id,
            actor_id=actor,
        )

    def _write_revision(
        self,
        derived: _DerivedCommit,
        *,
        actor: str,
        reason: str | None,
    ) -> bool:
        """Write the revision and its event, or journal the head conflict instead.

        A lost compare-and-swap is returned rather than raised, so the caller's
        unit of work still commits the decision and the conflict it recorded.
        """
        created_at = self._now()
        revision = derived.revision(created_at)
        provenance = derived.provenance(created_at)
        try:
            self._revisions.commit_revision(
                revision,
                expected_head_revision_id=derived.expected_head_revision_id,
            )
        except HeadMovedError as error:
            self._journal.append(
                CONFLICTED_REVISION_EVENT,
                derived.request_id,
                {
                    "actor_id": actor,
                    "design_id": derived.design_id,
                    "expected_head_revision_id": derived.expected_head_revision_id,
                    "actual_head_revision_id": error.actual_head_revision_id,
                    "planned_sequence": derived.sequence,
                    "planned_revision_id": derived.revision_id,
                },
                workflow_id=derived.workflow_id,
            )
            return False
        self._journal.append(
            COMMITTED_REVISION_EVENT,
            derived.revision_id,
            {
                "action_digest": derived.action_digest,
                "actor_id": actor,
                "artifact_digests": [derived.artifact.digest],
                "design_id": derived.design_id,
                "domain": derived.domain,
                "package_digest": derived.package_digests[0],
                "parameters_digest": derived.parameters_digest,
                "provenance": provenance.model_dump(mode="json"),
                "request_id": derived.request_id,
                "revision_id": derived.revision_id,
                "sequence": derived.sequence,
                "spec_digest": derived.spec_digest,
                "state_digest": derived.state_digest,
                "reason": reason,
            },
            workflow_id=derived.workflow_id,
        )
        return True

    @contextmanager
    def _unit_of_work(self) -> Iterator[None]:
        """Commit one operation's rows, events, and kernel state together.

        Every write an operation makes, from approval rows and revisions to
        journal events and the policy and budget state it ended with, lands in
        one database transaction, so a crash, a busy timeout, or a failed write
        leaves none of them rather than some. A unit opened inside another one
        becomes a savepoint of the outer transaction.

        A unit that fails also puts the policy engine back as it found it, so a
        decision whose rows were rolled back is not flushed later as though it
        had been recorded. Budget usage is left alone: what was charged was
        spent, and the ledger only ever undercounts by being rolled back.
        """
        policy_state = self._policy.export_state()
        try:
            with self._database.transaction():
                yield
                self._persist_kernel_state()
        except BaseException:
            self._policy.import_state(policy_state)
            raise

    def _persist_kernel_state(self) -> None:
        """Record the policy and budget state this process ended an operation with."""
        self._save_kernel_state(POLICY_STATE_AGGREGATE, self._policy.export_state())
        self._save_kernel_state(BUDGET_STATE_AGGREGATE, self._budget.export_state())

    def _save_kernel_state(self, aggregate: str, state: Mapping[str, object]) -> None:
        """Record one kernel state snapshot, treating the record as a copy.

        The identifier carries a random component because a state record is
        written by every operation and by every process that shares this data
        directory, and a name derived only from the previous record would let two
        writers pick the same one. Both the snapshot name and the aggregate it is
        filed under live in the reserved ``kernel-state:`` namespace, so no
        user-supplied workflow identifier can name either of them. A record that
        cannot be written fails the operation that produced it, because the
        record is written in that operation's transaction and nothing else of the
        operation survives without it.
        """
        self._snapshots.save(
            f"{kernel_state_aggregate(aggregate)}-{uuid4().hex[:12]}",
            kernel_state_aggregate(aggregate),
            state,
            created_at=self._now(),
        )

    def _restore_kernel_state(self) -> None:
        """Restore policy approvals and budget usage recorded by an earlier launch.

        Only the components this application built itself are restored, because a
        component a caller supplied belongs to that caller and starts as the
        caller built it. Each record is read back through the snapshot store,
        which verifies its digest, and is then handed to the component's own
        validation; a record that cannot be verified or does not validate stops
        the launch instead of silently resetting approvals or usage.

        A record that is missing altogether is a reset, and a reset is only
        explicitly safe on a data directory that holds no recorded history: the
        very first launch has nothing to restore. Once this directory holds any
        recorded event, a missing kernel state record is unexplained, and
        starting with default grants and zero usage would quietly widen authority
        and drop the budget bound, so the launch stops instead.
        """
        recorded_history = self._journal.verify_chain().event_count > 0
        for aggregate, subject, restore in (
            (POLICY_STATE_AGGREGATE, "policy", self._policy.import_state),
            (BUDGET_STATE_AGGREGATE, "budget", self._budget.import_state),
        ):
            state = self._read_kernel_state(aggregate, subject)
            if state is None:
                if recorded_history:
                    raise CorruptKernelStateError(
                        f"the recorded {subject} state is missing from a local data directory "
                        f"that already holds recorded history; the launch stops rather than "
                        f"restoring default {subject} state"
                    )
                continue
            try:
                restore(state)
            except (TypeError, ValueError) as error:
                raise CorruptKernelStateError(
                    f"the recorded {subject} state cannot be restored"
                ) from error

    def _read_kernel_state(self, aggregate: str, subject: str) -> dict[str, object] | None:
        """Return the verified recorded state of one kernel aggregate.

        The namespaced aggregate is read first, and the bare aggregate name is
        read as a fallback because a record written by an earlier version is
        still authoritative: an upgrade must not drop the approvals and usage
        that were already in force. The bare name remains reserved either way, so
        the fallback can only ever find kernel state and never a user record.
        """
        state = self._read_kernel_state_record(kernel_state_aggregate(aggregate), subject)
        if state is None:
            state = self._read_kernel_state_record(aggregate, subject)
        return state

    def _read_kernel_state_record(
        self,
        aggregate: str,
        subject: str,
    ) -> dict[str, object] | None:
        try:
            snapshot = self._snapshots.latest(aggregate)
        except ValueError as error:
            raise CorruptKernelStateError(
                f"the recorded {subject} state failed verification"
            ) from error
        if snapshot is None:
            return None
        try:
            decoded = json.loads(snapshot.state_json)
        except json.JSONDecodeError as error:
            raise CorruptKernelStateError(
                f"the recorded {subject} state is not readable"
            ) from error
        if not isinstance(decoded, dict):
            raise CorruptKernelStateError(f"the recorded {subject} state is not a JSON object")
        return cast("dict[str, object]", decoded)

    def _record_for(self, revision: DesignRevision) -> RevisionRecord:
        provenance_id = revision.provenance_id
        provenance: Provenance | None = None
        if provenance_id is not None:
            events = self._journal.read(
                event_type=COMMITTED_REVISION_EVENT,
                aggregate_id=revision.revision_id,
                limit=1,
            )
            for event in events:
                payload_provenance = event.payload.get("provenance")
                if isinstance(payload_provenance, Mapping):
                    try:
                        provenance = Provenance.model_validate(dict(payload_provenance))
                    except ValidationError as error:
                        raise ProvenanceUnavailableError(
                            "the stored provenance record is unreadable"
                        ) from error
        if provenance is None:
            raise ProvenanceUnavailableError(
                f"revision {revision.revision_id} has no recorded provenance"
            )
        return RevisionRecord(
            revision=revision,
            provenance=provenance,
            artifact_digests=tuple(artifact.digest for artifact in revision.artifacts),
        )

    def _mechanical_engine(self, workflow_id: str) -> WorkflowEngine:
        engine = WorkflowEngine(MECHANICAL_WORKFLOW, journal_callback=self._on_workflow_event)
        latest = self._snapshots.latest(_require_workflow_identifier(workflow_id))
        if latest is not None:
            engine.restore(latest.state_json)
        return engine

    def _load_state(self, workflow_id: str) -> WorkflowState:
        engine = self._mechanical_engine(workflow_id)
        state = engine.state
        if state is None:
            raise UnknownWorkflowError(f"workflow {workflow_id} has not been started")
        return state

    def _snapshot(self, engine: WorkflowEngine, state: WorkflowState) -> WorkflowSnapshot:
        subject = _require_workflow_identifier(state.workflow_id)
        latest = self._snapshots.latest(subject)
        if latest is not None and latest.state_json == state.to_json():
            return latest
        next_sequence = 1 if latest is None else latest.sequence + 1
        return self._snapshots.save(
            f"snapshot-{subject}-{next_sequence:06d}",
            subject,
            state.to_dict(),
            created_at=self._now(),
        )

    def _policy_request(self, approval: WorkflowApproval, workflow_id: str) -> PolicyRequest:
        return PolicyRequest(
            request_id=f"policy-{workflow_id}-{approval.transition_id}",
            state_digest=approval.state_digest,
            action_digest=approval.action_digest,
            requested_capabilities=PRE_COMMIT_CAPABILITIES,
            side_effect_level=SideEffectLevel.LOCAL_WRITE,
            checkpoint_type=approval.checkpoint_type,
        )

    def _approve_policy(
        self,
        request: PolicyRequest,
        digests: tuple[str, str],
        actor: str,
    ) -> PolicyDecision:
        state_digest, action_digest = digests
        try:
            decision = self._policy.approve(
                request,
                state_digest=state_digest,
                action_digest=action_digest,
                actor_id=actor,
            )
        except (PolicyRequestError, PolicyTransitionError) as error:
            raise CheckpointDecisionDeniedError(
                f"the policy engine refused the checkpoint approval: {error}"
            ) from None
        if not decision.allowed:
            raise CheckpointDecisionDeniedError(
                f"the policy engine refused the checkpoint approval: {decision.reason.value}"
            )
        return decision

    def _reject_policy(
        self,
        request: PolicyRequest,
        digests: tuple[str, str],
        actor: str,
        reason: str | None,
    ) -> PolicyDecision:
        state_digest, action_digest = digests
        try:
            return self._policy.reject(
                request,
                state_digest=state_digest,
                action_digest=action_digest,
                actor_id=actor,
                reason=reason,
            )
        except (PolicyRequestError, PolicyTransitionError) as error:
            raise CheckpointDecisionDeniedError(
                f"the policy engine refused the checkpoint rejection: {error}"
            ) from None

    def _register_checkpoint(
        self,
        state: WorkflowState,
        workflow_id: str,
        *,
        actor: str,
    ) -> tuple[CheckpointView, PolicyDecisionStatus, str]:
        approval = state.pending_approval
        if approval is None:
            raise CheckpointNotPendingError(f"workflow {workflow_id} has no pending checkpoint")
        request = self._policy_request(approval, workflow_id)
        record = self._policy.request_approval(request)
        decision = self._policy.decide(request)
        if record.status is not ApprovalStatus.PENDING:
            raise CheckpointNotPendingError(
                f"checkpoint {approval.approval_id} is already {record.status.value}"
            )
        awaiting = PolicyDecisionStatus.PENDING
        reason = (
            PolicyReason.APPROVAL_REQUIRED if decision.status is not awaiting else decision.reason
        )
        created_at = self._now()
        self._approvals.save(
            ApprovalRequest(
                request_id=request.request_id,
                checkpoint_type=approval.checkpoint_type,
                status=ApprovalStatus.PENDING,
                workflow_id=workflow_id,
                step_id=approval.step_id,
                action_digest=approval.action_digest,
                state_digest=approval.state_digest,
                options=APPROVAL_OPTIONS,
                selected_option_id=None,
                requested_capabilities=request.requested_capabilities,
                created_at=created_at,
            )
        )
        self._journal.append(
            "policy.checkpoint_requested",
            workflow_id,
            {
                "actor_id": actor,
                "approval_id": approval.approval_id,
                "capabilities": list(request.requested_capabilities),
                "policy_status": awaiting.value,
                "policy_reason": reason.value,
                "requires_approval": True,
                "state_digest": approval.state_digest,
                "action_digest": approval.action_digest,
                "step_id": approval.step_id,
                "transition_id": approval.transition_id,
            },
            workflow_id=workflow_id,
        )
        return _approval_view(approval), awaiting, reason.value

    def _checkpoint_created_at(self, request_id: str) -> datetime:
        existing = self._approvals.get(request_id)
        return self._now() if existing is None else existing.created_at

    def _design_id_for(self, workflow_id: str) -> str:
        events = self._journal.read(
            event_type="workflow.started",
            aggregate_id=workflow_id,
            limit=1,
        )
        for event in events:
            design_id = event.payload.get("design_id")
            if isinstance(design_id, str):
                return design_id
        raise UnknownWorkflowError(f"workflow {workflow_id} has no recorded design")

    def _record_plan(self, workflow_id: str) -> None:
        tasks = self._planner.plan(MECHANICAL_PLAN_OBJECTIVE)
        self._journal.append(
            "plan.created",
            workflow_id,
            {
                "objective": MECHANICAL_PLAN_OBJECTIVE,
                "planner": type(self._planner).__name__,
                "step_count": len(tasks),
                "task_ids": [task.step_id for task in tasks],
                "tool_ids": sorted({task.tool_id for task in tasks}),
            },
            workflow_id=workflow_id,
        )

    def _planned_task_ids(self, workflow_id: str) -> tuple[str, ...]:
        events = self._journal.read(
            event_type="plan.created",
            aggregate_id=workflow_id,
            workflow_id=workflow_id,
            limit=1,
        )
        for event in events:
            planned = event.payload.get("task_ids")
            if isinstance(planned, list):
                return tuple(
                    _require_identifier(value, "task_id", MAX_IDENTIFIER_LENGTH)
                    for value in planned
                )
        return ()

    def _load_evaluations(self, workflow_id: str) -> tuple[EvaluationResult, ...]:
        events = self._journal.read(
            event_type="workflow.step_evaluated",
            workflow_id=workflow_id,
            limit=MAX_EVENT_LIMIT,
        )
        evaluations: list[EvaluationResult] = []
        for event in events:
            payload = event.payload.get("evaluation")
            if not isinstance(payload, Mapping):
                continue
            try:
                evaluations.append(EvaluationResult.model_validate(dict(payload)))
            except ValidationError as error:
                raise ApplicationError("a journalled evaluation is unreadable") from error
        return tuple(evaluations)

    def _workflow_identifier(self, workflow_id: str | None, design_id: str) -> str:
        candidate = f"{WORKFLOW_ID_PREFIX}{design_id}" if workflow_id is None else workflow_id
        return _require_workflow_identifier(candidate)

    def _on_workflow_event(self, event: WorkflowEvent) -> None:
        self._journal.append(
            event.event_type,
            event.workflow_id,
            event.payload,
            workflow_id=event.workflow_id,
        )


def _report_from_run(
    binding: DomainBinding,
    run: ReferenceWorkflowRun,
    model: BaseModel,
    subject_id: str,
) -> DomainEvaluationReport:
    parameters_digest = canonical_json_digest(model.model_dump(mode="json"))
    return DomainEvaluationReport(
        domain=binding.domain,
        package_id=run.package_id,
        package_version=binding.package.version,
        package_digest=binding.package.digest(),
        subject_id=subject_id,
        status=run.status,
        passed=run.passed,
        score=run.score,
        parameters_digest=parameters_digest,
        completed_step_ids=run.completed_step_ids,
        blocked_step_id=run.blocked_step_id,
        evaluations=run.evaluations,
        findings=run.findings,
        disclaimer=binding.package.disclaimer,
    )


def _event_view(event: Event) -> tuple[EventView, bool]:
    payload = event.payload
    serialized = canonical_json_dumps(payload, max_bytes=MAX_EVENT_PAYLOAD_BYTES)
    if len(serialized.encode("utf-8")) > MAX_EVENT_PAYLOAD_BYTES:
        return (
            EventView(
                sequence=event.sequence,
                event_type=event.event_type,
                aggregate_id=event.aggregate_id,
                workflow_id=event.workflow_id,
                run_id=event.run_id,
                digest=event.digest,
                occurred_at=event.timestamp,
                payload=None,
                payload_omitted=True,
            ),
            True,
        )
    return (
        EventView(
            sequence=event.sequence,
            event_type=event.event_type,
            aggregate_id=event.aggregate_id,
            workflow_id=event.workflow_id,
            run_id=event.run_id,
            digest=event.digest,
            occurred_at=event.timestamp,
            payload=payload,
            payload_omitted=False,
        ),
        False,
    )


def artifact_digest(payload: str) -> str:
    """Return the digest an artifact holding this canonical text would carry."""
    return sha256_hex((payload + "\n").encode("utf-8"))


__all__ = (
    "APPROVAL_OPTIONS",
    "BUDGET_STATE_AGGREGATE",
    "CHECKPOINT_DECISIONS",
    "DEFAULT_MECHANICAL_DESIGN_ID",
    "DOMAIN_NAMES",
    "HARD_CONSTRAINT_CODE",
    "KERNEL_STATE_NAMESPACE",
    "LOCAL_ACTOR_ID",
    "MECHANICAL_DOMAIN",
    "MECHANICAL_PLAN_OBJECTIVE",
    "POLICY_STATE_AGGREGATE",
    "PRE_COMMIT_CAPABILITIES",
    "RESERVED_WORKFLOW_IDS",
    "REVISION_COMMIT_CHECKPOINT",
    "REVISION_COMMIT_OPERATION",
    "REVISION_DECISIONS",
    "ApplicationError",
    "BudgetUsageView",
    "CatalogStepView",
    "CheckpointDecisionDeniedError",
    "CheckpointDigestMismatchError",
    "CheckpointNotPendingError",
    "CheckpointView",
    "ClosedApplicationError",
    "CorruptKernelStateError",
    "DomainBinding",
    "DomainDetail",
    "DomainEvaluationReport",
    "DomainPackageView",
    "EventPage",
    "EventView",
    "InvalidIdentifierError",
    "InvalidParametersError",
    "KalaApplication",
    "MechanicalRunResult",
    "ProvenanceUnavailableError",
    "ReservedIdentifierError",
    "RevisionCommitRejectedError",
    "RevisionCommitView",
    "RevisionForkRefusedError",
    "RevisionHeadMovedError",
    "RevisionRecord",
    "StepAttemptView",
    "UnknownDomainError",
    "UnknownPreparedRevisionError",
    "UnknownRevisionError",
    "UnknownWorkflowError",
    "WorkflowDefinitionView",
    "WorkflowFinishedError",
    "WorkflowStateView",
    "WorkflowTransitionRecordView",
    "WorkflowTransitionView",
    "artifact_digest",
    "kernel_state_aggregate",
    "mechanical_step_catalog",
)
