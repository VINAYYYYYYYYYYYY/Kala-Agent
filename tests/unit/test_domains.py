from __future__ import annotations

import ast
import hashlib
import math
import sys
from collections.abc import Iterator, Mapping
from dataclasses import replace
from datetime import UTC, datetime
from pathlib import Path
from types import ModuleType
from typing import Any, cast

import pytest
from pydantic import BaseModel, ValidationError

from kala_agent.contracts import (
    ApprovalKind,
    DomainPackageManifest,
    EvaluationResult,
    StepStatus,
    WorkflowDefinition,
    WorkflowStatus,
    WorkflowTransition,
)
from kala_agent.domains import (
    NON_AUTHORITATIVE_FINDING,
    REFERENCE_EVALUATION_EPOCH,
    CapabilityRequest,
    CatalogStep,
    DomainCapabilityError,
    DomainEvaluator,
    DomainPackage,
    DomainPackageError,
    DomainParameterError,
    DomainStepError,
    EvaluationContext,
    ParameterSpec,
    ReferenceWorkflowRun,
    canonical_digest,
    clamp_unit_interval,
    evaluator_implementation,
    module_source_digest,
    reference_context,
    require_capabilities,
    require_name,
    run_reference_workflow,
)
from kala_agent.domains.building import (
    AREA_STEP_ID,
    BUILDING_PACKAGE,
    BUILDING_SCHEMA_DIGEST,
    BUILDING_STEPS,
    BUILDING_WORKFLOW,
    CODE_AUTHORITY_FINDING,
    NON_AUTHORITATIVE_CODE_VERSION,
    STOREY_STEP_ID,
    THERMAL_STEP_ID,
    BuildingCodeVersion,
    BuildingParameters,
    run_building_reference_workflow,
)
from kala_agent.domains.building import (
    PARAMETER_SPECS as BUILDING_PARAMETER_SPECS,
)
from kala_agent.domains.building import (
    ConstraintViolation as BuildingViolation,
)
from kala_agent.domains.building import (
    violations_for as building_violations_for,
)
from kala_agent.domains.mechanical import (
    BRACKET_SCHEMA_DIGEST,
    GEOMETRY_STEP_ID,
    LOAD_STEP_ID,
    MATERIAL_STEP_ID,
    MATERIALS,
    MAX_LOAD_PER_POINT_KN,
    MECHANICAL_PACKAGE,
    MECHANICAL_STEPS,
    MECHANICAL_WORKFLOW,
    REFERENCE_PARAMETERS,
    STRUCTURAL_STEP_ID,
    BracketParameters,
    GeometryEvaluator,
    MaterialKey,
    material_reference,
    run_bracket_reference_workflow,
    simulate_bracket,
)
from kala_agent.domains.mechanical import (
    PARAMETER_SPECS as MECHANICAL_PARAMETER_SPECS,
)
from kala_agent.domains.mechanical import (
    ConstraintViolation as MechanicalViolation,
)
from kala_agent.domains.mechanical import (
    violations_for as mechanical_violations_for,
)
from kala_agent.domains.wing import (
    AERODYNAMIC_STEP_ID,
    NON_CERTIFIED_FINDING,
    PLANFORM_STEP_ID,
    WING_PACKAGE,
    WING_SCHEMA_DIGEST,
    WING_STEPS,
    WING_WORKFLOW,
    WingParameters,
    run_wing_reference_workflow,
    simulate_wing,
)
from kala_agent.domains.wing import (
    LOAD_STEP_ID as WING_LOAD_STEP_ID,
)
from kala_agent.domains.wing import (
    PARAMETER_SPECS as WING_PARAMETER_SPECS,
)
from kala_agent.domains.wing import (
    ConstraintViolation as WingViolation,
)
from kala_agent.domains.wing import (
    violations_for as wing_violations_for,
)

DOMAINS_ROOT = Path(__file__).parents[2] / "src" / "kala_agent" / "domains"
DOMAIN_MODULE_COUNT = 16
CONTEXT = reference_context("domain-test-subject")
RECORDED_AT = datetime(2026, 3, 4, 5, 6, 7, tzinfo=UTC)
SURROGATE_CAPABILITY = "mechanical.surrogate.read"
PACKAGE_CAPABILITY = "mechanical.reference.run"
MECHANICAL_EVALUATOR_MODULE = "kala_agent.domains.mechanical.evaluation"
MECHANICAL_EVALUATORS: Mapping[str, Any] = {
    evaluator.step_id: evaluator for evaluator in MECHANICAL_PACKAGE.evaluators
}
PACKAGE_CASES: tuple[tuple[str, DomainPackage, BaseModel], ...] = (
    ("mechanical", MECHANICAL_PACKAGE, BracketParameters()),
    ("building", BUILDING_PACKAGE, BuildingParameters()),
    ("wing", WING_PACKAGE, WingParameters()),
)
SPEC_CASES: tuple[tuple[str, type[BaseModel], tuple[ParameterSpec, ...]], ...] = (
    ("mechanical", BracketParameters, MECHANICAL_PARAMETER_SPECS),
    ("building", BuildingParameters, BUILDING_PARAMETER_SPECS),
    ("wing", WingParameters, WING_PARAMETER_SPECS),
)
INVALID_BRACKET_RANGES: tuple[Mapping[str, object], ...] = (
    {"thickness_mm": 0.1},
    {"thickness_mm": 25.1},
    {"hole_count": 13},
    {"hole_diameter_mm": 50.1},
    {"load_kn": -1.0},
    {"load_points": 9},
    {"maximum_stress_mpa": 0.5},
    {"material": "unobtainium"},
    {"unknown_field": 1.0},
)
INVALID_BRACKET_PATTERNS: tuple[Mapping[str, object], ...] = (
    {"hole_count": 2, "hole_diameter_mm": 0.0},
    {"hole_count": 12, "hole_diameter_mm": 3.0, "length_mm": 20.0},
    {"hole_count": 0, "minimum_edge_margin_mm": 2.0},
)
INVALID_BUILDING_VALUES: tuple[Mapping[str, object], ...] = (
    {"storey_count": 0},
    {"storey_count": 201},
    {"storey_height_m": 1.9},
    {"gross_floor_area_m2": 9.0},
    {"perimeter_m": 4.9},
    {"window_to_wall_ratio": 0.95},
    {"heating_degree_days_k": 0.5},
    {"code_version": "asce-7-2022"},
    {"unexpected": 1},
)
INVALID_WING_VALUES: tuple[Mapping[str, object], ...] = (
    {"span_m": 0.9},
    {"span_m": 120.1},
    {"tip_chord_m": 12.1},
    {"spanwise_stations": 1},
    {"spanwise_stations": 65},
    {"thickness_ratio": 0.02},
    {"sweep_deg": 75.1},
    {"load_case": "gust"},
    {"nope": 1},
)


