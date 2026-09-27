from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import FrozenInstanceError
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, cast

import pytest

from kala_agent.contracts import (
    PlanStep,
    RunBudget,
    SideEffectLevel,
    ToolCall,
    ToolResult,
    ToolRunStatus,
)
from kala_agent.kernel.budget import BudgetExhaustedError, BudgetLedger
from kala_agent.runtime import agent as agent_module
from kala_agent.runtime.agent import (
    MAX_AGENT_TASKS,
    MAX_PARALLEL_TASKS,
    NEVER_DEGRADED_ERRORS,
    AgentCoordinator,
    AgentError,
    AgentMergeError,
    AgentPlanError,
    AgentPolicyError,
    AgentRunResult,
    AgentStepOutcome,
    AgentTask,
    CatalogStep,
    DegradingPlanner,
    ModelPlanner,
    OfflinePlanner,
    PlannerFailure,
    StepCatalog,
    _bounded_identifier,
    _bounded_reasons,
    _bounded_seconds,
    _bounded_text,
    _dependency_layers,
    _dependency_order,
    _freeze_object,
    _freeze_value,
    _identifier_tuple,
    _plan_parameters,
    _thaw,
    _thaw_object,
    _validate_task_graph,
)
from kala_agent.runtime.model_gateway import (
    EgressDeniedError,
    ModelCapability,
    ModelCapabilityError,
    ModelGateway,
    ModelRequest,
    ModelResponse,
    ProviderDegradedError,
    StructuredOutputError,
)
from kala_agent.tools.executor import IsolationMode, ToolExecutor
from kala_agent.tools.protocol import JSONValue, canonical_json_dumps
from kala_agent.tools.registry import ToolRegistry
from kala_agent.tools.simulator import register_reference_evaluator

ISSUED_AT = datetime(2026, 1, 2, 3, 4, 5, tzinfo=UTC)


def _catalog_step(catalog_step_id: str = "simulate") -> CatalogStep:
    return CatalogStep(
        catalog_step_id=catalog_step_id,
        tool_id="mechanical.reference-evaluator",
        default_arguments=cast("Mapping[str, JSONValue]", {"operation": "beam_volume"}),
        tool_version="1.0.0",
        keywords=("beam", "volume"),
        intent="Simulate the beam.",
    )


def _catalog() -> StepCatalog:
    return StepCatalog([_catalog_step("simulate"), _catalog_step("summarize")])


def _task(step_id: str = "step-1", **overrides: object) -> AgentTask:
    values: dict[str, Any] = {
        "step_id": step_id,
        "tool_id": "mechanical.reference-evaluator",
        "tool_version": "1.0.0",
        "arguments": cast("Mapping[str, JSONValue]", {"operation": "beam_volume"}),
    }
    values.update(cast("Mapping[str, Any]", overrides))
    return AgentTask(**values)


def _tool_result(
    status: ToolRunStatus = ToolRunStatus.SUCCEEDED,
    *,
    result_json: str | None = '{"operation":"beam_volume"}',
    error_code: str | None = None,
) -> ToolResult:
    return ToolResult(
        call_id="run-1-step-1",
        tool_id="mechanical.reference-evaluator",
        status=status,
        result_json=result_json,
        error_code=error_code,
        started_at=ISSUED_AT,
        finished_at=ISSUED_AT,
        duration_ms=1.0,
    )


def _executor(tmp_path: Path) -> ToolExecutor:
    registry = ToolRegistry()
    register_reference_evaluator(registry)
    return ToolExecutor(
        registry,
        run_directory=tmp_path / "run",
        isolation=_isolation(),
    )


def _isolation() -> IsolationMode:
    return IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR


def test_bounded_identifier_rejects_bad_values() -> None:
    with pytest.raises(AgentPlanError, match="must be a string"):
        _bounded_identifier(7, "step_id")
    with pytest.raises(AgentPlanError, match="safe identifier"):
        _bounded_identifier("has space", "step_id")
    with pytest.raises(AgentPlanError, match="safe identifier"):
        _bounded_identifier("", "step_id")
    assert _bounded_identifier("  step-1  ", "step_id") == "step-1"


def test_bounded_text_rejects_bad_values() -> None:
    with pytest.raises(AgentPlanError, match="must be a string"):
        _bounded_text(7, "intent")
    with pytest.raises(AgentPlanError, match="bounded length"):
        _bounded_text("x" * 600, "intent")
    assert _bounded_text("x", "intent", maximum=1) == "x"


def test_bounded_seconds_accepts_none_and_bounds_values() -> None:
    assert _bounded_seconds(None, "timeout_seconds") is None
    assert _bounded_seconds(5, "timeout_seconds") == 5.0
    with pytest.raises(AgentPlanError, match="must be a number"):
        _bounded_seconds(True, "timeout_seconds")
    with pytest.raises(AgentPlanError, match="must be a number"):
        _bounded_seconds("5", "timeout_seconds")
    with pytest.raises(AgentPlanError, match="positive and bounded"):
        _bounded_seconds(0.0, "timeout_seconds")
    with pytest.raises(AgentPlanError, match="positive and bounded"):
        _bounded_seconds(3_601.0, "timeout_seconds")


def test_identifier_tuple_sorts_and_deduplicates() -> None:
    assert _identifier_tuple(["b", "a", "a"], "caps") == ("a", "b")
    assert _identifier_tuple(frozenset({"z"}), "caps") == ("z",)
    with pytest.raises(AgentPlanError, match="collection of identifiers"):
        _identifier_tuple(cast(Any, "a"), "caps")
    with pytest.raises(AgentPlanError, match="collection of identifiers"):
        _identifier_tuple(cast(Any, b"a"), "caps")
    with pytest.raises(AgentPlanError, match="collection of identifiers"):
        _identifier_tuple(cast(Any, 7), "caps")


def test_bounded_reasons_validates_entries() -> None:
    assert _bounded_reasons(["because"], "reasons") == ("because",)
    with pytest.raises(AgentPlanError, match="collection of reasons"):
        _bounded_reasons(cast(Any, "because"), "reasons")
    with pytest.raises(AgentPlanError, match="reasons entry must be a string"):
        _bounded_reasons(cast(Any, [7]), "reasons")
    with pytest.raises(AgentPlanError, match="reasons entry exceeds"):
        _bounded_reasons(["x" * 2_000], "reasons")


