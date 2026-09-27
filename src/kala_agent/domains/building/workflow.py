"""Workflow definition, catalog steps, and reference run for the building package.

The package declares a four-state storey, area, and thermal envelope evaluation
on the shared workflow vocabulary. It contains no building code: the only
declared code reference is a non-authoritative placeholder, and the workflow
produces a surrogate envelope assessment rather than a compliance decision.
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
from kala_agent.domains.building.evaluation import (
    AREA_STEP_ID,
    STOREY_STEP_ID,
    THERMAL_STEP_ID,
    AreaModelEvaluator,
    StoreyModelEvaluator,
    ThermalEnvelopeEvaluator,
)
from kala_agent.domains.building.parameters import (
    BUILDING_SCHEMA_DIGEST,
    BuildingParameters,
)

PACKAGE_ID: Final[str] = "building.storey-envelope"
PACKAGE_VERSION: Final[str] = "0.1.0"
PACKAGE_DESCRIPTION: Final[str] = (
    "Building reference package for storey, area, and thermal envelope evaluation "
    "using a transmission-only surrogate and a non-authoritative code placeholder."
)
PACKAGE_DISCLAIMER: Final[str] = (
    "This package performs arithmetic surrogates only. It contains no building code, "
    "determines no compliance, no occupancy classification, and no safety approval, and "
    "it is not an energy model."
)
WORKFLOW_DEFINITION_ID: Final[str] = "building.envelope-evaluation"
WORKFLOW_VERSION: Final[str] = "0.1.0"
WORKFLOW_NAME: Final[str] = "Storey and thermal envelope reference evaluation"
REFERENCE_SUBJECT_ID: Final[str] = "building-reference"

_STOREY_INPUTS: Final[tuple[str, ...]] = (
    "storey_count",
    "storey_height_m",
    "maximum_height_m",
)
_AREA_INPUTS: Final[tuple[str, ...]] = (
    "gross_floor_area_m2",
    "perimeter_m",
    "window_to_wall_ratio",
    "minimum_storey_area_m2",
)
_THERMAL_INPUTS: Final[tuple[str, ...]] = (
    "wall_conductivity_w_m2k",
    "window_conductivity_w_m2k",
    "roof_conductivity_w_m2k",
    "heating_degree_days_k",
    "design_indoor_temperature_c",
    "design_outdoor_temperature_c",
    "maximum_heating_intensity_kwh_m2",
    "code_version",
)

BUILDING_WORKFLOW: Final[WorkflowDefinition] = WorkflowDefinition(
    definition_id=WORKFLOW_DEFINITION_ID,
    version=WORKFLOW_VERSION,
    name=WORKFLOW_NAME,
    description=PACKAGE_DISCLAIMER,
    initial_state="specified",
    states=("specified", "storeyed", "planned", "assessed"),
    transitions=(
        WorkflowTransition(
            transition_id="building.storey",
            from_state="specified",
            to_state="storeyed",
            step_id=STOREY_STEP_ID,
        ),
        WorkflowTransition(
            transition_id="building.area",
            from_state="storeyed",
            to_state="planned",
            step_id=AREA_STEP_ID,
        ),
        WorkflowTransition(
            transition_id="building.envelope",
            from_state="planned",
            to_state="assessed",
            step_id=THERMAL_STEP_ID,
            checkpoint_type=ApprovalKind.PRE_COMMIT,
        ),
    ),
    available_step_ids=(STOREY_STEP_ID, AREA_STEP_ID, THERMAL_STEP_ID),
    max_replanning_attempts=1,
)

BUILDING_STEPS: Final[tuple[CatalogStep, ...]] = (
    CatalogStep(
        step_id=STOREY_STEP_ID,
        name="Storey model",
        description=("Build the storey stack and enforce the declared placeholder height bound."),
        input_names=_STOREY_INPUTS,
        metric_names=(
            "storey.count",
            "storey.height",
            "storey.total_height",
            "storey.total_volume",
        ),
    ),
    CatalogStep(
        step_id=AREA_STEP_ID,
        name="Area model",
        description=(
            "Derive storey, floor, and envelope areas and enforce the declared placeholder "
            "minimum storey area."
        ),
        input_names=_AREA_INPUTS,
        metric_names=(
            "area.storey_gross",
            "area.total_floor",
            "area.wall",
            "area.window",
            "area.roof",
        ),
    ),
    CatalogStep(
        step_id=THERMAL_STEP_ID,
        name="Thermal envelope surrogate",
        description=(
            "Compute the transmission-only heating surrogate against the declared "
            "placeholder intensity bound, with the code reference marked non-authoritative."
        ),
        input_names=_THERMAL_INPUTS,
        metric_names=(
            "envelope.conductance_total",
            "envelope.annual_heating",
            "envelope.heating_intensity",
            "envelope.score",
            "code.version_is_placeholder",
        ),
        checkpoint_kind=ApprovalKind.PRE_COMMIT,
    ),
)

BUILDING_PACKAGE: Final[DomainPackage] = DomainPackage(
    package_id=PACKAGE_ID,
    version=PACKAGE_VERSION,
    description=PACKAGE_DESCRIPTION,
    disclaimer=PACKAGE_DISCLAIMER,
    workflow=BUILDING_WORKFLOW,
    steps=BUILDING_STEPS,
    evaluators=(
        StoreyModelEvaluator(),
        AreaModelEvaluator(),
        ThermalEnvelopeEvaluator(),
    ),
    schema_digests=(BUILDING_SCHEMA_DIGEST,),
)

REFERENCE_PARAMETERS: Final[BuildingParameters] = BuildingParameters()
REFERENCE_WORKFLOW_STEP_IDS: Final[tuple[str, ...]] = BUILDING_PACKAGE.step_ids


def evaluate_building(
    step_id: str,
    context: EvaluationContext,
    parameters: BuildingParameters,
) -> EvaluationResult:
    """Run one declared building catalog step against typed parameters."""
    return BUILDING_PACKAGE.evaluate(step_id, context, parameters)


def run_building_reference_workflow(
    parameters: BuildingParameters | None = None,
    context: EvaluationContext | None = None,
) -> ReferenceWorkflowRun:
    """Run the building reference workflow deterministically."""
    return run_reference_workflow(
        BUILDING_PACKAGE,
        reference_context(REFERENCE_SUBJECT_ID) if context is None else context,
        REFERENCE_PARAMETERS if parameters is None else parameters,
        REFERENCE_WORKFLOW_STEP_IDS,
    )


__all__ = (
    "BUILDING_PACKAGE",
    "BUILDING_STEPS",
    "BUILDING_WORKFLOW",
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
    "evaluate_building",
    "run_building_reference_workflow",
)
