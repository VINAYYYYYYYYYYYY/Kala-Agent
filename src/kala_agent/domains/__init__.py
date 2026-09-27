"""Shared support for declarative domain packages.

A domain package is a versioned bundle of declarations: typed parameters with
units and ranges, catalog step descriptors, one workflow definition, one bounded
evaluator per catalog step, and a manifest. This module holds only the
domain-neutral support code the packages share: parameter specifications,
catalog step descriptors, evaluation contexts, the evaluator protocol, manifest
construction, deterministic evaluator implementation digests, capability
enforcement, and a deterministic reference workflow runner.

Everything here is arithmetic and declaration handling. Nothing here is a CAD or
CAE model, a solver, a building-code implementation, or a certification step, and
no shared value may be presented as engineering validation.

Import boundary: domain code imports the shared contracts and this module only.
It never imports ``kernel``, ``tools``, ``persistence``, ``runtime``, ``api``,
or ``cli``, and it never changes them.
"""

from __future__ import annotations

import hashlib
import json
import math
import os
import re
import sys
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from threading import RLock
from typing import Any, Final, Protocol, TypeVar, runtime_checkable

from pydantic import TypeAdapter, ValidationError

from kala_agent.contracts import (
    ApprovalKind,
    DomainPackageManifest,
    EvaluationMetric,
    EvaluationResult,
    Identifier,
    Sha256Digest,
    StepStatus,
    WorkflowDefinition,
    WorkflowStatus,
    WorkflowTransition,
)

ROUNDING_DIGITS: Final[int] = 6
REFERENCE_EVALUATION_EPOCH: Final[datetime] = datetime(2026, 1, 1, tzinfo=UTC)
NON_AUTHORITATIVE_FINDING: Final[str] = (
    "Surrogate reference arithmetic only; not a certified analysis and not a "
    "substitute for a validated CAD or CAE model."
)
UNBOUNDED_SAFETY_FACTOR: Final[float] = 1.0e6
IMPLEMENTATION_DIGEST_VERSION: Final[str] = "evaluator-implementation-digest-v2"
MAX_IMPLEMENTATION_NAME_LENGTH: Final[int] = 256

_IDENTIFIER_ADAPTER: Final[TypeAdapter[str]] = TypeAdapter(Identifier)
_DIGEST_ADAPTER: Final[TypeAdapter[str]] = TypeAdapter(Sha256Digest)
_DECLARATION_NAME_PATTERN: Final[re.Pattern[str]] = re.compile(r"^[a-z][a-z0-9_.-]{0,63}$")
_IMPLEMENTATION_NAME_PATTERN: Final[re.Pattern[str]] = re.compile(
    r"^[A-Za-z0-9_.<>-]+$",
)
_MODULE_DIGEST_CACHE: Final[dict[tuple[str, int, int, int, int], str]] = {}
_MODULE_DIGEST_LOCK: Final[RLock] = RLock()


class DomainError(RuntimeError):
    """Base class for declarative domain package failures."""


class DomainParameterError(DomainError, ValueError):
    """Raised when a parameter or context value is outside its declared contract."""


class DomainPackageError(DomainError):
    """Raised when a package declaration is internally inconsistent."""


class DomainStepError(DomainError, LookupError):
    """Raised when a catalog step is not declared by a package."""


class DomainCapabilityError(DomainError, ValueError):
    """Raised when a run's declared capabilities are not granted exactly as declared."""


def require_identifier(value: object, field_name: str) -> str:
    """Validate a value against the shared contract identifier rules."""
    try:
        return _IDENTIFIER_ADAPTER.validate_python(value)
    except ValidationError as error:
        raise DomainParameterError(f"{field_name} must be a safe identifier") from error


def require_digest(value: object, field_name: str) -> str:
    """Validate a value against the shared contract digest rules."""
    try:
        return _DIGEST_ADAPTER.validate_python(value)
    except ValidationError as error:
        raise DomainParameterError(f"{field_name} must be a sha-256 digest") from error


