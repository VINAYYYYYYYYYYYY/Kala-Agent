from __future__ import annotations

import re
import threading
from collections.abc import Callable, Iterable, Mapping, Sequence
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from datetime import UTC, datetime
from types import MappingProxyType
from typing import Final, Protocol, TypeAlias, cast, runtime_checkable

from kala_agent.contracts import PlanStep, SideEffectLevel, ToolCall, ToolResult, ToolRunStatus
from kala_agent.kernel.budget import BudgetExhaustedError, BudgetLedger
from kala_agent.kernel.policy import PolicyEngine
from kala_agent.runtime.model_gateway import (
    EgressDeniedError,
    ModelCapabilityError,
    ModelGateway,
    ModelMessage,
    ModelRequest,
    ModelResponse,
    ModelRole,
    ProviderDegradedError,
    StructuredOutputError,
)
from kala_agent.tools.executor import ToolExecutor
from kala_agent.tools.protocol import (
    JsonEnvelopeError,
    JSONValue,
    SchemaValidationError,
    ToolErrorCode,
    canonical_json_digest,
    canonical_json_dumps,
    sha256_hex,
    strict_json_loads,
)
from kala_agent.tools.registry import RegisteredTool, ToolRegistry, ToolRegistryError

MAX_AGENT_TASKS: Final[int] = 64
MAX_PARALLEL_TASKS: Final[int] = 8
MAX_MERGED_RESULT_BYTES: Final[int] = 4_194_304
MAX_BOUNDED_TEXT: Final[int] = 512
MAX_ERROR_TEXT: Final[int] = 1_024
APPROVAL_REQUIRED_EFFECTS: Final[frozenset[SideEffectLevel]] = frozenset(
    {
        SideEffectLevel.LOCAL_WRITE,
        SideEffectLevel.NETWORK,
        SideEffectLevel.EGRESS,
    }
)
_TOKEN_PATTERN: Final[re.Pattern[str]] = re.compile(r"[a-z0-9]+")
_IDENTIFIER_PATTERN: Final[re.Pattern[str]] = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$")

Clock: TypeAlias = Callable[[], datetime]
Denial: TypeAlias = tuple[str, str]


class AgentError(RuntimeError):
    pass


class AgentPlanError(AgentError, ValueError):
    pass


class AgentPolicyError(AgentError):
    pass


class AgentMergeError(AgentError):
    pass


class PlannerFailure(AgentError):
    pass


def _bounded_identifier(value: object, field_name: str) -> str:
    if not isinstance(value, str):
        raise AgentPlanError(f"{field_name} must be a string")
    normalized = value.strip()
    if _IDENTIFIER_PATTERN.fullmatch(normalized) is None:
        raise AgentPlanError(f"{field_name} must be a safe identifier")
    return normalized


def _bounded_text(value: object, field_name: str, *, maximum: int = MAX_BOUNDED_TEXT) -> str:
    if not isinstance(value, str):
        raise AgentPlanError(f"{field_name} must be a string")
    if len(value) > maximum:
        raise AgentPlanError(f"{field_name} exceeds the bounded length")
    return value


def _bounded_seconds(value: object, field_name: str) -> float | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise AgentPlanError(f"{field_name} must be a number")
    parsed = float(value)
    if not 0.0 < parsed <= 3_600.0:
        raise AgentPlanError(f"{field_name} must be positive and bounded")
    return parsed


def _identifier_tuple(values: Iterable[str], field_name: str) -> tuple[str, ...]:
    if isinstance(values, (str, bytes)) or not isinstance(values, (Sequence, set, frozenset)):
        raise AgentPlanError(f"{field_name} must be a collection of identifiers")
    return tuple(sorted({_bounded_identifier(item, field_name) for item in values}))


def _bounded_reasons(values: Iterable[str], field_name: str) -> tuple[str, ...]:
    if isinstance(values, (str, bytes)) or not isinstance(values, (Sequence, set, frozenset)):
        raise AgentPlanError(f"{field_name} must be a collection of reasons")
    return tuple(
        _bounded_text(item, f"{field_name} entry", maximum=MAX_ERROR_TEXT) for item in values
    )


def _freeze_value(value: object, *, depth: int = 0) -> JSONValue:
    if depth > 16:
        raise AgentPlanError("task arguments exceed the supported nesting depth")
    if value is None or isinstance(value, (bool, int, str)):
        return cast(JSONValue, value)
    if isinstance(value, float):
        if value != value or value in (float("inf"), float("-inf")):
            raise AgentPlanError("task arguments must contain finite numbers")
        return value
    if isinstance(value, Mapping):
        nested: dict[str, JSONValue] = {}
        for key, item in value.items():
            if not isinstance(key, str) or not key:
                raise AgentPlanError("task argument keys must be nonempty strings")
            nested[key] = _freeze_value(item, depth=depth + 1)
        return cast(JSONValue, MappingProxyType(nested))
    if isinstance(value, (list, tuple)):
        return cast(JSONValue, tuple(_freeze_value(item, depth=depth + 1) for item in value))
    raise AgentPlanError("task arguments must be JSON serializable")


