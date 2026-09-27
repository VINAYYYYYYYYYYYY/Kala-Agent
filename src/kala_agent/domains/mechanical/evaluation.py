"""Bounded evaluators for the mechanical domain package.

Each evaluator is a pure function of a typed parameter model: a fixed number of
arithmetic operations, no clock, no I/O, no randomness, and no unbounded loop.
Every evaluator returns a contract :class:`EvaluationResult` whose ``passed``
flag is decided only by the package's declared hard constraints, and whose
findings always state that the result is a non-authoritative surrogate.
"""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from enum import StrEnum
from types import MappingProxyType
from typing import Final

from kala_agent.contracts import EvaluationResult
from kala_agent.domains import NON_AUTHORITATIVE_FINDING, EvaluationContext
from kala_agent.domains.mechanical.parameters import (
    AREA_UNIT,
    DENSITY_UNIT,
    EXPANSION_UNIT,
    FORCE_UNIT,
    LENGTH_UNIT,
    MASS_UNIT,
    MATERIALS,
    MODULUS_UNIT,
    MOMENT_UNIT,
    STRESS_UNIT,
    VOLUME_UNIT,
    BracketParameters,
    material_reference,
)
from kala_agent.domains.mechanical.simulation import simulate_bracket

GEOMETRY_STEP_ID: Final[str] = "mechanical.geometry-check"
MATERIAL_STEP_ID: Final[str] = "mechanical.material-check"
LOAD_STEP_ID: Final[str] = "mechanical.load-check"
STRUCTURAL_STEP_ID: Final[str] = "mechanical.constraint-evaluation"

MAX_LOAD_PER_POINT_KN: Final[float] = 150.0
MATERIAL_REFERENCE_FINDING: Final[str] = (
    "Material constants are illustrative placeholders with a declared reference status; "
    "they are not certified material data."
)
SAFETY_FACTOR_FINDING: Final[str] = (
    "Surrogate safety factor is below the declared target; this is a margin warning, "
    "not a hard constraint failure."
)


class ConstraintViolation(StrEnum):
    """Hard constraint identifiers declared by the mechanical package."""

    THICKNESS_BELOW_MINIMUM = "thickness_below_minimum"
    STRESS_ABOVE_MAXIMUM = "stress_above_maximum"
    EDGE_MARGIN_BELOW_MINIMUM = "edge_margin_below_minimum"
    LOAD_PER_POINT_ABOVE_BOUND = "load_per_point_above_bound"


_VIOLATION_FINDINGS: Final[Mapping[ConstraintViolation, str]] = MappingProxyType(
    {
        ConstraintViolation.THICKNESS_BELOW_MINIMUM: (
            "Hard constraint failed: thickness_below_minimum; the plate thickness is below the "
            "declared minimum thickness."
        ),
        ConstraintViolation.STRESS_ABOVE_MAXIMUM: (
            "Hard constraint failed: stress_above_maximum; the surrogate stress is above the "
            "declared maximum stress."
        ),
        ConstraintViolation.EDGE_MARGIN_BELOW_MINIMUM: (
            "Hard constraint failed: edge_margin_below_minimum; the hole pattern leaves less "
            "material beside it than the declared minimum edge margin."
        ),
        ConstraintViolation.LOAD_PER_POINT_ABOVE_BOUND: (
            "Hard constraint failed: load_per_point_above_bound; the load per introduction point "
            "is above the declared package bound."
        ),
    }
)


def violation_finding(violation: ConstraintViolation) -> str:
    """Return the finding text reported for a hard constraint violation."""
    return _VIOLATION_FINDINGS[violation]


def violations_for(parameters: BracketParameters) -> tuple[ConstraintViolation, ...]:
    """Return every hard constraint the given parameters violate, in order."""
    simulation = simulate_bracket(parameters)
    violations: list[ConstraintViolation] = []
    if parameters.thickness_mm < parameters.minimum_thickness_mm:
        violations.append(ConstraintViolation.THICKNESS_BELOW_MINIMUM)
    if simulation.stress_mpa > parameters.maximum_stress_mpa:
        violations.append(ConstraintViolation.STRESS_ABOVE_MAXIMUM)
    if simulation.edge_margin_mm < parameters.minimum_edge_margin_mm:
        violations.append(ConstraintViolation.EDGE_MARGIN_BELOW_MINIMUM)
    if simulation.load_per_point_n > MAX_LOAD_PER_POINT_KN * 1_000.0:
        violations.append(ConstraintViolation.LOAD_PER_POINT_ABOVE_BOUND)
    return tuple(violations)