def _run(
    package: DomainPackage,
    parameters: BaseModel,
    context: EvaluationContext,
) -> ReferenceWorkflowRun:
    return run_reference_workflow(package, context, parameters, package.step_ids)


def _metric(evaluation: EvaluationResult, name: str) -> float:
    for metric in evaluation.metrics:
        if metric.name == name:
            return metric.value
    raise AssertionError(f"{evaluation.evaluation_id} does not report {name}")


def _defaults_match(declared: object, model_default: object) -> bool:
    if isinstance(declared, (int, float)) and isinstance(model_default, (int, float)):
        return float(declared) == float(model_default)
    return declared == model_default


def _mechanical_package(
    package_id: str = "mechanical.test-copy",
    workflow: WorkflowDefinition = MECHANICAL_WORKFLOW,
    steps: tuple[CatalogStep, ...] = MECHANICAL_STEPS,
    evaluators: tuple[DomainEvaluator[Any], ...] = MECHANICAL_PACKAGE.evaluators,
    schema_digests: tuple[str, ...] = MECHANICAL_PACKAGE.schema_digests,
    required_capabilities: tuple[str, ...] = (),
) -> DomainPackage:
    return DomainPackage(
        package_id=package_id,
        version=MECHANICAL_PACKAGE.version,
        description=MECHANICAL_PACKAGE.description,
        disclaimer=MECHANICAL_PACKAGE.disclaimer,
        workflow=workflow,
        steps=steps,
        evaluators=evaluators,
        schema_digests=schema_digests,
        required_capabilities=required_capabilities,
    )


def _capability_package(
    step_capability: str | None = SURROGATE_CAPABILITY,
    *,
    evaluator_capability: str | None = None,
    package_capabilities: tuple[str, ...] = (),
    step_id: str = STRUCTURAL_STEP_ID,
) -> DomainPackage:
    steps = tuple(
        replace(step, required_capabilities=(step_capability,))
        if step_id == step.step_id and step_capability is not None
        else step
        for step in MECHANICAL_STEPS
    )
    declared = step_capability if evaluator_capability is None else evaluator_capability
    evaluators = tuple(
        replace(MECHANICAL_EVALUATORS[step_id], required_capabilities=(declared,))
        if step_id == evaluator.step_id and step_capability is not None
        else evaluator
        for evaluator in MECHANICAL_PACKAGE.evaluators
    )
    return _mechanical_package(
        steps=steps,
        evaluators=evaluators,
        required_capabilities=package_capabilities,
    )


class _GrantPolicy:
    """A policy that answers with one fixed grant set."""

    def __init__(self, granted: tuple[str, ...]) -> None:
        self.granted = granted
        self.requests: list[CapabilityRequest] = []

    def granted_capabilities(self, request: CapabilityRequest) -> tuple[str, ...]:
        self.requests.append(request)
        return self.granted


def _source_digest(module_name: str) -> str:
    origin = getattr(sys.modules[module_name], "__file__", None)
    assert isinstance(origin, str)
    return hashlib.sha256(Path(origin).read_bytes()).hexdigest()


def _sourceless_module_name() -> str:
    name = "kala_agent.domains.tests_sourceless_module"
    sys.modules.setdefault(name, ModuleType(name))
    return name


def _hidden_evaluator(module_name: str) -> DomainEvaluator[Any]:
    hidden: Any = type(
        "HiddenEvaluator",
        (),
        {
            "__module__": module_name,
            "step_id": STRUCTURAL_STEP_ID,
            "required_capabilities": (),
            "evaluate": lambda self, context, parameters: None,
        },
    )
    return cast("DomainEvaluator[Any]", hidden())


def _unidentified_evaluator() -> DomainEvaluator[Any]:
    return _hidden_evaluator("kala_agent.domains.tests_absent_module")


def _sourceless_evaluator() -> DomainEvaluator[Any]:
    return _hidden_evaluator(_sourceless_module_name())


def _capability_evaluator(capability: str) -> DomainEvaluator[Any]:
    replaced: Any = replace(
        MECHANICAL_EVALUATORS[STRUCTURAL_STEP_ID], required_capabilities=(capability,)
    )
    return cast("DomainEvaluator[Any]", replaced)


