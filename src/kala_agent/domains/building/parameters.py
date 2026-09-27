"""Typed parameters for the building domain package.

The package models a stack of storeys over a declared floor plate and evaluates a
transmission-only thermal envelope. The code reference is a placeholder and is
marked non-authoritative everywhere it appears: this package determines no code
compliance, no occupancy classification, and no safety approval, and it is not an
energy model. The declared bounds are illustrative placeholders, not limits taken
from any published code.
"""

from __future__ import annotations

from enum import StrEnum
from typing import Final, Self

from pydantic import BaseModel, ConfigDict, Field, model_validator

from kala_agent.domains import ParameterSpec, parameter_schema_digest

METRE_UNIT: Final[str] = "m"
SQUARE_METRE_UNIT: Final[str] = "m^2"
CUBIC_METRE_UNIT: Final[str] = "m^3"
CONDUCTIVITY_UNIT: Final[str] = "W/(m^2*K)"
TRANSMITTANCE_UNIT: Final[str] = "W/K"
TEMPERATURE_UNIT: Final[str] = "degC"
DEGREE_DAY_UNIT: Final[str] = "K.day"
ENERGY_UNIT: Final[str] = "kWh"
INTENSITY_UNIT: Final[str] = "kWh/m^2"
RATIO_UNIT: Final[str] = "1"

NON_AUTHORITATIVE_CODE_VERSION: Final[str] = "non-authoritative-placeholder"
CODE_AUTHORITY_FINDING: Final[str] = (
    "Code version 'non-authoritative-placeholder' is a declared placeholder; this package "
    "determines no building code compliance, no occupancy classification, and no safety "
    "approval."
)
THERMAL_SURROGATE_FINDING: Final[str] = (
    "Transmission-only degree-day surrogate; it excludes solar gain, internal load, "
    "ventilation, and thermal mass, so it is not a code or energy-model calculation."
)


class BuildingCodeVersion(StrEnum):
    """Declared code references. Every value is a non-authoritative placeholder."""

    NON_AUTHORITATIVE_PLACEHOLDER = "non-authoritative-placeholder"


class BuildingParameters(BaseModel):
    """Storey, area, and thermal envelope parameters with units and ranges."""

    model_config = ConfigDict(extra="forbid", frozen=True, validate_default=True)

    storey_count: int = Field(default=4, ge=1, le=200, description="Number of storeys.")
    storey_height_m: float = Field(default=3.0, ge=2.0, le=6.0, description="Height of one storey.")
    gross_floor_area_m2: float = Field(
        default=250.0, ge=10.0, le=100_000.0, description="Gross floor area of one storey."
    )
    perimeter_m: float = Field(default=32.0, ge=5.0, le=500.0, description="Plate perimeter.")
    wall_conductivity_w_m2k: float = Field(
        default=0.30, ge=0.0, le=5.0, description="Opaque wall transmittance."
    )
    window_conductivity_w_m2k: float = Field(
        default=1.6, ge=0.0, le=6.0, description="Window transmittance."
    )
    roof_conductivity_w_m2k: float = Field(
        default=0.20, ge=0.0, le=5.0, description="Roof transmittance."
    )
    window_to_wall_ratio: float = Field(
        default=0.30, ge=0.0, le=0.9, description="Glazed fraction of the wall area."
    )
    heating_degree_days_k: float = Field(
        default=2_500.0, ge=1.0, le=40_000.0, description="Heating degree days."
    )
    design_indoor_temperature_c: float = Field(
        default=20.0, ge=10.0, le=30.0, description="Design indoor temperature."
    )
    design_outdoor_temperature_c: float = Field(
        default=-8.0, ge=-40.0, le=20.0, description="Design outdoor temperature."
    )
    minimum_storey_area_m2: float = Field(
        default=20.0, ge=1.0, le=1_000.0, description="Placeholder minimum area per storey."
    )
    maximum_height_m: float = Field(
        default=200.0, ge=10.0, le=400.0, description="Placeholder maximum total height."
    )
    maximum_heating_intensity_kwh_m2: float = Field(
        default=600.0, ge=10.0, le=2_000.0, description="Placeholder maximum heating intensity."
    )
    code_version: BuildingCodeVersion = Field(
        default=BuildingCodeVersion.NON_AUTHORITATIVE_PLACEHOLDER,
        description="Declared code reference; only a non-authoritative placeholder exists.",
    )

    @model_validator(mode="after")
    def validate_heating_case(self) -> Self:
        """Reject combinations that cannot describe a heating season."""
        if self.design_outdoor_temperature_c >= self.design_indoor_temperature_c:
            raise ValueError("the design outdoor temperature must be below the indoor one")
        if not 0.0 <= self.window_to_wall_ratio <= 0.9:
            raise ValueError("the glazed fraction must be within the declared range")
        return self


