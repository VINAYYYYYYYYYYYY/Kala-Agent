from __future__ import annotations

import ipaddress
import json
import math
import posixpath
import re
from decimal import Decimal, InvalidOperation
from enum import StrEnum
from typing import Annotated, ClassVar, NoReturn, Self, TypeAlias, cast
from urllib.parse import unquote, urlsplit, urlunsplit

from pydantic import (
    AfterValidator,
    AwareDatetime,
    BaseModel,
    BeforeValidator,
    ConfigDict,
    StrictBool,
    StringConstraints,
    field_validator,
    model_validator,
)

_IDENTIFIER_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$")
_MEDIA_TYPE_PATTERN = re.compile(
    r"^[a-z0-9][a-z0-9!#$&^_.+-]{0,126}/[a-z0-9][a-z0-9!#$&^_.+-]{0,126}$"
)
_DIGEST_PATTERN = re.compile(r"^[0-9a-f]{64}$")
_INVALID_PERCENT_ESCAPE = re.compile(r"%(?![0-9A-Fa-f]{2})")
_ALLOWED_ARTIFACT_SCHEMES = frozenset({"artifact", "file", "https"})
_FloatConvertible: TypeAlias = str | bytes | bytearray | int | float


def _normalize_identifier(value: str) -> str:
    normalized = value.strip()
    if _IDENTIFIER_PATTERN.fullmatch(normalized) is None:
        raise ValueError("identifier must be nonempty and contain only safe characters")
    return normalized


def _normalize_digest(value: str) -> str:
    normalized = value.strip().lower()
    if _DIGEST_PATTERN.fullmatch(normalized) is None:
        raise ValueError("digest must be a 64-character hexadecimal SHA-256 digest")
    return normalized


def _normalize_media_type(value: str) -> str:
    normalized = value.strip().lower()
    if _MEDIA_TYPE_PATTERN.fullmatch(normalized) is None:
        raise ValueError("media_type must be a valid lowercase media type")
    return normalized


def _normalize_hostname(hostname: str) -> str:
    try:
        address = ipaddress.ip_address(hostname)
    except ValueError:
        try:
            return hostname.encode("idna").decode("ascii").lower()
        except UnicodeError as error:
            raise ValueError("artifact URI host is invalid") from error
    if address.version == 6:
        return f"[{address.compressed.lower()}]"
    return address.compressed.lower()


def _normalize_artifact_uri(value: str) -> str:
    if not value or len(value) > 4096:
        raise ValueError("artifact URI must be nonempty and no longer than 4096 characters")
    if any(
        character.isspace() or ord(character) < 32 or ord(character) == 127 for character in value
    ):
        raise ValueError("artifact URI cannot contain whitespace or control characters")
    try:
        parts = urlsplit(value)
        hostname = parts.hostname
        port = parts.port
    except ValueError as error:
        raise ValueError("artifact URI is invalid") from error

    scheme = parts.scheme.lower()
    if scheme not in _ALLOWED_ARTIFACT_SCHEMES:
        raise ValueError("artifact URI scheme is not allowed")
    if parts.username is not None or parts.password is not None:
        raise ValueError("artifact URI cannot contain credentials")
    if parts.query or parts.fragment:
        raise ValueError("artifact URI cannot contain query parameters or fragments")
    if not parts.path.startswith("/"):
        raise ValueError("artifact URI must contain an absolute path")

    normalized_host = _normalize_hostname(hostname) if hostname is not None else ""
    normalized_port = port
    if scheme in {"artifact", "https"} and not normalized_host:
        raise ValueError("artifact URI must contain a host")
    if scheme == "file" and normalized_host not in {"", "localhost"}:
        raise ValueError("file artifact URI must be local")
    if scheme == "https" and normalized_port in {80, 443}:
        normalized_port = None

    if _INVALID_PERCENT_ESCAPE.search(parts.path) is not None:
        raise ValueError("artifact URI contains an invalid percent escape")
    try:
        decoded_path = unquote(parts.path, encoding="utf-8", errors="strict")
    except UnicodeError as error:
        raise ValueError("artifact URI path is not valid UTF-8") from error
    if any(ord(character) < 32 or ord(character) == 127 for character in decoded_path):
        raise ValueError("artifact URI path cannot contain control characters")
    if "\\" in decoded_path:
        raise ValueError("artifact URI path cannot contain backslashes")
    if any(segment in {".", ".."} for segment in decoded_path.split("/")):
        raise ValueError("artifact URI path cannot contain traversal segments")

    normalized_path = posixpath.normpath(parts.path)
    if normalized_path == ".":
        normalized_path = "/"
    while normalized_path.startswith("//"):
        normalized_path = normalized_path[1:]

    netloc = normalized_host
    if normalized_port is not None:
        netloc = f"{netloc}:{normalized_port}"
    if scheme == "file":
        netloc = ""
    return urlunsplit((scheme, netloc, normalized_path, "", ""))