class _MalformedGrantPolicy:
    """A policy that reports its grants in a shape no policy should use."""

    def __init__(self, granted: object) -> None:
        self.granted: object = granted

    def granted_capabilities(self, request: CapabilityRequest) -> tuple[str, ...]:
        return cast("tuple[str, ...]", self.granted)


def _broken_workflow() -> WorkflowDefinition:
    return WorkflowDefinition.model_construct(
        definition_id="mechanical.broken",
        version="0.1.0",
        name="Broken declaration",
        initial_state="specified",
        states=("specified",),
        transitions=(
            WorkflowTransition.model_construct(
                transition_id="mechanical.broken",
                from_state="specified",
                to_state="unreachable-state",
                step_id=GEOMETRY_STEP_ID,
            ),
        ),
        available_step_ids=(GEOMETRY_STEP_ID,),
        max_replanning_attempts=1,
    )


def _parameter_grid(package_id: str) -> Iterator[BaseModel]:
    if package_id == MECHANICAL_PACKAGE.package_id:
        yield BracketParameters(
            thickness_mm=0.5, load_kn=0.0, hole_count=0, minimum_edge_margin_mm=0.0
        )
        yield BracketParameters(thickness_mm=8.0, load_kn=1.0, hole_count=2, hole_diameter_mm=1.0)
        yield BracketParameters(
            thickness_mm=25.0, load_kn=200.0, hole_count=12, hole_diameter_mm=1.0
        )
        yield BracketParameters(thickness_mm=1.0, load_kn=50.0)
        return
    if package_id == BUILDING_PACKAGE.package_id:
        yield BuildingParameters(
            storey_count=1, heating_degree_days_k=1.0, window_to_wall_ratio=0.0
        )
        yield BuildingParameters()
        yield BuildingParameters(
            storey_count=200,
            heating_degree_days_k=40_000.0,
            window_to_wall_ratio=0.9,
        )
        return
    yield WingParameters(
        span_m=1.0, root_chord_m=1.0, tip_chord_m=0.5, span_load_kn_m=0.0, spanwise_stations=2
    )
    yield WingParameters()
    yield WingParameters(
        span_m=60.0,
        root_chord_m=1.0,
        tip_chord_m=0.5,
        span_load_kn_m=5_000.0,
        spanwise_stations=64,
    )
    yield WingParameters(maximum_mass_kg=5.0, sweep_deg=-10.0, dihedral_deg=-20.0)


def test_packages_declare_valid_manifests_workflows_and_catalog_steps() -> None:
    for _, package, _ in PACKAGE_CASES:
        manifest = package.manifest()
        assert isinstance(manifest, DomainPackageManifest)
        assert manifest.package_id == package.package_id
        assert manifest.version == package.version
        assert len(manifest.digest) == 64
        assert manifest.workflow_definition_ids == (package.workflow.definition_id,)
        assert manifest.schema_digests == package.manifest_schema_digests()
        assert set(package.schema_digests) <= set(manifest.schema_digests)
        assert set(package.evaluator_implementation_digests) <= set(manifest.schema_digests)
        assert len(manifest.schema_digests) == len(
            set(package.schema_digests) | set(package.evaluator_implementation_digests)
        )
        assert manifest.signer_id is None
        assert manifest.signature_digest is None
        assert package.digest() == package.digest()
        assert manifest.digest == canonical_digest(package.package_payload())

        definition = package.workflow
        assert isinstance(definition, WorkflowDefinition)
        assert WorkflowDefinition.model_validate(definition.model_dump(mode="json")) == definition
        assert definition.initial_state in definition.states
        assert len(package.step_ids) == len(definition.available_step_ids)
        assert set(package.step_ids) == set(definition.available_step_ids)
        assert tuple(transition.step_id for transition in definition.transitions) == (
            package.step_ids
        )
        assert {transition.step_id for transition in definition.transitions} == set(
            definition.available_step_ids
        )
        transitions = {transition.step_id: transition for transition in definition.transitions}
        for step in package.steps:
            assert isinstance(step, CatalogStep)
            assert package.catalog_step(step.step_id) is step
            assert step.metric_names
            assert step.input_names
            assert transitions[step.step_id].checkpoint_type is step.checkpoint_kind
            assert package.evaluator(step.step_id).required_capabilities == (
                step.required_capabilities
            )

        assert [item.step_id for item in package.evaluator_implementations] == sorted(
            package.step_ids
        )
        assert len(package.evaluator_implementation_digests) == len(package.step_ids)
        assert set(package.evaluator_implementation_digests) == {
            item.digest for item in package.evaluator_implementations
        }

    assert MECHANICAL_PACKAGE.schema_digests == (BRACKET_SCHEMA_DIGEST,)
    assert BUILDING_PACKAGE.schema_digests == (BUILDING_SCHEMA_DIGEST,)
    assert WING_PACKAGE.schema_digests == (WING_SCHEMA_DIGEST,)
    assert MECHANICAL_STEPS[-1].step_id == STRUCTURAL_STEP_ID
    assert MECHANICAL_STEPS[-1].checkpoint_kind is ApprovalKind.PRE_COMMIT
    assert MECHANICAL_WORKFLOW.transitions[-1].checkpoint_type is ApprovalKind.PRE_COMMIT
    assert MECHANICAL_WORKFLOW.transitions[0].checkpoint_type is None
    assert BUILDING_STEPS[-1].step_id == THERMAL_STEP_ID
    assert BUILDING_STEPS[-1].checkpoint_kind is ApprovalKind.PRE_COMMIT
    assert WING_STEPS[-1].step_id == AERODYNAMIC_STEP_ID
    assert WING_STEPS[-1].checkpoint_kind is ApprovalKind.PRE_COMMIT
    assert BUILDING_WORKFLOW.definition_id != MECHANICAL_WORKFLOW.definition_id
    assert WING_WORKFLOW.definition_id != MECHANICAL_WORKFLOW.definition_id
    assert BUILDING_WORKFLOW.definition_id != WING_WORKFLOW.definition_id


