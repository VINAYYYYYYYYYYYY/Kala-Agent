"""Workflow definition, catalog steps, and reference run for the wing package.

The package declares a four-state planform, load, and aerodynamic surrogate
evaluation on the shared workflow vocabulary. It runs on the same kernel and the
same contracts as the mechanical and building packages, which is the evidence
that the shared surface is genuinely field-independent. Its outputs are geometric
and aerodynamic surrogates and are not certified.
"""

from __future__ import annotations

from typing import Final

from kala_agent.contracts import (
    ApprovalKind,
    EvaluationResult,
    WorkflowDefinition,
    WorkflowTransition,
)
from kala_agent.domains import (
    CatalogStep,
    DomainPackage,
    EvaluationContext,
    ReferenceWorkflowRun,
    reference_context,
    run_reference_workflow,
)
from kala_agent.domains.wing.evaluation import (
    AERODYNAMIC_STEP_ID,
    LOAD_STEP_ID,
    PLANFORM_STEP_ID,
    AerodynamicSurrogateEvaluator,
    PlanformGeometryEvaluator,
    SpanLoadEvaluator,
)
from kala_agent.domains.wing.parameters import WING_SCHEMA_DIGEST, WingParameters

PACKAGE_ID: Final[str] = "wing.parametric-tapered"
PACKAGE_VERSION: Final[str] = "0.1.0"
PACKAGE_DESCRIPTION: Final[str] = (
    "Wing reference package for span, chord, taper, and span load evaluation using "
    "geometric and aerodynamic surrogates that are not certified."
)
PACKAGE_DISCLAIMER: Final[str] = (
    "This package performs surrogate arithmetic only. It is not a lifting-line or panel "
    "method, not CFD, not a structural or fatigue analysis, and it certifies no "
    "airworthiness, stability, or control property."
)
WORKFLOW_DEFINITION_ID: Final[str] = "wing.surrogate-evaluation"
WORKFLOW_VERSION: Final[str] = "0.1.0"
WORKFLOW_NAME: Final[str] = "Parametric wing surrogate evaluation"
REFERENCE_SUBJECT_ID: Final[str] = "wing-reference"

_PLANFORM_INPUTS: Final[tuple[str, ...]] = (
    "span_m",
    "root_chord_m",
    "tip_chord_m",
    "sweep_deg",
    "dihedral_deg",
    "thickness_ratio",
    "spanwise_stations",
    "maximum_aspect_ratio",
)
_LOAD_INPUTS: Final[tuple[str, ...]] = (
    "load_case",
    "span_load_kn_m",
    "maximum_root_bending_moment_kn_m",
)
_AERODYNAMIC_INPUTS: Final[tuple[str, ...]] = (
    "span_m",
    "root_chord_m",
    "tip_chord_m",
    "maximum_mass_kg",
)

WING_WORKFLOW: Final[WorkflowDefinition] = WorkflowDefinition(
    definition_id=WORKFLOW_DEFINITION_ID,
    version=WORKFLOW_VERSION,
    name=WORKFLOW_NAME,
    description=PACKAGE_DISCLAIMER,
    initial_state="specified",
    states=("specified", "sized", "loaded", "assessed"),
    transitions=(
        WorkflowTransition(
            transition_id="wing.planform",
            from_state="specified",
            to_state="sized",
            step_id=PLANFORM_STEP_ID,
        ),
        WorkflowTransition(
            transition_id="wing.load",
            from_state="sized",
            to_state="loaded",
            step_id=LOAD_STEP_ID,
        ),
        WorkflowTransition(
            transition_id="wing.aerodynamics",
            from_state="loaded",
            to_state="assessed",
            step_id=AERODYNAMIC_STEP_ID,
            checkpoint_type=ApprovalKind.PRE_COMMIT,
        ),
    ),
    available_step_ids=(PLANFORM_STEP_ID, LOAD_STEP_ID, AERODYNAMIC_STEP_ID),
    max_replanning_attempts=1,
)

WING_STEPS: Final[tuple[CatalogStep, ...]] = (
    CatalogStep(
        step_id=PLANFORM_STEP_ID,
        name="Planform geometry",
        description=(
            "Integrate the declared chord distribution into a reference area and enforce "
            "the declared maximum aspect ratio."
        ),
        input_names=_PLANFORM_INPUTS,
        metric_names=(
            "planform.taper_ratio",
            "planform.reference_area",
            "planform.aspect_ratio",
            "planform.mean_chord",
            "planform.sweep_magnitude",
        ),
    ),
    CatalogStep(
        step_id=LOAD_STEP_ID,
        name="Span load",
        description=(
            "Integrate the declared span load into a surrogate root bending moment and "
            "enforce the declared maximum moment."
        ),
        input_names=_LOAD_INPUTS,
        metric_names=("load.span_load", "load.root_bending_moment"),
    ),
    CatalogStep(
        step_id=AERODYNAMIC_STEP_ID,
        name="Aerodynamic surrogate",
        description=(
            "Report the finite-wing lift and induced drag surrogates and enforce the "
            "declared reference mass bound. Results are not certified."
        ),
        input_names=_AERODYNAMIC_INPUTS,
        metric_names=(
            "aero.lift_curve_slope",
            "aero.induced_drag_factor",
            "aero.quarter_chord_sweep_magnitude",
            "mass.reference",
            "score.geometry",
        ),
        checkpoint_kind=ApprovalKind.PRE_COMMIT,
    ),
)

WING_PACKAGE: Final[DomainPackage] = DomainPackage(
    package_id=PACKAGE_ID,
    version=PACKAGE_VERSION,
    description=PACKAGE_DESCRIPTION,
    disclaimer=PACKAGE_DISCLAIMER,
    workflow=WING_WORKFLOW,
    steps=WING_STEPS,
    evaluators=(
        PlanformGeometryEvaluator(),
        SpanLoadEvaluator(),
        AerodynamicSurrogateEvaluator(),
    ),
    schema_digests=(WING_SCHEMA_DIGEST,),
)

REFERENCE_PARAMETERS: Final[WingParameters] = WingParameters()
REFERENCE_WORKFLOW_STEP_IDS: Final[tuple[str, ...]] = WING_PACKAGE.step_ids


def evaluate_wing(
    step_id: str,
    context: EvaluationContext,
    parameters: WingParameters,
) -> EvaluationResult:
    """Run one declared wing catalog step against typed parameters."""
    return WING_PACKAGE.evaluate(step_id, context, parameters)


def run_wing_reference_workflow(
    parameters: WingParameters | None = None,
    context: EvaluationContext | None = None,
) -> ReferenceWorkflowRun:
    """Run the wing reference workflow deterministically."""
    return run_reference_workflow(
        WING_PACKAGE,
        reference_context(REFERENCE_SUBJECT_ID) if context is None else context,
        REFERENCE_PARAMETERS if parameters is None else parameters,
        REFERENCE_WORKFLOW_STEP_IDS,
    )


__all__ = (
    "PACKAGE_DESCRIPTION",
    "PACKAGE_DISCLAIMER",
    "PACKAGE_ID",
    "PACKAGE_VERSION",
    "REFERENCE_PARAMETERS",
    "REFERENCE_SUBJECT_ID",
    "REFERENCE_WORKFLOW_STEP_IDS",
    "WING_PACKAGE",
    "WING_STEPS",
    "WING_WORKFLOW",
    "WORKFLOW_DEFINITION_ID",
    "WORKFLOW_NAME",
    "WORKFLOW_VERSION",
    "evaluate_wing",
    "run_wing_reference_workflow",
)
