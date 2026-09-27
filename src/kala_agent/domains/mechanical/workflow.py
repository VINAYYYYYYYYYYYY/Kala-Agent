"""Workflow definition, catalog steps, and reference run for the mechanical package.

The package is the first end-to-end exercise of the shared engine: it declares a
five-state workflow, one catalog step and one bounded evaluator per transition, a
parameter schema digest, a pre-commit checkpoint marker on the final assessment,
and a deterministic reference run. Running the reference workflow here does not
touch the shared workflow engine; a full run through that engine uses the same
declarations unchanged.
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
from kala_agent.domains.mechanical.evaluation import (
    GEOMETRY_STEP_ID,
    LOAD_STEP_ID,
    MATERIAL_STEP_ID,
    STRUCTURAL_STEP_ID,
    GeometryEvaluator,
    LoadCaseEvaluator,
    MaterialEvaluator,
    StructuralEvaluator,
)
from kala_agent.domains.mechanical.parameters import BRACKET_SCHEMA_DIGEST, BracketParameters

PACKAGE_ID: Final[str] = "mechanical.bracket"
PACKAGE_VERSION: Final[str] = "0.1.0"
PACKAGE_DESCRIPTION: Final[str] = (
    "Mechanical reference package for a bracket-like component: dimensions, material, "
    "and a static load case evaluated by bounded closed-form surrogates."
)
PACKAGE_DISCLAIMER: Final[str] = (
    "This package performs arithmetic surrogates only. It is not CAD or CAE, not finite "
    "element analysis, not a fastener or tolerance analysis, and not a certification of "
    "fitness for any purpose."
)
WORKFLOW_DEFINITION_ID: Final[str] = "mechanical.bracket-evaluation"
WORKFLOW_VERSION: Final[str] = "0.1.0"
WORKFLOW_NAME: Final[str] = "Bracket-like component reference evaluation"
REFERENCE_SUBJECT_ID: Final[str] = "bracket-reference"

_SPECIMEN_INPUTS: Final[tuple[str, ...]] = (
    "length_mm",
    "width_mm",
    "thickness_mm",
    "hole_diameter_mm",
    "hole_count",
    "material",
)
_LOAD_INPUTS: Final[tuple[str, ...]] = ("load_kn", "load_points")
_CONSTRAINT_INPUTS: Final[tuple[str, ...]] = (
    "minimum_thickness_mm",
    "maximum_stress_mpa",
    "target_safety_factor",
)

MECHANICAL_WORKFLOW: Final[WorkflowDefinition] = WorkflowDefinition(
    definition_id=WORKFLOW_DEFINITION_ID,
    version=WORKFLOW_VERSION,
    name=WORKFLOW_NAME,
    description=PACKAGE_DISCLAIMER,
    initial_state="specified",
    states=(
        "specified",
        "sectioned",
        "material-graded",
        "load-cased",
        "assessed",
    ),
    transitions=(
        WorkflowTransition(
            transition_id="mechanical.section",
            from_state="specified",
            to_state="sectioned",
            step_id=GEOMETRY_STEP_ID,
        ),
        WorkflowTransition(
            transition_id="mechanical.grade",
            from_state="sectioned",
            to_state="material-graded",
            step_id=MATERIAL_STEP_ID,
        ),
        WorkflowTransition(
            transition_id="mechanical.load",
            from_state="material-graded",
            to_state="load-cased",
            step_id=LOAD_STEP_ID,
        ),
        WorkflowTransition(
            transition_id="mechanical.assess",
            from_state="load-cased",
            to_state="assessed",
            step_id=STRUCTURAL_STEP_ID,
            checkpoint_type=ApprovalKind.PRE_COMMIT,
        ),
    ),
    available_step_ids=(
        GEOMETRY_STEP_ID,
        MATERIAL_STEP_ID,
        LOAD_STEP_ID,
        STRUCTURAL_STEP_ID,
    ),
    max_replanning_attempts=1,
)

MECHANICAL_STEPS: Final[tuple[CatalogStep, ...]] = (
    CatalogStep(
        step_id=GEOMETRY_STEP_ID,
        name="Plate section check",
        description=(
            "Compute the declared plate section and enforce the minimum thickness constraint."
        ),
        input_names=_SPECIMEN_INPUTS,
        metric_names=(
            "plate.length",
            "plate.width",
            "plate.thickness",
            "plate.minimum_thickness",
            "section.net_area",
            "section.volume",
            "mass",
        ),
    ),
    CatalogStep(
        step_id=MATERIAL_STEP_ID,
        name="Declared material check",
        description=(
            "Report the declared material constants and enforce the hole pattern edge "
            "margin constraint."
        ),
        input_names=("material", "hole_count", "hole_diameter_mm", "minimum_edge_margin_mm"),
        metric_names=("material.density", "material.yield_strength", "pattern.edge_margin"),
    ),
    CatalogStep(
        step_id=LOAD_STEP_ID,
        name="Load case check",
        description="Report the applied load and enforce the per-point load bound.",
        input_names=_LOAD_INPUTS,
        metric_names=("load.applied", "load.per_point", "load.bending_moment"),
    ),
    CatalogStep(
        step_id=STRUCTURAL_STEP_ID,
        name="Surrogate stress and constraint evaluation",
        description=(
            "Evaluate the surrogate stress score and enforce the maximum stress and "
            "minimum thickness hard constraints."
        ),
        input_names=_CONSTRAINT_INPUTS,
        metric_names=("stress.surrogate", "margin.safety_factor", "score.safety"),
        checkpoint_kind=ApprovalKind.PRE_COMMIT,
    ),
)

MECHANICAL_PACKAGE: Final[DomainPackage] = DomainPackage(
    package_id=PACKAGE_ID,
    version=PACKAGE_VERSION,
    description=PACKAGE_DESCRIPTION,
    disclaimer=PACKAGE_DISCLAIMER,
    workflow=MECHANICAL_WORKFLOW,
    steps=MECHANICAL_STEPS,
    evaluators=(
        GeometryEvaluator(),
        MaterialEvaluator(),
        LoadCaseEvaluator(),
        StructuralEvaluator(),
    ),
    schema_digests=(BRACKET_SCHEMA_DIGEST,),
)

REFERENCE_PARAMETERS: Final[BracketParameters] = BracketParameters()
REFERENCE_WORKFLOW_STEP_IDS: Final[tuple[str, ...]] = MECHANICAL_PACKAGE.step_ids


def evaluate_bracket(
    step_id: str,
    context: EvaluationContext,
    parameters: BracketParameters,
) -> EvaluationResult:
    """Run one declared mechanical catalog step against typed parameters."""
    return MECHANICAL_PACKAGE.evaluate(step_id, context, parameters)


def run_bracket_reference_workflow(
    parameters: BracketParameters | None = None,
    context: EvaluationContext | None = None,
) -> ReferenceWorkflowRun:
    """Run the simple mechanical reference workflow deterministically."""
    return run_reference_workflow(
        MECHANICAL_PACKAGE,
        reference_context(REFERENCE_SUBJECT_ID) if context is None else context,
        REFERENCE_PARAMETERS if parameters is None else parameters,
        REFERENCE_WORKFLOW_STEP_IDS,
    )


__all__ = (
    "MECHANICAL_PACKAGE",
    "MECHANICAL_STEPS",
    "MECHANICAL_WORKFLOW",
    "PACKAGE_DESCRIPTION",
    "PACKAGE_DISCLAIMER",
    "PACKAGE_ID",
    "PACKAGE_VERSION",
    "REFERENCE_PARAMETERS",
    "REFERENCE_SUBJECT_ID",
    "REFERENCE_WORKFLOW_STEP_IDS",
    "WORKFLOW_DEFINITION_ID",
    "WORKFLOW_NAME",
    "WORKFLOW_VERSION",
    "evaluate_bracket",
    "run_bracket_reference_workflow",
)
