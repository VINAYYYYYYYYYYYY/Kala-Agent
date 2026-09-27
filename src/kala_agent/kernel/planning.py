from __future__ import annotations

from collections.abc import Iterable, Iterator, Sequence
from dataclasses import dataclass
from enum import StrEnum
from typing import TypeAlias

from kala_agent.contracts import PlanStep, WorkflowDefinition

CapabilitySet: TypeAlias = frozenset[str]


class PlanValidationCode(StrEnum):
    INVALID_DEFINITION = "invalid_definition"
    EMPTY_PLAN = "empty_plan"
    TOO_MANY_STEPS = "too_many_steps"
    INVALID_STEP = "invalid_step"
    DUPLICATE_STEP_ID = "duplicate_step_id"
    UNKNOWN_CATALOG_STEP = "unknown_catalog_step"
    UNKNOWN_DEPENDENCY = "unknown_dependency"
    SELF_DEPENDENCY = "self_dependency"
    DEPENDENCY_CYCLE = "dependency_cycle"
    CAPABILITY_NOT_GRANTED = "capability_not_granted"
    INVALID_ATTEMPTS = "invalid_attempts"
    TOO_MANY_ATTEMPTS = "too_many_attempts"
    REPLANNING_LIMIT = "replanning_limit"


@dataclass(frozen=True, slots=True)
class PlanValidationError:
    code: PlanValidationCode
    message: str
    step_id: str | None = None
    details: tuple[tuple[str, str], ...] = ()

    @property
    def reason(self) -> PlanValidationCode:
        return self.code

    @property
    def field(self) -> str | None:
        return self.step_id

    def __str__(self) -> str:
        if self.step_id is None:
            return self.message
        return f"{self.message} ({self.step_id})"

    def as_dict(self) -> dict[str, object]:
        return {
            "code": self.code.value,
            "message": self.message,
            "step_id": self.step_id,
            "details": dict(self.details),
        }


class PlanRejectedError(ValueError):
    def __init__(self, error: PlanValidationError) -> None:
        self.error = error
        super().__init__(str(error))


@dataclass(frozen=True, slots=True)
class NormalizedPlan:
    workflow_definition_id: str
    steps: tuple[PlanStep, ...]
    required_capabilities: tuple[str, ...] = ()
    max_attempts: int = 1

    def __post_init__(self) -> None:
        if not self.workflow_definition_id.strip():
            raise ValueError("workflow_definition_id must be nonempty")
        if not self.steps:
            raise ValueError("a normalized plan cannot be empty")
        if isinstance(self.max_attempts, bool) or not isinstance(self.max_attempts, int):
            raise TypeError("max_attempts must be an integer")
        if self.max_attempts < 1:
            raise ValueError("max_attempts must be positive")
        if any(not isinstance(step, PlanStep) for step in self.steps):
            raise TypeError("normalized plan steps must be PlanStep objects")

    @property
    def step_ids(self) -> tuple[str, ...]:
        return tuple(step.step_id for step in self.steps)

    @property
    def capabilities(self) -> tuple[str, ...]:
        return self.required_capabilities

    def __iter__(self) -> Iterator[PlanStep]:
        return iter(self.steps)

    def __len__(self) -> int:
        return len(self.steps)

    def __getitem__(self, index: int) -> PlanStep:
        return self.steps[index]


ValidatedPlan = NormalizedPlan
Plan = NormalizedPlan


