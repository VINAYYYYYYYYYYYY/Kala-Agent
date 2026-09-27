"""Mechanical domain package: a bracket-like component reference evaluation.

Scope of this package, stated plainly: it parameterises a bracket-like component
by dimensions, material, and load, evaluates bounded closed-form surrogates for
mass, stress, and a safety score, and enforces a minimum thickness and a maximum
stress hard constraint. It does not replace CAD or CAE, it is not finite element
analysis, and it certifies nothing. Real design sign-off belongs to a competent
engineer using validated tools.
"""

from __future__ import annotations

from kala_agent.domains.mechanical.evaluation import (
    GEOMETRY_STEP_ID,
    LOAD_STEP_ID,
    MATERIAL_STEP_ID,
    MAX_LOAD_PER_POINT_KN,
    STRUCTURAL_STEP_ID,
    ConstraintViolation,
    GeometryEvaluator,
    LoadCaseEvaluator,
    MaterialEvaluator,
    StructuralEvaluator,
    violation_finding,
    violations_for,
)
from kala_agent.domains.mechanical.parameters import (
    BRACKET_SCHEMA_DIGEST,
    ILLUSTRATIVE_MATERIAL_STATUS,
    MATERIALS,
    PARAMETER_NAMES,
    PARAMETER_SPECS,
    BracketParameters,
    MaterialKey,
    MaterialReference,
    material_reference,
)
from kala_agent.domains.mechanical.simulation import (
    BracketSimulation,
    simulate_bracket,
)
from kala_agent.domains.mechanical.workflow import (
    MECHANICAL_PACKAGE,
    MECHANICAL_STEPS,
    MECHANICAL_WORKFLOW,
    PACKAGE_DESCRIPTION,
    PACKAGE_DISCLAIMER,
    PACKAGE_ID,
    PACKAGE_VERSION,
    REFERENCE_PARAMETERS,
    REFERENCE_SUBJECT_ID,
    REFERENCE_WORKFLOW_STEP_IDS,
    WORKFLOW_DEFINITION_ID,
    WORKFLOW_VERSION,
    evaluate_bracket,
    run_bracket_reference_workflow,
)

__all__ = (
    "BRACKET_SCHEMA_DIGEST",
    "GEOMETRY_STEP_ID",
    "ILLUSTRATIVE_MATERIAL_STATUS",
    "LOAD_STEP_ID",
    "MATERIALS",
    "MATERIAL_STEP_ID",
    "MAX_LOAD_PER_POINT_KN",
    "MECHANICAL_PACKAGE",
    "MECHANICAL_STEPS",
    "MECHANICAL_WORKFLOW",
    "PACKAGE_DESCRIPTION",
    "PACKAGE_DISCLAIMER",
    "PACKAGE_ID",
    "PACKAGE_VERSION",
    "PARAMETER_NAMES",
    "PARAMETER_SPECS",
    "REFERENCE_PARAMETERS",
    "REFERENCE_SUBJECT_ID",
    "REFERENCE_WORKFLOW_STEP_IDS",
    "STRUCTURAL_STEP_ID",
    "WORKFLOW_DEFINITION_ID",
    "WORKFLOW_VERSION",
    "BracketParameters",
    "BracketSimulation",
    "ConstraintViolation",
    "GeometryEvaluator",
    "LoadCaseEvaluator",
    "MaterialEvaluator",
    "MaterialKey",
    "MaterialReference",
    "StructuralEvaluator",
    "evaluate_bracket",
    "material_reference",
    "run_bracket_reference_workflow",
    "simulate_bracket",
    "violation_finding",
    "violations_for",
)