def _reject_json_constant(value: str) -> NoReturn:
    raise ValueError(f"JSON constant {value} is not allowed")


def _parse_json_float(value: str) -> float:
    parsed = float(value)
    if not math.isfinite(parsed):
        raise ValueError("JSON numbers must be finite")
    return parsed


def _object_without_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON object keys are not allowed")
        result[key] = value
    return result


def _canonical_json_object(value: object) -> str:
    try:
        if isinstance(value, str):
            decoded: object = json.loads(
                value,
                parse_constant=_reject_json_constant,
                parse_float=_parse_json_float,
                object_pairs_hook=_object_without_duplicate_keys,
            )
        else:
            serialized = json.dumps(value, allow_nan=False, ensure_ascii=True)
            decoded = json.loads(
                serialized,
                parse_constant=_reject_json_constant,
                parse_float=_parse_json_float,
                object_pairs_hook=_object_without_duplicate_keys,
            )
    except (json.JSONDecodeError, RecursionError, TypeError, ValueError) as error:
        raise ValueError("value must be a valid JSON object") from error
    if not isinstance(decoded, dict):
        raise ValueError("value must be a JSON object")
    try:
        return json.dumps(
            decoded,
            allow_nan=False,
            ensure_ascii=True,
            separators=(",", ":"),
            sort_keys=True,
        )
    except (RecursionError, TypeError, ValueError) as error:
        raise ValueError("value must be a valid JSON object") from error


def _to_finite_nonnegative_float(value: object) -> float:
    if isinstance(value, bool):
        raise ValueError("numeric values must not be booleans")
    try:
        parsed = float(cast(_FloatConvertible, value))
    except (OverflowError, TypeError, ValueError) as error:
        raise ValueError("numeric values must be finite and nonnegative") from error
    if not math.isfinite(parsed) or parsed < 0.0:
        raise ValueError("numeric values must be finite and nonnegative")
    return parsed


def _to_unit_interval(value: object) -> float:
    parsed = _to_finite_nonnegative_float(value)
    if parsed > 1.0:
        raise ValueError("score must be between zero and one")
    return parsed


def _to_nonnegative_int(value: object) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError("integer values must be nonnegative integers")
    if value < 0:
        raise ValueError("integer values must be nonnegative")
    return value


def _to_nonnegative_decimal(value: object) -> Decimal:
    if isinstance(value, bool):
        raise ValueError("cost must be a nonnegative decimal")
    try:
        parsed = value if isinstance(value, Decimal) else Decimal(str(value))
    except (InvalidOperation, TypeError, ValueError) as error:
        raise ValueError("cost must be a nonnegative decimal") from error
    if not parsed.is_finite() or parsed < 0:
        raise ValueError("cost must be finite and nonnegative")
    return parsed


def _reject_duplicate_strings(values: tuple[str, ...]) -> tuple[str, ...]:
    if len(values) != len(set(values)):
        raise ValueError("collection values must be unique")
    return values


