"""Request and response models for the local Kala HTTP adapter.

Every model here is a Pydantic v2 model. Requests are strict and bounded: extra
fields are refused, string and collection sizes are capped, and numeric values
must be finite. Responses mirror the frozen application views so that the HTTP
surface can never widen, rename, or reinterpret what the composition root
produced.
"""

from __future__ import annotations

from enum import StrEnum
from typing import Annotated, Final

from pydantic import BaseModel, ConfigDict, Field, field_validator

from kala_agent.api.envelope import (
    MAX_ERROR_DETAILS as MAX_ERROR_DETAILS,
)
from kala_agent.api.envelope import (
    MAX_ERROR_MESSAGE_LENGTH as MAX_ERROR_MESSAGE_LENGTH,
)
from kala_agent.api.envelope import (
    ErrorDetail,
    ErrorResponse,
    error_response,
)
from kala_agent.application import (
    CheckpointView,
    DomainEvaluationReport,
    DomainPackageView,
    MechanicalRunResult,
    RevisionRecord,
    WorkflowDefinitionView,
    WorkflowStateView,
)
from kala_agent.contracts import ApprovalRequest
from kala_agent.tools.protocol import JsonEnvelopeError, canonical_json_dumps

MAX_PARAMETERS_BYTES: Final[int] = 16_384
MAX_PARAMETERS_ENTRIES: Final[int] = 256
MAX_PARAMETER_KEY_LENGTH: Final[int] = 64
MAX_SUMMARY_LENGTH: Final[int] = 512
MAX_REASON_LENGTH: Final[int] = 512
MAX_ACTOR_LENGTH: Final[int] = 64
MAX_DESIGN_ID_LENGTH: Final[int] = 64
MAX_IDENTIFIER_LENGTH: Final[int] = 128
MAX_LIST_LIMIT: Final[int] = 200
DEFAULT_LIST_LIMIT: Final[int] = 50
MAX_AFTER_SEQUENCE: Final[int] = 1_000_000_000
IDENTIFIER_PATTERN: Final[str] = r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$"
DESIGN_ID_PATTERN: Final[str] = r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$"
DIGEST_PATTERN: Final[str] = r"^[0-9a-f]{64}$"
SERVICE_NAME: Final[str] = "kala-agent"


class DomainName(StrEnum):
    """The registered domain packages."""

    BUILDING = "building"
    MECHANICAL = "mechanical"
    WING = "wing"


class _StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)


class _RequestModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, str_strip_whitespace=True)


def _validate_parameters(value: dict[str, object] | None) -> dict[str, object] | None:
    if value is None:
        return None
    if len(value) > MAX_PARAMETERS_ENTRIES:
        raise ValueError(f"parameters cannot declare more than {MAX_PARAMETERS_ENTRIES} entries")
    for key in value:
        if not key or len(key) > MAX_PARAMETER_KEY_LENGTH:
            raise ValueError("parameter names must be nonempty and bounded")
    try:
        canonical_json_dumps(value, max_bytes=MAX_PARAMETERS_BYTES)
    except JsonEnvelopeError as error:
        raise ValueError(str(error)) from error
    return value


ParametersField = Annotated[
    dict[str, object] | None,
    Field(
        default=None,
        description=(
            "Optional overrides for the declared domain parameters. Values must be "
            "finite JSON and the whole object is size bounded."
        ),
    ),
]


class EvaluateDomainRequest(_RequestModel):
    """Ask for one deterministic reference design evaluation."""

    design_id: Annotated[
        str | None,
        Field(default=None, max_length=MAX_DESIGN_ID_LENGTH, pattern=DESIGN_ID_PATTERN),
    ] = None
    parameters: ParametersField = None

    @field_validator("parameters")
    @classmethod
    def check_parameters(cls, value: dict[str, object] | None) -> dict[str, object] | None:
        return _validate_parameters(value)


class CreateRevisionRequest(_RequestModel):
    """Ask for one immutable, content-addressed design revision.

    The write is prepared first and committed against a pre-commit checkpoint
    bound to the digests of the state it commits, so this request may also carry
    the digests a caller prepared and reviewed earlier; when it does, they must
    still describe the state now or nothing is written.
    """

    design_id: Annotated[
        str,
        Field(
            min_length=1,
            max_length=MAX_DESIGN_ID_LENGTH,
            pattern=DESIGN_ID_PATTERN,
            description="Stable design identifier the revision belongs to.",
        ),
    ]
    domain: DomainName
    summary: Annotated[
        str | None,
        Field(default=None, max_length=MAX_SUMMARY_LENGTH),
    ] = None
    parent_revision_id: Annotated[
        str | None,
        Field(default=None, max_length=MAX_IDENTIFIER_LENGTH, pattern=IDENTIFIER_PATTERN),
    ] = None
    parameters: ParametersField = None
    state_digest: Annotated[
        str | None,
        Field(default=None, pattern=DIGEST_PATTERN),
    ] = None
    action_digest: Annotated[
        str | None,
        Field(default=None, pattern=DIGEST_PATTERN),
    ] = None

    @field_validator("parameters")
    @classmethod
    def check_parameters(cls, value: dict[str, object] | None) -> dict[str, object] | None:
        return _validate_parameters(value)