def require_name(value: object, field_name: str) -> str:
    """Validate a domain declaration name, such as a parameter or metric name.

    Declaration names are lower-case and may contain underscores, dots, and
    hyphens. Contract identifier fields are narrower and use
    :func:`require_identifier` instead.
    """
    if not isinstance(value, str):
        raise DomainParameterError(f"{field_name} must be a string")
    normalized = value.strip()
    if _DECLARATION_NAME_PATTERN.fullmatch(normalized) is None:
        raise DomainParameterError(f"{field_name} must be a lowercase declaration name")
    return normalized


def require_capabilities(values: object, field_name: str) -> tuple[str, ...]:
    """Return a canonical, sorted, duplicate-free tuple of capability identifiers.

    Canonicalising here keeps declared capabilities, granted capabilities, and
    the capability component of a package digest independent of the order in
    which they were written down.
    """
    if isinstance(values, (str, bytes, bytearray)) or not isinstance(values, (list, set, tuple)):
        raise DomainParameterError(f"{field_name} must be a collection of capabilities")
    return tuple(sorted({require_identifier(value, field_name) for value in values}))


def _checkpoint_label(checkpoint_kind: object) -> str | None:
    """Return the canonical label of a declared checkpoint kind, if any."""
    return None if checkpoint_kind is None else str(checkpoint_kind)


def _require_checkpoint(checkpoint_kind: object) -> ApprovalKind | None:
    """Return a declared checkpoint kind as a known approval kind."""
    if checkpoint_kind is None or isinstance(checkpoint_kind, ApprovalKind):
        return checkpoint_kind
    try:
        return ApprovalKind(str(checkpoint_kind))
    except ValueError as error:
        raise DomainParameterError("a checkpoint kind must be a known approval kind") from error


def clamp_unit_interval(value: float) -> float:
    """Clamp a bounded term into the unit interval and round it for comparison."""
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise DomainParameterError("bounded terms must be finite numbers")
    return round(min(1.0, max(0.0, value)), ROUNDING_DIGITS)


def canonical_json(payload: object) -> str:
    """Serialize a declaration to canonical JSON for digesting."""
    return json.dumps(
        payload,
        allow_nan=False,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    )


def canonical_digest(payload: object) -> str:
    """Return the sha-256 digest of the canonical JSON form of a declaration."""
    return hashlib.sha256(canonical_json(payload).encode("utf-8")).hexdigest()


def module_source_digest(module_name: object) -> str:
    """Return the sha-256 digest of an imported module's source file.

    The digest covers the bytes on disk that would be imported for that module
    name, not the loaded module object, so it identifies the implementation
    rather than the declaration that references it. Repeated calls inside one
    process are cached per file identity, which keeps the digest stable and
    cheap, while a rewritten file yields a new digest.

    A module that is not imported, has no source file, or whose file cannot be
    read has no identifiable implementation, and is refused.
    """
    return _file_digest(_module_source_path(module_name), str(module_name))


def package_source_digest(module_name: object) -> str:
    """Return one digest over every source file an evaluator's arithmetic can come from.

    An evaluator's own module rarely holds all of its arithmetic: formulas and
    constants live in sibling modules of the same package. This digest covers
    every Python source file under the directory of the defining module and the
    ``__init__`` of every enclosing package, so editing a formula or a constant
    changes it even when the evaluator's own module is untouched.
    """
    origin = _module_source_path(module_name)
    name = str(module_name)
    root = origin.parent
    entries: list[list[str]] = []
    for directory, subdirectories, files in os.walk(root, followlinks=False):
        subdirectories.sort()
        for file_name in sorted(files):
            if not file_name.endswith(".py"):
                continue
            path = Path(directory) / file_name
            label = f"package:{path.relative_to(root).as_posix()}"
            entries.append([label, _file_digest(path, name)])
    parts = name.split(".")
    for index in range(1, len(parts)):
        ancestor = ".".join(parts[:index])
        ancestor_file = getattr(sys.modules.get(ancestor), "__file__", None)
        if isinstance(ancestor_file, str) and ancestor_file:
            entries.append([f"ancestor:{ancestor}", _file_digest(Path(ancestor_file), ancestor)])
    return canonical_digest(sorted(entries))


