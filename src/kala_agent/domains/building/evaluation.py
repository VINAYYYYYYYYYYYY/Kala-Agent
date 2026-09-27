"""Bounded evaluators for the building domain package.

Each evaluator is a pure function of a typed parameter model: a fixed number of
arithmetic operations, no clock, no I/O, no randomness, and no unbounded loop.
Every result reports the declared code reference as a non-authoritative
placeholder and states that the thermal figures are a transmission-only
surrogate, so no reading of the result can be mistaken for a compliance
determination.
"""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from enum import StrEnum
from types import MappingProxyType
from typing import Final

from kala_agent.contracts import EvaluationMetric, EvaluationResult
from kala_agent.domains import NON_AUTHORITATIVE_FINDING, EvaluationContext
from kala_agent.domains.building.parameters import (
    CODE_AUTHORITY_FINDING,
    CONDUCTIVITY_UNIT,
    CUBIC_METRE_UNIT,
    DEGREE_DAY_UNIT,
    ENERGY_UNIT,
    INTENSITY_UNIT,
    METRE_UNIT,
    SQUARE_METRE_UNIT,
    TEMPERATURE_UNIT,
    THERMAL_SURROGATE_FINDING,
    TRANSMITTANCE_UNIT,
    BuildingParameters,
)
from kala_agent.domains.building.simulation import simulate_building

STOREY_STEP_ID: Final[str] = "building.storey-model"
AREA_STEP_ID: Final[str] = "building.area-model"
THERMAL_STEP_ID: Final[str] = "building.thermal-envelope"


class ConstraintViolation(StrEnum):
    """Hard constraint identifiers declared by the building package."""

    TOTAL_HEIGHT_ABOVE_PLACEHOLDER_BOUND = "total_height_above_placeholder_bound"
    STOREY_AREA_BELOW_PLACEHOLDER_MINIMUM = "storey_area_below_placeholder_minimum"
    HEATING_INTENSITY_ABOVE_PLACEHOLDER_BOUND = "heating_intensity_above_placeholder_bound"
    ENVELOPE_CONDUCTIVITY_ZERO = "envelope_conductivity_zero"


_VIOLATION_FINDINGS: Final[Mapping[ConstraintViolation, str]] = MappingProxyType(
    {
        ConstraintViolation.TOTAL_HEIGHT_ABOVE_PLACEHOLDER_BOUND: (
            "Hard constraint failed: total_height_above_placeholder_bound; the stack height is "
            "above the declared placeholder height bound, which is not a code height limit."
        ),
        ConstraintViolation.STOREY_AREA_BELOW_PLACEHOLDER_MINIMUM: (
            "Hard constraint failed: storey_area_below_placeholder_minimum; the storey area is "
            "below the declared placeholder minimum area."
        ),
        ConstraintViolation.HEATING_INTENSITY_ABOVE_PLACEHOLDER_BOUND: (
            "Hard constraint failed: heating_intensity_above_placeholder_bound; the surrogate "
            "heating intensity is above the declared placeholder bound."
        ),
        ConstraintViolation.ENVELOPE_CONDUCTIVITY_ZERO: (
            "Hard constraint failed: envelope_conductivity_zero; the declared envelope has no "
            "transmittance, so no thermal evaluation is meaningful."
        ),
    }
)


def violation_finding(violation: ConstraintViolation) -> str:
    """Return the finding text reported for a hard constraint violation."""
    return _VIOLATION_FINDINGS[violation]


def violations_for(parameters: BuildingParameters) -> tuple[ConstraintViolation, ...]:
    """Return every hard constraint the given parameters violate, in order."""
    simulation = simulate_building(parameters)
    violations: list[ConstraintViolation] = []
    if simulation.total_height_m > parameters.maximum_height_m:
        violations.append(ConstraintViolation.TOTAL_HEIGHT_ABOVE_PLACEHOLDER_BOUND)
    if simulation.storey_area_m2 < parameters.minimum_storey_area_m2:
        violations.append(ConstraintViolation.STOREY_AREA_BELOW_PLACEHOLDER_MINIMUM)
    if simulation.heating_intensity_kwh_m2 > parameters.maximum_heating_intensity_kwh_m2:
        violations.append(ConstraintViolation.HEATING_INTENSITY_ABOVE_PLACEHOLDER_BOUND)
    if simulation.conductance_total_w_k <= 0.0:
        violations.append(ConstraintViolation.ENVELOPE_CONDUCTIVITY_ZERO)
    return tuple(violations)


