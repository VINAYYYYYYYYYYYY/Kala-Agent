"""Integrity of the live evaluator implementations behind a domain package.

A package digest is only useful if it covers the code that produces the numbers
a revision is signed off on. These tests are written from the position of
whoever can replace a package's evaluator between two runs: a stand-in that
computes exactly the same values, a rewritten module file, a module with no
readable source at all, an evaluator that quietly asks for a capability its
catalog step never declared, and a catalog step whose checkpoint disagrees with
the workflow transition meant to carry it. Each of those is either detected by a
changed digest or refused outright, and a denial happens before any evaluator
runs.
"""

from __future__ import annotations

import hashlib
import importlib.util
import subprocess
import sys
from dataclasses import dataclass, replace
from pathlib import Path
from types import ModuleType
from typing import Any, cast

import pytest

from kala_agent.contracts import ApprovalKind, EvaluationResult, WorkflowDefinition
from kala_agent.domains import (
    CapabilityGrantPolicy,
    CapabilityRequest,
    CatalogStep,
    DomainCapabilityError,
    DomainEvaluator,
    DomainPackage,
    DomainPackageError,
    DomainParameterError,
    DomainStepError,
    EvaluationContext,
    EvaluatorImplementation,
    enforce_capability_request,
    evaluator_implementation,
    module_source_digest,
    package_source_digest,
    reference_context,
    run_reference_workflow,
)
from kala_agent.domains.mechanical import (
    GEOMETRY_STEP_ID,
    MECHANICAL_PACKAGE,
    MECHANICAL_STEPS,
    MECHANICAL_WORKFLOW,
    REFERENCE_PARAMETERS,
    STRUCTURAL_STEP_ID,
    BracketParameters,
    StructuralEvaluator,
)

SRC_ROOT = Path(__file__).parents[2] / "src"
CONTEXT = reference_context("domain-integrity-subject")
SURROGATE_READ = "mechanical.surrogate.read"
REFERENCE_RUN = "mechanical.reference.run"
MECHANICAL_EVALUATOR_MODULE = "kala_agent.domains.mechanical.evaluation"
ABSENT_MODULE = "kala_agent.domains.tests_absent_module"
SOURCELESS_MODULE = "kala_agent.domains.tests_sourceless_module"
ZERO_DIGEST = "0" * 64
EVALUATED: list[str] = []
SHIPPED_EVALUATORS: dict[str, Any] = {
    evaluator.step_id: evaluator for evaluator in MECHANICAL_PACKAGE.evaluators
}
REWRITTEN_MODULE_SOURCE = """\
class RewrittenEvaluator:
    step_id = "mechanical.constraint-evaluation"
    required_capabilities = ()

    def evaluate(self, context, parameters):
        raise NotImplementedError
"""


@dataclass(frozen=True, slots=True)
class SubstitutedStructuralEvaluator:
    """A stand-in that returns exactly what the shipped evaluator returns."""

    step_id: str = STRUCTURAL_STEP_ID
    required_capabilities: tuple[str, ...] = ()

    def evaluate(
        self,
        context: EvaluationContext,
        parameters: BracketParameters,
    ) -> EvaluationResult:
        """Return the shipped result, so only the identity of the code differs."""
        return StructuralEvaluator().evaluate(context, parameters)


@dataclass(frozen=True, slots=True)
class RecordingStructuralEvaluator:
    """A stand-in that records that it was reached at all."""

    step_id: str = STRUCTURAL_STEP_ID
    required_capabilities: tuple[str, ...] = ()

    def evaluate(
        self,
        context: EvaluationContext,
        parameters: BracketParameters,
    ) -> EvaluationResult:
        """Record the call, then return the shipped result."""
        EVALUATED.append(self.step_id)
        return StructuralEvaluator().evaluate(context, parameters)


class _FixedGrantPolicy:
    """A domain-neutral policy that answers with one fixed grant set."""

    def __init__(self, granted: tuple[str, ...]) -> None:
        self.granted = granted
        self.requests: list[CapabilityRequest] = []

    def granted_capabilities(self, request: CapabilityRequest) -> tuple[str, ...]:
        self.requests.append(request)
        return self.granted