def _freeze_object(value: object, field_name: str) -> Mapping[str, JSONValue]:
    if not isinstance(value, Mapping):
        raise AgentPlanError(f"{field_name} must be a JSON object")
    frozen: dict[str, JSONValue] = {}
    for key, item in value.items():
        if not isinstance(key, str) or not key:
            raise AgentPlanError(f"{field_name} keys must be nonempty strings")
        frozen[key] = _freeze_value(item, depth=0)
    return MappingProxyType(frozen)


def _thaw(value: JSONValue) -> JSONValue:
    if isinstance(value, Mapping):
        return {key: _thaw(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_thaw(item) for item in value]
    return value


def _thaw_object(value: Mapping[str, JSONValue]) -> dict[str, JSONValue]:
    return {key: _thaw(item) for key, item in value.items()}


@dataclass(frozen=True, slots=True)
class AgentTask:
    step_id: str
    tool_id: str
    arguments: Mapping[str, JSONValue] = field(default_factory=lambda: MappingProxyType({}))
    tool_version: str | None = None
    required_capabilities: tuple[str, ...] = ()
    depends_on: tuple[str, ...] = ()
    intent: str = ""
    timeout_seconds: float | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "step_id", _bounded_identifier(self.step_id, "step_id"))
        object.__setattr__(self, "tool_id", _bounded_identifier(self.tool_id, "tool_id"))
        if self.tool_version is not None:
            object.__setattr__(
                self, "tool_version", _bounded_identifier(self.tool_version, "tool_version")
            )
        object.__setattr__(self, "arguments", _freeze_object(self.arguments, "arguments"))
        if self.step_id in _identifier_tuple(self.depends_on, "depends_on"):
            raise AgentPlanError("an agent task cannot depend on itself")
        object.__setattr__(
            self,
            "required_capabilities",
            _identifier_tuple(self.required_capabilities, "required_capabilities"),
        )
        object.__setattr__(self, "depends_on", _identifier_tuple(self.depends_on, "depends_on"))
        object.__setattr__(self, "intent", _bounded_text(self.intent, "intent"))
        object.__setattr__(
            self, "timeout_seconds", _bounded_seconds(self.timeout_seconds, "timeout_seconds")
        )

    @property
    def arguments_json(self) -> str:
        return canonical_json_dumps(_thaw_object(self.arguments))

    @property
    def arguments_digest(self) -> str:
        return sha256_hex(self.arguments_json)

    def call_id(self, run_id: str) -> str:
        return f"{_bounded_identifier(run_id, 'run_id')}-{self.step_id}"

    def with_arguments(self, arguments: Mapping[str, JSONValue]) -> AgentTask:
        return AgentTask(
            step_id=self.step_id,
            tool_id=self.tool_id,
            arguments=arguments,
            tool_version=self.tool_version,
            required_capabilities=self.required_capabilities,
            depends_on=self.depends_on,
            intent=self.intent,
            timeout_seconds=self.timeout_seconds,
        )


@dataclass(frozen=True, slots=True)
class CatalogStep:
    catalog_step_id: str
    tool_id: str
    default_arguments: Mapping[str, JSONValue] = field(default_factory=lambda: MappingProxyType({}))
    tool_version: str | None = None
    required_capabilities: tuple[str, ...] = ()
    keywords: tuple[str, ...] = ()
    intent: str = ""

    def __post_init__(self) -> None:
        object.__setattr__(
            self, "catalog_step_id", _bounded_identifier(self.catalog_step_id, "catalog_step_id")
        )
        object.__setattr__(self, "tool_id", _bounded_identifier(self.tool_id, "tool_id"))
        if self.tool_version is not None:
            object.__setattr__(
                self, "tool_version", _bounded_identifier(self.tool_version, "tool_version")
            )
        object.__setattr__(
            self, "default_arguments", _freeze_object(self.default_arguments, "default_arguments")
        )
        object.__setattr__(
            self,
            "required_capabilities",
            _identifier_tuple(self.required_capabilities, "required_capabilities"),
        )
        object.__setattr__(self, "keywords", _identifier_tuple(self.keywords, "keywords"))
        object.__setattr__(self, "intent", _bounded_text(self.intent, "intent"))

    def to_task(
        self,
        step_id: str,
        parameters: Mapping[str, JSONValue] | None = None,
        *,
        depends_on: Iterable[str] = (),
    ) -> AgentTask:
        merged = _thaw_object(self.default_arguments)
        for key, value in (parameters or {}).items():
            if key not in self.default_arguments:
                raise AgentPlanError(
                    f"parameter {key!r} is not declared by catalog step {self.catalog_step_id}"
                )
            merged[key] = value
        return AgentTask(
            step_id=step_id,
            tool_id=self.tool_id,
            tool_version=self.tool_version,
            arguments=merged,
            required_capabilities=self.required_capabilities,
            depends_on=tuple(depends_on),
            intent=self.intent,
        )