def _module_source_path(module_name: object) -> Path:
    if not isinstance(module_name, str) or not module_name.strip():
        raise DomainPackageError("an evaluator implementation must declare an importable module")
    module = sys.modules.get(module_name)
    if module is None:
        raise DomainPackageError(f"{module_name} is not an imported module")
    origin = getattr(module, "__file__", None)
    if not isinstance(origin, str) or not origin:
        raise DomainPackageError(f"{module_name} has no source file to digest")
    return Path(origin)


def _file_digest(path: Path, module_name: str) -> str:
    try:
        status = path.stat()
        key = (str(path), status.st_dev, status.st_ino, status.st_size, status.st_mtime_ns)
        with _MODULE_DIGEST_LOCK:
            cached = _MODULE_DIGEST_CACHE.get(key)
        if cached is not None:
            return cached
        content = path.read_bytes()
    except OSError as error:
        raise DomainPackageError(f"the source file of {module_name} cannot be read") from error
    digest = hashlib.sha256(content).hexdigest()
    with _MODULE_DIGEST_LOCK:
        _MODULE_DIGEST_CACHE[key] = digest
    return digest


@dataclass(frozen=True, slots=True)
class ParameterSpec:
    """A declared parameter with its unit, range, and default value."""

    name: str
    unit: str
    minimum: float
    maximum: float
    default: str | int | float
    description: str

    def __post_init__(self) -> None:
        object.__setattr__(self, "name", require_name(self.name, "parameter name"))
        if not self.unit.strip():
            raise DomainParameterError("parameter units cannot be empty")
        if not self.description.strip():
            raise DomainParameterError("parameter descriptions cannot be empty")
        if not math.isfinite(self.minimum) or not math.isfinite(self.maximum):
            raise DomainParameterError("parameter ranges must be finite")
        if self.maximum < self.minimum:
            raise DomainParameterError("a parameter maximum must not be below its minimum")

    def as_dict(self) -> dict[str, object]:
        """Return the inspectable declaration of this parameter."""
        return {
            "default": self.default,
            "description": self.description,
            "maximum": self.maximum,
            "minimum": self.minimum,
            "name": self.name,
            "unit": self.unit,
        }


def parameter_schema_digest(specs: tuple[ParameterSpec, ...]) -> str:
    """Return a stable digest over a package parameter declaration table."""
    ordered = [spec.as_dict() for spec in sorted(specs, key=lambda spec: spec.name)]
    return canonical_digest(ordered)


@dataclass(frozen=True, slots=True)
class CatalogStep:
    """A named workflow step offered by a domain package."""

    step_id: str
    name: str
    description: str
    input_names: tuple[str, ...] = ()
    metric_names: tuple[str, ...] = ()
    required_capabilities: tuple[str, ...] = ()
    estimated_duration_seconds: float = 0.0
    checkpoint_kind: ApprovalKind | None = None
    max_attempts: int = 1

    def __post_init__(self) -> None:
        require_identifier(self.step_id, "catalog step_id")
        if not self.name.strip() or not self.description.strip():
            raise DomainParameterError("catalog steps need a name and a description")
        for value in self.input_names:
            require_name(value, "catalog step input name")
        for value in self.metric_names:
            require_name(value, "catalog step metric name")
        object.__setattr__(
            self,
            "required_capabilities",
            require_capabilities(self.required_capabilities, "catalog step capability"),
        )
        object.__setattr__(self, "checkpoint_kind", _require_checkpoint(self.checkpoint_kind))
        if isinstance(self.max_attempts, bool) or self.max_attempts < 1:
            raise DomainParameterError("catalog steps must allow at least one attempt")
        if not math.isfinite(self.estimated_duration_seconds):
            raise DomainParameterError("catalog step duration must be finite")
        if self.estimated_duration_seconds < 0.0:
            raise DomainParameterError("catalog step duration must be finite and nonnegative")

    def as_dict(self) -> dict[str, object]:
        """Return the inspectable declaration of this step."""
        return {
            "checkpoint_kind": (
                None if self.checkpoint_kind is None else self.checkpoint_kind.value
            ),
            "description": self.description,
            "estimated_duration_seconds": self.estimated_duration_seconds,
            "input_names": list(self.input_names),
            "max_attempts": self.max_attempts,
            "metric_names": list(self.metric_names),
            "name": self.name,
            "required_capabilities": list(self.required_capabilities),
            "step_id": self.step_id,
        }