def _bind(
    replacement: type[SubstitutedStructuralEvaluator] | type[RecordingStructuralEvaluator],
    capability: str | None = None,
) -> tuple[DomainEvaluator[Any], ...]:
    return tuple(
        replacement(required_capabilities=() if capability is None else (capability,))
        if evaluator.step_id == STRUCTURAL_STEP_ID
        else evaluator
        for evaluator in MECHANICAL_PACKAGE.evaluators
    )


def _package(
    *,
    workflow: WorkflowDefinition = MECHANICAL_WORKFLOW,
    steps: tuple[CatalogStep, ...] = MECHANICAL_STEPS,
    evaluators: tuple[DomainEvaluator[Any], ...] | None = None,
    step_capability: str | None = None,
    package_capabilities: tuple[str, ...] = (),
) -> DomainPackage:
    declared_steps = tuple(
        replace(step, required_capabilities=(step_capability,))
        if step_capability is not None and step.step_id == STRUCTURAL_STEP_ID
        else step
        for step in steps
    )
    bound = evaluators
    if bound is None:
        bound = tuple(
            replace(
                SHIPPED_EVALUATORS[STRUCTURAL_STEP_ID], required_capabilities=(step_capability,)
            )
            if step_capability is not None and evaluator.step_id == STRUCTURAL_STEP_ID
            else evaluator
            for evaluator in MECHANICAL_PACKAGE.evaluators
        )
    return DomainPackage(
        package_id=MECHANICAL_PACKAGE.package_id,
        version=MECHANICAL_PACKAGE.version,
        description=MECHANICAL_PACKAGE.description,
        disclaimer=MECHANICAL_PACKAGE.disclaimer,
        workflow=workflow,
        steps=declared_steps,
        evaluators=bound,
        schema_digests=MECHANICAL_PACKAGE.schema_digests,
        required_capabilities=package_capabilities,
    )


def _substituted_package() -> DomainPackage:
    return _package(evaluators=_bind(SubstitutedStructuralEvaluator))


def _digests(package: DomainPackage) -> dict[str, str]:
    return dict(zip(package.step_ids, package.evaluator_implementation_digests, strict=True))


def _implementation(package: DomainPackage, step_id: str) -> EvaluatorImplementation:
    return next(item for item in package.evaluator_implementations if item.step_id == step_id)


def _digest_in_a_fresh_process() -> str:
    completed = subprocess.run(
        [
            sys.executable,
            "-c",
            (
                "from kala_agent.domains.mechanical import MECHANICAL_PACKAGE;"
                "print(MECHANICAL_PACKAGE.digest())"
            ),
        ],
        capture_output=True,
        check=True,
        cwd=SRC_ROOT,
        env={"PYTHONPATH": str(SRC_ROOT), "PYTHONHASHSEED": "0"},
        text=True,
    )
    return completed.stdout.strip()


def _checkpoint_workflow(step_id: str, checkpoint: ApprovalKind | None) -> WorkflowDefinition:
    return WorkflowDefinition(
        definition_id="mechanical.integrity-checkpoints",
        version="0.1.0",
        name="Checkpoint fixture",
        initial_state=MECHANICAL_WORKFLOW.initial_state,
        states=MECHANICAL_WORKFLOW.states,
        transitions=tuple(
            transition.model_copy(
                update={"checkpoint_type": checkpoint if transition.step_id == step_id else None}
            )
            for transition in MECHANICAL_WORKFLOW.transitions
        ),
        available_step_ids=MECHANICAL_WORKFLOW.available_step_ids,
        max_replanning_attempts=1,
    )


def _checkpoint_steps(step_id: str, checkpoint: ApprovalKind) -> tuple[CatalogStep, ...]:
    return tuple(
        replace(step, checkpoint_kind=checkpoint if step.step_id == step_id else None)
        for step in MECHANICAL_STEPS
    )


def _steps_without_checkpoint(step_id: str) -> tuple[CatalogStep, ...]:
    return tuple(
        replace(step, checkpoint_kind=None) if step.step_id == step_id else step
        for step in MECHANICAL_STEPS
    )


def _hidden_evaluator(
    module_name: object, qualname: str = "HiddenEvaluator"
) -> DomainEvaluator[Any]:
    built: Any = type(
        "HiddenEvaluator",
        (),
        {
            "__module__": module_name,
            "step_id": STRUCTURAL_STEP_ID,
            "required_capabilities": (),
            "evaluate": lambda self, context, parameters: None,
        },
    )
    built.__qualname__ = qualname
    return cast("DomainEvaluator[Any]", built())