def test_freeze_value_accepts_json_scalars_and_containers() -> None:
    assert _freeze_value(None) is None
    assert _freeze_value(True) is True
    assert _freeze_value(7) == 7
    assert _freeze_value("text") == "text"
    assert _freeze_value(1.5) == 1.5
    assert list(cast("Sequence[JSONValue]", _freeze_value([1, {"a": 2}]))) == [1, {"a": 2}]


def test_freeze_value_rejects_non_finite_floats() -> None:
    with pytest.raises(AgentPlanError, match="finite numbers"):
        _freeze_value(float("nan"))
    with pytest.raises(AgentPlanError, match="finite numbers"):
        _freeze_value(float("inf"))


def test_freeze_value_rejects_bad_keys_and_depth() -> None:
    with pytest.raises(AgentPlanError, match="keys must be nonempty strings"):
        _freeze_value({"": 1})
    with pytest.raises(AgentPlanError, match="keys must be nonempty strings"):
        _freeze_value(cast("Mapping[Any, Any]", {7: 1}))
    deep: Any = 1
    for _ in range(20):
        deep = {"a": deep}
    with pytest.raises(AgentPlanError, match="nesting depth"):
        _freeze_value(deep)


def test_freeze_value_rejects_unsupported_values() -> None:
    with pytest.raises(AgentPlanError, match="JSON serializable"):
        _freeze_value({1, 2})


def test_freeze_object_requires_a_mapping() -> None:
    with pytest.raises(AgentPlanError, match="must be a JSON object"):
        _freeze_object(["a"], "arguments")
    with pytest.raises(AgentPlanError, match="keys must be nonempty strings"):
        _freeze_object(cast("Mapping[Any, Any]", {"": 1}), "arguments")
    frozen = _freeze_object(cast("Mapping[str, JSONValue]", {"a": 1}), "arguments")
    with pytest.raises(TypeError):
        cast("dict[str, JSONValue]", frozen)["b"] = 2


def test_thaw_roundtrips_frozen_containers() -> None:
    assert _thaw(frozen := _freeze_value({"a": [1, {"b": 2}]})) == {"a": [1, {"b": 2}]}
    assert _thaw_object(cast("Mapping[str, JSONValue]", {"a": 1})) == {"a": 1}
    assert _thaw(7) == 7
    assert frozen is not None


def test_agent_task_validates_every_field() -> None:
    with pytest.raises(AgentPlanError, match="step_id"):
        AgentTask(step_id="bad id", tool_id="tool")
    with pytest.raises(AgentPlanError, match="tool_id"):
        AgentTask(step_id="step-1", tool_id="")
    with pytest.raises(AgentPlanError, match="tool_version"):
        AgentTask(step_id="step-1", tool_id="tool", tool_version="bad version")
    with pytest.raises(AgentPlanError, match="must be a JSON object"):
        AgentTask(step_id="step-1", tool_id="tool", arguments=cast(Any, ["a"]))
    with pytest.raises(AgentPlanError, match="cannot depend on itself"):
        AgentTask(step_id="step-1", tool_id="tool", depends_on=("step-1",))
    with pytest.raises(AgentPlanError, match="intent"):
        AgentTask(step_id="step-1", tool_id="tool", intent="x" * 600)
    with pytest.raises(AgentPlanError, match="timeout_seconds"):
        AgentTask(step_id="step-1", tool_id="tool", timeout_seconds=-1.0)
    with pytest.raises(AgentPlanError, match="required_capabilities"):
        AgentTask(
            step_id="step-1",
            tool_id="tool",
            required_capabilities=cast(Any, "artifact.read"),
        )


def test_agent_task_exposes_digests_and_call_ids() -> None:
    task = _task()
    assert task.arguments_json == '{"operation":"beam_volume"}'
    assert len(task.arguments_digest) == 64
    assert task.call_id("run-1") == "run-1-step-1"
    with pytest.raises(AgentPlanError, match="run_id"):
        task.call_id("bad run")
    assert (
        task.with_arguments(
            cast("Mapping[str, JSONValue]", {"operation": "bar_summary"})
        ).arguments["operation"]
        == "bar_summary"
    )
    frozen_field = "step_id"
    with pytest.raises(FrozenInstanceError):
        setattr(task, frozen_field, "changed")


def test_catalog_step_merges_declared_parameters() -> None:
    step = _catalog_step()
    task = step.to_task("step-1", cast("Mapping[str, JSONValue]", {"operation": "bar_summary"}))
    assert task.arguments["operation"] == "bar_summary"
    assert task.tool_id == "mechanical.reference-evaluator"
    assert task.tool_version == "1.0.0"
    assert task.intent == "Simulate the beam."
    assert step.to_task("step-1").arguments["operation"] == "beam_volume"


def test_catalog_step_rejects_undeclared_parameters() -> None:
    with pytest.raises(AgentPlanError, match="is not declared by catalog step"):
        _catalog_step().to_task("step-1", cast("Mapping[str, JSONValue]", {"made_up": 1}))


def test_catalog_step_validates_every_field() -> None:
    with pytest.raises(AgentPlanError, match="catalog_step_id"):
        CatalogStep(catalog_step_id="bad id", tool_id="tool")
    with pytest.raises(AgentPlanError, match="tool_id"):
        CatalogStep(catalog_step_id="c-1", tool_id="")
    with pytest.raises(AgentPlanError, match="tool_version"):
        CatalogStep(catalog_step_id="c-1", tool_id="tool", tool_version="bad version")
    with pytest.raises(AgentPlanError, match="default_arguments must be a JSON object"):
        CatalogStep(catalog_step_id="c-1", tool_id="tool", default_arguments=cast(Any, []))
    with pytest.raises(AgentPlanError, match="required_capabilities"):
        CatalogStep(
            catalog_step_id="c-1",
            tool_id="tool",
            required_capabilities=cast(Any, "artifact.read"),
        )
    with pytest.raises(AgentPlanError, match="keywords"):
        CatalogStep(catalog_step_id="c-1", tool_id="tool", keywords=cast(Any, "beam"))
    with pytest.raises(AgentPlanError, match="intent"):
        CatalogStep(catalog_step_id="c-1", tool_id="tool", intent="x" * 600)


