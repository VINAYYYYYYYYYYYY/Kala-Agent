"""Deterministic reference simulation for the parametric wing.

The simulation integrates a linear chord distribution over a bounded, declared
number of spanwise stations to get a reference area, converts that into an aspect
ratio, applies a finite-wing lift-curve slope and induced drag factor, integrates
a distributed span load into a root bending moment, and scales a declared areal
mass into a reference mass. These are surrogates for workflow testing and
comparison only. They are not a lifting-line or panel method, not CFD, not a
structural or fatigue analysis, and they are not certified.

The same inputs always produce the same outputs: the station loop is bounded by
the declared parameter range, and there is no clock, no random draw, and no
iteration to a tolerance.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Final

from kala_agent.domains import clamp_unit_interval
from kala_agent.domains.wing.parameters import WingParameters

KILONEWTON_TO_NEWTON: Final[float] = 1_000.0
ELLIPTICAL_ROOT_MOMENT_COEFFICIENT: Final[float] = 1.0 / (math.pi * math.sqrt(3.0))
AREAL_MASS_KG_M2: Final[float] = 12.0
INDUCED_DRAG_EFFICIENCY: Final[float] = 1.0
RADIANS_PER_DEGREE: Final[float] = math.pi / 180.0
NO_PENALTY_TERM: Final[float] = 1.0
ASPECT_TERM_WEIGHT: Final[float] = 0.35
BENDING_TERM_WEIGHT: Final[float] = 0.4
EFFICIENCY_TERM_WEIGHT: Final[float] = 0.25


@dataclass(frozen=True, slots=True)
class WingSimulation:
    """Reference quantities produced by :func:`simulate_wing`."""

    taper_ratio: float
    reference_area_m2: float
    mean_chord_m: float
    aspect_ratio: float
    structural_volume_m3: float
    mass_kg: float
    span_load_n_m: float
    root_bending_moment_kn_m: float
    lift_curve_slope_per_rad: float
    induced_drag_factor: float
    induced_drag_coefficient: float
    quarter_chord_sweep_rad: float
    dihedral_rad: float
    geometry_score: float


def _integrated_chord_length(
    parameters: WingParameters,
) -> float:
    """Integrate the linear chord distribution over the declared stations."""
    stations = parameters.spanwise_stations
    total = 0.0
    for index in range(stations):
        start_fraction = index / stations
        end_fraction = (index + 1) / stations
        start_chord = (
            parameters.root_chord_m
            + (parameters.tip_chord_m - parameters.root_chord_m) * start_fraction
        )
        end_chord = (
            parameters.root_chord_m
            + (parameters.tip_chord_m - parameters.root_chord_m) * end_fraction
        )
        total += 0.5 * (start_chord + end_chord) * (end_fraction - start_fraction)
    return total


def simulate_wing(parameters: WingParameters) -> WingSimulation:
    """Return deterministic reference quantities for a parametric wing."""
    integrated_chord_m = _integrated_chord_length(parameters)
    reference_area_m2 = parameters.span_m * integrated_chord_m
    mean_chord_m = integrated_chord_m
    aspect_ratio = parameters.span_m * parameters.span_m / reference_area_m2
    taper_ratio = parameters.tip_chord_m / parameters.root_chord_m
    structural_volume_m3 = reference_area_m2 * parameters.thickness_ratio
    mass_kg = reference_area_m2 * AREAL_MASS_KG_M2
    span_load_n_m = parameters.span_load_kn_m * KILONEWTON_TO_NEWTON
    root_bending_moment_kn_m = (
        ELLIPTICAL_ROOT_MOMENT_COEFFICIENT * span_load_n_m * parameters.span_m**2
    ) / KILONEWTON_TO_NEWTON
    lift_curve_slope_per_rad = 2.0 * math.pi * aspect_ratio / (aspect_ratio + 2.0)
    induced_drag_factor = (aspect_ratio + 2.0) / (2.0 * aspect_ratio)
    induced_drag_coefficient = 1.0 / (math.pi * aspect_ratio * INDUCED_DRAG_EFFICIENCY)
    quarter_chord_sweep_rad = parameters.sweep_deg * RADIANS_PER_DEGREE
    dihedral_rad = parameters.dihedral_deg * RADIANS_PER_DEGREE
    geometry_score = _geometry_score(
        parameters,
        aspect_ratio,
        root_bending_moment_kn_m,
        lift_curve_slope_per_rad,
    )
    return WingSimulation(
        taper_ratio=taper_ratio,
        reference_area_m2=reference_area_m2,
        mean_chord_m=mean_chord_m,
        aspect_ratio=aspect_ratio,
        structural_volume_m3=structural_volume_m3,
        mass_kg=mass_kg,
        span_load_n_m=span_load_n_m,
        root_bending_moment_kn_m=root_bending_moment_kn_m,
        lift_curve_slope_per_rad=lift_curve_slope_per_rad,
        induced_drag_factor=induced_drag_factor,
        induced_drag_coefficient=induced_drag_coefficient,
        quarter_chord_sweep_rad=quarter_chord_sweep_rad,
        dihedral_rad=dihedral_rad,
        geometry_score=geometry_score,
    )


def _geometry_score(
    parameters: WingParameters,
    aspect_ratio: float,
    root_bending_moment_kn_m: float,
    lift_curve_slope_per_rad: float,
) -> float:
    """Return a bounded, deterministic surrogate score in the unit interval."""
    aspect_term = clamp_unit_interval(1.0 - aspect_ratio / parameters.maximum_aspect_ratio)
    bending_term = (
        NO_PENALTY_TERM
        if root_bending_moment_kn_m <= 0.0
        else clamp_unit_interval(
            1.0 - root_bending_moment_kn_m / parameters.maximum_root_bending_moment_kn_m
        )
    )
    efficiency_term = clamp_unit_interval(lift_curve_slope_per_rad / (2.0 * math.pi))
    return clamp_unit_interval(
        ASPECT_TERM_WEIGHT * aspect_term
        + BENDING_TERM_WEIGHT * bending_term
        + EFFICIENCY_TERM_WEIGHT * efficiency_term
    )


__all__ = (
    "AREAL_MASS_KG_M2",
    "ELLIPTICAL_ROOT_MOMENT_COEFFICIENT",
    "INDUCED_DRAG_EFFICIENCY",
    "KILONEWTON_TO_NEWTON",
    "RADIANS_PER_DEGREE",
    "WingSimulation",
    "simulate_wing",
)