def _load_module(path: Path, name: str, monkeypatch: pytest.MonkeyPatch) -> ModuleType:
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    monkeypatch.setitem(sys.modules, name, module)
    spec.loader.exec_module(module)
    return module


def test_evaluator_substitution_is_detected_by_the_package_digest() -> None:
    substituted = _substituted_package()
    shipped_digests = _digests(MECHANICAL_PACKAGE)
    substituted_digests = _digests(substituted)

    assert substituted.digest() != MECHANICAL_PACKAGE.digest()
    assert substituted.manifest().digest != MECHANICAL_PACKAGE.manifest().digest
    assert substituted.manifest_schema_digests() != MECHANICAL_PACKAGE.manifest_schema_digests()
    assert substituted_digests[STRUCTURAL_STEP_ID] != shipped_digests[STRUCTURAL_STEP_ID]
    assert {
        step_id: digest
        for step_id, digest in substituted_digests.items()
        if step_id != STRUCTURAL_STEP_ID
    } == {
        step_id: digest
        for step_id, digest in shipped_digests.items()
        if step_id != STRUCTURAL_STEP_ID
    }
    assert substituted_digests[STRUCTURAL_STEP_ID] in substituted.manifest().schema_digests
    assert shipped_digests[STRUCTURAL_STEP_ID] not in substituted.manifest().schema_digests

    implementation = _implementation(substituted, STRUCTURAL_STEP_ID)
    shipped_implementation = _implementation(MECHANICAL_PACKAGE, STRUCTURAL_STEP_ID)
    assert implementation.step_id == STRUCTURAL_STEP_ID
    assert implementation.qualified_name == f"{__name__}.SubstitutedStructuralEvaluator"
    assert implementation.module_digest != shipped_implementation.module_digest
    assert implementation.digest != shipped_implementation.digest
    assert (
        implementation.digest
        == evaluator_implementation(substituted.evaluator(STRUCTURAL_STEP_ID)).digest
    )

    assert substituted.evaluate(STRUCTURAL_STEP_ID, CONTEXT, REFERENCE_PARAMETERS).model_dump(
        mode="json"
    ) == MECHANICAL_PACKAGE.evaluate(STRUCTURAL_STEP_ID, CONTEXT, REFERENCE_PARAMETERS).model_dump(
        mode="json"
    )
    assert _package().digest() == MECHANICAL_PACKAGE.digest()
    assert type(MECHANICAL_PACKAGE.evaluator(STRUCTURAL_STEP_ID)) is StructuralEvaluator


def test_a_substituted_evaluator_cannot_carry_an_undeclared_capability() -> None:
    with pytest.raises(DomainPackageError, match=STRUCTURAL_STEP_ID):
        _package(evaluators=_bind(SubstitutedStructuralEvaluator, REFERENCE_RUN))

    declared = _package(
        evaluators=_bind(RecordingStructuralEvaluator, SURROGATE_READ),
        step_capability=SURROGATE_READ,
    )
    assert (
        _digests(declared)[STRUCTURAL_STEP_ID] != _digests(MECHANICAL_PACKAGE)[STRUCTURAL_STEP_ID]
    )
    EVALUATED.clear()
    with pytest.raises(DomainCapabilityError, match=SURROGATE_READ):
        run_reference_workflow(declared, CONTEXT, REFERENCE_PARAMETERS, declared.step_ids)
    assert EVALUATED == []
    assert declared.manifest().required_capabilities == ()


def test_package_digests_are_stable_within_a_process_and_across_processes() -> None:
    assert MECHANICAL_PACKAGE.digest() == MECHANICAL_PACKAGE.digest()
    assert MECHANICAL_PACKAGE.package_payload() == MECHANICAL_PACKAGE.package_payload()
    assert MECHANICAL_PACKAGE.manifest().digest == MECHANICAL_PACKAGE.digest()
    assert (
        evaluator_implementation(MECHANICAL_PACKAGE.evaluator(STRUCTURAL_STEP_ID)).digest
        == evaluator_implementation(MECHANICAL_PACKAGE.evaluator(STRUCTURAL_STEP_ID)).digest
    )
    assert _package().digest() == MECHANICAL_PACKAGE.digest()
    assert _package().digest() == _package().digest()
    assert _digest_in_a_fresh_process() == MECHANICAL_PACKAGE.digest()


