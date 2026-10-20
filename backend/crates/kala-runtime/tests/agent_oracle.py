"""The live Python oracle for the agent runtime differential.

The oracle imports the repository's own ``kala_agent.runtime.agent`` and
drives it with a scripted model provider and the shipped simulator tools. It
reads a scenario document from stdin and writes a result document to stdout.

The oracle is a *test* artefact: the Rust crate never invokes Python at
runtime. The Rust side of the differential builds the same scenario, runs the
same cases and compares the two result documents byte for byte over canonical
JSON.
"""

from __future__ import annotations

import json
import sys
import tempfile
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from kala_agent.contracts import PlanStep, RunBudget
from kala_agent.kernel.budget import (
    BudgetDimension,
    BudgetLedger,
    ToolCallBudgetExhaustedError,
)
from kala_agent.kernel.policy import PolicyEngine
from kala_agent.runtime.agent import (
    AgentCoordinator,
    AgentError,
    AgentPlanError,
    AgentPolicyError,
    AgentTask,
    CatalogStep,
    DegradingPlanner,
    ModelPlanner,
    OfflinePlanner,
    PlannerFailure,
    StepCatalog,
)
from kala_agent.runtime.model_gateway import (
    EgressDeniedError,
    ModelCapabilityError,
    ModelGateway,
    ModelRequest,
    ModelResponse,
    ProviderDegradedError,
    StructuredOutputError,
)
from kala_agent.tools.executor import IsolationMode, ToolExecutor
from kala_agent.tools.protocol import canonical_json_dumps
from kala_agent.tools.registry import ToolRegistry
from kala_agent.tools.simulator import (
    register_reference_evaluator,
    register_simulator,
)

ISSUED_AT = datetime(2026, 1, 2, 3, 4, 5, tzinfo=UTC)

SIMULATOR_CAPABILITIES = (
    "artifact.read",
    "artifact.write",
    "design.read",
    "tool.execute",
)


class ScriptedClient:
    """A deterministic provider double."""

    def __init__(self, responses: list[dict[str, Any] | None], failure: str | None) -> None:
        self._responses = list(responses)
        self._failure = failure
        self.calls = 0

    @property
    def provider_id(self) -> str:
        return "local"

    @property
    def model_id(self) -> str:
        return "local"

    @property
    def requires_egress(self) -> bool:
        return False

    def supports(self, capability: object) -> bool:
        return True

    def complete(self, request: ModelRequest) -> ModelResponse:
        self.calls += 1
        if self._failure == "degraded":
            raise ProviderDegradedError("provider-a", "model-a", "circuit open")
        payload = self._responses.pop(0) if self._responses else None
        return ModelResponse(
            request_id=request.request_id,
            provider_id="local",
            model_id="local",
            content="{}" if payload is None else canonical_json_dumps(payload),
            structured=payload,
            input_tokens=10,
            output_tokens=5,
        )


class FailingPlanner:
    """A planner that always fails with the scripted error."""

    def __init__(self, failure: str) -> None:
        self._failure = failure

    def plan(self, objective: str) -> tuple[AgentTask, ...]:
        if self._failure == "degraded":
            raise ProviderDegradedError("provider-a", "model-a", "circuit open")
        if self._failure == "planner_failure":
            raise PlannerFailure("nothing matched")
        if self._failure == "structured":
            raise StructuredOutputError("no structured output")
        if self._failure == "plan_error":
            raise AgentPlanError("bad plan")
        if self._failure == "policy":
            raise AgentPolicyError("no policy")
        if self._failure == "egress":
            raise EgressDeniedError("egress refused")
        if self._failure == "capability":
            raise ModelCapabilityError("capability missing")
        if self._failure == "budget":
            raise ToolCallBudgetExhaustedError(BudgetDimension.TOOL_CALLS, 1, 0, 0)
        if self._failure == "base":
            raise AgentError("the base agent error")
        raise AssertionError(f"unknown failure: {self._failure}")


def _catalog(entry: dict[str, Any]) -> CatalogStep:
    return CatalogStep(
        catalog_step_id=entry["catalog_step_id"],
        tool_id=entry["tool_id"],
        default_arguments=entry.get("default_arguments", {}),
        tool_version=entry.get("tool_version"),
        required_capabilities=tuple(entry.get("required_capabilities", ())),
        keywords=tuple(entry.get("keywords", ())),
        intent=entry.get("intent", ""),
    )


def _task(entry: dict[str, Any]) -> AgentTask:
    return AgentTask(
        step_id=entry["step_id"],
        tool_id=entry["tool_id"],
        arguments=entry.get("arguments", {}),
        tool_version=entry.get("tool_version"),
        required_capabilities=tuple(entry.get("required_capabilities", ())),
        depends_on=tuple(entry.get("depends_on", ())),
        intent=entry.get("intent", ""),
        timeout_seconds=entry.get("timeout_seconds"),
    )