class PlanValidator:
    def __init__(
        self,
        definition: WorkflowDefinition,
        *,
        allowed_capabilities: Iterable[str] | None = None,
        granted_capabilities: Iterable[str] | None = None,
        max_steps: int = 256,
        max_attempts: int = 1,
        max_total_attempts: int | None = None,
    ) -> None:
        if not isinstance(definition, WorkflowDefinition):
            raise TypeError("definition must be a WorkflowDefinition")
        if isinstance(max_steps, bool) or not isinstance(max_steps, int):
            raise TypeError("max_steps must be an integer")
        if max_steps < 1:
            raise ValueError("max_steps must be positive")
        if isinstance(max_attempts, bool) or not isinstance(max_attempts, int):
            raise TypeError("max_attempts must be an integer")
        if max_attempts < 1:
            raise ValueError("max_attempts must be positive")
        if max_total_attempts is not None:
            if isinstance(max_total_attempts, bool) or not isinstance(max_total_attempts, int):
                raise TypeError("max_total_attempts must be an integer")
            if max_total_attempts < 1:
                raise ValueError("max_total_attempts must be positive")
        self._definition = definition
        self._max_steps = max_steps
        self._max_attempts = max_attempts
        self._max_total_attempts = max_total_attempts
        self._allowed_capabilities = _combine_capability_sets(
            allowed_capabilities,
            granted_capabilities,
        )

    @property
    def definition(self) -> WorkflowDefinition:
        return self._definition

    @property
    def allowed_capabilities(self) -> CapabilitySet | None:
        return self._allowed_capabilities

    granted_capabilities = allowed_capabilities

    @property
    def unrestricted(self) -> bool:
        return self._allowed_capabilities is None

    def validate(
        self,
        steps: Sequence[PlanStep],
        *,
        allowed_capabilities: Iterable[str] | None = None,
        granted_capabilities: Iterable[str] | None = None,
        replanning_attempts: int = 0,
    ) -> NormalizedPlan | PlanValidationError:
        candidates = _step_sequence(steps)
        if candidates is None:
            return PlanValidationError(
                code=PlanValidationCode.INVALID_STEP,
                message="steps must be a sequence of PlanStep objects",
            )
        if not candidates:
            return PlanValidationError(
                code=PlanValidationCode.EMPTY_PLAN,
                message="a plan must contain at least one step",
            )
        if len(candidates) > self._max_steps:
            return PlanValidationError(
                code=PlanValidationCode.TOO_MANY_STEPS,
                message=f"plan contains {len(candidates)} steps; maximum is {self._max_steps}",
                details=(
                    ("count", str(len(candidates))),
                    ("maximum", str(self._max_steps)),
                ),
            )
        if isinstance(replanning_attempts, bool) or not isinstance(replanning_attempts, int):
            return PlanValidationError(
                code=PlanValidationCode.REPLANNING_LIMIT,
                message="replanning_attempts must be an integer",
            )
        if replanning_attempts < 0:
            return PlanValidationError(
                code=PlanValidationCode.REPLANNING_LIMIT,
                message="replanning_attempts must be nonnegative",
            )
        if replanning_attempts > self._definition.max_replanning_attempts:
            return PlanValidationError(
                code=PlanValidationCode.REPLANNING_LIMIT,
                message="replanning attempts exceed the workflow definition limit",
            )

        selected_capabilities = self._selected_capabilities(
            allowed_capabilities,
            granted_capabilities,
        )

        normalized_steps: list[PlanStep] = []
        step_ids: set[str] = set()
        for candidate in candidates:
            if not isinstance(candidate, PlanStep):
                return PlanValidationError(
                    code=PlanValidationCode.INVALID_STEP,
                    message="every plan entry must be a PlanStep",
                )
            raw_dependencies = getattr(candidate, "depends_on", ())
            raw_step_id = getattr(candidate, "step_id", None)
            if isinstance(raw_step_id, str) and raw_step_id in raw_dependencies:
                return PlanValidationError(
                    code=PlanValidationCode.SELF_DEPENDENCY,
                    message="a plan step cannot depend on itself",
                    step_id=raw_step_id,
                )
            try:
                normalized = PlanStep.model_validate(candidate.model_dump(mode="python"))
            except Exception as error:
                return PlanValidationError(
                    code=PlanValidationCode.INVALID_STEP,
                    message="plan step failed contract validation",
                    step_id=getattr(candidate, "step_id", None),
                    details=(("reason", type(error).__name__),),
                )
            if normalized.step_id in step_ids:
                return PlanValidationError(
                    code=PlanValidationCode.DUPLICATE_STEP_ID,
                    message="plan step identifiers must be unique",
                    step_id=normalized.step_id,
                )
            if normalized.catalog_step_id not in self._definition.available_step_ids:
                return PlanValidationError(
                    code=PlanValidationCode.UNKNOWN_CATALOG_STEP,
                    message="catalog step is not declared by the workflow",
                    step_id=normalized.step_id,
                    details=(("catalog_step_id", normalized.catalog_step_id),),
                )
            if normalized.step_id in normalized.depends_on:
                return PlanValidationError(
                    code=PlanValidationCode.SELF_DEPENDENCY,
                    message="a plan step cannot depend on itself",
                    step_id=normalized.step_id,
                )
            if normalized.max_attempts < 1:
                return PlanValidationError(
                    code=PlanValidationCode.INVALID_ATTEMPTS,
                    message="max_attempts must be positive",
                    step_id=normalized.step_id,
                )
            if normalized.max_attempts > self._max_attempts:
                return PlanValidationError(
                    code=PlanValidationCode.TOO_MANY_ATTEMPTS,
                    message="step attempts exceed the configured bound",
                    step_id=normalized.step_id,
                    details=(
                        ("requested", str(normalized.max_attempts)),
                        ("maximum", str(self._max_attempts)),
                    ),
                )
            step_ids.add(normalized.step_id)
            normalized_steps.append(normalized)

        for step in normalized_steps:
            unknown = sorted(set(step.depends_on) - step_ids)
            if unknown:
                return PlanValidationError(
                    code=PlanValidationCode.UNKNOWN_DEPENDENCY,
                    message="plan dependency is not a declared step",
                    step_id=step.step_id,
                    details=tuple(("dependency", value) for value in unknown),
                )

        cycle = _find_cycle(normalized_steps)
        if cycle is not None:
            return PlanValidationError(
                code=PlanValidationCode.DEPENDENCY_CYCLE,
                message="plan dependencies contain a cycle",
                step_id=cycle[0],
                details=(("cycle", " -> ".join((*cycle, cycle[0]))),),
            )

        required_capabilities = tuple(
            sorted(
                {
                    capability
                    for step in normalized_steps
                    for capability in step.required_capabilities
                }
            )
        )
        missing_capabilities = (
            []
            if selected_capabilities is None
            else sorted(set(required_capabilities) - selected_capabilities)
        )
        if missing_capabilities:
            return PlanValidationError(
                code=PlanValidationCode.CAPABILITY_NOT_GRANTED,
                message="plan requests capabilities outside the active grant",
                details=tuple(("capability", value) for value in missing_capabilities),
            )

        total_attempts = sum(step.max_attempts for step in normalized_steps)
        if self._max_total_attempts is not None and total_attempts > self._max_total_attempts:
            return PlanValidationError(
                code=PlanValidationCode.TOO_MANY_ATTEMPTS,
                message="plan attempts exceed the aggregate bound",
                details=(
                    ("requested", str(total_attempts)),
                    ("maximum", str(self._max_total_attempts)),
                ),
            )

        return NormalizedPlan(
            workflow_definition_id=self._definition.definition_id,
            steps=tuple(normalized_steps),
            required_capabilities=required_capabilities,
            max_attempts=self._max_attempts,
        )

    def validate_or_raise(
        self,
        steps: Sequence[PlanStep],
        *,
        allowed_capabilities: Iterable[str] | None = None,
        granted_capabilities: Iterable[str] | None = None,
        replanning_attempts: int = 0,
    ) -> NormalizedPlan:
        result = self.validate(
            steps,
            allowed_capabilities=allowed_capabilities,
            granted_capabilities=granted_capabilities,
            replanning_attempts=replanning_attempts,
        )
        if isinstance(result, PlanValidationError):
            raise PlanRejectedError(result)
        return result

    def is_valid(
        self,
        steps: Sequence[PlanStep],
        *,
        allowed_capabilities: Iterable[str] | None = None,
        granted_capabilities: Iterable[str] | None = None,
        replanning_attempts: int = 0,
    ) -> bool:
        return not isinstance(
            self.validate(
                steps,
                allowed_capabilities=allowed_capabilities,
                granted_capabilities=granted_capabilities,
                replanning_attempts=replanning_attempts,
            ),
            PlanValidationError,
        )

    def _selected_capabilities(
        self,
        allowed_capabilities: Iterable[str] | None,
        granted_capabilities: Iterable[str] | None,
    ) -> CapabilitySet | None:
        supplied = _combine_capability_sets(allowed_capabilities, granted_capabilities)
        if supplied is None:
            return self._allowed_capabilities
        if self._allowed_capabilities is None:
            return supplied
        return self._allowed_capabilities & supplied