@dataclass(frozen=True, slots=True)
class GeometryEvaluator:
    """Plate section and minimum-thickness evaluation."""

    step_id: str = GEOMETRY_STEP_ID
    required_capabilities: tuple[str, ...] = ()

    def evaluate(
        self,
        context: EvaluationContext,
        parameters: BracketParameters,
    ) -> EvaluationResult:
        """Evaluate the declared plate section against the thickness constraint."""
        simulation = simulate_bracket(parameters)
        thickness_ok = parameters.thickness_mm >= parameters.minimum_thickness_mm
        metrics = (
            context.metric("plate.length", parameters.length_mm, LENGTH_UNIT),
            context.metric("plate.width", parameters.width_mm, LENGTH_UNIT),
            context.metric("plate.thickness", parameters.thickness_mm, LENGTH_UNIT, thickness_ok),
            context.metric("plate.minimum_thickness", parameters.minimum_thickness_mm, LENGTH_UNIT),
            context.metric("section.gross_area", simulation.gross_area_mm2, AREA_UNIT),
            context.metric("section.hole_area", simulation.hole_area_mm2, AREA_UNIT),
            context.metric("section.net_area", simulation.net_area_mm2, AREA_UNIT),
            context.metric("section.volume", simulation.volume_mm3, VOLUME_UNIT),
            context.metric("mass", simulation.mass_kg, MASS_UNIT),
        )
        findings: tuple[str, ...] = (NON_AUTHORITATIVE_FINDING,)
        if not thickness_ok:
            findings = (*findings, violation_finding(ConstraintViolation.THICKNESS_BELOW_MINIMUM))
        return context.result(
            self.step_id,
            passed=thickness_ok,
            score=None,
            metrics=metrics,
            findings=findings,
        )


@dataclass(frozen=True, slots=True)
class MaterialEvaluator:
    """Declared material constants and the hole-pattern edge margin."""

    step_id: str = MATERIAL_STEP_ID
    required_capabilities: tuple[str, ...] = ()

    def evaluate(
        self,
        context: EvaluationContext,
        parameters: BracketParameters,
    ) -> EvaluationResult:
        """Evaluate declared material constants and the edge margin constraint."""
        material = material_reference(parameters.material)
        simulation = simulate_bracket(parameters)
        margin_ok = simulation.edge_margin_mm >= parameters.minimum_edge_margin_mm
        metrics = (
            context.metric("material.density", material.density_kg_m3, DENSITY_UNIT),
            context.metric("material.yield_strength", material.yield_strength_mpa, STRESS_UNIT),
            context.metric("material.elastic_modulus", material.elastic_modulus_gpa, MODULUS_UNIT),
            context.metric(
                "material.thermal_expansion",
                material.thermal_expansion_per_kelvin,
                EXPANSION_UNIT,
            ),
            context.metric("material.declared_count", float(len(MATERIALS)), None),
            context.metric("pattern.material_web", simulation.material_web_mm, LENGTH_UNIT),
            context.metric(
                "pattern.edge_margin",
                simulation.edge_margin_mm,
                LENGTH_UNIT,
                margin_ok,
            ),
            context.metric(
                "pattern.minimum_edge_margin",
                parameters.minimum_edge_margin_mm,
                LENGTH_UNIT,
            ),
        )
        findings: tuple[str, ...] = (
            NON_AUTHORITATIVE_FINDING,
            MATERIAL_REFERENCE_FINDING,
        )
        if not margin_ok:
            findings = (
                *findings,
                violation_finding(ConstraintViolation.EDGE_MARGIN_BELOW_MINIMUM),
            )
        return context.result(
            self.step_id,
            passed=margin_ok,
            score=None,
            metrics=metrics,
            findings=findings,
        )


