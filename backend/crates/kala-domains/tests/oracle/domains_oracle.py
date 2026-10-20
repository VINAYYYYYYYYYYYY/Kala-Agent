"""Live Python oracle for the native domain differential tests.

Reads one JSON request on stdin and writes one JSON response on stdout. This is
test-only tooling: the Rust crate never invokes Python at production time.
"""

from __future__ import annotations

import dataclasses
import hashlib
import json
import struct
import sys
from datetime import UTC, datetime
from pathlib import Path

SRC = Path(__file__).resolve().parents[5] / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

from kala_agent.contracts import (  # noqa: E402
    EvaluationResult,
    StepStatus,
    WorkflowStatus,
)
from kala_agent.domains import (  # noqa: E402
    DomainPackage,
    ReferenceWorkflowRun,
    StepOutcome,
    reference_context,
    run_reference_workflow,
)
from kala_agent.domains.building import (  # noqa: E402
    BUILDING_PACKAGE,
    BUILDING_SCHEMA_DIGEST,
    BuildingParameters,
    violations_for as building_violations,
)
from kala_agent.domains.building.simulation import simulate_building  # noqa: E402
from kala_agent.domains.mechanical import (  # noqa: E402
    BRACKET_SCHEMA_DIGEST,
    MECHANICAL_PACKAGE,
    MECHANICAL_STEPS,
    STRUCTURAL_STEP_ID,
    BracketParameters,
    StructuralEvaluator,
    violations_for as mechanical_violations,
)
from kala_agent.domains.mechanical.simulation import simulate_bracket  # noqa: E402
from kala_agent.domains.wing import (  # noqa: E402
    WING_PACKAGE,
    WING_SCHEMA_DIGEST,
    WingParameters,
    violations_for as wing_violations,
)
from kala_agent.domains.wing.simulation import simulate_wing  # noqa: E402

PACKAGES = {
    "mechanical": (MECHANICAL_PACKAGE, BracketParameters, BRACKET_SCHEMA_DIGEST),
    "building": (BUILDING_PACKAGE, BuildingParameters, BUILDING_SCHEMA_DIGEST),
    "wing": (WING_PACKAGE, WingParameters, WING_SCHEMA_DIGEST),
}
SIMULATIONS = {
    "mechanical": simulate_bracket,
    "building": simulate_building,
    "wing": simulate_wing,
}
VIOLATIONS = {
    "mechanical": mechanical_violations,
    "building": building_violations,
    "wing": wing_violations,
}


def float_bits(value: float) -> str:
    return f"{struct.unpack('<Q', struct.pack('<d', value))[0]:016x}"


def canonical(value: object) -> str:
    return json.dumps(
        value,
        allow_nan=False,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    )


def evaluation_payload(evaluation):
    dump = evaluation.model_dump(mode="json")
    return {
        "canonical": canonical(dump),
        "evaluation_id": dump["evaluation_id"],
        "subject_id": dump["subject_id"],
        "passed": dump["passed"],
        "score_bits": None
        if dump["score"] is None
        else float_bits(dump["score"]),
        "evaluated_at": dump["evaluated_at"],
        "findings": dump["findings"],
        "metrics": [
            {
                "name": metric["name"],
                "value_bits": float_bits(metric["value"]),
                "unit": metric["unit"],
                "passed": metric["passed"],
            }
            for metric in dump["metrics"]
        ],
    }


def simulation_payload(value):
    return {
        field.name: float_bits(getattr(value, field.name))
        for field in dataclasses.fields(value)
    }


def context_for(case):
    evaluated_at = case.get("evaluated_at")
    subject = case.get("subject", f"case-{case['index']}")
    if evaluated_at is None:
        return reference_context(subject)
    return reference_context(subject, datetime.fromisoformat(evaluated_at))