def _step_sequence(value: object) -> tuple[object, ...] | None:
    if isinstance(value, (str, bytes, bytearray)) or not isinstance(value, Sequence):
        return None
    return tuple(value)


def _combine_capability_sets(*values: Iterable[str] | None) -> CapabilitySet | None:
    supplied = [value for value in values if value is not None]
    if not supplied:
        return None
    combined = _capability_set(supplied[0])
    for value in supplied[1:]:
        combined &= _capability_set(value)
    return combined


def _capability_set(values: Iterable[str]) -> CapabilitySet:
    result: set[str] = set()
    for value in values:
        if not isinstance(value, str):
            raise TypeError("capabilities must be strings")
        normalized = value.strip()
        if not normalized:
            raise ValueError("capabilities cannot be empty")
        result.add(normalized)
    return frozenset(result)


def _find_cycle(steps: Sequence[PlanStep]) -> tuple[str, ...] | None:
    graph = {step.step_id: tuple(step.depends_on) for step in steps}
    visiting: list[str] = []
    visited: set[str] = set()

    def visit(step_id: str) -> tuple[str, ...] | None:
        if step_id in visiting:
            start = visiting.index(step_id)
            return tuple((*visiting[start:], step_id))
        if step_id in visited:
            return None
        visiting.append(step_id)
        for dependency in graph.get(step_id, ()):
            cycle = visit(dependency)
            if cycle is not None:
                return cycle
        visiting.pop()
        visited.add(step_id)
        return None

    for step in steps:
        cycle = visit(step.step_id)
        if cycle is not None:
            return cycle
    return None


__all__ = (
    "CapabilitySet",
    "NormalizedPlan",
    "Plan",
    "PlanRejectedError",
    "PlanValidationCode",
    "PlanValidationError",
    "PlanValidator",
    "ValidatedPlan",
)
