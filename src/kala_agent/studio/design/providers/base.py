"""The CAD provider contract (ADR-0011).

Kala orchestrates; a CAD provider builds. Every provider accepts the same
``ConstructionScript`` and returns the same :class:`ProviderBuild`: the STEP
bytes it exported, its own measured properties, its version, and optional
native bytes. The OpenCascade viewer then imports the provider's STEP for
display and never re-measures it as the authority.

This module is deliberately kernel-free. It imports only the shared
construction contract, so the provider registry, the API schema, and the chat
passthrough can be imported on a host where ``cadquery-ocp`` is not installed.
``FreeCAD`` and the adapter distribution are never imported by core code at
all; the FreeCAD provider reaches them only inside a subprocess.

There is no default provider. ``ALLOWED_PROVIDER_IDS`` is the closed set a
caller must choose from; an absent or unknown choice is refused rather than
silently mapped to the built-in builder.
"""

from __future__ import annotations

import math
from collections.abc import Sequence
from dataclasses import dataclass
from enum import StrEnum
from typing import TYPE_CHECKING, ClassVar, Final, Protocol

from kala_agent.contracts.construction import ConstructionScript

if TYPE_CHECKING:
    from kala_agent.studio.design.intent import DesignIntent

MAX_REASON_LENGTH: Final[int] = 512
MAX_VERSION_LENGTH: Final[int] = 64
MAX_WARNING_LENGTH: Final[int] = 256
MAX_WARNINGS: Final[int] = 16


class CadProviderId(StrEnum):
    """The CAD providers ADR-0011 defines. This set is closed."""

    OPENCASCADE = "opencascade"
    FREECAD = "freecad"
    SOLIDEDGE = "solidedge"


ALLOWED_PROVIDER_IDS: Final[tuple[str, ...]] = tuple(item.value for item in CadProviderId)


@dataclass(frozen=True, slots=True)
class Availability:
    """One provider's probe result, never assumed.

    ``available`` is the only field a caller may act on. ``version`` and
    ``reason`` are diagnostic provenance; a missing version is never filled
    with a guess.
    """

    available: bool
    version: str | None = None
    reason: str | None = None

    def __post_init__(self) -> None:
        if self.version is not None and len(self.version) > MAX_VERSION_LENGTH:
            object.__setattr__(self, "version", self.version[:MAX_VERSION_LENGTH])
        if self.reason is not None and len(self.reason) > MAX_REASON_LENGTH:
            object.__setattr__(self, "reason", self.reason[:MAX_REASON_LENGTH])


@dataclass(frozen=True, slots=True)
class MeasuredBounds:
    """An axis-aligned box the provider measured, in millimetres."""

    min_mm: tuple[float, float, float]
    max_mm: tuple[float, float, float]

    def __post_init__(self) -> None:
        if len(self.min_mm) != 3 or len(self.max_mm) != 3:
            raise ValueError("provider bounds require three minimum and three maximum values")
        for value in (*self.min_mm, *self.max_mm):
            if not math.isfinite(value):
                raise ValueError("provider bounds must be finite numbers")
        for axis in range(3):
            if self.min_mm[axis] > self.max_mm[axis]:
                raise ValueError("provider bounds minimum exceeds maximum")

    @property
    def size_mm(self) -> tuple[float, float, float]:
        """Return the box size along X, Y, and Z."""
        return (
            self.max_mm[0] - self.min_mm[0],
            self.max_mm[1] - self.min_mm[1],
            self.max_mm[2] - self.min_mm[2],
        )