def test_rewriting_an_evaluator_module_changes_its_digest(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    shipped = module_source_digest(MECHANICAL_EVALUATOR_MODULE)
    path = tmp_path / "rewritten_evaluator.py"
    path.write_text(REWRITTEN_MODULE_SOURCE, encoding="utf-8")
    module = _load_module(path, "kala_agent.domains.tests_rewritten", monkeypatch)
    rewritten = cast("DomainEvaluator[Any]", module.RewrittenEvaluator())

    first = module_source_digest(module.__name__)
    assert first == hashlib.sha256(REWRITTEN_MODULE_SOURCE.encode("utf-8")).hexdigest()
    with pytest.raises(NotImplementedError):
        rewritten.evaluate(CONTEXT, REFERENCE_PARAMETERS)
    implementation = evaluator_implementation(rewritten)
    assert implementation.module_digest == first
    assert implementation.digest == evaluator_implementation(rewritten).digest

    path.write_text(
        REWRITTEN_MODULE_SOURCE.replace("NotImplementedError", "ValueError"), encoding="utf-8"
    )
    assert module_source_digest(module.__name__) != first
    assert evaluator_implementation(rewritten).digest != implementation.digest
    assert module_source_digest(MECHANICAL_EVALUATOR_MODULE) == shipped
    assert MECHANICAL_PACKAGE.digest() == _package().digest()

    module.__file__ = str(tmp_path / "vanished_evaluator.py")
    with pytest.raises(DomainPackageError, match="cannot be read"):
        module_source_digest(module.__name__)
    with pytest.raises(DomainPackageError):
        _package(evaluators=(*MECHANICAL_PACKAGE.evaluators[:3], rewritten))


def test_editing_a_sibling_formula_module_changes_the_evaluator_digest(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    package = tmp_path / "surrogate_package"
    package.mkdir()
    evaluator_path = package / "evaluation.py"
    evaluator_path.write_text(REWRITTEN_MODULE_SOURCE, encoding="utf-8")
    formula_path = package / "simulation.py"
    formula_path.write_text("YIELD_STRENGTH_MPA = 250.0\n", encoding="utf-8")
    module = _load_module(evaluator_path, "kala_agent.domains.tests_sibling", monkeypatch)
    evaluator = cast("DomainEvaluator[Any]", module.RewrittenEvaluator())

    before = evaluator_implementation(evaluator)
    formula_path.write_text("YIELD_STRENGTH_MPA = 9999.0\n", encoding="utf-8")
    after = evaluator_implementation(evaluator)

    assert after.module_digest == before.module_digest
    assert after.package_digest != before.package_digest
    assert after.digest != before.digest
    assert package_source_digest(MECHANICAL_EVALUATOR_MODULE) != module_source_digest(
        MECHANICAL_EVALUATOR_MODULE
    )


def test_evaluators_without_an_identifiable_implementation_are_refused() -> None:
    module = sys.modules.setdefault(SOURCELESS_MODULE, ModuleType(SOURCELESS_MODULE))
    assert getattr(module, "__file__", None) is None

    for hidden in (
        _hidden_evaluator(ABSENT_MODULE),
        _hidden_evaluator(SOURCELESS_MODULE),
        _hidden_evaluator(""),
        _hidden_evaluator(None),
        _hidden_evaluator(__name__, "Hidden Evaluator"),
        _hidden_evaluator(__name__, "a" * 300),
    ):
        with pytest.raises(DomainPackageError):
            evaluator_implementation(hidden)
        with pytest.raises(DomainPackageError):
            _package(evaluators=(*MECHANICAL_PACKAGE.evaluators[:3], hidden))

    for unimportable in (ABSENT_MODULE, SOURCELESS_MODULE, "", None, 7):
        with pytest.raises(DomainPackageError):
            module_source_digest(unimportable)
    with pytest.raises(DomainPackageError):
        EvaluatorImplementation(
            step_id=STRUCTURAL_STEP_ID,
            qualified_name="a.b.C",
            module_name="not a module",
            module_digest=ZERO_DIGEST,
            package_digest=ZERO_DIGEST,
            required_capabilities=(),
            digest=ZERO_DIGEST,
        )
    with pytest.raises(DomainPackageError):
        EvaluatorImplementation(
            step_id=STRUCTURAL_STEP_ID,
            qualified_name="other.module.C",
            module_name="a.b",
            module_digest=ZERO_DIGEST,
            package_digest=ZERO_DIGEST,
            required_capabilities=(),
            digest=ZERO_DIGEST,
        )
    assert MECHANICAL_PACKAGE.digest() == _package().digest()


def test_declared_capabilities_are_denied_before_any_evaluation() -> None:
    EVALUATED.clear()
    capable = _package(
        evaluators=_bind(RecordingStructuralEvaluator, SURROGATE_READ),
        step_capability=SURROGATE_READ,
        package_capabilities=(REFERENCE_RUN,),
    )
    required = (REFERENCE_RUN, SURROGATE_READ)
    assert capable.manifest().required_capabilities == (REFERENCE_RUN,)
    assert capable.catalog_step(STRUCTURAL_STEP_ID).required_capabilities == (SURROGATE_READ,)
    assert MECHANICAL_PACKAGE.required_capabilities == ()
    assert run_reference_workflow(
        MECHANICAL_PACKAGE, CONTEXT, REFERENCE_PARAMETERS, capable.step_ids
    ).passed
    assert EVALUATED == []

    with pytest.raises(DomainCapabilityError, match=SURROGATE_READ):
        run_reference_workflow(capable, CONTEXT, REFERENCE_PARAMETERS, capable.step_ids)
    assert EVALUATED == []

    for granted in ((), (SURROGATE_READ,), (REFERENCE_RUN,), (*required, "mechanical.extra")):
        with pytest.raises(DomainCapabilityError):
            run_reference_workflow(
                capable,
                CONTEXT,
                REFERENCE_PARAMETERS,
                capable.step_ids,
                capability_policy=_FixedGrantPolicy(granted),
            )
        assert EVALUATED == []

    policy = _FixedGrantPolicy(required)
    assert isinstance(policy, CapabilityGrantPolicy)
    assert run_reference_workflow(
        capable,
        CONTEXT,
        REFERENCE_PARAMETERS,
        capable.step_ids,
        capability_policy=policy,
    ).passed
    assert EVALUATED == [STRUCTURAL_STEP_ID]
    assert policy.requests == [
        CapabilityRequest(
            package_id=capable.package_id,
            step_ids=capable.step_ids,
            required_capabilities=required,
        )
    ]

    partial = _package(step_capability=SURROGATE_READ)
    assert partial.manifest().required_capabilities == ()
    with pytest.raises(DomainCapabilityError, match=SURROGATE_READ):
        run_reference_workflow(
            partial,
            CONTEXT,
            REFERENCE_PARAMETERS,
            partial.step_ids,
            capability_policy=_FixedGrantPolicy((REFERENCE_RUN,)),
        )
    assert run_reference_workflow(
        partial,
        CONTEXT,
        REFERENCE_PARAMETERS,
        partial.step_ids,
        capability_policy=_FixedGrantPolicy((SURROGATE_READ,)),
    ).passed


def test_a_surplus_grant_to_a_capability_free_run_is_refused() -> None:
    EVALUATED.clear()
    plain = _package()
    assert plain.required_capabilities == ()
    with pytest.raises(DomainCapabilityError, match=SURROGATE_READ):
        run_reference_workflow(
            plain,
            CONTEXT,
            REFERENCE_PARAMETERS,
            plain.step_ids,
            capability_policy=_FixedGrantPolicy((SURROGATE_READ,)),
        )
    assert EVALUATED == []
    assert (
        enforce_capability_request(
            plain, plain.step_ids, _FixedGrantPolicy(())
        ).required_capabilities
        == ()
    )


def test_single_step_evaluation_enforces_the_capability_request() -> None:
    capable = _package(
        evaluators=_bind(RecordingStructuralEvaluator, SURROGATE_READ),
        step_capability=SURROGATE_READ,
    )
    with pytest.raises(DomainCapabilityError, match=SURROGATE_READ):
        enforce_capability_request(capable, (STRUCTURAL_STEP_ID,))
    with pytest.raises(DomainStepError):
        enforce_capability_request(capable, ("mechanical.undeclared",))
    request = enforce_capability_request(
        capable, (STRUCTURAL_STEP_ID,), _FixedGrantPolicy((SURROGATE_READ,))
    )
    assert request.required_capabilities == (SURROGATE_READ,)


def test_a_checkpoint_mismatch_rejects_the_package() -> None:
    for checkpoint in ApprovalKind:
        with pytest.raises(DomainPackageError, match=STRUCTURAL_STEP_ID):
            _package(workflow=_checkpoint_workflow(STRUCTURAL_STEP_ID, None))
        with pytest.raises(DomainPackageError, match=STRUCTURAL_STEP_ID):
            _package(steps=_steps_without_checkpoint(STRUCTURAL_STEP_ID))
        if checkpoint is not ApprovalKind.PRE_COMMIT:
            with pytest.raises(DomainPackageError, match=STRUCTURAL_STEP_ID):
                _package(workflow=_checkpoint_workflow(STRUCTURAL_STEP_ID, checkpoint))
        with pytest.raises(DomainPackageError, match=GEOMETRY_STEP_ID):
            _package(workflow=_checkpoint_workflow(GEOMETRY_STEP_ID, checkpoint))
        with pytest.raises(DomainPackageError, match=GEOMETRY_STEP_ID):
            _package(steps=_checkpoint_steps(GEOMETRY_STEP_ID, checkpoint))

        agreeing = _package(
            workflow=_checkpoint_workflow(GEOMETRY_STEP_ID, checkpoint),
            steps=_checkpoint_steps(GEOMETRY_STEP_ID, checkpoint),
        )
        assert agreeing.catalog_step(GEOMETRY_STEP_ID).checkpoint_kind is checkpoint
        assert agreeing.workflow.transitions[0].checkpoint_type is checkpoint
        assert agreeing.digest() != MECHANICAL_PACKAGE.digest()

    with pytest.raises(DomainPackageError):
        _package(
            workflow=WorkflowDefinition(
                definition_id="mechanical.integrity-no-transition",
                version="0.1.0",
                name="No transition fixture",
                initial_state="specified",
                states=("specified",),
                available_step_ids=(GEOMETRY_STEP_ID,),
                max_replanning_attempts=1,
            ),
            steps=(replace(MECHANICAL_STEPS[0], checkpoint_kind=ApprovalKind.EXPORT),),
            evaluators=(SHIPPED_EVALUATORS[GEOMETRY_STEP_ID],),
        )
    uncheckpointed = _package(
        workflow=WorkflowDefinition(
            definition_id="mechanical.integrity-no-transition",
            version="0.1.0",
            name="No transition fixture",
            initial_state="specified",
            states=("specified",),
            available_step_ids=(GEOMETRY_STEP_ID,),
            max_replanning_attempts=1,
        ),
        steps=(replace(MECHANICAL_STEPS[0], checkpoint_kind=None),),
        evaluators=(SHIPPED_EVALUATORS[GEOMETRY_STEP_ID],),
    )
    assert uncheckpointed.workflow.transitions == ()
    assert uncheckpointed.catalog_step(GEOMETRY_STEP_ID).checkpoint_kind is None
    assert uncheckpointed.digest() != MECHANICAL_PACKAGE.digest()
    with pytest.raises(DomainParameterError):
        _package(
            steps=(
                replace(MECHANICAL_STEPS[0], checkpoint_kind=cast("ApprovalKind", "export.typo")),
                *MECHANICAL_STEPS[1:],
            )
        )
    with pytest.raises(DomainParameterError):
        CatalogStep(
            step_id=GEOMETRY_STEP_ID,
            name="Broken",
            description="Broken descriptor",
            checkpoint_kind=cast("ApprovalKind", "export.typo"),
        )
    assert MECHANICAL_PACKAGE.catalog_step(STRUCTURAL_STEP_ID).checkpoint_kind is (
        ApprovalKind.PRE_COMMIT
    )
    assert MECHANICAL_PACKAGE.workflow.transitions[-1].checkpoint_type is ApprovalKind.PRE_COMMIT
    assert _package().digest() == MECHANICAL_PACKAGE.digest()
