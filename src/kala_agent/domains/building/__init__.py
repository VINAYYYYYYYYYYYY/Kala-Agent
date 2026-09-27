"""Building domain package: storey, area, and thermal envelope evaluation.

Scope of this package, stated plainly: it derives a storey stack, floor and
envelope areas, and a transmission-only heating surrogate, and it measures them
against declared placeholder bounds. Its only declared code reference is marked
non-authoritative. It is not a building code, it determines no compliance or
safety approval, and it is not an energy model. Real design sign-off belongs to a
competent engineer using a validated tool and the governing code.
"""

from __future__ import annotations

from kala_agent.domains.building.evaluation import (
    AREA_STEP_ID,
    STOREY_STEP_ID,
    THERMAL_STEP_ID,
    AreaModelEvaluator,
    ConstraintViolation,
    StoreyModelEvaluator,
    ThermalEnvelopeEvaluator,
    placeholder_code_metrics,
    violation_finding,
    violations_for,
)
from kala_agent.domains.building.parameters import (
    BUILDING_SCHEMA_DIGEST,
    CODE_AUTHORITY_FINDING,
    NON_AUTHORITATIVE_CODE_VERSION,
    PARAMETER_SPECS,
    THERMAL_SURROGATE_FINDING,
    BuildingCodeVersion,
    BuildingParameters,
)
from kala_agent.domains.building.simulation import (
    BuildingSimulation,
    simulate_building,
)
from kala_agent.domains.building.workflow import (
    BUILDING_PACKAGE,
    BUILDING_STEPS,
    BUILDING_WORKFLOW,
    PACKAGE_DESCRIPTION,
    PACKAGE_DISCLAIMER,
    PACKAGE_ID,
    PACKAGE_VERSION,
    REFERENCE_PARAMETERS,
    REFERENCE_SUBJECT_ID,
    REFERENCE_WORKFLOW_STEP_IDS,
    WORKFLOW_DEFINITION_ID,
    WORKFLOW_VERSION,
    evaluate_building,
    run_building_reference_workflow,
)

__all__ = (
    "AREA_STEP_ID",
    "BUILDING_PACKAGE",
    "BUILDING_SCHEMA_DIGEST",
    "BUILDING_STEPS",
    "BUILDING_WORKFLOW",
    "CODE_AUTHORITY_FINDING",
    "NON_AUTHORITATIVE_CODE_VERSION",
    "PACKAGE_DESCRIPTION",
    "PACKAGE_DISCLAIMER",
    "PACKAGE_ID",
    "PACKAGE_VERSION",
    "PARAMETER_SPECS",
    "REFERENCE_PARAMETERS",
    "REFERENCE_SUBJECT_ID",
    "REFERENCE_WORKFLOW_STEP_IDS",
    "STOREY_STEP_ID",
    "THERMAL_STEP_ID",
    "THERMAL_SURROGATE_FINDING",
    "WORKFLOW_DEFINITION_ID",
    "WORKFLOW_VERSION",
    "AreaModelEvaluator",
    "BuildingCodeVersion",
    "BuildingParameters",
    "BuildingSimulation",
    "ConstraintViolation",
    "StoreyModelEvaluator",
    "ThermalEnvelopeEvaluator",
    "evaluate_building",
    "placeholder_code_metrics",
    "run_building_reference_workflow",
    "simulate_building",
    "violation_finding",
    "violations_for",
)
