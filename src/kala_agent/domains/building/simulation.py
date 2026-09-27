"""Deterministic reference simulation for the building envelope.

The simulation is transmission-only arithmetic: declared areas multiplied by
declared transmittances give a conductance, the conductance multiplied by a
declared temperature difference and by declared heating degree days gives a
heating energy, and that energy divided by floor area gives an intensity. It is a
reference for workflow testing and comparison only. It is not a building energy
model, it is not a code calculation, and it establishes no compliance, safety, or
occupancy conclusion. The bounds it is measured against are declared
placeholders, not code limits.

The same inputs always produce the same outputs: there is no clock, no random
draw, and no iteration to a tolerance.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Final

from kala_agent.domains import clamp_unit_interval
from kala_agent.domains.building.parameters import BuildingParameters

HOURS_PER_DAY: Final[float] = 24.0
WATT_TO_KILOWATT: Final[float] = 1.0e-3
REFERENCE_TRANSMITTANCE_PER_FLOOR_AREA: Final[float] = 0.9
GLAZING_SCORE_RATIO: Final[float] = 0.5
UA_TERM_WEIGHT: Final[float] = 0.45
INTENSITY_TERM_WEIGHT: Final[float] = 0.35
GLAZING_TERM_WEIGHT: Final[float] = 0.20
NO_PENALTY_TERM: Final[float] = 1.0


@dataclass(frozen=True, slots=True)
class BuildingSimulation:
    """Reference quantities produced by :func:`simulate_building`."""

    storey_area_m2: float
    total_floor_area_m2: float
    total_height_m: float
    total_volume_m3: float
    wall_area_m2: float
    window_area_m2: float
    opaque_wall_area_m2: float
    roof_area_m2: float
    conductance_wall_w_k: float
    conductance_window_w_k: float
    conductance_roof_w_k: float
    conductance_total_w_k: float
    conductance_per_floor_area_w_m2_k: float
    temperature_difference_k: float
    transmission_loss_w_k: float
    annual_heating_kwh: float
    heating_intensity_kwh_m2: float
    envelope_score: float


def simulate_building(parameters: BuildingParameters) -> BuildingSimulation:
    """Return deterministic reference quantities for a storey stack."""
    storey_area_m2 = parameters.gross_floor_area_m2
    total_floor_area_m2 = storey_area_m2 * parameters.storey_count
    total_height_m = parameters.storey_height_m * parameters.storey_count
    total_volume_m3 = total_floor_area_m2 * parameters.storey_height_m
    wall_area_m2 = parameters.perimeter_m * parameters.storey_height_m * parameters.storey_count
    window_area_m2 = wall_area_m2 * parameters.window_to_wall_ratio
    opaque_wall_area_m2 = wall_area_m2 - window_area_m2
    roof_area_m2 = storey_area_m2
    conductance_wall_w_k = parameters.wall_conductivity_w_m2k * opaque_wall_area_m2
    conductance_window_w_k = parameters.window_conductivity_w_m2k * window_area_m2
    conductance_roof_w_k = parameters.roof_conductivity_w_m2k * roof_area_m2
    conductance_total_w_k = conductance_wall_w_k + conductance_window_w_k + conductance_roof_w_k
    conductance_per_floor_area_w_m2_k = conductance_total_w_k / total_floor_area_m2
    temperature_difference_k = (
        parameters.design_indoor_temperature_c - parameters.design_outdoor_temperature_c
    )
    transmission_loss_w_k = conductance_total_w_k * temperature_difference_k
    annual_heating_kwh = (
        transmission_loss_w_k * parameters.heating_degree_days_k * HOURS_PER_DAY * WATT_TO_KILOWATT
    )
    heating_intensity_kwh_m2 = annual_heating_kwh / total_floor_area_m2
    envelope_score = _envelope_score(
        parameters,
        conductance_per_floor_area_w_m2_k,
        heating_intensity_kwh_m2,
    )
    return BuildingSimulation(
        storey_area_m2=storey_area_m2,
        total_floor_area_m2=total_floor_area_m2,
        total_height_m=total_height_m,
        total_volume_m3=total_volume_m3,
        wall_area_m2=wall_area_m2,
        window_area_m2=window_area_m2,
        opaque_wall_area_m2=opaque_wall_area_m2,
        roof_area_m2=roof_area_m2,
        conductance_wall_w_k=conductance_wall_w_k,
        conductance_window_w_k=conductance_window_w_k,
        conductance_roof_w_k=conductance_roof_w_k,
        conductance_total_w_k=conductance_total_w_k,
        conductance_per_floor_area_w_m2_k=conductance_per_floor_area_w_m2_k,
        temperature_difference_k=temperature_difference_k,
        transmission_loss_w_k=transmission_loss_w_k,
        annual_heating_kwh=annual_heating_kwh,
        heating_intensity_kwh_m2=heating_intensity_kwh_m2,
        envelope_score=envelope_score,
    )


def _envelope_score(
    parameters: BuildingParameters,
    conductance_per_floor_area_w_m2_k: float,
    heating_intensity_kwh_m2: float,
) -> float:
    """Return a bounded, deterministic envelope score in the unit interval."""
    conductance_term = (
        NO_PENALTY_TERM
        if conductance_per_floor_area_w_m2_k <= 0.0
        else clamp_unit_interval(
            REFERENCE_TRANSMITTANCE_PER_FLOOR_AREA / conductance_per_floor_area_w_m2_k
        )
    )
    intensity_term = (
        NO_PENALTY_TERM
        if heating_intensity_kwh_m2 <= 0.0
        else clamp_unit_interval(
            parameters.maximum_heating_intensity_kwh_m2 / heating_intensity_kwh_m2
        )
    )
    glazing_term = clamp_unit_interval(1.0 - parameters.window_to_wall_ratio / GLAZING_SCORE_RATIO)
    return clamp_unit_interval(
        UA_TERM_WEIGHT * conductance_term
        + INTENSITY_TERM_WEIGHT * intensity_term
        + GLAZING_TERM_WEIGHT * glazing_term
    )


__all__ = (
    "GLAZING_SCORE_RATIO",
    "HOURS_PER_DAY",
    "INTENSITY_TERM_WEIGHT",
    "REFERENCE_TRANSMITTANCE_PER_FLOOR_AREA",
    "BuildingSimulation",
    "simulate_building",
)