def test_step_catalog_rejects_bad_configuration() -> None:
    with pytest.raises(AgentPlanError, match="max_steps must be a positive integer"):
        StepCatalog([], max_steps=0)
    with pytest.raises(AgentPlanError, match="max_steps must be a positive integer"):
        StepCatalog([], max_steps=cast(int, True))
    with pytest.raises(TypeError, match="CatalogStep"):
        StepCatalog(cast(Any, [object()]))
    with pytest.raises(AgentPlanError, match="declared twice"):
        StepCatalog([_catalog_step("simulate"), _catalog_step("simulate")])
    with pytest.raises(AgentPlanError, match="exceeds 1 entries"):
        StepCatalog([_catalog_step("a"), _catalog_step("b")], max_steps=1)


def test_step_catalog_exposes_its_entries() -> None:
    catalog = _catalog()
    assert len(catalog) == 2
    assert "simulate" in catalog
    assert 7 not in catalog
    assert catalog.step_ids == ("simulate", "summarize")
    assert catalog.require("simulate").catalog_step_id == "simulate"
    assert catalog.require("  simulate  ").catalog_step_id == "simulate"
    with pytest.raises(AgentPlanError, match="is not declared"):
        catalog.require("absent")


def test_step_catalog_rejects_oversized_plans() -> None:
    catalog = _catalog()
    with pytest.raises(AgentPlanError, match="plan steps must be a sequence"):
        catalog.to_tasks(cast(Any, 7))
    with pytest.raises(AgentPlanError, match="at least one step"):
        catalog.to_tasks(())
    plan_steps = tuple(
        PlanStep(
            step_id=f"step-{index}",
            catalog_step_id="simulate",
            intent="Simulate the beam.",
        )
        for index in range(3)
    )
    with pytest.raises(AgentPlanError, match="exceeds the bounded step count"):
        catalog.to_tasks(plan_steps)


def test_step_catalog_rejects_non_plan_steps() -> None:
    with pytest.raises(AgentPlanError, match="PlanStep values"):
        _catalog().to_tasks(cast(Any, (object(),)))


def test_step_catalog_converts_a_valid_plan() -> None:
    tasks = _catalog().to_tasks(
        (
            PlanStep(
                step_id="step-1",
                catalog_step_id="simulate",
                intent="Simulate the beam.",
                parameters_json='{"operation":"bar_summary"}',
            ),
        )
    )
    assert [task.step_id for task in tasks] == ["step-1"]
    assert tasks[0].arguments["operation"] == "bar_summary"


def test_plan_parameters_rejects_malformed_json() -> None:
    with pytest.raises(AgentPlanError, match="not strict JSON"):
        _plan_parameters("{not json", _catalog_step())


def test_plan_parameters_rejects_non_objects() -> None:
    with pytest.raises(AgentPlanError, match="must be a JSON object"):
        _plan_parameters("[]", _catalog_step())


def test_plan_parameters_rejects_undeclared_keys() -> None:
    with pytest.raises(AgentPlanError, match="not declared by catalog step"):
        _plan_parameters('{"made_up":1}', _catalog_step())


def test_offline_planner_requires_a_catalog() -> None:
    with pytest.raises(TypeError, match="StepCatalog"):
        OfflinePlanner(cast(Any, object()))


def test_offline_planner_validates_the_fallback_step() -> None:
    catalog = _catalog()
    assert OfflinePlanner(catalog, fallback_step_id="simulate").catalog is catalog
    with pytest.raises(AgentPlanError, match="is not declared"):
        OfflinePlanner(catalog, fallback_step_id="absent")


def test_offline_planner_scores_matching_catalog_steps() -> None:
    planner = OfflinePlanner(_catalog())
    assert [task.step_id for task in planner.plan("Compute the beam volume")] == [
        "step-1",
        "step-2",
    ]
    assert [task.step_id for task in planner.plan("volume")] == ["step-1", "step-2"]


def test_offline_planner_prefers_the_fallback_when_nothing_matches() -> None:
    planner = OfflinePlanner(_catalog(), fallback_step_id="summarize")
    tasks = planner.plan("nothing in the catalog matches this")
    assert [task.step_id for task in tasks] == ["step-1"]
    assert tasks[0].tool_id == "mechanical.reference-evaluator"


def test_offline_planner_reports_a_total_miss() -> None:
    with pytest.raises(PlannerFailure, match="no catalog step matches"):
        OfflinePlanner(_catalog()).plan("nothing matches")


def test_offline_planner_validates_the_objective() -> None:
    planner = OfflinePlanner(_catalog())
    with pytest.raises(AgentPlanError, match="must be a string"):
        planner.plan(cast(Any, 7))
    with pytest.raises(AgentPlanError, match="exceeds the bounded length"):
        planner.plan("x" * 600)


def test_degrading_planner_requires_planner_objects() -> None:
    planner = OfflinePlanner(_catalog())
    with pytest.raises(TypeError, match="primary planner must implement plan"):
        DegradingPlanner(cast(Any, object()), planner)
    with pytest.raises(TypeError, match="fallback planner must implement plan"):
        DegradingPlanner(planner, cast(Any, object()))


def test_degrading_planner_forwards_a_successful_plan() -> None:
    planner = OfflinePlanner(_catalog())
    degrading = DegradingPlanner(planner, planner, allow_degraded=True)
    assert degrading.allow_degraded is True
    assert degrading.degradations == ()
    assert len(degrading.plan("beam volume")) == 2
    assert degrading.degradations == ()


def test_degrading_planner_never_degrades_authoritative_errors() -> None:
    class _Raising:
        def __init__(self, error: BaseException) -> None:
            self.error = error

        def plan(self, objective: str) -> tuple[AgentTask, ...]:
            raise self.error

    raisable = (
        AgentPolicyError("no policy"),
        EgressDeniedError("egress refused"),
        ModelCapabilityError("capability missing"),
    )
    for error in raisable:
        with pytest.raises(type(error)):
            DegradingPlanner(_Raising(error), OfflinePlanner(_catalog())).plan("beam")
    assert AgentPolicyError in NEVER_DEGRADED_ERRORS
    assert BudgetExhaustedError in NEVER_DEGRADED_ERRORS
    assert EgressDeniedError in NEVER_DEGRADED_ERRORS
    assert ModelCapabilityError in NEVER_DEGRADED_ERRORS
    assert len(NEVER_DEGRADED_ERRORS) == 4