def parameter_case(case):
    domain = case["domain"]
    package, model, schema_digest = PACKAGES[domain]
    try:
        parameters = model.model_validate(case.get("values") or {})
    except Exception as error:  # noqa: BLE001 - oracle reports the class
        errors = []
        if hasattr(error, "errors"):
            errors = [
                {
                    "loc": list(item.get("loc", ())),
                    "type": item.get("type"),
                    "msg": item.get("msg"),
                }
                for item in error.errors()
            ]
        return {
            "ok": False,
            "error_type": type(error).__name__,
            "errors": errors,
        }
    context = context_for(case)
    simulation = SIMULATIONS[domain](parameters)
    evaluations = [
        evaluation_payload(package.evaluate(step_id, context, parameters))
        for step_id in package.step_ids
    ]
    run = run_reference_workflow(package, context, parameters, package.step_ids)
    return {
        "ok": True,
        "wire_json": json.dumps(
            parameters.model_dump(mode="json"),
            ensure_ascii=True,
            separators=(",", ":"),
        ),
        "canonical_digest": hashlib.sha256(
            canonical(parameters.model_dump(mode="json")).encode()
        ).hexdigest(),
        "schema_digest": schema_digest,
        "simulation": simulation_payload(simulation),
        "violations": [violation.value for violation in VIOLATIONS[domain](parameters)],
        "evaluations": evaluations,
        "workflow": {
            "status": run.status.value,
            "passed": run.passed,
            "blocked_step_id": run.blocked_step_id,
            "ordered_step_ids": list(run.ordered_step_ids),
            "completed_step_ids": list(run.completed_step_ids),
            "definition_id": run.definition_id,
            "definition_version": run.definition_version,
            "package_id": run.package_id,
            "score_bits": None if run.score is None else float_bits(run.score),
            "findings": list(run.findings),
            "outcomes": [
                {
                    "step_id": outcome.step_id,
                    "status": outcome.status.value,
                    "reason": outcome.reason,
                    "evaluation": None
                    if outcome.evaluation is None
                    else evaluation_payload(outcome.evaluation),
                }
                for outcome in run.outcomes
            ],
        },
    }


def validation_case(case):
    domain = case["domain"]
    _package, model, _schema = PACKAGES[domain]
    try:
        model.model_validate(case.get("values") or {})
    except Exception as error:  # noqa: BLE001
        errors = []
        if hasattr(error, "errors"):
            errors = [
                {
                    "loc": list(item.get("loc", ())),
                    "type": item.get("type"),
                    "msg": item.get("msg"),
                }
                for item in error.errors()
            ]
        return {"ok": False, "error_type": type(error).__name__, "errors": errors}
    return {"ok": True}


def model_json_case(case):
    domain = case["domain"]
    _package, model, _schema = PACKAGES[domain]
    try:
        parameters = model.model_validate_json(case["text"])
    except Exception as error:  # noqa: BLE001 - oracle reports the class
        errors = []
        if hasattr(error, "errors"):
            errors = [
                {
                    "loc": list(item.get("loc", ())),
                    "type": item.get("type"),
                    "msg": item.get("msg"),
                }
                for item in error.errors()
            ]
        return {
            "ok": False,
            "error_type": type(error).__name__,
            "errors": errors,
        }
    dump = parameters.model_dump(mode="json")
    return {
        "ok": True,
        "wire_json": json.dumps(dump, ensure_ascii=False, separators=(",", ":")),
        "canonical_digest": hashlib.sha256(canonical(dump).encode()).hexdigest(),
    }


def score_case(case):
    scores = case.get("scores", [])
    evaluations = [
        EvaluationResult(
            evaluation_id=f"evaluation-{index}",
            subject_id="score-audit",
            passed=True,
            score=score,
            metrics=(),
            findings=(),
            artifact_ids=(),
            evaluated_at=datetime(2026, 1, 1, tzinfo=UTC),
        )
        for index, score in enumerate(scores)
    ]
    outcomes = tuple(
        StepOutcome(
            step_id=f"step-{index}",
            status=StepStatus.SUCCEEDED,
            evaluation=evaluation,
        )
        for index, evaluation in enumerate(evaluations)
    )
    run = ReferenceWorkflowRun(
        package_id="score.audit",
        definition_id="score.audit",
        definition_version="1",
        status=WorkflowStatus.COMPLETED,
        ordered_step_ids=tuple(outcome.step_id for outcome in outcomes),
        completed_step_ids=tuple(outcome.step_id for outcome in outcomes),
        outcomes=outcomes,
    )
    present = [score for score in scores if score is not None]
    return {
        "score_bits": None if run.score is None else float_bits(run.score),
        "sum_bits": None if not present else float_bits(sum(present)),
        "mean_bits": None if not present else float_bits(sum(present) / len(present)),
    }


CALLS: list[str] = []