class StepCatalog:
    def __init__(
        self,
        steps: Iterable[CatalogStep],
        *,
        max_steps: int = MAX_AGENT_TASKS,
    ) -> None:
        if isinstance(max_steps, bool) or not isinstance(max_steps, int) or max_steps < 1:
            raise AgentPlanError("max_steps must be a positive integer")
        selected: dict[str, CatalogStep] = {}
        for step in steps:
            if not isinstance(step, CatalogStep):
                raise TypeError("catalog steps must be CatalogStep values")
            if step.catalog_step_id in selected:
                raise AgentPlanError(f"catalog step {step.catalog_step_id} is declared twice")
            selected[step.catalog_step_id] = step
        if len(selected) > max_steps:
            raise AgentPlanError(f"the step catalog exceeds {max_steps} entries")
        self._steps = MappingProxyType(selected)
        self._order = tuple(selected)

    def __len__(self) -> int:
        return len(self._steps)

    def __contains__(self, key: object) -> bool:
        return isinstance(key, str) and key in self._steps

    @property
    def step_ids(self) -> tuple[str, ...]:
        return self._order

    def require(self, catalog_step_id: str) -> CatalogStep:
        key = _bounded_identifier(catalog_step_id, "catalog_step_id")
        step = self._steps.get(key)
        if step is None:
            raise AgentPlanError(f"catalog step {key} is not declared")
        return step

    def to_tasks(self, plan_steps: Sequence[PlanStep]) -> tuple[AgentTask, ...]:
        if not isinstance(plan_steps, Sequence):
            raise AgentPlanError("plan steps must be a sequence of PlanStep values")
        if not plan_steps:
            raise AgentPlanError("a plan must contain at least one step")
        if len(plan_steps) > min(len(self._steps), MAX_AGENT_TASKS):
            raise AgentPlanError("the plan exceeds the bounded step count")
        tasks: list[AgentTask] = []
        for plan_step in plan_steps:
            if not isinstance(plan_step, PlanStep):
                raise AgentPlanError("plan steps must be PlanStep values")
            entry = self.require(plan_step.catalog_step_id)
            parameters = _plan_parameters(plan_step.parameters_json, entry)
            tasks.append(
                entry.to_task(
                    plan_step.step_id,
                    parameters,
                    depends_on=plan_step.depends_on,
                )
            )
        return _validate_task_graph(tasks)


def _plan_parameters(
    parameters_json: str,
    entry: CatalogStep,
) -> Mapping[str, JSONValue]:
    try:
        decoded = strict_json_loads(parameters_json)
    except JsonEnvelopeError as error:
        raise AgentPlanError(f"plan parameters are not strict JSON: {error}") from error
    if not isinstance(decoded, Mapping):
        raise AgentPlanError("plan parameters must be a JSON object")
    unknown = sorted(set(decoded) - set(entry.default_arguments))
    if unknown:
        raise AgentPlanError(
            f"parameters are not declared by catalog step {entry.catalog_step_id}: "
            f"{', '.join(unknown)}"
        )
    return cast(Mapping[str, JSONValue], decoded)


@runtime_checkable
class Planner(Protocol):
    def plan(self, objective: str) -> tuple[AgentTask, ...]: ...


class OfflinePlanner:
    def __init__(
        self,
        catalog: StepCatalog,
        *,
        fallback_step_id: str | None = None,
    ) -> None:
        if not isinstance(catalog, StepCatalog):
            raise TypeError("catalog must be a StepCatalog")
        if fallback_step_id is not None:
            catalog.require(fallback_step_id)
        self._catalog = catalog
        self._fallback_step_id = fallback_step_id

    @property
    def catalog(self) -> StepCatalog:
        return self._catalog

    def plan(self, objective: str) -> tuple[AgentTask, ...]:
        if not isinstance(objective, str):
            raise AgentPlanError("the planning objective must be a string")
        objective = _bounded_text(objective, "objective", maximum=MAX_BOUNDED_TEXT)
        tokens = set(_TOKEN_PATTERN.findall(objective.lower()))
        scored: list[tuple[int, str]] = []
        for catalog_step_id in self._catalog.step_ids:
            entry = self._catalog.require(catalog_step_id)
            keywords = {token for keyword in entry.keywords for token in keyword.split(".")}
            score = len(tokens & keywords)
            if entry.tool_id.lower() in tokens:
                score += 1
            if score > 0:
                scored.append((score, catalog_step_id))
        scored.sort(key=lambda item: (-item[0], item[1]))
        if scored:
            selected = tuple(catalog_step_id for _, catalog_step_id in scored)
        elif self._fallback_step_id is not None:
            selected = (self._fallback_step_id,)
        else:
            raise PlannerFailure("no catalog step matches the objective")
        tasks = tuple(
            self._catalog.require(catalog_step_id).to_task(f"step-{index}", None)
            for index, catalog_step_id in enumerate(selected, start=1)
        )
        return _validate_task_graph(tasks)


NEVER_DEGRADED_ERRORS: Final[tuple[type[BaseException], ...]] = (
    AgentPolicyError,
    BudgetExhaustedError,
    EgressDeniedError,
    ModelCapabilityError,
)