PARAMETER_SPECS: Final[tuple[ParameterSpec, ...]] = (
    ParameterSpec(
        name="storey_count",
        unit=RATIO_UNIT,
        minimum=1.0,
        maximum=200.0,
        default=4.0,
        description="Number of storeys in the stack.",
    ),
    ParameterSpec(
        name="storey_height_m",
        unit=METRE_UNIT,
        minimum=2.0,
        maximum=6.0,
        default=3.0,
        description="Height of one storey in metres.",
    ),
    ParameterSpec(
        name="gross_floor_area_m2",
        unit=SQUARE_METRE_UNIT,
        minimum=10.0,
        maximum=100_000.0,
        default=250.0,
        description="Gross floor area of one storey in square metres.",
    ),
    ParameterSpec(
        name="perimeter_m",
        unit=METRE_UNIT,
        minimum=5.0,
        maximum=500.0,
        default=32.0,
        description="Plate perimeter in metres.",
    ),
    ParameterSpec(
        name="wall_conductivity_w_m2k",
        unit=CONDUCTIVITY_UNIT,
        minimum=0.0,
        maximum=5.0,
        default=0.30,
        description="Opaque wall transmittance.",
    ),
    ParameterSpec(
        name="window_conductivity_w_m2k",
        unit=CONDUCTIVITY_UNIT,
        minimum=0.0,
        maximum=6.0,
        default=1.6,
        description="Window transmittance.",
    ),
    ParameterSpec(
        name="roof_conductivity_w_m2k",
        unit=CONDUCTIVITY_UNIT,
        minimum=0.0,
        maximum=5.0,
        default=0.20,
        description="Roof transmittance.",
    ),
    ParameterSpec(
        name="window_to_wall_ratio",
        unit=RATIO_UNIT,
        minimum=0.0,
        maximum=0.9,
        default=0.30,
        description="Glazed fraction of the wall area.",
    ),
    ParameterSpec(
        name="heating_degree_days_k",
        unit=DEGREE_DAY_UNIT,
        minimum=1.0,
        maximum=40_000.0,
        default=2_500.0,
        description="Heating degree days for the transmission surrogate.",
    ),
    ParameterSpec(
        name="design_indoor_temperature_c",
        unit=TEMPERATURE_UNIT,
        minimum=10.0,
        maximum=30.0,
        default=20.0,
        description="Design indoor temperature in degrees Celsius.",
    ),
    ParameterSpec(
        name="design_outdoor_temperature_c",
        unit=TEMPERATURE_UNIT,
        minimum=-40.0,
        maximum=20.0,
        default=-8.0,
        description="Design outdoor temperature in degrees Celsius.",
    ),
    ParameterSpec(
        name="minimum_storey_area_m2",
        unit=SQUARE_METRE_UNIT,
        minimum=1.0,
        maximum=1_000.0,
        default=20.0,
        description="Placeholder minimum storey area in square metres.",
    ),
    ParameterSpec(
        name="maximum_height_m",
        unit=METRE_UNIT,
        minimum=10.0,
        maximum=400.0,
        default=200.0,
        description="Placeholder maximum total height in metres.",
    ),
    ParameterSpec(
        name="maximum_heating_intensity_kwh_m2",
        unit=INTENSITY_UNIT,
        minimum=10.0,
        maximum=2_000.0,
        default=600.0,
        description="Placeholder maximum heating intensity in kilowatt hours per square metre.",
    ),
    ParameterSpec(
        name="code_version",
        unit="key",
        minimum=0.0,
        maximum=0.0,
        default=BuildingCodeVersion.NON_AUTHORITATIVE_PLACEHOLDER.value,
        description="Declared code reference; only a non-authoritative placeholder exists.",
    ),
)

BUILDING_SCHEMA_DIGEST: Final[str] = parameter_schema_digest(PARAMETER_SPECS)


__all__ = (
    "BUILDING_SCHEMA_DIGEST",
    "CODE_AUTHORITY_FINDING",
    "CONDUCTIVITY_UNIT",
    "CUBIC_METRE_UNIT",
    "DEGREE_DAY_UNIT",
    "ENERGY_UNIT",
    "INTENSITY_UNIT",
    "METRE_UNIT",
    "NON_AUTHORITATIVE_CODE_VERSION",
    "PARAMETER_SPECS",
    "RATIO_UNIT",
    "SQUARE_METRE_UNIT",
    "TEMPERATURE_UNIT",
    "THERMAL_SURROGATE_FINDING",
    "TRANSMITTANCE_UNIT",
    "BuildingCodeVersion",
    "BuildingParameters",
)