@dataclass(frozen=True, slots=True)
class EvaluationContext:
    """Bounded evaluation inputs that are not part of a parameter model."""

    subject_id: str
    evaluated_at: datetime
    evaluation_id_prefix: str = "eval"

    def __post_init__(self) -> None:
        object.__setattr__(self, "subject_id", require_identifier(self.subject_id, "subject_id"))
        object.__setattr__(
            self,
            "evaluation_id_prefix",
            require_identifier(self.evaluation_id_prefix, "evaluation_id_prefix"),
        )
        if self.evaluated_at.tzinfo is None or self.evaluated_at.utcoffset() is None:
            raise DomainParameterError("evaluated_at must include a timezone")
        object.__setattr__(self, "evaluated_at", self.evaluated_at.astimezone(UTC))

    def evaluation_id(self, step_id: str) -> str:
        """Return the deterministic evaluation identifier for a catalog step."""
        return f"{self.evaluation_id_prefix}.{require_identifier(step_id, 'step_id')}"

    def metric(
        self,
        name: str,
        value: float,
        unit: str | None = None,
        passed: bool | None = None,
    ) -> EvaluationMetric:
        """Build a contract evaluation metric for this context."""
        return EvaluationMetric(name=name, value=value, unit=unit, passed=passed)

    def result(
        self,
        step_id: str,
        *,
        passed: bool,
        score: float | None,
        metrics: tuple[EvaluationMetric, ...],
        findings: tuple[str, ...],
    ) -> EvaluationResult:
        """Build a contract evaluation result for this context."""
        return EvaluationResult(
            evaluation_id=self.evaluation_id(step_id),
            subject_id=self.subject_id,
            passed=passed,
            score=score,
            metrics=metrics,
            findings=findings,
            evaluated_at=self.evaluated_at,
        )


def reference_context(
    subject_id: str,
    evaluated_at: datetime | None = None,
) -> EvaluationContext:
    """Build a deterministic context, defaulting to the reference epoch."""
    return EvaluationContext(
        subject_id=subject_id,
        evaluated_at=REFERENCE_EVALUATION_EPOCH if evaluated_at is None else evaluated_at,
    )


ParametersT_contra = TypeVar("ParametersT_contra", contravariant=True)


@runtime_checkable
class DomainEvaluator(Protocol[ParametersT_contra]):
    """The common evaluator interface implemented by every domain package."""

    @property
    def step_id(self) -> str: ...

    @property
    def required_capabilities(self) -> tuple[str, ...]: ...

    def evaluate(
        self,
        context: EvaluationContext,
        parameters: ParametersT_contra,
    ) -> EvaluationResult: ...


@dataclass(frozen=True, slots=True)
class EvaluatorImplementation:
    """The identified implementation of one evaluator bound to a catalog step."""

    step_id: str
    qualified_name: str
    module_name: str
    module_digest: str
    package_digest: str
    required_capabilities: tuple[str, ...]
    digest: str

    def __post_init__(self) -> None:
        require_identifier(self.step_id, "evaluator implementation step_id")
        if (
            not self.module_name
            or len(self.module_name) > MAX_IMPLEMENTATION_NAME_LENGTH
            or _IMPLEMENTATION_NAME_PATTERN.fullmatch(self.module_name) is None
        ):
            raise DomainPackageError(
                "an evaluator implementation must declare an importable module"
            )
        if (
            not self.qualified_name
            or len(self.qualified_name) > MAX_IMPLEMENTATION_NAME_LENGTH
            or _IMPLEMENTATION_NAME_PATTERN.fullmatch(self.qualified_name) is None
        ):
            raise DomainPackageError("an evaluator qualified name must be a safe dotted name")
        if not self.qualified_name.startswith(f"{self.module_name}."):
            raise DomainPackageError("an evaluator qualified name must name its own module")
        require_digest(self.module_digest, "evaluator module digest")
        require_digest(self.package_digest, "evaluator package digest")
        require_digest(self.digest, "evaluator implementation digest")
        object.__setattr__(
            self,
            "required_capabilities",
            require_capabilities(self.required_capabilities, "evaluator required capability"),
        )

    def as_dict(self) -> dict[str, object]:
        """Return the inspectable declaration of this implementation."""
        return {
            "digest": self.digest,
            "module": self.module_name,
            "module_digest": self.module_digest,
            "package_digest": self.package_digest,
            "qualified_name": self.qualified_name,
            "required_capabilities": list(self.required_capabilities),
            "step_id": self.step_id,
        }