class DegradingPlanner:
    def __init__(
        self,
        primary: Planner,
        fallback: Planner,
        *,
        allow_degraded: bool = False,
    ) -> None:
        for name, planner in (("primary", primary), ("fallback", fallback)):
            if not hasattr(planner, "plan"):
                raise TypeError(f"the {name} planner must implement plan")
        self._primary = primary
        self._fallback = fallback
        self._allow_degraded = bool(allow_degraded)
        self._degradations: list[str] = []
        self._lock = threading.RLock()

    @property
    def allow_degraded(self) -> bool:
        return self._allow_degraded

    @property
    def degradations(self) -> tuple[str, ...]:
        with self._lock:
            return tuple(self._degradations)

    def plan(self, objective: str) -> tuple[AgentTask, ...]:
        try:
            return self._primary.plan(objective)
        except NEVER_DEGRADED_ERRORS:
            raise
        except ProviderDegradedError as error:
            reason = f"provider degraded: {error.provider_id}/{error.model_id}"
        except (PlannerFailure, StructuredOutputError) as error:
            reason = f"planner failure: {type(error).__name__}"
        except AgentError as error:
            reason = f"planner rejection: {type(error).__name__}"
        if not self._allow_degraded:
            raise ProviderDegradedError(
                "offline",
                "offline",
                f"{reason}; deterministic planning was not approved",
            ) from None
        with self._lock:
            self._degradations.append(reason[:MAX_ERROR_TEXT])
        return self._fallback.plan(objective)


@dataclass(frozen=True, slots=True)
class AgentStepOutcome:
    step_id: str
    call_id: str
    status: ToolRunStatus
    dispatched: bool
    tool_id: str = ""
    result: Mapping[str, JSONValue] | None = None
    error_code: str | None = None
    error_message: str | None = None
    artifact_ids: tuple[str, ...] = ()
    duration_ms: float = 0.0
    policy_reason: str | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "step_id", _bounded_identifier(self.step_id, "step_id"))
        object.__setattr__(self, "call_id", _bounded_identifier(self.call_id, "call_id"))
        object.__setattr__(self, "tool_id", _bounded_identifier(self.tool_id, "tool_id"))
        if not isinstance(self.status, ToolRunStatus):
            raise AgentMergeError("the outcome status must be a tool run status")
        if not isinstance(self.dispatched, bool):
            raise AgentMergeError("dispatched must be a boolean")
        if self.result is not None and not isinstance(self.result, Mapping):
            raise AgentMergeError("the outcome result must be a JSON object")
        object.__setattr__(
            self, "artifact_ids", _identifier_tuple(self.artifact_ids, "artifact_ids")
        )
        if self.error_message is not None:
            object.__setattr__(
                self,
                "error_message",
                _bounded_text(self.error_message, "error_message", maximum=MAX_ERROR_TEXT),
            )
        if self.policy_reason is not None:
            object.__setattr__(
                self,
                "policy_reason",
                _bounded_text(self.policy_reason, "policy_reason", maximum=MAX_ERROR_TEXT),
            )

    @property
    def succeeded(self) -> bool:
        return self.status is ToolRunStatus.SUCCEEDED and self.result is not None

    @property
    def denied(self) -> bool:
        return self.status is ToolRunStatus.DENIED

    @classmethod
    def from_tool_result(
        cls,
        step_id: str,
        result: ToolResult,
        *,
        tool_id: str,
    ) -> AgentStepOutcome:
        payload: Mapping[str, JSONValue] | None = None
        if result.result_json is not None:
            decoded = strict_json_loads(result.result_json)
            if not isinstance(decoded, Mapping):
                raise AgentMergeError("a tool result payload must be a JSON object")
            payload = cast(Mapping[str, JSONValue], decoded)
        return cls(
            step_id=step_id,
            call_id=result.call_id,
            status=result.status,
            dispatched=True,
            tool_id=tool_id,
            result=payload,
            error_code=result.error_code,
            error_message=result.error_message,
            artifact_ids=tuple(artifact.artifact_id for artifact in result.artifacts),
            duration_ms=0.0 if result.duration_ms is None else result.duration_ms,
        )

    def to_payload(self) -> dict[str, object]:
        return {
            "artifact_ids": list(self.artifact_ids),
            "call_id": self.call_id,
            "dispatched": self.dispatched,
            "duration_ms": self.duration_ms,
            "error_code": self.error_code,
            "error_message": self.error_message,
            "policy_reason": self.policy_reason,
            "result": None if self.result is None else _thaw_object(self.result),
            "status": self.status.value,
            "step_id": self.step_id,
            "tool_id": self.tool_id,
        }