def test_mechanical_reference_workflow_completes_and_is_reproducible() -> None:
    first = run_bracket_reference_workflow()
    second = run_bracket_reference_workflow()

    assert first.status is WorkflowStatus.COMPLETED
    assert first.passed
    assert first.blocked_step_id is None
    assert first.completed_step_ids == MECHANICAL_PACKAGE.step_ids
    assert first.ordered_step_ids == (
        GEOMETRY_STEP_ID,
        MATERIAL_STEP_ID,
        LOAD_STEP_ID,
        STRUCTURAL_STEP_ID,
    )
    assert all(outcome.status is StepStatus.SUCCEEDED for outcome in first.outcomes)
    assert first.definition_id == MECHANICAL_WORKFLOW.definition_id
    assert first.definition_version == MECHANICAL_WORKFLOW.version
    assert [evaluation.model_dump(mode="json") for evaluation in first.evaluations] == [
        evaluation.model_dump(mode="json") for evaluation in second.evaluations
    ]
    assert [evaluation.evaluated_at for evaluation in first.evaluations] == [
        REFERENCE_EVALUATION_EPOCH
    ] * len(first.evaluations)
    assert first.score is not None
    assert 0.0 <= first.score <= 1.0
    assert NON_AUTHORITATIVE_FINDING in first.findings

    recorded = run_bracket_reference_workflow(
        REFERENCE_PARAMETERS, reference_context("domain-test-subject", RECORDED_AT)
    )
    assert all(evaluation.evaluated_at == RECORDED_AT for evaluation in recorded.evaluations)
    assert [evaluation.evaluated_at for evaluation in recorded.evaluations] != [
        evaluation.evaluated_at for evaluation in first.evaluations
    ]
    assert [evaluation.model_dump(mode="json") for evaluation in recorded.evaluations] != [
        evaluation.model_dump(mode="json") for evaluation in first.evaluations
    ]


def test_mechanical_parameters_reject_invalid_values_and_patterns() -> None:
    assert BracketParameters() == REFERENCE_PARAMETERS
    assert BracketParameters(material=MaterialKey.STEEL_S355).material is MaterialKey.STEEL_S355
    assert material_reference("steel-s355").yield_strength_mpa > 0.0
    assert material_reference(MaterialKey.ALUMINIUM_6061_T6).reference_status
    assert set(MATERIALS) == {key.value for key in MaterialKey}

    for overrides in (*INVALID_BRACKET_RANGES, *INVALID_BRACKET_PATTERNS):
        with pytest.raises(ValidationError):
            BracketParameters(**overrides)

    with pytest.raises(DomainParameterError):
        material_reference("unobtainium")


def test_mechanical_evaluators_enforce_minimum_thickness_and_maximum_stress() -> None:
    thin = BracketParameters(thickness_mm=1.0)
    assert MechanicalViolation.THICKNESS_BELOW_MINIMUM in mechanical_violations_for(thin)
    thin_run = run_bracket_reference_workflow(thin, CONTEXT)
    assert thin_run.status is WorkflowStatus.FAILED
    assert not thin_run.passed
    assert thin_run.blocked_step_id == GEOMETRY_STEP_ID
    thin_geometry = thin_run.evaluations[0]
    assert not thin_geometry.passed
    assert thin_geometry.score is None
    assert any("thickness_below_minimum" in finding for finding in thin_geometry.findings)
    assert _metric(thin_geometry, "plate.thickness") == 1.0

    stressed = BracketParameters(load_kn=200.0)
    assert MechanicalViolation.STRESS_ABOVE_MAXIMUM in mechanical_violations_for(stressed)
    stressed_run = run_bracket_reference_workflow(stressed, CONTEXT)
    assert stressed_run.blocked_step_id == STRUCTURAL_STEP_ID
    assert any(outcome.status is StepStatus.FAILED for outcome in stressed_run.outcomes)
    structural = stressed_run.evaluations[-1]
    assert not structural.passed
    assert structural.score is not None
    assert 0.0 <= structural.score <= 1.0
    assert any("stress_above_maximum" in finding for finding in structural.findings)
    assert _metric(structural, "stress.surrogate") > _metric(structural, "stress.maximum")
    assert _metric(structural, "margin.safety_factor") < _metric(
        structural, "margin.target_safety_factor"
    )

    bounded = BracketParameters(
        length_mm=30.0, minimum_edge_margin_mm=10.0, hole_count=12, hole_diameter_mm=1.0
    )
    assert MechanicalViolation.EDGE_MARGIN_BELOW_MINIMUM in mechanical_violations_for(bounded)
    assert run_bracket_reference_workflow(bounded, CONTEXT).blocked_step_id == MATERIAL_STEP_ID

    overload = BracketParameters(load_kn=200.0, load_points=1)
    assert MechanicalViolation.LOAD_PER_POINT_ABOVE_BOUND in mechanical_violations_for(overload)

    reference_run = run_bracket_reference_workflow(REFERENCE_PARAMETERS, CONTEXT)
    assert reference_run.passed
    assert _metric(reference_run.evaluations[2], "load.bound_per_point") == MAX_LOAD_PER_POINT_KN
    assert _metric(reference_run.evaluations[-1], "stress.surrogate") < _metric(
        reference_run.evaluations[-1], "stress.maximum"
    )

    unloaded = run_bracket_reference_workflow(BracketParameters(load_kn=0.0), CONTEXT)
    assert unloaded.passed
    assert unloaded.score == 1.0


