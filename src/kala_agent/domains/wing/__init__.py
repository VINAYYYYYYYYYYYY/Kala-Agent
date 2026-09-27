"""Wing domain package: span, chord, taper, and span load evaluation.

Scope of this package, stated plainly: it parameterises a tapered wing by span,
chord, sweep, dihedral, and a distributed span load, and it evaluates geometric
and aerodynamic surrogates. Those surrogates are not certified: they are not a
lifting-line or panel method, not CFD, and not a structural or fatigue analysis.
Nothing here establishes airworthiness, stability, or control behaviour, and it
does not replace validated aerodynamic and structural tools.
"""

from __future__ import annotations

from kala_agent.domains.wing.evaluation import (
    AERODYNAMIC_STEP_ID,
    LOAD_STEP_ID,
    PLANFORM_STEP_ID,
    AerodynamicSurrogateEvaluator,
    ConstraintViolation,
    PlanformGeometryEvaluator,
    SpanLoadEvaluator,
    violation_finding,
    violations_for,
)
from kala_agent.domains.wing.parameters import (
    NON_CERTIFIED_FINDING,
    PARAMETER_SPECS,
    TAPER_FINDING,
    WING_SCHEMA_DIGEST,
    WingLoadCase,
    WingParameters,
)
from kala_agent.domains.wing.simulation import (
    WingSimulation,
    simulate_wing,
)
from kala_agent.domains.wing.workflow import (
    PACKAGE_DESCRIPTION,
    PACKAGE_DISCLAIMER,
    PACKAGE_ID,
    PACKAGE_VERSION,
    REFERENCE_PARAMETERS,
    REFERENCE_SUBJECT_ID,
    REFERENCE_WORKFLOW_STEP_IDS,
    WING_PACKAGE,
    WING_STEPS,
    WING_WORKFLOW,
    WORKFLOW_DEFINITION_ID,
    WORKFLOW_VERSION,
    evaluate_wing,
    run_wing_reference_workflow,
)

__all__ = (
    "AERODYNAMIC_STEP_ID",
    "LOAD_STEP_ID",
    "NON_CERTIFIED_FINDING",
    "PACKAGE_DESCRIPTION",
    "PACKAGE_DISCLAIMER",
    "PACKAGE_ID",
    "PACKAGE_VERSION",
    "PARAMETER_SPECS",
    "PLANFORM_STEP_ID",
    "REFERENCE_PARAMETERS",
    "REFERENCE_SUBJECT_ID",
    "REFERENCE_WORKFLOW_STEP_IDS",
    "TAPER_FINDING",
    "WING_PACKAGE",
    "WING_SCHEMA_DIGEST",
    "WING_STEPS",
    "WING_WORKFLOW",
    "WORKFLOW_DEFINITION_ID",
    "WORKFLOW_VERSION",
    "AerodynamicSurrogateEvaluator",
    "ConstraintViolation",
    "PlanformGeometryEvaluator",
    "SpanLoadEvaluator",
    "WingLoadCase",
    "WingParameters",
    "WingSimulation",
    "evaluate_wing",
    "run_wing_reference_workflow",
    "simulate_wing",
    "violation_finding",
    "violations_for",
)