def placeholder_code_metrics(context: EvaluationContext) -> tuple[EvaluationMetric, ...]:
    """Return metrics that keep the non-authoritative placeholder visible."""
    return (
        context.metric("code.version_is_placeholder", 1.0, None, True),
        context.metric("code.compliance_determined", 0.0, None, False),
    )


@dataclass(frozen=True, slots=True)
class StoreyModelEvaluator:
    """Storey stack geometry evaluated against the placeholder height bound."""

    step_id: str = STOREY_STEP_ID
    required_capabilities: tuple[str, ...] = ()

    def evaluate(
        self,
        context: EvaluationContext,
        parameters: BuildingParameters,
    ) -> EvaluationResult:
        """Evaluate the storey stack and the declared height bound."""
        simulation = simulate_building(parameters)
        height_ok = simulation.total_height_m <= parameters.maximum_height_m
        metrics = (
            context.metric("storey.count", float(parameters.storey_count), None),
            context.metric("storey.height", parameters.storey_height_m, METRE_UNIT),
            context.metric(
                "storey.total_height",
                simulation.total_height_m,
                METRE_UNIT,
                height_ok,
            ),
            context.metric(
                "storey.maximum_height",
                parameters.maximum_height_m,
                METRE_UNIT,
                height_ok,
            ),
            context.metric("storey.total_volume", simulation.total_volume_m3, CUBIC_METRE_UNIT),
            *placeholder_code_metrics(context),
        )
        findings: tuple[str, ...] = (NON_AUTHORITATIVE_FINDING, CODE_AUTHORITY_FINDING)
        if not height_ok:
            findings = (
                *findings,
                violation_finding(ConstraintViolation.TOTAL_HEIGHT_ABOVE_PLACEHOLDER_BOUND),
            )
        return context.result(
            self.step_id,
            passed=height_ok,
            score=None,
            metrics=metrics,
            findings=findings,
        )


@dataclass(frozen=True, slots=True)
class AreaModelEvaluator:
    """Storey and envelope areas evaluated against the placeholder minimum."""

    step_id: str = AREA_STEP_ID
    required_capabilities: tuple[str, ...] = ()

    def evaluate(
        self,
        context: EvaluationContext,
        parameters: BuildingParameters,
    ) -> EvaluationResult:
        """Evaluate storey and envelope areas against the declared minimum."""
        simulation = simulate_building(parameters)
        area_ok = simulation.storey_area_m2 >= parameters.minimum_storey_area_m2
        metrics = (
            context.metric(
                "area.storey_gross",
                simulation.storey_area_m2,
                SQUARE_METRE_UNIT,
                area_ok,
            ),
            context.metric(
                "area.minimum_storey",
                parameters.minimum_storey_area_m2,
                SQUARE_METRE_UNIT,
                area_ok,
            ),
            context.metric("area.total_floor", simulation.total_floor_area_m2, SQUARE_METRE_UNIT),
            context.metric("area.wall", simulation.wall_area_m2, SQUARE_METRE_UNIT),
            context.metric("area.opaque_wall", simulation.opaque_wall_area_m2, SQUARE_METRE_UNIT),
            context.metric("area.window", simulation.window_area_m2, SQUARE_METRE_UNIT),
            context.metric("area.roof", simulation.roof_area_m2, SQUARE_METRE_UNIT),
            context.metric(
                "area.window_ratio",
                parameters.window_to_wall_ratio,
                None,
            ),
            *placeholder_code_metrics(context),
        )
        findings: tuple[str, ...] = (NON_AUTHORITATIVE_FINDING, CODE_AUTHORITY_FINDING)
        if not area_ok:
            findings = (
                *findings,
                violation_finding(ConstraintViolation.STOREY_AREA_BELOW_PLACEHOLDER_MINIMUM),
            )
        return context.result(
            self.step_id,
            passed=area_ok,
            score=None,
            metrics=metrics,
            findings=findings,
        )