def test_mechanical_reference_simulation_is_deterministic_and_ordered() -> None:
    first = simulate_bracket(REFERENCE_PARAMETERS)
    second = simulate_bracket(REFERENCE_PARAMETERS)
    assert first == second
    assert first.mass_kg > 0.0
    assert first.net_area_mm2 < first.gross_area_mm2
    assert first.stress_mpa >= first.bending_stress_mpa > 0.0
    assert first.stress_mpa >= first.shear_stress_mpa
    material = material_reference(REFERENCE_PARAMETERS.material)
    assert first.safety_factor == pytest.approx(material.yield_strength_mpa / first.stress_mpa)
    assert 0.0 < first.utilisation < 1.0
    assert simulate_bracket(BracketParameters(load_kn=2.0)).stress_mpa > first.stress_mpa
    assert simulate_bracket(BracketParameters(thickness_mm=12.0)).stress_mpa < first.stress_mpa
    steel = simulate_bracket(BracketParameters(material=MaterialKey.STEEL_S355))
    assert steel.bending_stress_mpa == pytest.approx(first.bending_stress_mpa)
    assert steel.mass_kg > first.mass_kg


def test_building_package_keeps_the_code_reference_non_authoritative() -> None:
    parameters = BuildingParameters()
    assert parameters.code_version is BuildingCodeVersion.NON_AUTHORITATIVE_PLACEHOLDER
    assert parameters.code_version.value == NON_AUTHORITATIVE_CODE_VERSION
    assert "non-authoritative" in parameters.code_version.value
    assert BuildingCodeVersion.NON_AUTHORITATIVE_PLACEHOLDER.value in CODE_AUTHORITY_FINDING

    run = run_building_reference_workflow(parameters, CONTEXT)
    assert run.passed
    assert run.completed_step_ids == (STOREY_STEP_ID, AREA_STEP_ID, THERMAL_STEP_ID)
    assert run.score is not None
    assert 0.0 <= run.score <= 1.0
    assert all(CODE_AUTHORITY_FINDING in evaluation.findings for evaluation in run.evaluations)
    assert all(NON_AUTHORITATIVE_FINDING in evaluation.findings for evaluation in run.evaluations)
    thermal = run.evaluations[-1]
    assert _metric(thermal, "code.version_is_placeholder") == 1.0
    assert _metric(thermal, "code.compliance_determined") == 0.0
    assert _metric(thermal, "envelope.conductance_total") > 0.0
    assert _metric(thermal, "envelope.heating_intensity") <= _metric(
        thermal, "envelope.maximum_heating_intensity"
    )
    assert _metric(thermal, "envelope.annual_heating") > 0.0


def test_building_parameters_and_placeholder_constraints_are_enforced() -> None:
    for overrides in INVALID_BUILDING_VALUES:
        with pytest.raises(ValidationError):
            BuildingParameters(**overrides)

    with pytest.raises(ValidationError):
        BuildingParameters(design_outdoor_temperature_c=25.0, design_indoor_temperature_c=20.0)

    intense = BuildingParameters(heating_degree_days_k=40_000.0)
    assert BuildingViolation.HEATING_INTENSITY_ABOVE_PLACEHOLDER_BOUND in (
        building_violations_for(intense)
    )
    intense_run = run_building_reference_workflow(intense, CONTEXT)
    assert intense_run.blocked_step_id == THERMAL_STEP_ID
    assert any(
        "heating_intensity_above_placeholder_bound" in finding for finding in intense_run.findings
    )

    open_envelope = BuildingParameters(
        wall_conductivity_w_m2k=0.0,
        window_conductivity_w_m2k=0.0,
        roof_conductivity_w_m2k=0.0,
    )
    assert BuildingViolation.ENVELOPE_CONDUCTIVITY_ZERO in building_violations_for(open_envelope)
    assert (
        run_building_reference_workflow(open_envelope, CONTEXT).blocked_step_id == THERMAL_STEP_ID
    )

    thin_storey = BuildingParameters(
        storey_count=100,
        gross_floor_area_m2=10.0,
        minimum_storey_area_m2=20.0,
        maximum_height_m=400.0,
    )
    assert BuildingViolation.STOREY_AREA_BELOW_PLACEHOLDER_MINIMUM in building_violations_for(
        thin_storey
    )
    assert run_building_reference_workflow(thin_storey, CONTEXT).blocked_step_id == AREA_STEP_ID

    tall = BuildingParameters(storey_count=200, storey_height_m=6.0, maximum_height_m=200.0)
    assert BuildingViolation.TOTAL_HEIGHT_ABOVE_PLACEHOLDER_BOUND in building_violations_for(tall)
    assert run_building_reference_workflow(tall, CONTEXT).blocked_step_id == STOREY_STEP_ID