def evaluator_implementation(evaluator: object) -> EvaluatorImplementation:
    """Identify the live implementation behind an evaluator and digest it.

    The digest commits to the evaluator's stable qualified name, the digest of
    the source file of the module that defines it, the digest of every source
    file of its package, the catalog step it is bound to, and the capabilities
    it declares. It therefore changes when an evaluator is substituted by a
    different class, a different module, or a different set of declared
    capabilities, even when the substitute computes exactly the same numbers,
    and when a formula or constant it imports from a sibling module changes. An evaluator whose implementation cannot be
    identified from a source file is refused rather than digested blind.
    """
    implementation = type(evaluator)
    module_name = getattr(implementation, "__module__", "")
    local_name = getattr(implementation, "__qualname__", "")
    if not isinstance(module_name, str) or not isinstance(local_name, str):
        raise DomainPackageError("an evaluator implementation must expose a module and a name")
    qualified_name = f"{module_name}.{local_name}"
    source_digest = module_source_digest(module_name)
    package_digest = package_source_digest(module_name)
    step_id = require_identifier(getattr(evaluator, "step_id", None), "evaluator step_id")
    capabilities = require_capabilities(
        getattr(evaluator, "required_capabilities", None),
        "evaluator required capability",
    )
    return EvaluatorImplementation(
        step_id=step_id,
        qualified_name=qualified_name,
        module_name=module_name,
        module_digest=source_digest,
        package_digest=package_digest,
        required_capabilities=capabilities,
        digest=canonical_digest(
            {
                "implementation_digest_version": IMPLEMENTATION_DIGEST_VERSION,
                "module": module_name,
                "module_digest": source_digest,
                "package_digest": package_digest,
                "qualified_name": qualified_name,
                "required_capabilities": list(capabilities),
                "step_id": step_id,
            }
        ),
    )