class PrepareRevisionRequest(_RequestModel):
    """Ask for one prepared revision commit, and for nothing to be written.

    The digests a caller wants to review before approving belong on the commit
    that follows, not on the preparation, so they are refused here rather than
    read and dropped.
    """

    design_id: Annotated[
        str,
        Field(
            min_length=1,
            max_length=MAX_DESIGN_ID_LENGTH,
            pattern=DESIGN_ID_PATTERN,
            description="Stable design identifier the revision belongs to.",
        ),
    ]
    domain: DomainName
    summary: Annotated[
        str | None,
        Field(default=None, max_length=MAX_SUMMARY_LENGTH),
    ] = None
    parent_revision_id: Annotated[
        str | None,
        Field(default=None, max_length=MAX_IDENTIFIER_LENGTH, pattern=IDENTIFIER_PATTERN),
    ] = None
    parameters: ParametersField = None

    @field_validator("parameters")
    @classmethod
    def check_parameters(cls, value: dict[str, object] | None) -> dict[str, object] | None:
        return _validate_parameters(value)


class RunMechanicalWorkflowRequest(_RequestModel):
    """Start or resume the mechanical reference workflow."""

    design_id: Annotated[
        str,
        Field(
            default="mechanical-reference",
            min_length=1,
            max_length=MAX_DESIGN_ID_LENGTH,
            pattern=DESIGN_ID_PATTERN,
        ),
    ] = "mechanical-reference"
    workflow_id: Annotated[
        str | None,
        Field(default=None, max_length=MAX_IDENTIFIER_LENGTH, pattern=IDENTIFIER_PATTERN),
    ] = None
    actor_id: Annotated[
        str,
        Field(
            default="local-user",
            min_length=1,
            max_length=MAX_ACTOR_LENGTH,
            pattern=IDENTIFIER_PATTERN,
        ),
    ] = "local-user"


class CheckpointDecisionRequest(_RequestModel):
    """Record a local approve or reject decision for a pending checkpoint."""

    actor_id: Annotated[
        str,
        Field(
            default="local-user",
            min_length=1,
            max_length=MAX_ACTOR_LENGTH,
            pattern=IDENTIFIER_PATTERN,
        ),
    ] = "local-user"
    state_digest: Annotated[
        str | None,
        Field(default=None, pattern=DIGEST_PATTERN),
    ] = None
    action_digest: Annotated[
        str | None,
        Field(default=None, pattern=DIGEST_PATTERN),
    ] = None
    reason: Annotated[
        str | None,
        Field(default=None, max_length=MAX_REASON_LENGTH),
    ] = None


class HealthResponse(_StrictModel):
    """Liveness of the local process, with no credentials or endpoints echoed.

    ``offline`` and ``egress_configured`` describe the model client this process
    actually holds, so a hosted provider is reported only when one is wired.
    ``provider_endpoint_declared`` reports configuration that exists for a future
    adapter and is deliberately not a claim that this process can reach it.
    """

    status: str
    service: str
    version: str
    provider: str
    model: str
    offline: bool
    egress_configured: bool
    provider_endpoint_declared: bool
    data_dir: str
    schema_version: int
    domains: tuple[str, ...]
    disclaimer: str


class DomainListResponse(_StrictModel):
    """Every registered domain package and workflow definition."""

    count: int
    packages: tuple[DomainPackageView, ...]
    workflows: tuple[WorkflowDefinitionView, ...]
    domains: tuple[str, ...]


class EvaluationResponse(_StrictModel):
    """One deterministic reference design evaluation."""

    evaluation: DomainEvaluationReport
    evaluations: tuple[DomainEvaluationReport, ...] = ()


class RevisionListResponse(_StrictModel):
    """Every stored revision of one design, in sequence order."""

    design_id: str
    count: int
    revisions: tuple[RevisionRecord, ...]


class CheckpointListResponse(_StrictModel):
    """Every recorded policy checkpoint of one workflow."""

    workflow_id: str
    count: int
    pending: CheckpointView | None
    approvals: tuple[ApprovalRequest, ...]


class RunResponse(_StrictModel):
    """One mechanical reference run or one checkpoint decision outcome."""

    run: MechanicalRunResult
    state: WorkflowStateView
    revision: RevisionRecord | None = None
    checkpoint: CheckpointView | None = None


__all__ = (
    "DEFAULT_LIST_LIMIT",
    "MAX_AFTER_SEQUENCE",
    "MAX_LIST_LIMIT",
    "MAX_PARAMETERS_BYTES",
    "SERVICE_NAME",
    "CheckpointDecisionRequest",
    "CheckpointListResponse",
    "CreateRevisionRequest",
    "DomainListResponse",
    "DomainName",
    "ErrorDetail",
    "ErrorResponse",
    "EvaluateDomainRequest",
    "EvaluationResponse",
    "HealthResponse",
    "PrepareRevisionRequest",
    "RevisionListResponse",
    "RunMechanicalWorkflowRequest",
    "RunResponse",
    "error_response",
)