def test_wing_package_reports_non_certified_surrogates_only() -> None:
    parameters = WingParameters()
    run = run_wing_reference_workflow(parameters, CONTEXT)
    assert run.passed
    assert run.completed_step_ids == (PLANFORM_STEP_ID, WING_LOAD_STEP_ID, AERODYNAMIC_STEP_ID)
    assert run.score is not None
    assert 0.0 <= run.score <= 1.0
    assert all(NON_CERTIFIED_FINDING in evaluation.findings for evaluation in run.evaluations)

    simulation = simulate_wing(parameters)
    assert simulation == simulate_wing(parameters)
    assert 0.0 < simulation.taper_ratio < 1.0
    expected_aspect = 2.0 * parameters.span_m / (parameters.root_chord_m + parameters.tip_chord_m)
    assert simulation.aspect_ratio == pytest.approx(expected_aspect, rel=1e-9)
    assert 0.0 < simulation.lift_curve_slope_per_rad < 2.0 * math.pi
    assert simulation.induced_drag_factor > 0.0
    assert simulation.mass_kg > 0.0
    assert _metric(run.evaluations[0], "planform.taper_ratio") == pytest.approx(
        simulation.taper_ratio
    )
    assert _metric(run.evaluations[1], "load.root_bending_moment") == pytest.approx(
        simulation.root_bending_moment_kn_m
    )
    assert _metric(run.evaluations[2], "mass.reference") == pytest.approx(simulation.mass_kg)

    for overrides in INVALID_WING_VALUES:
        with pytest.raises(ValidationError):
            WingParameters(**overrides)

    with pytest.raises(ValidationError):
        WingParameters(root_chord_m=1.0, tip_chord_m=2.0)
    with pytest.raises(ValidationError):
        WingParameters(span_m=1.0, root_chord_m=6.0, tip_chord_m=1.0)


def test_wing_surrogate_constraints_are_enforced() -> None:
    high_aspect = WingParameters(span_m=100.0, root_chord_m=0.05, tip_chord_m=0.01)
    assert WingViolation.ASPECT_RATIO_ABOVE_MAXIMUM in wing_violations_for(high_aspect)
    aspect_run = run_wing_reference_workflow(high_aspect, CONTEXT)
    assert aspect_run.blocked_step_id == PLANFORM_STEP_ID
    assert any("aspect_ratio_above_maximum" in finding for finding in aspect_run.findings)

    heavy_load = WingParameters(span_load_kn_m=5_000.0, maximum_root_bending_moment_kn_m=150.0)
    assert WingViolation.ROOT_BENDING_MOMENT_ABOVE_MAXIMUM in wing_violations_for(heavy_load)
    load_run = run_wing_reference_workflow(heavy_load, CONTEXT)
    assert load_run.blocked_step_id == WING_LOAD_STEP_ID
    assert any("root_bending_moment_above_maximum" in finding for finding in load_run.findings)

    heavy_wing = WingParameters(maximum_mass_kg=5.0)
    assert WingViolation.MASS_ABOVE_MAXIMUM in wing_violations_for(heavy_wing)
    mass_run = run_wing_reference_workflow(heavy_wing, CONTEXT)
    assert mass_run.blocked_step_id == AERODYNAMIC_STEP_ID
    assert any("mass_above_maximum" in finding for finding in mass_run.findings)


def test_declared_parameter_tables_match_models_and_enforce_ranges() -> None:
    for name, model, specs in SPEC_CASES:
        declared = {spec.name: spec for spec in specs}
        assert len(declared) == len(specs), name
        assert set(declared) == set(model.model_fields), name
        for field_name, spec in declared.items():
            require_name(spec.name, "parameter name")
            assert spec.unit.strip(), (name, field_name)
            assert spec.description.strip(), (name, field_name)
            assert _defaults_match(spec.default, model.model_fields[field_name].default), (
                name,
                field_name,
            )
            if not isinstance(spec.default, (int, float)):
                continue
            with pytest.raises(ValidationError):
                model(**{field_name: spec.maximum + 1.0})
            with pytest.raises(ValidationError):
                model(**{field_name: spec.minimum - 1.0})


def test_scores_and_metrics_stay_within_declared_bounds() -> None:
    for _, package, _ in PACKAGE_CASES:
        for candidate in _parameter_grid(package.package_id):
            run = _run(package, candidate, CONTEXT)
            assert run.evaluations
            for evaluation in run.evaluations:
                assert isinstance(evaluation, EvaluationResult)
                assert evaluation.metrics, evaluation.evaluation_id
                names = [metric.name for metric in evaluation.metrics]
                assert len(names) == len(set(names)), evaluation.evaluation_id
                for metric in evaluation.metrics:
                    assert math.isfinite(metric.value), evaluation.evaluation_id
                    assert metric.value >= 0.0, evaluation.evaluation_id
                if evaluation.score is not None:
                    assert 0.0 <= evaluation.score <= 1.0, evaluation.evaluation_id
            if run.score is not None:
                assert 0.0 <= run.score <= 1.0

    assert clamp_unit_interval(-5.0) == 0.0
    assert clamp_unit_interval(5.0) == 1.0
    for invalid in (math.inf, -math.inf, math.nan, True):
        with pytest.raises(DomainParameterError):
            clamp_unit_interval(invalid)


