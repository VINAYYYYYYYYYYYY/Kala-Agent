"""Typed parameters for the wing domain package.

The package models a parametric tapered wing by span, root and tip chord, sweep,
dihedral, a spanwise station count, and a span load. Everything it derives is a
geometric or aerodynamic surrogate. Nothing in this package is certified: it is
not a lifting-line or panel solution, not CFD, and it establishes no structural
margin, stability, control, or airworthiness conclusion.
"""

from __future__ import annotations

from enum import StrEnum
from typing import Final, Self

from pydantic import BaseModel, ConfigDict, Field, model_validator

from kala_agent.domains import ParameterSpec, parameter_schema_digest

LENGTH_UNIT: Final[str] = "m"
AREA_UNIT: Final[str] = "m^2"
VOLUME_UNIT: Final[str] = "m^3"
ANGLE_UNIT: Final[str] = "deg"
RATIO_UNIT: Final[str] = "1"
STATION_UNIT: Final[str] = "count"
SPAN_LOAD_UNIT: Final[str] = "kN/m"
ROOT_MOMENT_UNIT: Final[str] = "kN*m"
MASS_UNIT: Final[str] = "kg"
PER_RADIAN_UNIT: Final[str] = "1/rad"

NON_CERTIFIED_FINDING: Final[str] = (
    "Geometric and aerodynamic surrogates are not certified analysis: they are not a "
    "lifting-line or panel solution, not CFD, and they establish no structural margin, "
    "stability, control, or airworthiness conclusion."
)
TAPER_FINDING: Final[str] = (
    "Taper ratio and sweep are declared geometry only; no lifting surface method is "
    "applied to them."
)


class WingLoadCase(StrEnum):
    """Declared load case names. Each is a surrogate, not a flight condition."""

    REFERENCE_SURFACE = "reference-surface"
    SYMMETRIC_CRUISE = "symmetric-cruise"


class WingParameters(BaseModel):
    """Wing planform, load, and bound parameters with units and ranges."""

    model_config = ConfigDict(extra="forbid", frozen=True, validate_default=True)

    span_m: float = Field(default=12.0, ge=1.0, le=120.0, description="Wing span.")
    root_chord_m: float = Field(default=1.8, ge=0.05, le=12.0, description="Root chord.")
    tip_chord_m: float = Field(default=0.5, ge=0.01, le=12.0, description="Tip chord.")
    sweep_deg: float = Field(
        default=22.0, ge=-10.0, le=75.0, description="Quarter-chord sweep of the declared planform."
    )
    dihedral_deg: float = Field(
        default=5.0, ge=-20.0, le=45.0, description="Dihedral of the declared planform."
    )
    thickness_ratio: float = Field(
        default=0.12, ge=0.03, le=0.25, description="Declared maximum thickness ratio."
    )
    spanwise_stations: int = Field(
        default=8, ge=2, le=64, description="Spanwise stations used by the reference integration."
    )
    load_case: WingLoadCase = Field(
        default=WingLoadCase.REFERENCE_SURFACE, description="Declared surrogate load case."
    )
    span_load_kn_m: float = Field(
        default=4.0, ge=0.0, le=5_000.0, description="Distributed span load."
    )
    maximum_aspect_ratio: float = Field(
        default=20.0, ge=0.5, le=60.0, description="Hard maximum aspect ratio."
    )
    maximum_root_bending_moment_kn_m: float = Field(
        default=150.0, ge=1.0, le=5_000.0, description="Hard maximum root bending moment."
    )
    maximum_mass_kg: float = Field(
        default=400.0, ge=5.0, le=5_000.0, description="Hard maximum reference mass."
    )

    @model_validator(mode="after")
    def validate_planform(self) -> Self:
        """Reject planform declarations that cannot describe a tapered wing."""
        if self.tip_chord_m > self.root_chord_m:
            raise ValueError("tip_chord_m must not exceed root_chord_m")
        if self.span_m < self.root_chord_m:
            raise ValueError("span_m must be at least the root chord")
        return self


PARAMETER_SPECS: Final[tuple[ParameterSpec, ...]] = (
    ParameterSpec(
        name="span_m",
        unit=LENGTH_UNIT,
        minimum=1.0,
        maximum=120.0,
        default=12.0,
        description="Wing span in metres.",
    ),
    ParameterSpec(
        name="root_chord_m",
        unit=LENGTH_UNIT,
        minimum=0.05,
        maximum=12.0,
        default=1.8,
        description="Root chord in metres.",
    ),
    ParameterSpec(
        name="tip_chord_m",
        unit=LENGTH_UNIT,
        minimum=0.01,
        maximum=12.0,
        default=0.5,
        description="Tip chord in metres.",
    ),
    ParameterSpec(
        name="sweep_deg",
        unit=ANGLE_UNIT,
        minimum=-10.0,
        maximum=75.0,
        default=22.0,
        description="Declared quarter-chord sweep in degrees.",
    ),
    ParameterSpec(
        name="dihedral_deg",
        unit=ANGLE_UNIT,
        minimum=-20.0,
        maximum=45.0,
        default=5.0,
        description="Declared dihedral in degrees.",
    ),
    ParameterSpec(
        name="thickness_ratio",
        unit=RATIO_UNIT,
        minimum=0.03,
        maximum=0.25,
        default=0.12,
        description="Declared maximum thickness ratio.",
    ),
    ParameterSpec(
        name="spanwise_stations",
        unit=STATION_UNIT,
        minimum=2.0,
        maximum=64.0,
        default=8.0,
        description="Spanwise stations used by the reference integration.",
    ),
    ParameterSpec(
        name="load_case",
        unit="key",
        minimum=0.0,
        maximum=0.0,
        default=WingLoadCase.REFERENCE_SURFACE.value,
        description="Declared surrogate load case name.",
    ),
    ParameterSpec(
        name="span_load_kn_m",
        unit=SPAN_LOAD_UNIT,
        minimum=0.0,
        maximum=5_000.0,
        default=4.0,
        description="Distributed span load in kilonewtons per metre.",
    ),
    ParameterSpec(
        name="maximum_aspect_ratio",
        unit=RATIO_UNIT,
        minimum=0.5,
        maximum=60.0,
        default=20.0,
        description="Hard maximum aspect ratio.",
    ),
    ParameterSpec(
        name="maximum_root_bending_moment_kn_m",
        unit=ROOT_MOMENT_UNIT,
        minimum=1.0,
        maximum=5_000.0,
        default=150.0,
        description="Hard maximum root bending moment in kilonewton metres.",
    ),
    ParameterSpec(
        name="maximum_mass_kg",
        unit=MASS_UNIT,
        minimum=5.0,
        maximum=5_000.0,
        default=400.0,
        description="Hard maximum reference mass in kilograms.",
    ),
)

WING_SCHEMA_DIGEST: Final[str] = parameter_schema_digest(PARAMETER_SPECS)


__all__ = (
    "ANGLE_UNIT",
    "AREA_UNIT",
    "LENGTH_UNIT",
    "MASS_UNIT",
    "NON_CERTIFIED_FINDING",
    "PARAMETER_SPECS",
    "PER_RADIAN_UNIT",
    "RATIO_UNIT",
    "ROOT_MOMENT_UNIT",
    "SPAN_LOAD_UNIT",
    "STATION_UNIT",
    "TAPER_FINDING",
    "VOLUME_UNIT",
    "WING_SCHEMA_DIGEST",
    "WingLoadCase",
    "WingParameters",
)