@dataclass(frozen=True, slots=True)
class AgentRunResult:
    run_id: str
    outcomes: tuple[AgentStepOutcome, ...]
    merged: Mapping[str, JSONValue]
    digests: Mapping[str, str]
    degraded_reasons: tuple[str, ...] = ()
    budget_denied: tuple[str, ...] = ()

    def __post_init__(self) -> None:
        object.__setattr__(self, "run_id", _bounded_identifier(self.run_id, "run_id"))
        if not isinstance(self.outcomes, (tuple, list)):
            raise AgentMergeError("outcomes must be a sequence of step outcomes")
        object.__setattr__(self, "outcomes", tuple(self.outcomes))
        object.__setattr__(self, "merged", _freeze_object(self.merged, "merged"))
        if not isinstance(self.digests, Mapping):
            raise AgentMergeError("digests must be a mapping of step identifiers")
        digests: dict[str, str] = {}
        for key, value in self.digests.items():
            name = _bounded_identifier(key, "digest key")
            if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
                raise AgentMergeError("digests must be sha256 hexadecimal digests")
            digests[name] = value
        object.__setattr__(self, "digests", MappingProxyType(digests))
        object.__setattr__(
            self, "degraded_reasons", _bounded_reasons(self.degraded_reasons, "degraded_reasons")
        )
        object.__setattr__(
            self, "budget_denied", _identifier_tuple(self.budget_denied, "budget_denied")
        )

    @property
    def succeeded(self) -> bool:
        return bool(self.outcomes) and all(outcome.succeeded for outcome in self.outcomes)

    @property
    def step_ids(self) -> tuple[str, ...]:
        return tuple(outcome.step_id for outcome in self.outcomes)

    @property
    def artifacts(self) -> tuple[str, ...]:
        collected: list[str] = []
        for outcome in self.outcomes:
            for artifact_id in outcome.artifact_ids:
                if artifact_id not in collected:
                    collected.append(artifact_id)
        return tuple(collected)

    def outcome_for(self, step_id: str) -> AgentStepOutcome:
        key = _bounded_identifier(step_id, "step_id")
        for outcome in self.outcomes:
            if outcome.step_id == key:
                return outcome
        raise AgentMergeError(f"no outcome was recorded for step {key}")

    def to_payload(self) -> dict[str, object]:
        return {
            "artifacts": list(self.artifacts),
            "budget_denied": list(self.budget_denied),
            "degraded_reasons": list(self.degraded_reasons),
            "digests": dict(self.digests),
            "merged": _thaw_object(self.merged),
            "outcomes": [outcome.to_payload() for outcome in self.outcomes],
            "run_id": self.run_id,
            "status": "completed" if self.succeeded else "incomplete",
        }


def _validate_task_graph(tasks: Sequence[AgentTask]) -> tuple[AgentTask, ...]:
    if not isinstance(tasks, Sequence):
        raise AgentPlanError("tasks must be a sequence of agent tasks")
    if not tasks:
        raise AgentPlanError("a run requires at least one task")
    if len(tasks) > MAX_AGENT_TASKS:
        raise AgentPlanError(f"a run cannot exceed {MAX_AGENT_TASKS} tasks")
    step_ids: set[str] = set()
    for task in tasks:
        if not isinstance(task, AgentTask):
            raise AgentPlanError("tasks must contain AgentTask values")
        if task.step_id in step_ids:
            raise AgentPlanError(f"duplicate task identifier: {task.step_id}")
        step_ids.add(task.step_id)
    for task in tasks:
        unknown = sorted(set(task.depends_on) - step_ids)
        if unknown:
            raise AgentPlanError(
                f"task {task.step_id} depends on undeclared tasks: {', '.join(unknown)}"
            )
    return _dependency_order(tasks)


def _dependency_order(tasks: Sequence[AgentTask]) -> tuple[AgentTask, ...]:
    by_id = {task.step_id: task for task in tasks}
    remaining = dict(by_id)
    ordered: list[AgentTask] = []
    while remaining:
        ready = [
            task
            for _, task in sorted(remaining.items())
            if all(dependency not in remaining for dependency in task.depends_on)
        ]
        if not ready:
            raise AgentPlanError("task dependencies contain a cycle")
        for task in ready:
            ordered.append(task)
            del remaining[task.step_id]
    return tuple(ordered)


def _dependency_layers(tasks: Sequence[AgentTask]) -> tuple[tuple[AgentTask, ...], ...]:
    remaining = {task.step_id: task for task in tasks}
    completed: set[str] = set()
    layers: list[tuple[AgentTask, ...]] = []
    while remaining:
        ready = tuple(
            task for _, task in sorted(remaining.items()) if set(task.depends_on) <= completed
        )
        if not ready:
            raise AgentPlanError("task dependencies contain a cycle")
        layers.append(ready)
        for task in ready:
            del remaining[task.step_id]
        completed.update(task.step_id for task in ready)
    return tuple(layers)


