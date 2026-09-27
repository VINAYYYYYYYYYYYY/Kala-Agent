"""Deterministic closed-form reference simulation for the bracket component.

The simulation is elementary arithmetic: a prismatic plate section, a bolt-hole
pattern, an elementary beam-bending term, an average shear term, and a
plane-stress combination of the two. It is a reference for workflow testing and
comparison only. It is not finite element analysis, not a stress validation, not
a fastener or tolerance analysis, and it does not replace a CAD or CAE model
reviewed by a competent engineer.

The same inputs always produce the same outputs: there is no clock, no random
draw, no iteration to a tolerance, and no floating-point accumulation order that
depends on anything outside this module.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Final

from kala_agent.domains import UNBOUNDED_SAFETY_FACTOR, clamp_unit_interval
from kala_agent.domains.mechanical.parameters import BracketParameters, material_reference

HOLE_AREA_FRACTION: Final[float] = math.pi / 4.0
MILLIMETRE_CUBED_TO_METRE_CUBED: Final[float] = 1.0e-9
KILONEWTON_TO_NEWTON: Final[float] = 1_000.0
LEVER_ARM_FRACTION: Final[float] = 0.5
BENDING_SIGMA: Final[float] = 1.0
SHEAR_TAU: Final[float] = 0.5
VON_MISES_SHEAR_COEFFICIENT: Final[float] = 3.0
SECTION_MODULUS_COEFFICIENT: Final[float] = 6.0
THICKNESS_SCORE_MULTIPLE: Final[float] = 2.0
STRESS_TERM_WEIGHT: Final[float] = 0.5
THICKNESS_TERM_WEIGHT: Final[float] = 0.2
UTILISATION_TERM_WEIGHT: Final[float] = 0.3


@dataclass(frozen=True, slots=True)
class BracketSimulation:
    """Reference quantities produced by :func:`simulate_bracket`."""

    gross_area_mm2: float
    hole_area_mm2: float
    net_area_mm2: float
    volume_mm3: float
    mass_kg: float
    load_n: float
    load_per_point_n: float
    lever_arm_mm: float
    bending_moment_n_mm: float
    section_modulus_mm3: float
    bending_stress_mpa: float
    shear_stress_mpa: float
    stress_mpa: float
    utilisation: float
    safety_factor: float
    material_web_mm: float
    edge_margin_mm: float
    safety_score: float


def simulate_bracket(parameters: BracketParameters) -> BracketSimulation:
    """Return deterministic reference quantities for a bracket-like component."""
    material = material_reference(parameters.material)
    gross_area_mm2 = parameters.length_mm * parameters.width_mm
    hole_area_mm2 = HOLE_AREA_FRACTION * parameters.hole_count * parameters.hole_diameter_mm**2
    net_area_mm2 = gross_area_mm2 - hole_area_mm2
    volume_mm3 = net_area_mm2 * parameters.thickness_mm
    mass_kg = volume_mm3 * MILLIMETRE_CUBED_TO_METRE_CUBED * material.density_kg_m3
    load_n = parameters.load_kn * KILONEWTON_TO_NEWTON
    load_per_point_n = load_n / parameters.load_points
    lever_arm_mm = LEVER_ARM_FRACTION * parameters.length_mm
    bending_moment_n_mm = load_n * lever_arm_mm
    section_modulus_mm3 = (
        parameters.width_mm * parameters.thickness_mm**2 / SECTION_MODULUS_COEFFICIENT
    )
    bending_stress_mpa = BENDING_SIGMA * bending_moment_n_mm / section_modulus_mm3
    shear_stress_mpa = SHEAR_TAU * load_n / (parameters.width_mm * parameters.thickness_mm)
    stress_mpa = math.sqrt(
        bending_stress_mpa**2 + VON_MISES_SHEAR_COEFFICIENT * shear_stress_mpa**2
    )
    utilisation = stress_mpa / material.yield_strength_mpa
    safety_factor = (
        UNBOUNDED_SAFETY_FACTOR if stress_mpa == 0.0 else material.yield_strength_mpa / stress_mpa
    )
    material_web_mm = parameters.length_mm - parameters.hole_count * parameters.hole_diameter_mm
    edge_margin_mm = material_web_mm / 2.0
    safety_score = _safety_score(parameters, safety_factor, utilisation)
    return BracketSimulation(
        gross_area_mm2=gross_area_mm2,
        hole_area_mm2=hole_area_mm2,
        net_area_mm2=net_area_mm2,
        volume_mm3=volume_mm3,
        mass_kg=mass_kg,
        load_n=load_n,
        load_per_point_n=load_per_point_n,
        lever_arm_mm=lever_arm_mm,
        bending_moment_n_mm=bending_moment_n_mm,
        section_modulus_mm3=section_modulus_mm3,
        bending_stress_mpa=bending_stress_mpa,
        shear_stress_mpa=shear_stress_mpa,
        stress_mpa=stress_mpa,
        utilisation=utilisation,
        safety_factor=safety_factor,
        material_web_mm=material_web_mm,
        edge_margin_mm=edge_margin_mm,
        safety_score=safety_score,
    )


def _safety_score(
    parameters: BracketParameters,
    safety_factor: float,
    utilisation: float,
) -> float:
    """Return a bounded, deterministic score in the unit interval."""
    stress_term = clamp_unit_interval(safety_factor / parameters.target_safety_factor)
    thickness_term = clamp_unit_interval(
        parameters.thickness_mm / (THICKNESS_SCORE_MULTIPLE * parameters.minimum_thickness_mm)
    )
    utilisation_term = clamp_unit_interval(1.0 - utilisation)
    return clamp_unit_interval(
        STRESS_TERM_WEIGHT * stress_term
        + THICKNESS_TERM_WEIGHT * thickness_term
        + UTILISATION_TERM_WEIGHT * utilisation_term
    )


__all__ = (
    "BENDING_SIGMA",
    "KILONEWTON_TO_NEWTON",
    "LEVER_ARM_FRACTION",
    "SHEAR_TAU",
    "THICKNESS_SCORE_MULTIPLE",
    "BracketSimulation",
    "simulate_bracket",
)