@dataclass(frozen=True, slots=True)
class DomainPackage:
    """A versioned, inspectable domain package declaration."""

    package_id: str
    version: str
    description: str
    disclaimer: str
    workflow: WorkflowDefinition
    steps: tuple[CatalogStep, ...]
    evaluators: tuple[DomainEvaluator[Any], ...]
    schema_digests: tuple[str, ...] = ()
    required_capabilities: tuple[str, ...] = ()
    evaluator_implementations: tuple[EvaluatorImplementation, ...] = field(init=False, default=())

    def __post_init__(self) -> None:
        require_identifier(self.package_id, "package_id")
        if not self.version.strip():
            raise DomainPackageError("a domain package must declare a version")
        if not self.description.strip() or not self.disclaimer.strip():
            raise DomainPackageError("a domain package must be described and disclaimed")
        object.__setattr__(
            self,
            "required_capabilities",
            require_capabilities(self.required_capabilities, "package required capability"),
        )
        self._validate_workflow()
        if not self.steps or not self.evaluators:
            raise DomainPackageError("a domain package must declare steps and evaluators")
        step_ids = tuple(step.step_id for step in self.steps)
        if len(step_ids) != len(set(step_ids)):
            raise DomainPackageError("catalog step ids must be unique")
        if set(step_ids) != set(self.workflow.available_step_ids):
            raise DomainPackageError(
                "catalog steps must match the workflow available step ids exactly"
            )
        evaluator_ids = tuple(evaluator.step_id for evaluator in self.evaluators)
        if len(evaluator_ids) != len(set(evaluator_ids)):
            raise DomainPackageError("each catalog step needs exactly one evaluator")
        if set(evaluator_ids) != set(step_ids):
            raise DomainPackageError("every catalog step must have an evaluator")
        if not self.schema_digests:
            raise DomainPackageError("a domain package must publish schema digests")
        for digest in self.schema_digests:
            require_digest(digest, "schema digest")
        self._validate_checkpoints()
        object.__setattr__(self, "evaluator_implementations", self._identify_evaluators())

    def _validate_workflow(self) -> None:
        if not isinstance(self.workflow, WorkflowDefinition):
            raise DomainPackageError("a domain package must declare a workflow definition")
        try:
            WorkflowDefinition.model_validate(self.workflow.model_dump(mode="json"))
        except (AttributeError, TypeError, ValueError) as error:
            raise DomainPackageError(
                "the declared workflow definition does not satisfy the contract"
            ) from error

    def _validate_checkpoints(self) -> None:
        transitions: dict[str, WorkflowTransition] = {
            transition.step_id: transition for transition in self.workflow.transitions
        }
        for step in self.steps:
            declared = _checkpoint_label(step.checkpoint_kind)
            transition = transitions.get(step.step_id)
            if transition is None:
                if declared is not None:
                    raise DomainPackageError(
                        f"{step.step_id} declares a checkpoint that no workflow transition carries"
                    )
                continue
            if _checkpoint_label(transition.checkpoint_type) != declared:
                raise DomainPackageError(
                    f"{step.step_id} declares a checkpoint kind that its workflow transition "
                    "does not carry"
                )

    def _identify_evaluators(self) -> tuple[EvaluatorImplementation, ...]:
        implementations: list[EvaluatorImplementation] = []
        for evaluator in self.evaluators:
            implementation = evaluator_implementation(evaluator)
            step = self.catalog_step(implementation.step_id)
            if step.required_capabilities != implementation.required_capabilities:
                raise DomainPackageError(
                    f"{step.step_id} must declare the same capabilities on its catalog step "
                    "and on its evaluator"
                )
            implementations.append(implementation)
        return tuple(sorted(implementations, key=lambda item: item.step_id))

    @property
    def step_ids(self) -> tuple[str, ...]:
        """Return the declared catalog step ids in workflow order."""
        return tuple(step.step_id for step in self.steps)

    @property
    def evaluator_implementation_digests(self) -> tuple[str, ...]:
        """Return the bound evaluator implementation digests, in workflow order."""
        implementations = {item.step_id: item for item in self.evaluator_implementations}
        return tuple(implementations[step_id].digest for step_id in self.step_ids)

    def catalog_step(self, step_id: str) -> CatalogStep:
        """Return a declared catalog step descriptor."""
        for step in self.steps:
            if step.step_id == step_id:
                return step
        raise DomainStepError(f"{step_id} is not declared by {self.package_id}")

    def evaluator(self, step_id: str) -> DomainEvaluator[Any]:
        """Return the evaluator bound to a declared catalog step."""
        for evaluator in self.evaluators:
            if evaluator.step_id == step_id:
                return evaluator
        raise DomainStepError(f"{step_id} has no evaluator in {self.package_id}")

    def evaluate(
        self,
        step_id: str,
        context: EvaluationContext,
        parameters: object,
    ) -> EvaluationResult:
        """Run the evaluator bound to a declared catalog step."""
        return self.evaluator(step_id).evaluate(context, parameters)

    def package_payload(self) -> dict[str, object]:
        """Return the canonical payload that identifies this package content."""
        return {
            "description": self.description,
            "disclaimer": self.disclaimer,
            "evaluators": [item.as_dict() for item in self.evaluator_implementations],
            "package_id": self.package_id,
            "required_capabilities": list(self.required_capabilities),
            "schema_digests": list(self.schema_digests),
            "steps": [step.as_dict() for step in self.steps],
            "version": self.version,
            "workflow": self.workflow.model_dump(mode="json"),
        }

    def digest(self) -> str:
        """Return the content digest of this package declaration.

        The digest covers the bound evaluator implementations as well as the
        declarations, so it changes when an evaluator is substituted behind an
        otherwise unchanged catalog.
        """
        return canonical_digest(self.package_payload())

    def manifest_schema_digests(self) -> tuple[str, ...]:
        """Return the content digests pinned by this package's manifest."""
        return tuple(sorted({*self.schema_digests, *self.evaluator_implementation_digests}))

    def manifest(self) -> DomainPackageManifest:
        """Return the contract manifest describing this package."""
        return DomainPackageManifest(
            package_id=self.package_id,
            version=self.version,
            digest=self.digest(),
            description=self.description,
            workflow_definition_ids=(self.workflow.definition_id,),
            schema_digests=self.manifest_schema_digests(),
            required_capabilities=self.required_capabilities,
        )


