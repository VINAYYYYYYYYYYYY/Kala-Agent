"""Bounded evaluators for the wing domain package.

Each evaluator is a pure function of a typed parameter model: a fixed number of
arithmetic operations over a bounded station loop, no clock, no I/O, and no
randomness. Every result carries the non-certified marker, so a geometric or
aerodynamic surrogate cannot be read as an airworthiness, stability, or
structural conclusion.
"""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from enum import StrEnum
from types import MappingProxyType
from typing import Final

from kala_agent.contracts import EvaluationResult
from kala_agent.domains import EvaluationContext
from kala_agent.domains.wing.parameters import (
    ANGLE_UNIT,
    AREA_UNIT,
    LENGTH_UNIT,
    MASS_UNIT,
    NON_CERTIFIED_FINDING,
    PER_RADIAN_UNIT,
    ROOT_MOMENT_UNIT,
    SPAN_LOAD_UNIT,
    STATION_UNIT,
    TAPER_FINDING,
    VOLUME_UNIT,
    WingParameters,
)
from kala_agent.domains.wing.simulation import simulate_wing

PLANFORM_STEP_ID: Final[str] = "wing.planform-geometry"
LOAD_STEP_ID: Final[str] = "wing.span-load"
AERODYNAMIC_STEP_ID: Final[str] = "wing.aerodynamic-surrogate"


class ConstraintViolation(StrEnum):
    """Hard constraint identifiers declared by the wing package."""

    ASPECT_RATIO_ABOVE_MAXIMUM = "aspect_ratio_above_maximum"
    ROOT_BENDING_MOMENT_ABOVE_MAXIMUM = "root_bending_moment_above_maximum"
    MASS_ABOVE_MAXIMUM = "mass_above_maximum"


_VIOLATION_FINDINGS: Final[Mapping[ConstraintViolation, str]] = MappingProxyType(
    {
        ConstraintViolation.ASPECT_RATIO_ABOVE_MAXIMUM: (
            "Hard constraint failed: aspect_ratio_above_maximum; the reference aspect ratio is "
            "above the declared maximum."
        ),
        ConstraintViolation.ROOT_BENDING_MOMENT_ABOVE_MAXIMUM: (
            "Hard constraint failed: root_bending_moment_above_maximum; the surrogate root "
            "bending moment is above the declared maximum."
        ),
        ConstraintViolation.MASS_ABOVE_MAXIMUM: (
            "Hard constraint failed: mass_above_maximum; the reference mass is above the "
            "declared maximum."
        ),
    }
)


def violation_finding(violation: ConstraintViolation) -> str:
    """Return the finding text reported for a hard constraint violation."""
    return _VIOLATION_FINDINGS[violation]


def violations_for(parameters: WingParameters) -> tuple[ConstraintViolation, ...]:
    """Return every hard constraint the given parameters violate, in order."""
    simulation = simulate_wing(parameters)
    violations: list[ConstraintViolation] = []
    if simulation.aspect_ratio > parameters.maximum_aspect_ratio:
        violations.append(ConstraintViolation.ASPECT_RATIO_ABOVE_MAXIMUM)
    if simulation.root_bending_moment_kn_m > parameters.maximum_root_bending_moment_kn_m:
        violations.append(ConstraintViolation.ROOT_BENDING_MOMENT_ABOVE_MAXIMUM)
    if simulation.mass_kg > parameters.maximum_mass_kg:
        violations.append(ConstraintViolation.MASS_ABOVE_MAXIMUM)
    return tuple(violations)


@dataclass(frozen=True, slots=True)
class PlanformGeometryEvaluator:
    """Span, chord, taper, and reference aspect ratio against the declared bound.

    Signed planform angles are reported as magnitudes because contract metrics
    carry non-negative values; the signed values remain in the parameter model
    and in the simulation record.
    """

    step_id: str = PLANFORM_STEP_ID
    required_capabilities: tuple[str, ...] = ()

    def evaluate(
        self,
        context: EvaluationContext,
        parameters: WingParameters,
    ) -> EvaluationResult:
        """Evaluate the reference planform geometry and aspect ratio bound."""
        simulation = simulate_wing(parameters)
        aspect_ok = simulation.aspect_ratio <= parameters.maximum_aspect_ratio
        metrics = (
            context.metric("planform.span", parameters.span_m, LENGTH_UNIT),
            context.metric("planform.root_chord", parameters.root_chord_m, LENGTH_UNIT),
            context.metric("planform.tip_chord", parameters.tip_chord_m, LENGTH_UNIT),
            context.metric("planform.taper_ratio", simulation.taper_ratio, None),
            context.metric("planform.mean_chord", simulation.mean_chord_m, LENGTH_UNIT),
            context.metric("planform.reference_area", simulation.reference_area_m2, AREA_UNIT),
            context.metric(
                "planform.aspect_ratio",
                simulation.aspect_ratio,
                None,
                aspect_ok,
            ),
            context.metric(
                "planform.maximum_aspect_ratio",
                parameters.maximum_aspect_ratio,
                None,
                aspect_ok,
            ),
            context.metric("planform.sweep_magnitude", abs(parameters.sweep_deg), ANGLE_UNIT),
            context.metric(
                "planform.dihedral_magnitude",
                abs(parameters.dihedral_deg),
                ANGLE_UNIT,
            ),
            context.metric(
                "planform.thickness_ratio",
                parameters.thickness_ratio,
                None,
            ),
            context.metric(
                "planform.reference_volume",
                simulation.structural_volume_m3,
                VOLUME_UNIT,
            ),
            context.metric(
                "planform.spanwise_stations",
                float(parameters.spanwise_stations),
                STATION_UNIT,
            ),
        )
        findings: tuple[str, ...] = (NON_CERTIFIED_FINDING, TAPER_FINDING)
        if not aspect_ok:
            findings = (
                *findings,
                violation_finding(ConstraintViolation.ASPECT_RATIO_ABOVE_MAXIMUM),
            )
        return context.result(
            self.step_id,
            passed=aspect_ok,
            score=None,
            metrics=metrics,
            findings=findings,
        )