class ModelPlanner:
    def __init__(
        self,
        gateway: ModelGateway,
        catalog: StepCatalog,
        *,
        max_tokens: int = 1_024,
    ) -> None:
        if not isinstance(gateway, ModelGateway):
            raise TypeError("gateway must be a ModelGateway")
        if not isinstance(catalog, StepCatalog):
            raise TypeError("catalog must be a StepCatalog")
        if isinstance(max_tokens, bool) or not isinstance(max_tokens, int) or max_tokens < 1:
            raise AgentPlanError("max_tokens must be a positive integer")
        self._gateway = gateway
        self._catalog = catalog
        self._max_tokens = max_tokens
        self._lock = threading.RLock()

    @property
    def catalog(self) -> StepCatalog:
        return self._catalog

    @property
    def gateway(self) -> ModelGateway:
        return self._gateway

    def response_schema(self) -> dict[str, object]:
        return {
            "additionalProperties": False,
            "properties": {
                "steps": {
                    "items": {
                        "additionalProperties": False,
                        "properties": {
                            "catalog_step_id": {
                                "enum": list(self._catalog.step_ids),
                                "type": "string",
                            },
                            "intent": {"maxLength": MAX_BOUNDED_TEXT, "type": "string"},
                            "parameters": {"type": "object"},
                        },
                        "required": ["catalog_step_id", "intent", "parameters"],
                        "type": "object",
                    },
                    "maxItems": MAX_AGENT_TASKS,
                    "type": "array",
                }
            },
            "required": ["steps"],
            "title": "kala.plan",
            "type": "object",
        }

    def plan(self, objective: str) -> tuple[AgentTask, ...]:
        if not isinstance(objective, str) or not objective.strip():
            raise AgentPlanError("the planning objective must be a nonempty string")
        prompt = _bounded_text(objective, "objective", maximum=MAX_BOUNDED_TEXT)
        request = ModelRequest(
            request_id=f"plan-{sha256_hex(prompt)[:16]}",
            messages=(
                ModelMessage(
                    role=ModelRole.SYSTEM,
                    content="Return a bounded plan as a JSON object.",
                ),
                ModelMessage(role=ModelRole.USER, content=prompt),
            ),
            response_schema=self.response_schema(),
            max_output_tokens=self._max_tokens,
        )
        with self._lock:
            response = self._gateway.complete(request)
        return self._catalog.to_tasks(self._plan_steps(response))

    def _plan_steps(self, response: ModelResponse) -> tuple[PlanStep, ...]:
        structured = response.structured
        if structured is None:
            raise StructuredOutputError("the planner response carried no structured output")
        payload = structured.get("steps")
        if not isinstance(payload, (list, tuple)) or not payload:
            raise StructuredOutputError("the planner response contained no steps")
        steps: list[PlanStep] = []
        for index, entry in enumerate(payload, start=1):
            if not isinstance(entry, Mapping):
                raise StructuredOutputError("a planned step is not a JSON object")
            catalog_step_id = entry.get("catalog_step_id")
            intent = entry.get("intent")
            parameters = entry.get("parameters")
            if not isinstance(catalog_step_id, str) or not isinstance(intent, str):
                raise StructuredOutputError("a planned step is missing its identity")
            if not isinstance(parameters, Mapping):
                raise StructuredOutputError("planned step parameters must be a JSON object")
            steps.append(
                PlanStep(
                    step_id=f"step-{index}",
                    catalog_step_id=catalog_step_id,
                    intent=intent[:MAX_BOUNDED_TEXT] or "planned step",
                    parameters_json=canonical_json_dumps(
                        _thaw_object(cast(Mapping[str, JSONValue], parameters))
                    ),
                )
            )
        return tuple(steps)