def _plan_step(entry: dict[str, Any]) -> PlanStep:
    return PlanStep(
        step_id=entry["step_id"],
        catalog_step_id=entry["catalog_step_id"],
        intent=entry["intent"],
        parameters_json=entry.get("parameters_json", "{}"),
        depends_on=tuple(entry.get("depends_on", ())),
    )


def _call_payload(report: object) -> dict[str, Any]:
    payload = report.to_payload()  # type: ignore[attr-defined]
    return {
        "artifact_bytes": payload["artifact_bytes"],
        "call_id": payload["call_id"],
        "error_code": payload["error_code"],
        "granted_capabilities": list(payload["granted_capabilities"]),
        "status": payload["status"],
        "tool_id": payload["tool_id"],
        "tool_version": payload["tool_version"],
    }


def run_case(case: dict[str, Any], workdir: Path) -> dict[str, Any]:
    """Run one case and return its result document."""
    outcome: dict[str, Any] = {"name": case["name"]}
    executor: ToolExecutor | None = None
    ledger: BudgetLedger | None = None
    coordinator: AgentCoordinator | None = None
    try:
        registry = ToolRegistry()
        register_reference_evaluator(registry)
        register_simulator(registry)

        limits = case.get("ledger")
        if limits is not None:
            ledger = BudgetLedger(RunBudget(**limits))

        catalog = StepCatalog([_catalog(entry) for entry in case["catalog"]])

        executor = ToolExecutor(
            registry,
            run_directory=workdir / "run",
            granted_capabilities=SIMULATOR_CAPABILITIES,
            ledger=ledger if case.get("ledger_on_executor") else None,
            isolation=IsolationMode.TRUSTED_IN_PROCESS_EVALUATOR,
        )

        planner: object | None = None
        gateway: ModelGateway | None = None
        kind = case.get("planner")
        if kind == "offline":
            planner = OfflinePlanner(catalog, fallback_step_id=case.get("fallback_step_id"))
        elif kind == "model":
            model = case.get("model", {})
            client = ScriptedClient(model.get("responses", []), model.get("failure"))
            gateway = ModelGateway(client)
            planner = ModelPlanner(gateway, catalog, max_tokens=case.get("max_tokens", 1024))
        elif kind == "degrading":
            model = case.get("model", {})
            client = ScriptedClient(model.get("responses", []), model.get("failure"))
            gateway = ModelGateway(client)
            primary = case.get("primary", "degraded")
            if primary == "model":
                planner = DegradingPlanner(
                    ModelPlanner(gateway, catalog, max_tokens=case.get("max_tokens", 1024)),
                    OfflinePlanner(catalog, fallback_step_id=case.get("fallback_step_id")),
                    allow_degraded=case.get("allow_degraded", False),
                )
            else:
                planner = DegradingPlanner(
                    FailingPlanner(primary),
                    OfflinePlanner(catalog, fallback_step_id=case.get("fallback_step_id")),
                    allow_degraded=case.get("allow_degraded", False),
                )
        elif kind == "failing":
            planner = FailingPlanner(case.get("primary", "degraded"))

        coordinator = AgentCoordinator(
            executor,
            policy=PolicyEngine() if case.get("policy") else None,
            planner=planner,
            ledger=ledger,
            gateway=gateway,
            catalog=catalog,
            max_parallel_tasks=case.get("max_parallel_tasks", 4),
            clock=lambda: ISSUED_AT,
        )

        run_id = case.get("run_id", "run-1")
        if case.get("plan_steps") is not None:
            result = coordinator.run_plan(
                tuple(_plan_step(entry) for entry in case["plan_steps"]), run_id=run_id
            )
        elif case.get("objective") is not None:
            result = coordinator.run_objective(case["objective"], run_id=run_id)
        else:
            result = coordinator.run(
                tuple(_task(entry) for entry in case["tasks"]), run_id=run_id
            )
        outcome["status"] = "ok"
        outcome["result"] = result.to_payload()
        outcome["degradations"] = list(result.degraded_reasons)
    except BaseException as error:  # noqa: BLE001 - the oracle reports every failure
        outcome["status"] = "error"
        outcome["error_class"] = type(error).__name__
        outcome["error_message"] = str(error)
    finally:
        if ledger is not None:
            outcome["ledger"] = ledger.export_state()
        if executor is not None:
            outcome["calls"] = [_call_payload(report) for report in executor.reports]
    return outcome


def main() -> int:
    """Run the scenario and write the result document."""
    scenario = json.loads(sys.stdin.read())
    results: list[dict[str, Any]] = []
    with tempfile.TemporaryDirectory(prefix="kala-agent-oracle-") as directory:
        root = Path(directory)
        for case in scenario["cases"]:
            results.append(run_case(case, root))
    json.dump({"cases": results}, sys.stdout, ensure_ascii=True, sort_keys=True, separators=(",", ":"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