@dataclass(frozen=True, slots=True)
class SpanLoadEvaluator:
    """Distributed span load integrated into a surrogate root bending moment."""

    step_id: str = LOAD_STEP_ID
    required_capabilities: tuple[str, ...] = ()

    def evaluate(
        self,
        context: EvaluationContext,
        parameters: WingParameters,
    ) -> EvaluationResult:
        """Evaluate the span load against the declared root moment bound."""
        simulation = simulate_wing(parameters)
        moment_ok = (
            simulation.root_bending_moment_kn_m <= parameters.maximum_root_bending_moment_kn_m
        )
        metrics = (
            context.metric("load.span_load", parameters.span_load_kn_m, SPAN_LOAD_UNIT),
            context.metric("load.span_load_newtons", simulation.span_load_n_m, "N/m"),
            context.metric(
                "load.root_bending_moment",
                simulation.root_bending_moment_kn_m,
                ROOT_MOMENT_UNIT,
                moment_ok,
            ),
            context.metric(
                "load.maximum_root_bending_moment",
                parameters.maximum_root_bending_moment_kn_m,
                ROOT_MOMENT_UNIT,
                moment_ok,
            ),
        )
        findings: tuple[str, ...] = (NON_CERTIFIED_FINDING,)
        if not moment_ok:
            findings = (
                *findings,
                violation_finding(ConstraintViolation.ROOT_BENDING_MOMENT_ABOVE_MAXIMUM),
            )
        return context.result(
            self.step_id,
            passed=moment_ok,
            score=None,
            metrics=metrics,
            findings=findings,
        )


@dataclass(frozen=True, slots=True)
class AerodynamicSurrogateEvaluator:
    """Finite-wing lift and induced drag surrogates with the reference mass bound."""

    step_id: str = AERODYNAMIC_STEP_ID
    required_capabilities: tuple[str, ...] = ()

    def evaluate(
        self,
        context: EvaluationContext,
        parameters: WingParameters,
    ) -> EvaluationResult:
        """Evaluate the aerodynamic surrogates and the reference mass bound."""
        simulation = simulate_wing(parameters)
        mass_ok = simulation.mass_kg <= parameters.maximum_mass_kg
        metrics = (
            context.metric(
                "aero.lift_curve_slope",
                simulation.lift_curve_slope_per_rad,
                PER_RADIAN_UNIT,
            ),
            context.metric("aero.induced_drag_factor", simulation.induced_drag_factor, None),
            context.metric(
                "aero.induced_drag_coefficient",
                simulation.induced_drag_coefficient,
                None,
            ),
            context.metric(
                "aero.quarter_chord_sweep_magnitude",
                abs(simulation.quarter_chord_sweep_rad),
                "rad",
            ),
            context.metric("aero.dihedral_magnitude", abs(simulation.dihedral_rad), "rad"),
            context.metric("mass.reference", simulation.mass_kg, MASS_UNIT, mass_ok),
            context.metric(
                "mass.maximum",
                parameters.maximum_mass_kg,
                MASS_UNIT,
                mass_ok,
            ),
            context.metric("score.geometry", simulation.geometry_score, None),
        )
        findings: tuple[str, ...] = (NON_CERTIFIED_FINDING,)
        if not mass_ok:
            findings = (*findings, violation_finding(ConstraintViolation.MASS_ABOVE_MAXIMUM))
        return context.result(
            self.step_id,
            passed=mass_ok,
            score=simulation.geometry_score,
            metrics=metrics,
            findings=findings,
        )


__all__ = (
    "AERODYNAMIC_STEP_ID",
    "LOAD_STEP_ID",
    "PLANFORM_STEP_ID",
    "AerodynamicSurrogateEvaluator",
    "ConstraintViolation",
    "PlanformGeometryEvaluator",
    "SpanLoadEvaluator",
    "violation_finding",
    "violations_for",
)