class AgentCoordinator:
    def __init__(
        self,
        executor: ToolExecutor,
        *,
        policy: PolicyEngine | None = None,
        planner: Planner | None = None,
        ledger: BudgetLedger | None = None,
        gateway: ModelGateway | None = None,
        catalog: StepCatalog | None = None,
        max_parallel_tasks: int = 4,
        clock: Clock | None = None,
    ) -> None:
        if not isinstance(executor, ToolExecutor):
            raise TypeError("executor must be a ToolExecutor")
        if policy is not None and not isinstance(policy, PolicyEngine):
            raise TypeError("policy must be a PolicyEngine")
        if ledger is not None and not isinstance(ledger, BudgetLedger):
            raise TypeError("ledger must be a BudgetLedger")
        if gateway is not None and not isinstance(gateway, ModelGateway):
            raise TypeError("gateway must be a ModelGateway")
        if catalog is not None and not isinstance(catalog, StepCatalog):
            raise TypeError("catalog must be a StepCatalog")
        if planner is not None and not hasattr(planner, "plan"):
            raise TypeError("planner must implement the Planner protocol")
        if (
            isinstance(max_parallel_tasks, bool)
            or not isinstance(max_parallel_tasks, int)
            or not 1 <= max_parallel_tasks <= MAX_PARALLEL_TASKS
        ):
            raise AgentPlanError(f"max_parallel_tasks must be between 1 and {MAX_PARALLEL_TASKS}")
        if clock is not None and not callable(clock):
            raise TypeError("clock must be callable")
        self._executor = executor
        self._policy = policy
        self._planner = planner
        self._ledger = ledger
        self._gateway = gateway
        self._catalog = catalog
        self._max_parallel = max_parallel_tasks
        self._clock = clock if clock is not None else lambda: datetime.now(UTC)

    @property
    def executor(self) -> ToolExecutor:
        return self._executor

    @property
    def registry(self) -> ToolRegistry:
        return self._executor.registry

    @property
    def policy(self) -> PolicyEngine | None:
        return self._policy

    @property
    def planner(self) -> Planner | None:
        return self._planner

    @property
    def ledger(self) -> BudgetLedger | None:
        return self._ledger

    @property
    def gateway(self) -> ModelGateway | None:
        return self._gateway

    @property
    def catalog(self) -> StepCatalog | None:
        return self._catalog

    @property
    def max_parallel_tasks(self) -> int:
        return self._max_parallel

    def plan(self, objective: str) -> tuple[AgentTask, ...]:
        if self._planner is None:
            raise AgentPlanError("no planner is configured for this coordinator")
        if not isinstance(objective, str) or not objective.strip():
            raise AgentPlanError("the planning objective must be a nonempty string")
        return _validate_task_graph(
            self._planner.plan(_bounded_text(objective, "objective", maximum=MAX_BOUNDED_TEXT))
        )

    def run(self, tasks: Sequence[AgentTask], *, run_id: str = "run-1") -> AgentRunResult:
        identifier = _bounded_identifier(run_id, "run_id")
        normalized = _dependency_order(_validate_task_graph(tasks))
        outcomes: dict[str, AgentStepOutcome] = {}
        budget_denied: list[str] = []
        for layer in _dependency_layers(normalized):
            dispatchable: list[AgentTask] = []
            for task in layer:
                blocked = sorted(
                    dependency
                    for dependency in task.depends_on
                    if not outcomes[dependency].succeeded
                )
                if blocked:
                    outcomes[task.step_id] = self._blocked_outcome(
                        task, identifier, f"dependency did not succeed: {', '.join(blocked)}"
                    )
                    continue
                dispatchable.append(task)
            for outcome in self._dispatch_layer(dispatchable, identifier):
                outcomes[outcome.step_id] = outcome
                if outcome.error_code == str(ToolErrorCode.BUDGET_EXHAUSTED):
                    budget_denied.append(outcome.step_id)
        ordered = tuple(outcomes[task.step_id] for task in normalized)
        merged, digests = self._merge(ordered)
        return AgentRunResult(
            run_id=identifier,
            outcomes=ordered,
            merged=merged,
            digests=digests,
            degraded_reasons=self._degradation_reasons(),
            budget_denied=tuple(sorted(set(budget_denied))),
        )

    def run_plan(self, plan_steps: Sequence[PlanStep], *, run_id: str = "run-1") -> AgentRunResult:
        catalog = self._catalog
        if catalog is None:
            raise AgentPlanError("no step catalog is configured for this coordinator")
        return self.run(catalog.to_tasks(plan_steps), run_id=run_id)

    def run_objective(self, objective: str, *, run_id: str = "run-1") -> AgentRunResult:
        return self.run(self.plan(objective), run_id=run_id)

    def _dispatch_layer(
        self,
        tasks: Sequence[AgentTask],
        run_id: str,
    ) -> tuple[AgentStepOutcome, ...]:
        if not tasks:
            return ()
        denied = self._admission_check(tasks, run_id)
        if denied is not None:
            return denied
        if len(tasks) == 1:
            return (self._dispatch(tasks[0], run_id),)
        with ThreadPoolExecutor(
            max_workers=min(self._max_parallel, len(tasks)),
            thread_name_prefix="agent-task",
        ) as pool:
            return tuple(pool.map(lambda task: self._dispatch(task, run_id), tasks))

    def _admission_check(
        self,
        tasks: Sequence[AgentTask],
        run_id: str,
    ) -> tuple[AgentStepOutcome, ...] | None:
        ledger = self._ledger
        if ledger is None:
            return None
        try:
            ledger.check(tool_calls=len(tasks))
        except BudgetExhaustedError as error:
            return tuple(
                AgentStepOutcome(
                    step_id=task.step_id,
                    call_id=task.call_id(run_id),
                    status=ToolRunStatus.DENIED,
                    dispatched=False,
                    tool_id=task.tool_id,
                    error_code=str(ToolErrorCode.BUDGET_EXHAUSTED),
                    error_message=str(error)[:MAX_ERROR_TEXT],
                )
                for task in tasks
            )
        return None

    def _blocked_outcome(self, task: AgentTask, run_id: str, reason: str) -> AgentStepOutcome:
        return AgentStepOutcome(
            step_id=task.step_id,
            call_id=task.call_id(run_id),
            status=ToolRunStatus.CANCELLED,
            dispatched=False,
            tool_id=task.tool_id,
            error_message=reason,
        )

    def _dispatch(self, task: AgentTask, run_id: str) -> AgentStepOutcome:
        call_id = task.call_id(run_id)
        try:
            tool = self.registry.require(task.tool_id, task.tool_version)
        except ToolRegistryError as error:
            return AgentStepOutcome(
                step_id=task.step_id,
                call_id=call_id,
                status=ToolRunStatus.DENIED,
                dispatched=False,
                tool_id=task.tool_id,
                error_code=str(ToolErrorCode.UNKNOWN_TOOL),
                error_message=str(error),
            )
        undeclared = sorted(set(task.required_capabilities) - set(tool.required_capabilities))
        if undeclared:
            return AgentStepOutcome(
                step_id=task.step_id,
                call_id=call_id,
                status=ToolRunStatus.DENIED,
                dispatched=False,
                tool_id=tool.tool_id,
                error_code=str(ToolErrorCode.CAPABILITY_DENIED),
                error_message=(
                    f"the task requests undeclared capabilities: {', '.join(undeclared)}"
                ),
            )
        call = self._build_call(task, tool, call_id)
        denial = self._preflight(tool, call)
        if denial is not None:
            reason, code = denial
            return AgentStepOutcome(
                step_id=task.step_id,
                call_id=call_id,
                status=ToolRunStatus.DENIED,
                dispatched=False,
                tool_id=tool.tool_id,
                error_code=str(code),
                error_message=reason,
                policy_reason=reason,
            )
        result = self._executor.execute(call, timeout_seconds=task.timeout_seconds)
        return AgentStepOutcome.from_tool_result(task.step_id, result, tool_id=tool.tool_id)

    def _build_call(self, task: AgentTask, tool: RegisteredTool, call_id: str) -> ToolCall:
        return ToolCall(
            call_id=call_id,
            tool_id=tool.tool_id,
            tool_version=tool.version,
            arguments_json=task.arguments_json,
            requested_capabilities=task.required_capabilities,
            side_effect_level=tool.side_effect_level,
            timeout_seconds=self._executor.profile.timeout_for(
                task.timeout_seconds, tool.descriptor.timeout_seconds
            ),
            issued_at=self._now(),
        )

    def _preflight(self, tool: RegisteredTool, call: ToolCall) -> Denial | None:
        if not self._requires_approval(tool):
            return None
        policy = self._policy
        if policy is None:
            raise AgentPolicyError(
                f"tool {tool.tool_id} requires a policy decision and no policy engine is attached"
            )
        decision = policy.decide(self._executor.policy_request(call))
        if decision.allowed:
            return None
        reason = f"policy decision {decision.reason.value} for request {decision.request_id}"
        code = (
            ToolErrorCode.CAPABILITY_DENIED
            if decision.missing_capabilities
            else ToolErrorCode.APPROVAL_REQUIRED
        )
        return reason, code

    @staticmethod
    def _requires_approval(tool: RegisteredTool) -> bool:
        return tool.descriptor.destructive or tool.side_effect_level in APPROVAL_REQUIRED_EFFECTS

    def _merge(
        self,
        outcomes: Sequence[AgentStepOutcome],
    ) -> tuple[Mapping[str, JSONValue], Mapping[str, str]]:
        merged: dict[str, JSONValue] = {}
        digests: dict[str, str] = {}
        for outcome in outcomes:
            if not outcome.succeeded or outcome.result is None:
                continue
            tool = self.registry.require(outcome.tool_id)
            try:
                tool.validate_result(cast(Mapping[str, object], outcome.result))
            except (SchemaValidationError, JsonEnvelopeError) as error:
                raise AgentMergeError(
                    f"the merged result of step {outcome.step_id} failed schema validation: {error}"
                ) from error
            if outcome.step_id in merged:
                raise AgentMergeError(f"a merged result already exists for {outcome.step_id}")
            canonical = _thaw_object(outcome.result)
            merged[outcome.step_id] = canonical
            digests[outcome.step_id] = canonical_json_digest(canonical)
        serialized = canonical_json_dumps(_thaw_object(merged), max_bytes=MAX_MERGED_RESULT_BYTES)
        if len(serialized.encode("utf-8")) > MAX_MERGED_RESULT_BYTES:
            raise AgentMergeError("the merged results exceed the bounded size")
        return MappingProxyType(merged), MappingProxyType(digests)

    def _degradation_reasons(self) -> tuple[str, ...]:
        planner = self._planner
        if isinstance(planner, DegradingPlanner):
            return planner.degradations
        return ()

    def _now(self) -> datetime:
        value = self._clock()
        if value.tzinfo is None or value.utcoffset() is None:
            raise AgentError("the coordinator clock must return an aware datetime")
        return value.astimezone(UTC)


__all__ = (
    "MAX_AGENT_TASKS",
    "MAX_MERGED_RESULT_BYTES",
    "MAX_PARALLEL_TASKS",
    "NEVER_DEGRADED_ERRORS",
    "AgentCoordinator",
    "AgentError",
    "AgentMergeError",
    "AgentPlanError",
    "AgentPolicyError",
    "AgentRunResult",
    "AgentStepOutcome",
    "AgentTask",
    "CatalogStep",
    "Clock",
    "DegradingPlanner",
    "ModelPlanner",
    "OfflinePlanner",
    "Planner",
    "PlannerFailure",
    "StepCatalog",
)