def test_degrading_planner_refuses_unapproved_degradation() -> None:
    class _Degraded:
        def plan(self, objective: str) -> tuple[AgentTask, ...]:
            raise ProviderDegradedError("provider-a", "model-a", "circuit open")

    with pytest.raises(ProviderDegradedError) as error:
        DegradingPlanner(_Degraded(), OfflinePlanner(_catalog())).plan("beam")
    assert "provider degraded: provider-a/model-a" in str(error.value)
    assert "deterministic planning was not approved" in str(error.value)


def test_degrading_planner_falls_back_on_a_provider_failure() -> None:
    class _Degraded:
        def plan(self, objective: str) -> tuple[AgentTask, ...]:
            raise ProviderDegradedError("provider-a", "model-a", "circuit open")

    degrading = DegradingPlanner(
        _Degraded(),
        OfflinePlanner(_catalog(), fallback_step_id="simulate"),
        allow_degraded=True,
    )
    tasks = degrading.plan("nothing in the catalog matches")
    assert [task.step_id for task in tasks] == ["step-1"]
    assert degrading.degradations == ("provider degraded: provider-a/model-a",)


def test_degrading_planner_falls_back_on_a_planner_failure() -> None:
    class _Failing:
        def __init__(self, error: BaseException) -> None:
            self.error = error

        def plan(self, objective: str) -> tuple[AgentTask, ...]:
            raise self.error

    for error in (
        PlannerFailure("nothing matched"),
        StructuredOutputError("no structured output"),
        AgentPlanError("bad plan"),
    ):
        degrading = DegradingPlanner(
            _Failing(error),
            OfflinePlanner(_catalog(), fallback_step_id="summarize"),
            allow_degraded=True,
        )
        degrading.plan("beam volume")
        assert degrading.degradations == (f"planner failure: {type(error).__name__}",) or (
            degrading.degradations == (f"planner rejection: {type(error).__name__}",)
        )


def test_degrading_planner_truncates_long_reasons() -> None:
    long_name = "E" * 2_000

    rejection = type(long_name, (AgentError,), {})

    class _Rejects:
        def plan(self, objective: str) -> tuple[AgentTask, ...]:
            raise rejection("bad plan")

    degrading = DegradingPlanner(
        _Rejects(),
        OfflinePlanner(_catalog(), fallback_step_id="simulate"),
        allow_degraded=True,
    )
    degrading.plan("nothing matches")
    assert len(degrading.degradations[0]) == 1_024
    assert degrading.degradations[0].startswith("planner rejection: EEE")


def test_agent_step_outcome_validates_every_field() -> None:
    outcome = AgentStepOutcome(
        step_id="step-1",
        call_id="run-1-step-1",
        status=ToolRunStatus.SUCCEEDED,
        dispatched=True,
        tool_id="mechanical.reference-evaluator",
        result=cast("Mapping[str, JSONValue]", {"operation": "beam_volume"}),
    )
    assert outcome.succeeded is True
    assert outcome.denied is False
    with pytest.raises(AgentMergeError, match="must be a tool run status"):
        AgentStepOutcome(
            step_id="step-1",
            call_id="call-1",
            status=cast(Any, "succeeded"),
            dispatched=True,
            tool_id="tool",
        )
    with pytest.raises(AgentMergeError, match="dispatched must be a boolean"):
        AgentStepOutcome(
            step_id="step-1",
            call_id="call-1",
            status=ToolRunStatus.SUCCEEDED,
            dispatched=cast(Any, "yes"),
            tool_id="tool",
        )
    with pytest.raises(AgentMergeError, match="result must be a JSON object"):
        AgentStepOutcome(
            step_id="step-1",
            call_id="call-1",
            status=ToolRunStatus.SUCCEEDED,
            dispatched=True,
            tool_id="tool",
            result=cast(Any, ["nope"]),
        )
    with pytest.raises(AgentPlanError, match="artifact_ids"):
        AgentStepOutcome(
            step_id="step-1",
            call_id="call-1",
            status=ToolRunStatus.SUCCEEDED,
            dispatched=True,
            tool_id="tool",
            artifact_ids=cast(Any, "artifact-1"),
        )
    with pytest.raises(AgentPlanError, match="error_message exceeds"):
        AgentStepOutcome(
            step_id="step-1",
            call_id="call-1",
            status=ToolRunStatus.FAILED,
            dispatched=True,
            tool_id="tool",
            error_message="x" * 2_000,
        )
    with pytest.raises(AgentPlanError, match="policy_reason exceeds"):
        AgentStepOutcome(
            step_id="step-1",
            call_id="call-1",
            status=ToolRunStatus.DENIED,
            dispatched=True,
            tool_id="tool",
            policy_reason="x" * 2_000,
        )
    with pytest.raises(AgentPlanError, match="call_id"):
        AgentStepOutcome(
            step_id="step-1",
            call_id="bad call",
            status=ToolRunStatus.SUCCEEDED,
            dispatched=True,
            tool_id="tool",
        )
    with pytest.raises(AgentPlanError, match="tool_id"):
        AgentStepOutcome(
            step_id="step-1",
            call_id="call-1",
            status=ToolRunStatus.SUCCEEDED,
            dispatched=True,
        )


def test_agent_step_outcome_reports_denial() -> None:
    outcome = AgentStepOutcome(
        step_id="step-1",
        call_id="call-1",
        status=ToolRunStatus.DENIED,
        dispatched=False,
        tool_id="mechanical.reference-evaluator",
    )
    assert outcome.denied is True
    assert outcome.succeeded is False


def test_agent_step_outcome_is_built_from_a_tool_result() -> None:
    outcome = AgentStepOutcome.from_tool_result(
        "step-1",
        _tool_result(),
        tool_id="mechanical.reference-evaluator",
    )
    assert outcome.succeeded is True
    assert outcome.result is not None
    assert outcome.result["operation"] == "beam_volume"
    payload = outcome.to_payload()
    assert payload["status"] == "succeeded"
    assert payload["dispatched"] is True
    assert payload["result"] == {"operation": "beam_volume"}
    assert payload["artifact_ids"] == []