@dataclass(frozen=True, slots=True)
class StepOutcome:
    """The outcome of one catalog step inside a reference workflow run."""

    step_id: str
    status: StepStatus
    evaluation: EvaluationResult | None = None
    reason: str | None = None

    def __post_init__(self) -> None:
        require_identifier(self.step_id, "step outcome step_id")


@dataclass(frozen=True, slots=True)
class ReferenceWorkflowRun:
    """A deterministic, bounded run of a package's reference workflow."""

    package_id: str
    definition_id: str
    definition_version: str
    status: WorkflowStatus
    ordered_step_ids: tuple[str, ...]
    completed_step_ids: tuple[str, ...]
    outcomes: tuple[StepOutcome, ...]
    blocked_step_id: str | None = None

    @property
    def evaluations(self) -> tuple[EvaluationResult, ...]:
        """Return every evaluation produced by the run, in execution order."""
        return tuple(
            outcome.evaluation for outcome in self.outcomes if outcome.evaluation is not None
        )

    @property
    def passed(self) -> bool:
        """Return whether every declared step satisfied its hard constraints."""
        return self.status is WorkflowStatus.COMPLETED

    @property
    def findings(self) -> tuple[str, ...]:
        """Return every finding produced by the run, in execution order."""
        return tuple(finding for evaluation in self.evaluations for finding in evaluation.findings)

    @property
    def score(self) -> float | None:
        """Return the mean bounded score of the run, or None when unscored."""
        scores = [
            evaluation.score for evaluation in self.evaluations if evaluation.score is not None
        ]
        if not scores:
            return None
        return clamp_unit_interval(sum(scores) / len(scores))


@dataclass(frozen=True, slots=True)
class CapabilityRequest:
    """The domain capabilities one reference workflow run requires up front."""

    package_id: str
    step_ids: tuple[str, ...]
    required_capabilities: tuple[str, ...]

    def __post_init__(self) -> None:
        require_identifier(self.package_id, "capability request package_id")
        if not self.step_ids:
            raise DomainParameterError("a capability request must declare at least one step")
        for step_id in self.step_ids:
            require_identifier(step_id, "capability request step_id")
        object.__setattr__(
            self,
            "required_capabilities",
            require_capabilities(self.required_capabilities, "capability request capability"),
        )


@runtime_checkable
class CapabilityGrantPolicy(Protocol):
    """A domain-neutral decision point for the capabilities a run requires.

    A kernel policy engine, an operator prompt, or a fixed test grant can each
    satisfy this protocol. The domain layer only compares what the policy grants
    against what the package and its catalog steps declare, and never decides
    authority for itself.
    """

    def granted_capabilities(self, request: CapabilityRequest) -> tuple[str, ...]: ...


def _capability_request(package: DomainPackage, step_ids: tuple[str, ...]) -> CapabilityRequest:
    required: set[str] = set(package.required_capabilities)
    for step_id in step_ids:
        required.update(package.catalog_step(step_id).required_capabilities)
    return CapabilityRequest(
        package_id=package.package_id,
        step_ids=step_ids,
        required_capabilities=tuple(sorted(required)),
    )


def enforce_capability_request(
    package: DomainPackage,
    step_ids: tuple[str, ...],
    policy: CapabilityGrantPolicy | None = None,
) -> CapabilityRequest:
    """Refuse an evaluation unless its grants equal what the package declares.

    Every evaluation path must call this before any evaluator runs. Without a
    policy, only steps that declare no capability may run. With a policy, the
    grant must match the declaration exactly: a withheld capability and a
    surplus one are both refused, including a surplus grant to a step that
    declares nothing.
    """
    for step_id in step_ids:
        package.catalog_step(step_id)
    request = _capability_request(package, step_ids)
    _require_capability_grants(request, policy)
    return request


