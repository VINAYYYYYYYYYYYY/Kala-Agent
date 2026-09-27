"""Typed parameters for the mechanical domain package.

The package models a bracket-like component: a prismatic plate with a hole
pattern, a declared material, and a static load case. Every value below is an
arithmetic input with a unit and a range, not a manufacturing instruction and
not a certified material record. The material constants are illustrative
placeholders with a declared ``reference_status`` so that no downstream consumer
can mistake them for certified material data.
"""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from enum import StrEnum
from types import MappingProxyType
from typing import Final, Self

from pydantic import BaseModel, ConfigDict, Field, model_validator

from kala_agent.domains import (
    DomainParameterError,
    ParameterSpec,
    parameter_schema_digest,
    require_name,
)

LENGTH_UNIT: Final[str] = "mm"
AREA_UNIT: Final[str] = "mm^2"
VOLUME_UNIT: Final[str] = "mm^3"
MASS_UNIT: Final[str] = "kg"
FORCE_UNIT: Final[str] = "N"
MOMENT_UNIT: Final[str] = "N*mm"
STRESS_UNIT: Final[str] = "MPa"
RATIO_UNIT: Final[str] = "1"
DENSITY_UNIT: Final[str] = "kg/m^3"
MODULUS_UNIT: Final[str] = "GPa"
EXPANSION_UNIT: Final[str] = "1/K"

ILLUSTRATIVE_MATERIAL_STATUS: Final[str] = "illustrative-constant-not-certified-material-data"


class MaterialKey(StrEnum):
    """Material keys declared by this package."""

    ALUMINIUM_6061_T6 = "aluminium-6061-t6"
    STEEL_S355 = "steel-s355"
    TITANIUM_TI6AL4V = "titanium-ti6al4v"


@dataclass(frozen=True, slots=True)
class MaterialReference:
    """Illustrative material constants used by the reference simulation."""

    key: MaterialKey
    label: str
    density_kg_m3: float
    yield_strength_mpa: float
    elastic_modulus_gpa: float
    thermal_expansion_per_kelvin: float
    reference_status: str = ILLUSTRATIVE_MATERIAL_STATUS


MATERIALS: Final[Mapping[str, MaterialReference]] = MappingProxyType(
    {
        MaterialKey.ALUMINIUM_6061_T6.value: MaterialReference(
            key=MaterialKey.ALUMINIUM_6061_T6,
            label="Aluminium 6061-T6 (illustrative)",
            density_kg_m3=2_700.0,
            yield_strength_mpa=276.0,
            elastic_modulus_gpa=68.9,
            thermal_expansion_per_kelvin=23.6e-6,
        ),
        MaterialKey.STEEL_S355.value: MaterialReference(
            key=MaterialKey.STEEL_S355,
            label="Structural steel S355 (illustrative)",
            density_kg_m3=7_850.0,
            yield_strength_mpa=355.0,
            elastic_modulus_gpa=210.0,
            thermal_expansion_per_kelvin=12.0e-6,
        ),
        MaterialKey.TITANIUM_TI6AL4V.value: MaterialReference(
            key=MaterialKey.TITANIUM_TI6AL4V,
            label="Titanium Ti-6Al-4V (illustrative)",
            density_kg_m3=4_430.0,
            yield_strength_mpa=880.0,
            elastic_modulus_gpa=113.8,
            thermal_expansion_per_kelvin=8.6e-6,
        ),
    }
)


def material_reference(key: MaterialKey | str) -> MaterialReference:
    """Return the declared material constants for a key."""
    try:
        resolved = MaterialKey(key)
    except ValueError as error:
        raise DomainParameterError(f"material {key!r} is not declared by this package") from error
    reference = MATERIALS.get(resolved.value)
    if reference is None:
        raise DomainParameterError(f"material {resolved.value} has no declared constants")
    return reference