@dataclass(frozen=True, slots=True)
class MeasuredProperties:
    """The provider's own measured numbers.

    ADR-0011 names volume, area, bounds, and mass. ``centroid_mm`` is carried
    because the built-in provider reports it; a provider that does not report
    a property leaves it ``None`` and the UI shows ``—``. No value is ever
    substituted by OpenCascade.
    """

    volume_mm3: float | None = None
    area_mm2: float | None = None
    bounds: MeasuredBounds | None = None
    mass_g: float | None = None
    centroid_mm: tuple[float, float, float] | None = None

    def __post_init__(self) -> None:
        for name in ("volume_mm3", "area_mm2", "mass_g"):
            value = getattr(self, name)
            if value is not None and not math.isfinite(value):
                raise ValueError(f"provider {name} must be a finite number")
        if self.centroid_mm is not None:
            if len(self.centroid_mm) != 3 or not all(
                math.isfinite(value) for value in self.centroid_mm
            ):
                raise ValueError("provider centroid_mm must be three finite numbers")


@dataclass(frozen=True, slots=True)
class ProviderBuild:
    """One provider build: the exported STEP and the provider's own facts."""

    step_bytes: bytes
    measured: MeasuredProperties
    provider_version: str | None = None
    warnings: tuple[str, ...] = ()
    native_bytes: bytes | None = None
    native_kind: str | None = None
    valid: bool | None = None
    solid_count: int | None = None

    def __post_init__(self) -> None:
        if not isinstance(self.step_bytes, bytes) or not self.step_bytes:
            raise ValueError("a provider build must return STEP bytes")
        if self.native_bytes is not None and not isinstance(self.native_bytes, bytes):
            raise ValueError("native provider bytes must be bytes")
        if self.provider_version is not None:
            object.__setattr__(self, "provider_version", self.provider_version[:MAX_VERSION_LENGTH])
        if self.native_kind is not None and not self.native_kind:
            raise ValueError("a native artifact kind must not be empty")
        bounded = tuple(
            str(warning)[:MAX_WARNING_LENGTH] for warning in self.warnings[:MAX_WARNINGS]
        )
        object.__setattr__(self, "warnings", bounded)
        if self.solid_count is not None and self.solid_count < 0:
            raise ValueError("a solid count must be non-negative")


@dataclass(frozen=True, slots=True)
class ProviderStatus:
    """A probe result plus the static identity the API publishes."""

    id: str
    name: str
    available: bool
    version: str | None
    reason: str | None
    measured_by: str
    mode: str


class CadProviderError(ValueError):
    """Base class for an honest provider refusal."""


class CadProviderUnavailableError(CadProviderError):
    """The selected provider cannot build on this host."""

    def __init__(
        self,
        provider: str,
        message: str,
        *,
        reason: str | None = None,
        available: Sequence[str] = (),
    ) -> None:
        super().__init__(message)
        self.provider = provider
        self.reason = (reason or message)[:MAX_REASON_LENGTH]
        self.available = tuple(available)


class CadProviderBuildError(CadProviderError):
    """The provider was available but refused or failed the build."""

    def __init__(self, message: str, *, op: str | None = None) -> None:
        super().__init__(message[:MAX_REASON_LENGTH])
        self.op = op


class CadProvider(Protocol):
    """The interface every CAD provider implements (ADR-0011)."""

    name: ClassVar[str]
    display_name: ClassVar[str]
    measured_by: ClassVar[str]
    mode: ClassVar[str]

    def probe(self) -> Availability:
        """Return whether this provider can build on this host right now."""
        ...

    def build(self, script: ConstructionScript, intent: DesignIntent) -> ProviderBuild:
        """Build ``script``, export STEP, and report the provider's own facts."""
        ...


__all__ = (
    "ALLOWED_PROVIDER_IDS",
    "MAX_REASON_LENGTH",
    "MAX_VERSION_LENGTH",
    "MAX_WARNINGS",
    "MAX_WARNING_LENGTH",
    "Availability",
    "CadProvider",
    "CadProviderBuildError",
    "CadProviderError",
    "CadProviderId",
    "CadProviderUnavailableError",
    "MeasuredBounds",
    "MeasuredProperties",
    "ProviderBuild",
    "ProviderStatus",
)