def _require_capability_grants(
    request: CapabilityRequest,
    policy: CapabilityGrantPolicy | None,
) -> None:
    if policy is None:
        if request.required_capabilities:
            raise DomainCapabilityError(
                f"{request.package_id} declares capabilities that no policy granted: "
                f"{', '.join(request.required_capabilities)}"
            )
        return
    granted = require_capabilities(policy.granted_capabilities(request), "granted capability")
    missing = [
        capability for capability in request.required_capabilities if capability not in granted
    ]
    if missing:
        raise DomainCapabilityError(
            f"{request.package_id} declares capabilities that are not granted: {', '.join(missing)}"
        )
    extra = [
        capability for capability in granted if capability not in request.required_capabilities
    ]
    if extra:
        raise DomainCapabilityError(
            f"the granted capabilities are not the capabilities {request.package_id} declares: "
            f"{', '.join(extra)}"
        )


def run_reference_workflow(
    package: DomainPackage,
    context: EvaluationContext,
    parameters: object,
    step_ids: tuple[str, ...],
    *,
    capability_policy: CapabilityGrantPolicy | None = None,
) -> ReferenceWorkflowRun:
    """Run declared steps in order and stop at the first hard-constraint failure.

    The runner is a fixed sequence of pure evaluator calls: no clock, no network,
    no unbounded loops, and no dependency on the shared workflow engine.

    A run whose package and catalog steps declare no capability needs no policy.
    A run that declares one must be handed a ``capability_policy`` that grants
    exactly the declared capabilities; a withheld or surplus grant is refused
    before any evaluator is called, so a denied run evaluates nothing.
    """
    if not step_ids:
        raise DomainParameterError("a reference workflow must declare at least one step")
    if len(step_ids) != len(set(step_ids)):
        raise DomainParameterError("reference workflow steps must be unique")
    enforce_capability_request(package, step_ids, capability_policy)
    outcomes: list[StepOutcome] = []
    completed: list[str] = []
    blocked_step_id: str | None = None
    status = WorkflowStatus.COMPLETED
    for step_id in step_ids:
        evaluation = package.evaluate(step_id, context, parameters)
        if evaluation.passed:
            outcomes.append(
                StepOutcome(
                    step_id=step_id,
                    status=StepStatus.SUCCEEDED,
                    evaluation=evaluation,
                )
            )
            completed.append(step_id)
            continue
        reason = f"hard constraint failure reported by {evaluation.evaluation_id}"
        outcomes.append(
            StepOutcome(
                step_id=step_id,
                status=StepStatus.FAILED,
                evaluation=evaluation,
                reason=reason,
            )
        )
        blocked_step_id = step_id
        status = WorkflowStatus.FAILED
        break
    return ReferenceWorkflowRun(
        package_id=package.package_id,
        definition_id=package.workflow.definition_id,
        definition_version=package.workflow.version,
        status=status,
        ordered_step_ids=step_ids,
        completed_step_ids=tuple(completed),
        outcomes=tuple(outcomes),
        blocked_step_id=blocked_step_id,
    )


__all__ = (
    "IMPLEMENTATION_DIGEST_VERSION",
    "MAX_IMPLEMENTATION_NAME_LENGTH",
    "NON_AUTHORITATIVE_FINDING",
    "REFERENCE_EVALUATION_EPOCH",
    "ROUNDING_DIGITS",
    "UNBOUNDED_SAFETY_FACTOR",
    "CapabilityGrantPolicy",
    "CapabilityRequest",
    "CatalogStep",
    "DomainCapabilityError",
    "DomainError",
    "DomainEvaluator",
    "DomainPackage",
    "DomainPackageError",
    "DomainParameterError",
    "DomainStepError",
    "EvaluationContext",
    "EvaluatorImplementation",
    "ParameterSpec",
    "ReferenceWorkflowRun",
    "StepOutcome",
    "canonical_digest",
    "canonical_json",
    "clamp_unit_interval",
    "enforce_capability_request",
    "evaluator_implementation",
    "module_source_digest",
    "package_source_digest",
    "parameter_schema_digest",
    "reference_context",
    "require_capabilities",
    "require_digest",
    "require_identifier",
    "require_name",
    "run_reference_workflow",
)