def test_agent_step_outcome_handles_a_result_free_run() -> None:
    outcome = AgentStepOutcome.from_tool_result(
        "step-1",
        _tool_result(ToolRunStatus.FAILED, result_json=None, error_code="tool_error"),
        tool_id="mechanical.reference-evaluator",
    )
    assert outcome.result is None
    assert outcome.succeeded is False
    assert outcome.error_code == "tool_error"


def test_agent_step_outcome_rejects_a_non_object_payload(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(agent_module, "strict_json_loads", lambda _payload: ["nope"])
    with pytest.raises(AgentMergeError, match="must be a JSON object"):
        AgentStepOutcome.from_tool_result(
            "step-1",
            _tool_result(),
            tool_id="mechanical.reference-evaluator",
        )


def test_agent_run_result_validates_its_digests() -> None:
    outcome = AgentStepOutcome(
        step_id="step-1",
        call_id="call-1",
        status=ToolRunStatus.SUCCEEDED,
        dispatched=True,
        tool_id="mechanical.reference-evaluator",
        result=cast("Mapping[str, JSONValue]", {"operation": "beam_volume"}),
    )
    result = AgentRunResult(
        run_id="run-1",
        outcomes=(outcome,),
        merged=cast("Mapping[str, JSONValue]", {"step-1": {"operation": "beam_volume"}}),
        digests=cast("Mapping[str, str]", {"step-1": "a" * 64}),
    )
    assert result.succeeded is True
    assert result.step_ids == ("step-1",)
    assert result.artifacts == ()
    assert result.outcome_for("step-1") is outcome
    with pytest.raises(AgentMergeError, match="no outcome was recorded"):
        result.outcome_for("step-9")
    with pytest.raises(AgentPlanError, match="step_id"):
        result.outcome_for("bad id")
    payload = result.to_payload()
    assert payload["status"] == "completed"
    assert payload["merged"] == {"step-1": {"operation": "beam_volume"}}
    assert payload["digests"] == {"step-1": "a" * 64}


def test_agent_run_result_validates_every_field() -> None:
    outcome = AgentStepOutcome(
        step_id="step-1",
        call_id="call-1",
        status=ToolRunStatus.SUCCEEDED,
        dispatched=True,
        tool_id="mechanical.reference-evaluator",
    )
    with pytest.raises(AgentPlanError, match="run_id"):
        AgentRunResult(run_id="bad run", outcomes=(), merged={}, digests={})
    with pytest.raises(AgentMergeError, match="outcomes must be a sequence"):
        AgentRunResult(
            run_id="run-1",
            outcomes=cast(Any, outcome),
            merged={},
            digests={},
        )
    with pytest.raises(AgentPlanError, match="merged must be a JSON object"):
        AgentRunResult(
            run_id="run-1",
            outcomes=(outcome,),
            merged=cast(Any, ["a"]),
            digests={},
        )
    with pytest.raises(AgentMergeError, match="digests must be a mapping"):
        AgentRunResult(
            run_id="run-1",
            outcomes=(outcome,),
            merged={},
            digests=cast(Any, [("step-1", "a" * 64)]),
        )
    with pytest.raises(AgentMergeError, match="sha256 hexadecimal"):
        AgentRunResult(
            run_id="run-1",
            outcomes=(outcome,),
            merged={},
            digests={"step-1": "not-a-digest"},
        )
    with pytest.raises(AgentMergeError, match="sha256 hexadecimal"):
        AgentRunResult(
            run_id="run-1",
            outcomes=(outcome,),
            merged={},
            digests={"step-1": cast(str, 7)},
        )
    with pytest.raises(AgentPlanError, match="digest key"):
        AgentRunResult(
            run_id="run-1",
            outcomes=(outcome,),
            merged={},
            digests={"bad key": "a" * 64},
        )


def test_agent_run_result_collects_distinct_artifacts() -> None:
    outcomes = tuple(
        AgentStepOutcome(
            step_id=f"step-{index}",
            call_id=f"call-{index}",
            status=ToolRunStatus.SUCCEEDED,
            dispatched=True,
            tool_id="mechanical.reference-evaluator",
            artifact_ids=("artifact-1", "artifact-2") if index == 1 else ("artifact-1",),
        )
        for index in (1, 2)
    )
    result = AgentRunResult(
        run_id="run-1",
        outcomes=outcomes,
        merged={},
        digests={},
        degraded_reasons=("provider degraded",),
        budget_denied=("step-2", "step-2"),
    )
    assert result.artifacts == ("artifact-1", "artifact-2")
    assert result.degraded_reasons == ("provider degraded",)
    assert result.budget_denied == ("step-2",)
    assert result.to_payload()["status"] == "incomplete"
    assert result.succeeded is False
    assert (
        AgentRunResult(
            run_id="run-1",
            outcomes=(),
            merged={},
            digests={},
        ).succeeded
        is False
    )


def test_validate_task_graph_rejects_bad_graphs() -> None:
    with pytest.raises(AgentPlanError, match="tasks must be a sequence"):
        _validate_task_graph(cast(Any, 7))
    with pytest.raises(AgentPlanError, match="at least one task"):
        _validate_task_graph(())
    with pytest.raises(AgentPlanError, match="AgentTask values"):
        _validate_task_graph(cast(Any, (object(),)))
    with pytest.raises(AgentPlanError, match="duplicate task identifier"):
        _validate_task_graph((_task("step-1"), _task("step-1")))
    with pytest.raises(AgentPlanError, match="depends on undeclared tasks"):
        _validate_task_graph((_task("step-1", depends_on=("step-9",)),))
    oversized = tuple(_task(f"step-{index}") for index in range(MAX_AGENT_TASKS + 1))
    with pytest.raises(AgentPlanError, match="cannot exceed"):
        _validate_task_graph(oversized)


def test_dependency_order_and_layers_follow_dependencies() -> None:
    tasks = (
        _task("step-1"),
        _task("step-2", depends_on=("step-1",)),
        _task("step-3", depends_on=("step-1",)),
    )
    assert [task.step_id for task in _dependency_order(tasks)] == ["step-1", "step-2", "step-3"]
    layers = _dependency_layers(tasks)
    assert [task.step_id for task in layers[0]] == ["step-1"]
    assert sorted(task.step_id for task in layers[1]) == ["step-2", "step-3"]
    assert len(layers) == 2


def test_dependency_helpers_detect_cycles() -> None:
    cyclic = (
        _task("step-1", depends_on=("step-2",)),
        _task("step-2", depends_on=("step-1",)),
    )
    with pytest.raises(AgentPlanError, match="contain a cycle"):
        _dependency_order(cyclic)
    with pytest.raises(AgentPlanError, match="contain a cycle"):
        _dependency_layers(cyclic)


def test_coordinator_validates_its_dependencies(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    with pytest.raises(TypeError, match="ToolExecutor"):
        AgentCoordinator(cast(Any, object()))
    with pytest.raises(TypeError, match="PolicyEngine"):
        AgentCoordinator(executor, policy=cast(Any, object()))
    with pytest.raises(TypeError, match="BudgetLedger"):
        AgentCoordinator(executor, ledger=cast(Any, object()))
    with pytest.raises(TypeError, match="ModelGateway"):
        AgentCoordinator(executor, gateway=cast(Any, object()))
    with pytest.raises(TypeError, match="StepCatalog"):
        AgentCoordinator(executor, catalog=cast(Any, object()))
    with pytest.raises(TypeError, match="must implement the Planner protocol"):
        AgentCoordinator(executor, planner=cast(Any, object()))
    with pytest.raises(TypeError, match="clock must be callable"):
        AgentCoordinator(executor, clock=cast(Any, object()))


def test_coordinator_bounds_parallelism(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    with pytest.raises(AgentPlanError, match="max_parallel_tasks must be between"):
        AgentCoordinator(executor, max_parallel_tasks=0)
    with pytest.raises(AgentPlanError, match="max_parallel_tasks must be between"):
        AgentCoordinator(executor, max_parallel_tasks=MAX_PARALLEL_TASKS + 1)
    with pytest.raises(AgentPlanError, match="max_parallel_tasks must be between"):
        AgentCoordinator(executor, max_parallel_tasks=cast(int, True))
    assert AgentCoordinator(executor, max_parallel_tasks=2).max_parallel_tasks == 2


def test_coordinator_exposes_its_configuration(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    catalog = _catalog()
    planner = OfflinePlanner(catalog)
    gateway = ModelGateway(cast(Any, _StubClient()))
    coordinator = AgentCoordinator(
        executor,
        planner=planner,
        catalog=catalog,
        gateway=gateway,
        clock=lambda: ISSUED_AT,
    )
    assert coordinator.executor is executor
    assert coordinator.registry is executor.registry
    assert coordinator.policy is None
    assert coordinator.planner is planner
    assert coordinator.ledger is None
    assert coordinator.gateway is gateway
    assert coordinator.catalog is catalog


def test_coordinator_plan_requires_a_planner(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(_executor(tmp_path))
    with pytest.raises(AgentPlanError, match="no planner is configured"):
        coordinator.plan("beam volume")


def test_coordinator_plan_validates_the_objective(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(_executor(tmp_path), planner=OfflinePlanner(_catalog()))
    with pytest.raises(AgentPlanError, match="nonempty string"):
        coordinator.plan("   ")
    with pytest.raises(AgentPlanError, match="nonempty string"):
        coordinator.plan(cast(Any, 7))


def test_coordinator_plan_validates_the_graph(tmp_path: Path) -> None:
    class _BadPlanner:
        def plan(self, objective: str) -> tuple[AgentTask, ...]:
            return (_task("step-1", depends_on=("step-9",)),)

    coordinator = AgentCoordinator(_executor(tmp_path), planner=cast(Any, _BadPlanner()))
    with pytest.raises(AgentPlanError, match="depends on undeclared tasks"):
        coordinator.plan("beam volume")


def test_coordinator_runs_a_trusted_evaluator(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(
        _executor(tmp_path),
        clock=lambda: ISSUED_AT,
    )
    result = coordinator.run((_task("step-1"),), run_id="run-1")
    assert result.succeeded is True
    assert result.step_ids == ("step-1",)
    assert set(result.digests) == {"step-1"}
    merged = cast("Mapping[str, JSONValue]", result.merged["step-1"])
    assert merged["operation"] == "beam_volume"
    assert result.degraded_reasons == ()
    assert result.budget_denied == ()


def test_coordinator_blocks_a_failed_dependency(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(_executor(tmp_path), clock=lambda: ISSUED_AT)
    tasks = (
        _task("step-1", arguments=cast("Mapping[str, JSONValue]", {"operation": "shear"})),
        _task("step-2", depends_on=("step-1",)),
    )
    result = coordinator.run(tasks, run_id="run-1")
    assert result.succeeded is False
    blocked = result.outcome_for("step-2")
    assert blocked.status is ToolRunStatus.CANCELLED
    assert blocked.dispatched is False
    assert blocked.error_message is not None
    assert "dependency did not succeed" in blocked.error_message


def test_coordinator_runs_independent_tasks_in_parallel(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(
        _executor(tmp_path),
        clock=lambda: ISSUED_AT,
        max_parallel_tasks=2,
    )
    result = coordinator.run((_task("step-1"), _task("step-2")), run_id="run-1")
    assert result.succeeded is True
    assert sorted(result.digests) == ["step-1", "step-2"]


def test_coordinator_denies_an_exhausted_budget(tmp_path: Path) -> None:
    ledger = BudgetLedger(
        RunBudget(
            max_model_calls=1,
            max_tool_calls=0,
            max_total_tokens=10,
            max_artifact_bytes=10,
            max_duration_seconds=5.0,
            max_replanning_attempts=0,
        )
    )
    coordinator = AgentCoordinator(
        _executor(tmp_path),
        ledger=ledger,
        clock=lambda: ISSUED_AT,
    )
    result = coordinator.run((_task("step-1"),), run_id="run-1")
    outcome = result.outcome_for("step-1")
    assert outcome.status is ToolRunStatus.DENIED
    assert outcome.dispatched is False
    assert outcome.error_code == "budget_exhausted"
    assert result.budget_denied == ("step-1",)
    assert result.succeeded is False


def test_coordinator_denies_an_unknown_tool(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(_executor(tmp_path), clock=lambda: ISSUED_AT)
    result = coordinator.run((_task("step-1", tool_id="absent.tool"),), run_id="run-1")
    outcome = result.outcome_for("step-1")
    assert outcome.status is ToolRunStatus.DENIED
    assert outcome.error_code == "unknown_tool"
    assert "is not registered" in str(outcome.error_message)


def test_coordinator_denies_undeclared_capabilities(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(_executor(tmp_path), clock=lambda: ISSUED_AT)
    result = coordinator.run(
        (_task("step-1", required_capabilities=("artifact.write",)),),
        run_id="run-1",
    )
    outcome = result.outcome_for("step-1")
    assert outcome.status is ToolRunStatus.DENIED
    assert outcome.error_code == "capability_denied"
    assert "undeclared capabilities" in str(outcome.error_message)


def test_coordinator_requires_a_policy_for_side_effecting_tools(tmp_path: Path) -> None:
    from kala_agent.tools.simulator import (
        INPUT_SCHEMA,
        OUTPUT_SCHEMA,
        register_simulator,
        simulator_descriptor,
        simulator_handler,
    )

    registry = ToolRegistry()
    register_simulator(registry)
    executor = ToolExecutor(
        registry,
        run_directory=tmp_path / "run",
        granted_capabilities=("artifact.read", "artifact.write", "design.read", "tool.execute"),
    )
    coordinator = AgentCoordinator(executor, clock=lambda: ISSUED_AT)
    with pytest.raises(AgentPolicyError, match="no policy engine is attached"):
        coordinator.run(
            (
                AgentTask(
                    step_id="step-1",
                    tool_id="mechanical.reference-simulator",
                    tool_version="1.0.0",
                    arguments=cast("Mapping[str, JSONValue]", {"operation": "beam_volume"}),
                    required_capabilities=("artifact.write",),
                ),
            ),
            run_id="run-1",
        )
    assert simulator_descriptor().tool_id == "mechanical.reference-simulator"
    assert simulator_handler().descriptor.version == "1.0.0"
    assert INPUT_SCHEMA["type"] == "object"
    assert OUTPUT_SCHEMA["type"] == "object"


def test_coordinator_run_plan_requires_a_catalog(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(_executor(tmp_path), clock=lambda: ISSUED_AT)
    with pytest.raises(AgentPlanError, match="no step catalog is configured"):
        coordinator.run_plan(())


def test_coordinator_run_plan_dispatches_declared_steps(tmp_path: Path) -> None:
    catalog = _catalog()
    coordinator = AgentCoordinator(
        _executor(tmp_path),
        catalog=catalog,
        clock=lambda: ISSUED_AT,
    )
    result = coordinator.run_plan(
        (
            PlanStep(
                step_id="step-1",
                catalog_step_id="simulate",
                intent="Simulate the beam.",
            ),
        )
    )
    assert result.succeeded is True
    assert result.run_id == "run-1"


def test_coordinator_run_objective_plans_and_runs(tmp_path: Path) -> None:
    catalog = _catalog()
    coordinator = AgentCoordinator(
        _executor(tmp_path),
        catalog=catalog,
        planner=OfflinePlanner(catalog, fallback_step_id="simulate"),
        clock=lambda: ISSUED_AT,
    )
    result = coordinator.run_objective("beam volume")
    assert result.succeeded is True


def test_coordinator_reports_degradation_reasons(tmp_path: Path) -> None:
    class _Degraded:
        def plan(self, objective: str) -> tuple[AgentTask, ...]:
            raise ProviderDegradedError("provider-a", "model-a", "circuit open")

    degrading = DegradingPlanner(
        _Degraded(),
        OfflinePlanner(_catalog(), fallback_step_id="simulate"),
        allow_degraded=True,
    )
    coordinator = AgentCoordinator(
        _executor(tmp_path),
        planner=degrading,
        clock=lambda: ISSUED_AT,
    )
    result = coordinator.run_objective("beam volume")
    assert result.degraded_reasons == ("provider degraded: provider-a/model-a",)


def test_coordinator_requires_an_aware_clock(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(
        _executor(tmp_path),
        clock=lambda: datetime(2026, 1, 1),
    )
    with pytest.raises(AgentError, match="aware datetime"):
        coordinator.run((_task("step-1"),))


def test_coordinator_rejects_a_merge_schema_violation(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(_executor(tmp_path), clock=lambda: ISSUED_AT)
    outcome = AgentStepOutcome(
        step_id="step-1",
        call_id="call-1",
        status=ToolRunStatus.SUCCEEDED,
        dispatched=True,
        tool_id="mechanical.reference-evaluator",
        result=cast("Mapping[str, JSONValue]", {"made_up": True}),
    )
    with pytest.raises(AgentMergeError, match="failed schema validation"):
        coordinator._merge((outcome,))


def test_coordinator_rejects_a_duplicate_merged_result(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(_executor(tmp_path), clock=lambda: ISSUED_AT)
    outcome = coordinator.run((_task("step-1"),), run_id="run-1").outcome_for("step-1")
    with pytest.raises(AgentMergeError, match="already exists for step-1"):
        coordinator._merge((outcome, outcome))


def test_coordinator_dispatch_requires_no_approval_for_pure_reads(tmp_path: Path) -> None:
    coordinator = AgentCoordinator(_executor(tmp_path), clock=lambda: ISSUED_AT)
    tool = coordinator.registry.require("mechanical.reference-evaluator", "1.0.0")
    assert AgentCoordinator._requires_approval(tool) is False
    call = ToolCall(
        call_id="run-1-step-1",
        tool_id=tool.tool_id,
        tool_version=tool.version,
        arguments_json='{"operation":"beam_volume"}',
        side_effect_level=SideEffectLevel.NONE,
        timeout_seconds=5.0,
        issued_at=ISSUED_AT,
    )
    assert coordinator._preflight(tool, call) is None


def test_model_planner_validates_its_dependencies() -> None:
    gateway = ModelGateway(cast(Any, _StubClient()))
    with pytest.raises(TypeError, match="ModelGateway"):
        ModelPlanner(cast(Any, object()), _catalog())
    with pytest.raises(TypeError, match="StepCatalog"):
        ModelPlanner(gateway, cast(Any, object()))
    with pytest.raises(AgentPlanError, match="max_tokens must be a positive integer"):
        ModelPlanner(gateway, _catalog(), max_tokens=0)
    with pytest.raises(AgentPlanError, match="max_tokens must be a positive integer"):
        ModelPlanner(gateway, _catalog(), max_tokens=cast(int, True))


def test_model_planner_exposes_its_response_schema() -> None:
    gateway = ModelGateway(cast(Any, _StubClient()))
    catalog = _catalog()
    planner = ModelPlanner(gateway, catalog)
    assert planner.gateway is gateway
    assert planner.catalog is catalog
    schema = planner.response_schema()
    assert schema["title"] == "kala.plan"
    assert schema["required"] == ["steps"]
    items = cast("dict[str, Any]", cast("Mapping[str, Any]", schema["properties"])["steps"])[
        "items"
    ]
    assert items["required"] == ["catalog_step_id", "intent", "parameters"]
    assert cast("dict[str, Any]", items["properties"])["catalog_step_id"]["enum"] == [
        "simulate",
        "summarize",
    ]


def test_model_planner_validates_the_objective() -> None:
    planner = ModelPlanner(ModelGateway(cast(Any, _StubClient())), _catalog())
    with pytest.raises(AgentPlanError, match="nonempty string"):
        planner.plan("   ")
    with pytest.raises(AgentPlanError, match="nonempty string"):
        planner.plan(cast(Any, 7))


def _planner() -> ModelPlanner:
    return ModelPlanner(ModelGateway(cast(Any, _StubClient())), _catalog())


def _response(structured: Mapping[str, Any] | None) -> ModelResponse:
    return ModelResponse(
        request_id="plan-1",
        provider_id="local",
        model_id="local",
        content="{}",
        structured=structured,
    )


def test_model_planner_reports_missing_structured_output() -> None:
    with pytest.raises(StructuredOutputError, match="no structured output"):
        _planner()._plan_steps(_response(None))


def test_model_planner_reports_an_empty_plan() -> None:
    with pytest.raises(StructuredOutputError, match="no steps"):
        _planner()._plan_steps(_response({"steps": []}))
    with pytest.raises(StructuredOutputError, match="no steps"):
        _planner()._plan_steps(_response({"steps": "beam"}))


def test_model_planner_rejects_malformed_steps() -> None:
    malformed = (
        ([1], "not a JSON object"),
        ([{"catalog_step_id": 7, "intent": "x", "parameters": {}}], "missing its identity"),
        ([{"catalog_step_id": "simulate", "intent": 7, "parameters": {}}], "missing its identity"),
        ([{"catalog_step_id": "simulate", "intent": "x", "parameters": []}], "parameters must be"),
    )
    for steps, message in malformed:
        with pytest.raises(StructuredOutputError, match=message):
            _planner()._plan_steps(_response({"steps": steps}))


def test_model_planner_converts_a_valid_response() -> None:
    structured = {
        "steps": [
            {
                "catalog_step_id": "simulate",
                "intent": "Simulate the beam.",
                "parameters": {"operation": "beam_volume"},
            },
            {
                "catalog_step_id": "summarize",
                "intent": "",
                "parameters": {"operation": "bar_summary"},
            },
        ]
    }
    assert [step.step_id for step in _planner()._plan_steps(_response(structured))] == [
        "step-1",
        "step-2",
    ]
    tasks = _planner().plan("beam volume")
    assert [task.step_id for task in tasks] == ["step-1"]
    assert tasks[0].arguments["operation"] == "beam_volume"


def test_model_planner_rejects_an_unknown_catalog_step() -> None:
    structured = {
        "steps": [
            {
                "catalog_step_id": "made_up",
                "intent": "Do something else.",
                "parameters": {},
            }
        ]
    }
    with pytest.raises(StructuredOutputError, match="not a permitted enum member"):
        ModelPlanner(
            ModelGateway(cast(Any, _StubClient(structured))),
            _catalog(),
        ).plan("beam volume")
    with pytest.raises(AgentPlanError, match="is not declared"):
        _catalog().to_tasks(
            (
                PlanStep(
                    step_id="step-1",
                    catalog_step_id="made_up",
                    intent="Do something else.",
                ),
            )
        )


def test_model_planner_bounds_the_intent_length() -> None:
    structured = {
        "steps": [
            {
                "catalog_step_id": "simulate",
                "intent": "x" * 5_000,
                "parameters": {},
            }
        ]
    }
    steps = _planner()._plan_steps(_response(cast("Mapping[str, Any]", structured)))
    assert steps[0].step_id == "step-1"
    assert steps[0].intent == "x" * 512
    blank = _planner()._plan_steps(
        _response({"steps": [{"catalog_step_id": "simulate", "intent": "", "parameters": {}}]})
    )
    assert blank[0].intent == "planned step"
    with pytest.raises(AgentPlanError, match="exceeds the bounded length"):
        _planner().plan("x" * 600)


class _StubClient:
    def __init__(self, payload: Mapping[str, Any] | None = None) -> None:
        self._structured: Mapping[str, Any] | None = (
            {
                "steps": [
                    {
                        "catalog_step_id": "simulate",
                        "intent": "Simulate the beam.",
                        "parameters": {},
                    }
                ]
            }
            if payload is None
            else payload
        )

    @property
    def provider_id(self) -> str:
        return "local"

    @property
    def model_id(self) -> str:
        return "local"

    def supports(self, capability: ModelCapability) -> bool:
        return True

    @property
    def requires_egress(self) -> bool:
        return False

    def complete(self, request: ModelRequest) -> ModelResponse:
        return ModelResponse(
            request_id=request.request_id,
            provider_id="local",
            model_id="local",
            content=(
                "{}"
                if self._structured is None
                else canonical_json_dumps(cast("Mapping[str, JSONValue]", self._structured))
            ),
            structured=self._structured,
            input_tokens=10,
            output_tokens=5,
        )