@dataclass(frozen=True, slots=True)
class ThermalEnvelopeEvaluator:
    """Transmission-only envelope evaluation against the placeholder intensity bound."""

    step_id: str = THERMAL_STEP_ID
    required_capabilities: tuple[str, ...] = ()

    def evaluate(
        self,
        context: EvaluationContext,
        parameters: BuildingParameters,
    ) -> EvaluationResult:
        """Evaluate the surrogate heating intensity and envelope score."""
        simulation = simulate_building(parameters)
        intensity_ok = (
            simulation.heating_intensity_kwh_m2 <= parameters.maximum_heating_intensity_kwh_m2
        )
        conductance_ok = simulation.conductance_total_w_k > 0.0
        metrics = (
            context.metric(
                "envelope.conductance_wall",
                simulation.conductance_wall_w_k,
                TRANSMITTANCE_UNIT,
            ),
            context.metric(
                "envelope.conductance_window",
                simulation.conductance_window_w_k,
                TRANSMITTANCE_UNIT,
            ),
            context.metric(
                "envelope.conductance_roof",
                simulation.conductance_roof_w_k,
                TRANSMITTANCE_UNIT,
            ),
            context.metric(
                "envelope.conductance_total",
                simulation.conductance_total_w_k,
                TRANSMITTANCE_UNIT,
                conductance_ok,
            ),
            context.metric(
                "envelope.conductance_per_floor_area",
                simulation.conductance_per_floor_area_w_m2_k,
                CONDUCTIVITY_UNIT,
            ),
            context.metric(
                "envelope.designed_wall_conductivity",
                parameters.wall_conductivity_w_m2k,
                CONDUCTIVITY_UNIT,
            ),
            context.metric(
                "envelope.temperature_difference",
                simulation.temperature_difference_k,
                TEMPERATURE_UNIT,
            ),
            context.metric(
                "envelope.degree_days",
                parameters.heating_degree_days_k,
                DEGREE_DAY_UNIT,
            ),
            context.metric(
                "envelope.annual_heating",
                simulation.annual_heating_kwh,
                ENERGY_UNIT,
            ),
            context.metric(
                "envelope.heating_intensity",
                simulation.heating_intensity_kwh_m2,
                INTENSITY_UNIT,
                intensity_ok,
            ),
            context.metric(
                "envelope.maximum_heating_intensity",
                parameters.maximum_heating_intensity_kwh_m2,
                INTENSITY_UNIT,
                intensity_ok,
            ),
            context.metric("envelope.score", simulation.envelope_score, None),
            *placeholder_code_metrics(context),
        )
        findings: tuple[str, ...] = (
            NON_AUTHORITATIVE_FINDING,
            CODE_AUTHORITY_FINDING,
            THERMAL_SURROGATE_FINDING,
        )
        if not intensity_ok:
            findings = (
                *findings,
                violation_finding(ConstraintViolation.HEATING_INTENSITY_ABOVE_PLACEHOLDER_BOUND),
            )
        if not conductance_ok:
            findings = (
                *findings,
                violation_finding(ConstraintViolation.ENVELOPE_CONDUCTIVITY_ZERO),
            )
        return context.result(
            self.step_id,
            passed=intensity_ok and conductance_ok,
            score=simulation.envelope_score,
            metrics=metrics,
            findings=findings,
        )


__all__ = (
    "AREA_STEP_ID",
    "STOREY_STEP_ID",
    "THERMAL_STEP_ID",
    "AreaModelEvaluator",
    "ConstraintViolation",
    "StoreyModelEvaluator",
    "ThermalEnvelopeEvaluator",
    "placeholder_code_metrics",
    "violation_finding",
    "violations_for",
)