def test_all_three_domains_run_through_the_common_evaluator_interface() -> None:
    covered: set[str] = set()
    for name, package, parameters in PACKAGE_CASES:
        run = _run(package, parameters, CONTEXT)
        assert run.package_id == package.package_id
        assert run.passed, name
        assert run.blocked_step_id is None, name
        assert (
            tuple(outcome.evaluation for outcome in run.outcomes if outcome.evaluation is not None)
            == run.evaluations
        )
        for evaluator in package.evaluators:
            assert isinstance(evaluator, DomainEvaluator), name
            assert evaluator.step_id in package.step_ids
            assert evaluator.required_capabilities == ()
        for step_id in package.step_ids:
            evaluation = package.evaluate(step_id, CONTEXT, parameters)
            assert isinstance(evaluation, EvaluationResult)
            assert evaluation.evaluation_id.endswith(step_id)
            assert evaluation.subject_id == CONTEXT.subject_id
            assert evaluation.evaluated_at == CONTEXT.evaluated_at
        covered.add(name)
    assert covered == {"mechanical", "building", "wing"}


def test_domain_packages_reject_inconsistent_declarations() -> None:
    evaluators = MECHANICAL_PACKAGE.evaluators

    with pytest.raises(DomainStepError):
        MECHANICAL_PACKAGE.catalog_step("mechanical.not-declared")
    with pytest.raises(DomainStepError):
        MECHANICAL_PACKAGE.evaluator("mechanical.not-declared")
    with pytest.raises(DomainParameterError):
        run_reference_workflow(MECHANICAL_PACKAGE, CONTEXT, REFERENCE_PARAMETERS, ())
    with pytest.raises(DomainParameterError):
        run_reference_workflow(
            MECHANICAL_PACKAGE,
            CONTEXT,
            REFERENCE_PARAMETERS,
            (GEOMETRY_STEP_ID, GEOMETRY_STEP_ID),
        )
    with pytest.raises(DomainStepError):
        run_reference_workflow(
            MECHANICAL_PACKAGE, CONTEXT, REFERENCE_PARAMETERS, ("mechanical.not-declared",)
        )
    with pytest.raises(DomainParameterError):
        EvaluationContext(subject_id="domain-test-subject", evaluated_at=datetime(2026, 1, 1))
    with pytest.raises(DomainParameterError):
        EvaluationContext(subject_id="not a subject", evaluated_at=RECORDED_AT)
    with pytest.raises(DomainParameterError):
        require_name("Not A Name", "parameter name")
    with pytest.raises(DomainParameterError):
        ParameterSpec(
            name="width",
            unit="mm",
            minimum=10.0,
            maximum=1.0,
            default=0.0,
            description="broken",
        )
    with pytest.raises(DomainParameterError):
        CatalogStep(
            step_id="mechanical.broken",
            name="Broken",
            description="Broken descriptor",
            metric_names=("Not A Metric",),
        )
    with pytest.raises(DomainParameterError):
        CatalogStep(
            step_id="mechanical.broken",
            name="Broken",
            description="Broken descriptor",
            max_attempts=0,
        )

    with pytest.raises(DomainPackageError):
        _mechanical_package(steps=())
    with pytest.raises(DomainPackageError):
        _mechanical_package(evaluators=())
    with pytest.raises(DomainPackageError):
        _mechanical_package(schema_digests=())
    with pytest.raises(DomainParameterError):
        _mechanical_package(schema_digests=("not-a-digest",))
    with pytest.raises(DomainPackageError):
        _mechanical_package(steps=MECHANICAL_STEPS[:2])
    with pytest.raises(DomainPackageError):
        _mechanical_package(evaluators=evaluators[:2])
    with pytest.raises(DomainPackageError):
        _mechanical_package(workflow=_broken_workflow())
    with pytest.raises(DomainParameterError):
        _mechanical_package(required_capabilities=("Not A Capability",))
    with pytest.raises(DomainParameterError):
        _mechanical_package(
            required_capabilities=cast("tuple[str, ...]", "mechanical.reference.run")
        )
    with pytest.raises(DomainPackageError):
        _mechanical_package(evaluators=(*evaluators[:3], _unidentified_evaluator()))
    with pytest.raises(DomainPackageError):
        _mechanical_package(evaluators=(*evaluators[:3], _sourceless_evaluator()))
    with pytest.raises(DomainPackageError):
        _mechanical_package(
            evaluators=(*evaluators[:3], _capability_evaluator(SURROGATE_CAPABILITY))
        )


def test_evaluator_implementations_are_identified_and_digested() -> None:
    for _, package, _ in PACKAGE_CASES:
        assert package.evaluator_implementations
        assert len(package.evaluator_implementation_digests) == len(package.step_ids)
        assert len(set(package.evaluator_implementation_digests)) == len(package.step_ids)
        for item in package.evaluator_implementations:
            assert item.qualified_name == (
                f"{item.module_name}.{item.qualified_name.rsplit('.', 1)[-1]}"
            )
            assert item.qualified_name.rsplit(".", 1)[-1] in item.qualified_name
            assert item.module_digest == _source_digest(item.module_name)
            assert item.module_digest == module_source_digest(item.module_name)
            assert item.digest == evaluator_implementation(package.evaluator(item.step_id)).digest
            assert item.digest != item.module_digest
            assert item.as_dict()["digest"] == item.digest
        payload = package.package_payload()
        declared = payload["evaluators"]
        assert isinstance(declared, list)
        assert [entry["step_id"] for entry in declared] == sorted(package.step_ids)
        assert {entry["digest"] for entry in declared} == set(
            package.evaluator_implementation_digests
        )

    shipped = evaluator_implementation(GeometryEvaluator())
    assert shipped.step_id == GEOMETRY_STEP_ID
    assert shipped.module_name == MECHANICAL_EVALUATOR_MODULE
    assert shipped.qualified_name == f"{MECHANICAL_EVALUATOR_MODULE}.GeometryEvaluator"
    assert shipped.required_capabilities == ()
    assert _mechanical_package().digest() == _mechanical_package().digest()

    with pytest.raises(DomainPackageError):
        module_source_digest("")
    with pytest.raises(DomainPackageError):
        module_source_digest("kala_agent.domains.not_imported")
    with pytest.raises(DomainPackageError):
        module_source_digest(_sourceless_module_name())