def _canonicalize_strings(values: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(sorted(_reject_duplicate_strings(values)))


Identifier: TypeAlias = Annotated[str, AfterValidator(_normalize_identifier)]
NonEmptyText: TypeAlias = Annotated[
    str,
    StringConstraints(strip_whitespace=True, min_length=1, max_length=16_384),
]
Sha256Digest: TypeAlias = Annotated[str, AfterValidator(_normalize_digest)]
MediaType: TypeAlias = Annotated[str, AfterValidator(_normalize_media_type)]
ArtifactUri: TypeAlias = Annotated[str, AfterValidator(_normalize_artifact_uri)]
CanonicalJsonObject: TypeAlias = Annotated[str, BeforeValidator(_canonical_json_object)]
FiniteNonNegativeFloat: TypeAlias = Annotated[
    float,
    BeforeValidator(_to_finite_nonnegative_float),
]
NonNegativeInteger: TypeAlias = Annotated[int, BeforeValidator(_to_nonnegative_int)]
UnitInterval: TypeAlias = Annotated[float, BeforeValidator(_to_unit_interval)]
NonNegativeDecimal: TypeAlias = Annotated[Decimal, BeforeValidator(_to_nonnegative_decimal)]


class RequirementKind(StrEnum):
    FUNCTIONAL = "functional"
    NON_FUNCTIONAL = "non_functional"
    CONSTRAINT = "constraint"
    SAFETY = "safety"
    QUALITY = "quality"
    PERFORMANCE = "performance"


class RequirementPriority(StrEnum):
    LOW = "low"
    MEDIUM = "medium"
    HIGH = "high"
    CRITICAL = "critical"


class RequirementStatus(StrEnum):
    DRAFT = "draft"
    PROPOSED = "proposed"
    APPROVED = "approved"
    REJECTED = "rejected"
    IMPLEMENTED = "implemented"
    VERIFIED = "verified"


class SideEffectLevel(StrEnum):
    NONE = "none"
    READ = "read"
    LOCAL_WRITE = "local_write"
    NETWORK = "network"
    EGRESS = "egress"


class WorkflowStatus(StrEnum):
    CREATED = "created"
    RUNNING = "running"
    WAITING_FOR_APPROVAL = "waiting_for_approval"
    COMPLETED = "completed"
    FAILED = "failed"
    CANCELLED = "cancelled"


class StepStatus(StrEnum):
    PENDING = "pending"
    RUNNING = "running"
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    SKIPPED = "skipped"
    CANCELLED = "cancelled"


class ToolRunStatus(StrEnum):
    REQUESTED = "requested"
    RUNNING = "running"
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    DENIED = "denied"
    TIMED_OUT = "timed_out"
    CANCELLED = "cancelled"


class ApprovalKind(StrEnum):
    PLAN_APPROVAL = "plan_approval"
    TOOL_CLASS_CHANGE = "tool_class_change"
    EGRESS = "egress"
    PRE_COMMIT = "pre_commit"
    EXPORT = "export"
    PROVIDER_DEGRADED = "provider_degraded"


class ApprovalStatus(StrEnum):
    PENDING = "pending"
    APPROVED = "approved"
    REJECTED = "rejected"
    ABORTED = "aborted"
    EXPIRED = "expired"
    INVALIDATED = "invalidated"


class ArtifactClassification(StrEnum):
    METADATA = "metadata"
    TEXT = "text"
    DERIVED_DATA = "derived_data"
    DESIGN_BINARY = "design_binary"


class _ContractModel(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(
        extra="forbid",
        frozen=True,
        validate_default=True,
    )


class Requirement(_ContractModel):
    id: Identifier
    title: NonEmptyText
    kind: RequirementKind
    description: NonEmptyText | None = None
    priority: RequirementPriority = RequirementPriority.MEDIUM
    status: RequirementStatus = RequirementStatus.DRAFT
    acceptance_criteria: tuple[NonEmptyText, ...] = ()
    tags: tuple[Identifier, ...] = ()

    @field_validator("tags")
    @classmethod
    def canonicalize_tags(cls, values: tuple[Identifier, ...]) -> tuple[Identifier, ...]:
        return _canonicalize_strings(values)


class DesignSpec(_ContractModel):
    id: Identifier
    name: NonEmptyText
    summary: NonEmptyText
    parameter_schema_digest: Sha256Digest | None = None
    attributes_json: CanonicalJsonObject = "{}"
    requirement_ids: tuple[Identifier, ...] = ()
    constraints: tuple[NonEmptyText, ...] = ()

    @field_validator("requirement_ids")
    @classmethod
    def unique_requirement_ids(
        cls,
        values: tuple[Identifier, ...],
    ) -> tuple[Identifier, ...]:
        return _canonicalize_strings(values)

    @field_validator("constraints")
    @classmethod
    def unique_constraints(
        cls,
        values: tuple[NonEmptyText, ...],
    ) -> tuple[NonEmptyText, ...]:
        return _reject_duplicate_strings(values)


class DomainPackageManifest(_ContractModel):
    package_id: Identifier
    version: NonEmptyText
    digest: Sha256Digest
    description: NonEmptyText | None = None
    workflow_definition_ids: tuple[Identifier, ...] = ()
    tool_ids: tuple[Identifier, ...] = ()
    schema_digests: tuple[Sha256Digest, ...] = ()
    required_capabilities: tuple[Identifier, ...] = ()
    policy_overlay_digest: Sha256Digest | None = None
    signer_id: Identifier | None = None
    signature_digest: Sha256Digest | None = None

    @field_validator("workflow_definition_ids", "tool_ids")
    @classmethod
    def unique_package_references(
        cls,
        values: tuple[Identifier, ...],
    ) -> tuple[Identifier, ...]:
        return _canonicalize_strings(values)

    @field_validator("schema_digests")
    @classmethod
    def unique_schema_digests(
        cls,
        values: tuple[Sha256Digest, ...],
    ) -> tuple[Sha256Digest, ...]:
        return _reject_duplicate_strings(values)

    @field_validator("required_capabilities")
    @classmethod
    def canonicalize_capabilities(
        cls,
        values: tuple[Identifier, ...],
    ) -> tuple[Identifier, ...]:
        return _canonicalize_strings(values)

    @model_validator(mode="after")
    def validate_signature_pair(self) -> Self:
        if (self.signer_id is None) != (self.signature_digest is None):
            raise ValueError("signer_id and signature_digest must be provided together")
        return self


class ArtifactRef(_ContractModel):
    artifact_id: Identifier
    uri: ArtifactUri
    digest: Sha256Digest
    media_type: MediaType
    size_bytes: NonNegativeInteger
    classification: ArtifactClassification = ArtifactClassification.METADATA
    preview_uri: ArtifactUri | None = None
    provenance_id: Identifier | None = None


class DesignRevision(_ContractModel):
    revision_id: Identifier
    design_id: Identifier
    sequence: NonNegativeInteger
    spec: DesignSpec
    parent_revision_id: Identifier | None = None
    artifacts: tuple[ArtifactRef, ...] = ()
    domain_packages: tuple[DomainPackageManifest, ...] = ()
    provenance_id: Identifier | None = None
    created_at: AwareDatetime

    @field_validator("artifacts")
    @classmethod
    def unique_artifacts(
        cls,
        values: tuple[ArtifactRef, ...],
    ) -> tuple[ArtifactRef, ...]:
        artifact_ids = tuple(value.artifact_id for value in values)
        _reject_duplicate_strings(artifact_ids)
        return values

    @field_validator("domain_packages")
    @classmethod
    def unique_domain_packages(
        cls,
        values: tuple[DomainPackageManifest, ...],
    ) -> tuple[DomainPackageManifest, ...]:
        package_ids = tuple(value.package_id for value in values)
        _reject_duplicate_strings(package_ids)
        return values

    @model_validator(mode="after")
    def reject_self_parent(self) -> Self:
        if self.parent_revision_id == self.revision_id:
            raise ValueError("a revision cannot be its own parent")
        return self


class PlanStep(_ContractModel):
    step_id: Identifier
    catalog_step_id: Identifier
    intent: NonEmptyText
    parameters_json: CanonicalJsonObject = "{}"
    input_artifact_ids: tuple[Identifier, ...] = ()
    required_capabilities: tuple[Identifier, ...] = ()
    depends_on: tuple[Identifier, ...] = ()
    estimated_duration_seconds: FiniteNonNegativeFloat | None = None
    max_attempts: NonNegativeInteger = 1

    @field_validator("input_artifact_ids", "required_capabilities", "depends_on")
    @classmethod
    def canonicalize_plan_references(
        cls,
        values: tuple[Identifier, ...],
    ) -> tuple[Identifier, ...]:
        return _canonicalize_strings(values)

    @model_validator(mode="after")
    def reject_self_dependency(self) -> Self:
        if self.step_id in self.depends_on:
            raise ValueError("a plan step cannot depend on itself")
        return self


class WorkflowTransition(_ContractModel):
    transition_id: Identifier
    from_state: Identifier
    to_state: Identifier
    step_id: Identifier
    checkpoint_type: ApprovalKind | None = None


class WorkflowDefinition(_ContractModel):
    definition_id: Identifier
    version: NonEmptyText
    name: NonEmptyText
    description: NonEmptyText | None = None
    initial_state: Identifier
    states: tuple[Identifier, ...]
    transitions: tuple[WorkflowTransition, ...] = ()
    available_step_ids: tuple[Identifier, ...]
    max_replanning_attempts: NonNegativeInteger = 1

    @field_validator("states", "available_step_ids")
    @classmethod
    def canonicalize_definition_references(
        cls,
        values: tuple[Identifier, ...],
    ) -> tuple[Identifier, ...]:
        return _canonicalize_strings(values)

    @field_validator("transitions")
    @classmethod
    def unique_transitions(
        cls,
        values: tuple[WorkflowTransition, ...],
    ) -> tuple[WorkflowTransition, ...]:
        transition_ids = tuple(value.transition_id for value in values)
        _reject_duplicate_strings(transition_ids)
        edges = tuple((value.from_state, value.to_state, value.step_id) for value in values)
        if len(edges) != len(set(edges)):
            raise ValueError("workflow transitions must be unique")
        return values

    @model_validator(mode="after")
    def validate_state_machine(self) -> Self:
        if not self.states:
            raise ValueError("states cannot be empty")
        if self.initial_state not in self.states:
            raise ValueError("initial_state must be declared in states")
        state_set = set(self.states)
        step_set = set(self.available_step_ids)
        for transition in self.transitions:
            if transition.from_state not in state_set or transition.to_state not in state_set:
                raise ValueError("transitions can only reference declared states")
            if transition.step_id not in step_set:
                raise ValueError("transitions can only reference available steps")

        reachable: set[str] = {self.initial_state}
        pending: list[str] = [self.initial_state]
        while pending:
            current = pending.pop()
            for transition in self.transitions:
                if transition.from_state == current and transition.to_state not in reachable:
                    reachable.add(transition.to_state)
                    pending.append(transition.to_state)
        unreachable = state_set - reachable
        if unreachable:
            raise ValueError("all workflow states must be reachable")
        return self


class ToolDescriptor(_ContractModel):
    tool_id: Identifier
    version: NonEmptyText
    name: NonEmptyText
    description: NonEmptyText
    input_schema_digest: Sha256Digest
    output_schema_digest: Sha256Digest
    side_effect_level: SideEffectLevel
    required_capabilities: tuple[Identifier, ...] = ()
    timeout_seconds: FiniteNonNegativeFloat = 0.0
    destructive: StrictBool = False
    supports_structured_output: StrictBool = True
    idempotent: StrictBool = True

    @field_validator("required_capabilities")
    @classmethod
    def canonicalize_tool_capabilities(
        cls,
        values: tuple[Identifier, ...],
    ) -> tuple[Identifier, ...]:
        return _canonicalize_strings(values)

    @model_validator(mode="after")
    def validate_side_effects(self) -> Self:
        if self.destructive and self.side_effect_level is SideEffectLevel.NONE:
            raise ValueError("destructive tools must declare a side effect")
        return self


class ToolCall(_ContractModel):
    call_id: Identifier
    tool_id: Identifier
    tool_version: NonEmptyText
    arguments_json: CanonicalJsonObject = "{}"
    requested_capabilities: tuple[Identifier, ...] = ()
    side_effect_level: SideEffectLevel
    timeout_seconds: FiniteNonNegativeFloat
    issued_at: AwareDatetime

    @field_validator("requested_capabilities")
    @classmethod
    def canonicalize_requested_capabilities(
        cls,
        values: tuple[Identifier, ...],
    ) -> tuple[Identifier, ...]:
        return _canonicalize_strings(values)


class ToolResult(_ContractModel):
    call_id: Identifier
    tool_id: Identifier
    status: ToolRunStatus
    result_json: CanonicalJsonObject | None = None
    error_code: Identifier | None = None
    error_message: NonEmptyText | None = None
    artifacts: tuple[ArtifactRef, ...] = ()
    started_at: AwareDatetime | None = None
    finished_at: AwareDatetime | None = None
    duration_ms: FiniteNonNegativeFloat | None = None

    @field_validator("artifacts")
    @classmethod
    def unique_result_artifacts(
        cls,
        values: tuple[ArtifactRef, ...],
    ) -> tuple[ArtifactRef, ...]:
        artifact_ids = tuple(value.artifact_id for value in values)
        _reject_duplicate_strings(artifact_ids)
        return values

    @model_validator(mode="after")
    def validate_result_state(self) -> Self:
        failure_statuses = {
            ToolRunStatus.FAILED,
            ToolRunStatus.DENIED,
            ToolRunStatus.TIMED_OUT,
        }
        if self.status in failure_statuses:
            if self.error_code is None:
                raise ValueError("failed tool runs require an error_code")
            if self.result_json is not None:
                raise ValueError("failed tool runs cannot contain result_json")
        elif self.error_code is not None:
            raise ValueError("successful or active tool runs cannot contain an error_code")
        if self.finished_at is not None and self.started_at is None:
            raise ValueError("finished_at requires started_at")
        if (
            self.started_at is not None
            and self.finished_at is not None
            and self.finished_at < self.started_at
        ):
            raise ValueError("finished_at cannot precede started_at")
        return self


class EvaluationMetric(_ContractModel):
    name: Identifier
    value: FiniteNonNegativeFloat
    unit: NonEmptyText | None = None
    passed: StrictBool | None = None


class EvaluationResult(_ContractModel):
    evaluation_id: Identifier
    subject_id: Identifier
    requirement_id: Identifier | None = None
    passed: StrictBool
    score: UnitInterval | None = None
    metrics: tuple[EvaluationMetric, ...] = ()
    findings: tuple[NonEmptyText, ...] = ()
    artifact_ids: tuple[Identifier, ...] = ()
    evaluated_at: AwareDatetime

    @field_validator("artifact_ids")
    @classmethod
    def canonicalize_artifact_ids(
        cls,
        values: tuple[Identifier, ...],
    ) -> tuple[Identifier, ...]:
        return _canonicalize_strings(values)

    @field_validator("metrics")
    @classmethod
    def unique_metrics(
        cls,
        values: tuple[EvaluationMetric, ...],
    ) -> tuple[EvaluationMetric, ...]:
        metric_names = tuple(value.name for value in values)
        _reject_duplicate_strings(metric_names)
        return values

    @field_validator("findings")
    @classmethod
    def unique_findings(
        cls,
        values: tuple[NonEmptyText, ...],
    ) -> tuple[NonEmptyText, ...]:
        return _reject_duplicate_strings(values)


class ApprovalRequest(_ContractModel):
    request_id: Identifier
    checkpoint_type: ApprovalKind
    status: ApprovalStatus
    workflow_id: Identifier
    step_id: Identifier | None = None
    action_digest: Sha256Digest
    state_digest: Sha256Digest
    options: tuple[Identifier, ...]
    selected_option_id: Identifier | None = None
    requested_capabilities: tuple[Identifier, ...] = ()
    artifact_ids: tuple[Identifier, ...] = ()
    created_at: AwareDatetime
    decided_at: AwareDatetime | None = None
    actor_id: Identifier | None = None
    modification_digest: Sha256Digest | None = None

    @field_validator("options")
    @classmethod
    def unique_approval_options(
        cls,
        values: tuple[Identifier, ...],
    ) -> tuple[Identifier, ...]:
        if not values:
            raise ValueError("approval options cannot be empty")
        return _canonicalize_strings(values)

    @field_validator("requested_capabilities", "artifact_ids")
    @classmethod
    def canonicalize_approval_references(
        cls,
        values: tuple[Identifier, ...],
    ) -> tuple[Identifier, ...]:
        return _canonicalize_strings(values)

    @model_validator(mode="after")
    def validate_decision(self) -> Self:
        if self.selected_option_id is not None and self.selected_option_id not in self.options:
            raise ValueError("selected_option_id must be declared in options")
        if self.status is ApprovalStatus.PENDING:
            if self.selected_option_id is not None:
                raise ValueError("pending approvals cannot have a selected option")
            if self.decided_at is not None or self.actor_id is not None:
                raise ValueError("pending approvals cannot have a decision")
            if self.modification_digest is not None:
                raise ValueError("pending approvals cannot have a modification")
        else:
            if self.selected_option_id is None:
                raise ValueError("resolved approvals require a selected option")
            if self.modification_digest is not None and self.status is not ApprovalStatus.APPROVED:
                raise ValueError("only approved requests can contain a modification")
        if self.decided_at is not None and self.decided_at < self.created_at:
            raise ValueError("decided_at cannot precede created_at")
        return self


class RunBudget(_ContractModel):
    max_model_calls: NonNegativeInteger = 32
    max_tool_calls: NonNegativeInteger = 64
    max_total_tokens: NonNegativeInteger = 100_000
    max_artifact_bytes: NonNegativeInteger = 1_000_000_000
    max_duration_seconds: FiniteNonNegativeFloat = 3_600.0
    max_cost: NonNegativeDecimal = Decimal("0")
    max_replanning_attempts: NonNegativeInteger = 3


class Provenance(_ContractModel):
    provenance_id: Identifier
    subject_ids: tuple[Identifier, ...]
    source_artifact_ids: tuple[Identifier, ...] = ()
    output_artifact_ids: tuple[Identifier, ...] = ()
    tool_call_ids: tuple[Identifier, ...] = ()
    domain_package_digests: tuple[Sha256Digest, ...] = ()
    parent_provenance_ids: tuple[Identifier, ...] = ()
    transformation_ids: tuple[Identifier, ...] = ()
    model_transcript_digest: Sha256Digest | None = None
    seed: NonNegativeInteger | None = None
    recorded_at: AwareDatetime

    @field_validator("subject_ids")
    @classmethod
    def unique_subjects(
        cls,
        values: tuple[Identifier, ...],
    ) -> tuple[Identifier, ...]:
        if not values:
            raise ValueError("subject_ids cannot be empty")
        return _canonicalize_strings(values)

    @field_validator(
        "source_artifact_ids",
        "output_artifact_ids",
        "tool_call_ids",
        "domain_package_digests",
        "parent_provenance_ids",
        "transformation_ids",
    )
    @classmethod
    def unique_provenance_references(
        cls,
        values: tuple[str, ...],
    ) -> tuple[str, ...]:
        return _canonicalize_strings(values)

    @model_validator(mode="after")
    def reject_self_parent(self) -> Self:
        if self.provenance_id in self.parent_provenance_ids:
            raise ValueError("provenance cannot be its own parent")
        return self


__all__ = (
    "ApprovalKind",
    "ApprovalRequest",
    "ApprovalStatus",
    "ArtifactClassification",
    "ArtifactRef",
    "ArtifactUri",
    "CanonicalJsonObject",
    "DesignRevision",
    "DesignSpec",
    "DomainPackageManifest",
    "EvaluationMetric",
    "EvaluationResult",
    "FiniteNonNegativeFloat",
    "Identifier",
    "MediaType",
    "NonEmptyText",
    "NonNegativeDecimal",
    "NonNegativeInteger",
    "PlanStep",
    "Provenance",
    "Requirement",
    "RequirementKind",
    "RequirementPriority",
    "RequirementStatus",
    "RunBudget",
    "Sha256Digest",
    "SideEffectLevel",
    "StepStatus",
    "ToolCall",
    "ToolDescriptor",
    "ToolResult",
    "ToolRunStatus",
    "UnitInterval",
    "WorkflowDefinition",
    "WorkflowStatus",
    "WorkflowTransition",
)