class BracketParameters(BaseModel):
    """Bracket-like component parameters with units and inclusive ranges."""

    model_config = ConfigDict(extra="forbid", frozen=True, validate_default=True)

    length_mm: float = Field(default=120.0, ge=20.0, le=500.0, description="Plate length.")
    width_mm: float = Field(default=80.0, ge=20.0, le=300.0, description="Plate width.")
    thickness_mm: float = Field(default=8.0, ge=0.5, le=25.0, description="Plate thickness.")
    hole_diameter_mm: float = Field(
        default=6.0, ge=0.0, le=50.0, description="Fastener hole diameter."
    )
    hole_count: int = Field(default=2, ge=0, le=12, description="Fastener hole count.")
    minimum_edge_margin_mm: float = Field(
        default=2.0, ge=0.0, le=10.0, description="Required material beside the hole pattern."
    )
    material: MaterialKey = Field(
        default=MaterialKey.ALUMINIUM_6061_T6, description="Declared material key."
    )
    load_kn: float = Field(default=1.0, ge=0.0, le=200.0, description="Applied static load.")
    load_points: int = Field(default=2, ge=1, le=8, description="Load introduction points.")
    minimum_thickness_mm: float = Field(
        default=2.0, ge=0.1, le=25.0, description="Hard minimum plate thickness."
    )
    maximum_stress_mpa: float = Field(
        default=150.0, ge=1.0, le=2_000.0, description="Hard maximum surrogate stress."
    )
    target_safety_factor: float = Field(
        default=2.0, ge=1.0, le=10.0, description="Safety factor the score is measured against."
    )

    @model_validator(mode="after")
    def validate_pattern(self) -> Self:
        """Reject hole patterns that cannot exist on the declared plate."""
        if self.hole_count > 0 and self.hole_diameter_mm <= 0.0:
            raise ValueError("hole_diameter_mm must be positive when holes are declared")
        if self.hole_count * self.hole_diameter_mm > self.length_mm:
            raise ValueError("the hole pattern does not fit along the declared length")
        if self.hole_count * self.hole_diameter_mm**2 >= self.length_mm * self.width_mm:
            raise ValueError("the hole pattern cannot fit on the declared plate")
        if self.hole_count == 0 and self.minimum_edge_margin_mm > 0.0:
            raise ValueError("minimum_edge_margin_mm requires at least one declared hole")
        return self

    @property
    def material_reference(self) -> MaterialReference:
        """Return the illustrative constants for the declared material."""
        return material_reference(self.material)


PARAMETER_SPECS: Final[tuple[ParameterSpec, ...]] = (
    ParameterSpec(
        name="length_mm",
        unit=LENGTH_UNIT,
        minimum=20.0,
        maximum=500.0,
        default=120.0,
        description="Plate length in millimetres.",
    ),
    ParameterSpec(
        name="width_mm",
        unit=LENGTH_UNIT,
        minimum=20.0,
        maximum=300.0,
        default=80.0,
        description="Plate width in millimetres.",
    ),
    ParameterSpec(
        name="thickness_mm",
        unit=LENGTH_UNIT,
        minimum=0.5,
        maximum=25.0,
        default=8.0,
        description="Plate thickness in millimetres.",
    ),
    ParameterSpec(
        name="hole_diameter_mm",
        unit=LENGTH_UNIT,
        minimum=0.0,
        maximum=50.0,
        default=6.0,
        description="Fastener hole diameter in millimetres.",
    ),
    ParameterSpec(
        name="hole_count",
        unit=RATIO_UNIT,
        minimum=0.0,
        maximum=12.0,
        default=2.0,
        description="Number of fastener holes.",
    ),
    ParameterSpec(
        name="minimum_edge_margin_mm",
        unit=LENGTH_UNIT,
        minimum=0.0,
        maximum=10.0,
        default=2.0,
        description="Material required beside the hole pattern in millimetres.",
    ),
    ParameterSpec(
        name="material",
        unit="key",
        minimum=0.0,
        maximum=0.0,
        default=MaterialKey.ALUMINIUM_6061_T6.value,
        description="Declared material key with illustrative constants.",
    ),
    ParameterSpec(
        name="load_kn",
        unit="kN",
        minimum=0.0,
        maximum=200.0,
        default=1.0,
        description="Applied static load in kilonewtons.",
    ),
    ParameterSpec(
        name="load_points",
        unit=RATIO_UNIT,
        minimum=1.0,
        maximum=8.0,
        default=2.0,
        description="Number of load introduction points.",
    ),
    ParameterSpec(
        name="minimum_thickness_mm",
        unit=LENGTH_UNIT,
        minimum=0.1,
        maximum=25.0,
        default=2.0,
        description="Hard minimum plate thickness in millimetres.",
    ),
    ParameterSpec(
        name="maximum_stress_mpa",
        unit=STRESS_UNIT,
        minimum=1.0,
        maximum=2_000.0,
        default=150.0,
        description="Hard maximum surrogate stress in megapascals.",
    ),
    ParameterSpec(
        name="target_safety_factor",
        unit=RATIO_UNIT,
        minimum=1.0,
        maximum=10.0,
        default=2.0,
        description="Safety factor target used by the bounded score.",
    ),
)

PARAMETER_NAMES: Final[tuple[str, ...]] = tuple(
    sorted(require_name(spec.name, "parameter name") for spec in PARAMETER_SPECS)
)

BRACKET_SCHEMA_DIGEST: Final[str] = parameter_schema_digest(PARAMETER_SPECS)


__all__ = (
    "AREA_UNIT",
    "BRACKET_SCHEMA_DIGEST",
    "DENSITY_UNIT",
    "EXPANSION_UNIT",
    "FORCE_UNIT",
    "ILLUSTRATIVE_MATERIAL_STATUS",
    "LENGTH_UNIT",
    "MASS_UNIT",
    "MATERIALS",
    "MODULUS_UNIT",
    "MOMENT_UNIT",
    "PARAMETER_NAMES",
    "PARAMETER_SPECS",
    "RATIO_UNIT",
    "STRESS_UNIT",
    "VOLUME_UNIT",
    "BracketParameters",
    "MaterialKey",
    "MaterialReference",
    "material_reference",
)