@dataclass(frozen=True, slots=True)
class LoadCaseEvaluator:
    """Applied load and the load introduced at each point."""

    step_id: str = LOAD_STEP_ID
    required_capabilities: tuple[str, ...] = ()

    def evaluate(
        self,
        context: EvaluationContext,
        parameters: BracketParameters,
    ) -> EvaluationResult:
        """Evaluate the applied load against the per-point package bound."""
        simulation = simulate_bracket(parameters)
        load_per_point_ok = simulation.load_per_point_n <= MAX_LOAD_PER_POINT_KN * 1_000.0
        metrics = (
            context.metric("load.applied", parameters.load_kn, "kN"),
            context.metric("load.applied_newtons", simulation.load_n, FORCE_UNIT),
            context.metric("load.points", float(parameters.load_points), None),
            context.metric(
                "load.per_point",
                simulation.load_per_point_n,
                FORCE_UNIT,
                load_per_point_ok,
            ),
            context.metric("load.lever_arm", simulation.lever_arm_mm, LENGTH_UNIT),
            context.metric("load.bending_moment", simulation.bending_moment_n_mm, MOMENT_UNIT),
            context.metric("load.bound_per_point", MAX_LOAD_PER_POINT_KN, "kN"),
        )
        findings: tuple[str, ...] = (NON_AUTHORITATIVE_FINDING,)
        if not load_per_point_ok:
            findings = (
                *findings,
                violation_finding(ConstraintViolation.LOAD_PER_POINT_ABOVE_BOUND),
            )
        return context.result(
            self.step_id,
            passed=load_per_point_ok,
            score=None,
            metrics=metrics,
            findings=findings,
        )


@dataclass(frozen=True, slots=True)
class StructuralEvaluator:
    """Surrogate stress evaluation enforcing the declared hard constraints."""

    step_id: str = STRUCTURAL_STEP_ID
    required_capabilities: tuple[str, ...] = ()

    def evaluate(
        self,
        context: EvaluationContext,
        parameters: BracketParameters,
    ) -> EvaluationResult:
        """Evaluate surrogate stress against the maximum stress constraint."""
        simulation = simulate_bracket(parameters)
        stress_ok = simulation.stress_mpa <= parameters.maximum_stress_mpa
        thickness_ok = parameters.thickness_mm >= parameters.minimum_thickness_mm
        safety_ok = simulation.safety_factor >= parameters.target_safety_factor
        metrics = (
            context.metric("section.section_modulus", simulation.section_modulus_mm3, "mm^3"),
            context.metric("stress.bending", simulation.bending_stress_mpa, STRESS_UNIT),
            context.metric("stress.shear", simulation.shear_stress_mpa, STRESS_UNIT),
            context.metric("stress.surrogate", simulation.stress_mpa, STRESS_UNIT, stress_ok),
            context.metric(
                "stress.maximum",
                parameters.maximum_stress_mpa,
                STRESS_UNIT,
                stress_ok,
            ),
            context.metric("margin.utilisation", simulation.utilisation, None),
            context.metric("margin.safety_factor", simulation.safety_factor, None, safety_ok),
            context.metric(
                "margin.target_safety_factor",
                parameters.target_safety_factor,
                None,
            ),
            context.metric("score.safety", simulation.safety_score, None),
        )
        findings: tuple[str, ...] = (NON_AUTHORITATIVE_FINDING,)
        if not stress_ok:
            findings = (*findings, violation_finding(ConstraintViolation.STRESS_ABOVE_MAXIMUM))
        if not thickness_ok:
            findings = (*findings, violation_finding(ConstraintViolation.THICKNESS_BELOW_MINIMUM))
        if not safety_ok:
            findings = (*findings, SAFETY_FACTOR_FINDING)
        return context.result(
            self.step_id,
            passed=stress_ok and thickness_ok,
            score=simulation.safety_score,
            metrics=metrics,
            findings=findings,
        )


__all__ = (
    "GEOMETRY_STEP_ID",
    "LOAD_STEP_ID",
    "MATERIAL_REFERENCE_FINDING",
    "MATERIAL_STEP_ID",
    "MAX_LOAD_PER_POINT_KN",
    "SAFETY_FACTOR_FINDING",
    "STRUCTURAL_STEP_ID",
    "ConstraintViolation",
    "GeometryEvaluator",
    "LoadCaseEvaluator",
    "MaterialEvaluator",
    "StructuralEvaluator",
    "violation_finding",
    "violations_for",
)