@dataclasses.dataclass(frozen=True, slots=True)
class OracleRecordingEvaluator:
    step_id: str = STRUCTURAL_STEP_ID
    required_capabilities: tuple[str, ...] = ()

    def evaluate(self, context, parameters):
        CALLS.append(self.step_id)
        return StructuralEvaluator().evaluate(context, parameters)


class OraclePolicy:
    def __init__(self, granted):
        self.granted = tuple(granted)
        self.requests = []

    def granted_capabilities(self, request):
        self.requests.append(request)
        return self.granted


def capability_case(case):
    step_capability = case.get("step_capability")
    package_capability = case.get("package_capability")
    step_capabilities = ()
    if step_capability is not None:
        step_capabilities = (step_capability,)
    steps = tuple(
        dataclasses.replace(step, required_capabilities=step_capabilities)
        if step.step_id == STRUCTURAL_STEP_ID
        else step
        for step in MECHANICAL_STEPS
    )
    package_capabilities = ()
    evaluators = tuple(
        OracleRecordingEvaluator(required_capabilities=step_capabilities)
        if evaluator.step_id == STRUCTURAL_STEP_ID
        else evaluator
        for evaluator in MECHANICAL_PACKAGE.evaluators
    )
    if package_capability is not None:
        package_capabilities = (package_capability,)
    package = DomainPackage(
        package_id="mechanical.test-copy",
        version=MECHANICAL_PACKAGE.version,
        description=MECHANICAL_PACKAGE.description,
        disclaimer=MECHANICAL_PACKAGE.disclaimer,
        workflow=MECHANICAL_PACKAGE.workflow,
        steps=steps,
        evaluators=evaluators,
        schema_digests=MECHANICAL_PACKAGE.schema_digests,
        required_capabilities=package_capabilities,
    )
    context = reference_context("capability-subject")
    parameters = BracketParameters()
    CALLS.clear()
    grants = case.get("grants")
    policy = None if grants is None else OraclePolicy(grants)
    try:
        run = run_reference_workflow(
            package,
            context,
            parameters,
            package.step_ids,
            capability_policy=policy,
        )
        return {
            "ok": True,
            "passed": run.passed,
            "calls": list(CALLS),
            "requests": [
                {
                    "package_id": request.package_id,
                    "step_ids": list(request.step_ids),
                    "required_capabilities": list(request.required_capabilities),
                }
                for request in (policy.requests if policy else [])
            ],
        }
    except Exception as error:  # noqa: BLE001
        return {
            "ok": False,
            "error_class": type(error).__name__,
            "message": str(error),
            "calls": list(CALLS),
            "requests": [
                {
                    "package_id": request.package_id,
                    "step_ids": list(request.step_ids),
                    "required_capabilities": list(request.required_capabilities),
                }
                for request in (policy.requests if policy else [])
            ],
        }


def package_payload(domain):
    package, _model, schema_digest = PACKAGES[domain]
    return {
        "description": package.description,
        "disclaimer": package.disclaimer,
        "required_capabilities": list(package.required_capabilities),
        "schema_digests": list(package.schema_digests),
        "package_digest": package.digest(),
        "manifest": package.manifest().model_dump(mode="json"),
        "implementation_digest_version": "evaluator-implementation-digest-v2",
        "evaluators": [item.as_dict() for item in package.evaluator_implementations],
        "schema_digest": schema_digest,
        "step_ids": list(package.step_ids),
        "workflow_payload": package.workflow.model_dump(mode="json"),
        "step_payloads": [step.as_dict() for step in package.steps],
    }


def main():
    request = json.load(sys.stdin)
    response = {
        "packages": {
            domain: package_payload(domain) for domain in PACKAGES
        },
        "parameter_cases": [
            parameter_case({**case, "index": index})
            for index, case in enumerate(request.get("parameter_cases", []))
        ],
        "validation_cases": [
            validation_case(case) for case in request.get("validation_cases", [])
        ],
        "model_json_cases": [
            model_json_case(case) for case in request.get("model_json_cases", [])
        ],
        "score_cases": [
            score_case(case) for case in request.get("score_cases", [])
        ],
        "capability_cases": [
            capability_case(case) for case in request.get("capability_cases", [])
        ],
    }
    json.dump(response, sys.stdout, ensure_ascii=False)


if __name__ == "__main__":
    main()