def test_capabilities_are_canonicalised_and_required_capability_grants_are_enforced() -> None:
    assert require_capabilities(["b.capability", "a.capability", "b.capability"], "granted") == (
        "a.capability",
        "b.capability",
    )
    assert require_capabilities((), "granted") == ()
    for invalid in ("a.capability", ["Not A Capability"], [1], 3):
        with pytest.raises(DomainParameterError):
            require_capabilities(invalid, "granted")

    uncapped = _mechanical_package(required_capabilities=())
    assert run_reference_workflow(uncapped, CONTEXT, REFERENCE_PARAMETERS, uncapped.step_ids)
    assert run_bracket_reference_workflow().passed

    step_only = _capability_package()
    assert step_only.manifest().required_capabilities == ()
    assert step_only.manifest_schema_digests() != MECHANICAL_PACKAGE.manifest_schema_digests()
    with pytest.raises(DomainCapabilityError, match=SURROGATE_CAPABILITY):
        run_reference_workflow(step_only, CONTEXT, REFERENCE_PARAMETERS, step_only.step_ids)
    with pytest.raises(DomainCapabilityError):
        run_reference_workflow(
            step_only,
            CONTEXT,
            REFERENCE_PARAMETERS,
            step_only.step_ids,
            capability_policy=_GrantPolicy(()),
        )
    with pytest.raises(DomainCapabilityError):
        run_reference_workflow(
            step_only,
            CONTEXT,
            REFERENCE_PARAMETERS,
            step_only.step_ids,
            capability_policy=_GrantPolicy((SURROGATE_CAPABILITY, PACKAGE_CAPABILITY)),
        )
    with pytest.raises(DomainParameterError):
        run_reference_workflow(
            step_only,
            CONTEXT,
            REFERENCE_PARAMETERS,
            step_only.step_ids,
            capability_policy=_MalformedGrantPolicy("mechanical.surrogate.read"),
        )

    policy = _GrantPolicy((SURROGATE_CAPABILITY,))
    step_run = run_reference_workflow(
        step_only,
        CONTEXT,
        REFERENCE_PARAMETERS,
        step_only.step_ids,
        capability_policy=policy,
    )
    assert step_run.passed
    assert [request.required_capabilities for request in policy.requests] == [
        (SURROGATE_CAPABILITY,)
    ]
    assert policy.requests[0].package_id == step_only.package_id
    assert policy.requests[0].step_ids == step_only.step_ids

    package_only = _capability_package(None, package_capabilities=(PACKAGE_CAPABILITY,))
    assert package_only.catalog_step(STRUCTURAL_STEP_ID).required_capabilities == ()
    package_policy = _GrantPolicy((PACKAGE_CAPABILITY,))
    assert run_reference_workflow(
        package_only,
        CONTEXT,
        REFERENCE_PARAMETERS,
        package_only.step_ids,
        capability_policy=package_policy,
    ).passed
    assert package_policy.requests[0].required_capabilities == (PACKAGE_CAPABILITY,)

    both = _capability_package(package_capabilities=(PACKAGE_CAPABILITY,))
    both_policy = _GrantPolicy((SURROGATE_CAPABILITY, PACKAGE_CAPABILITY))
    assert run_reference_workflow(
        both,
        CONTEXT,
        REFERENCE_PARAMETERS,
        both.step_ids,
        capability_policy=both_policy,
    ).passed
    assert both_policy.requests[0].required_capabilities == (
        PACKAGE_CAPABILITY,
        SURROGATE_CAPABILITY,
    )
    with pytest.raises(DomainCapabilityError, match=PACKAGE_CAPABILITY):
        run_reference_workflow(
            both,
            CONTEXT,
            REFERENCE_PARAMETERS,
            both.step_ids,
            capability_policy=_GrantPolicy((SURROGATE_CAPABILITY,)),
        )

    with pytest.raises(DomainParameterError):
        CapabilityRequest(package_id="mechanical.test-copy", step_ids=(), required_capabilities=())
    with pytest.raises(DomainParameterError):
        CapabilityRequest(
            package_id="mechanical.test-copy",
            step_ids=("mechanical.geometry-check",),
            required_capabilities=("Not A Capability",),
        )


def test_domain_code_imports_contracts_and_the_shared_domain_module_only() -> None:
    allowed = ("kala_agent.contracts", "kala_agent.domains")
    forbidden = (
        "kala_agent.kernel",
        "kala_agent.tools",
        "kala_agent.persistence",
        "kala_agent.runtime",
        "kala_agent.api",
        "kala_agent.cli",
        "kala_agent.config",
    )
    checked = 0
    for path in sorted(DOMAINS_ROOT.rglob("*.py")):
        tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
        checked += 1
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                modules = [alias.name for alias in node.names]
            elif isinstance(node, ast.ImportFrom):
                modules = [node.module or ""]
            else:
                continue
            for module in modules:
                assert not module.startswith(forbidden), (path, module)
                if module.split(".")[0] == "kala_agent":
                    assert module.startswith(allowed), (path, module)
    assert checked == DOMAIN_MODULE_COUNT
