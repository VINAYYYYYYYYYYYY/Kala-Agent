"""CAD provider registry for Studio design turns (ADR-0011).

The registry is explicit and closed: ``opencascade`` (opt-in built-in),
``freecad`` (external, subprocess), and ``solidedge`` (Windows bridge). There
is no automatic fallback. The module imports only kernel-free provider
modules, so the public Studio API can start on a host without OCP.
"""

from __future__ import annotations

from collections.abc import Mapping
from typing import Final

from kala_agent.studio.design.providers.base import (
    ALLOWED_PROVIDER_IDS,
    Availability,
    CadProvider,
    CadProviderBuildError,
    CadProviderError,
    CadProviderId,
    CadProviderUnavailableError,
    MeasuredBounds,
    MeasuredProperties,
    ProviderBuild,
    ProviderStatus,
)
from kala_agent.studio.design.providers.freecad import FreeCadProvider
from kala_agent.studio.design.providers.opencascade import OpenCascadeProvider
from kala_agent.studio.design.providers.solidedge import SolidEdgeProvider

DEFAULT_PROVIDER_ORDER: Final[tuple[str, ...]] = ALLOWED_PROVIDER_IDS


def build_providers(
    *,
    max_step_bytes: int | None = None,
    environ: Mapping[str, str] | None = None,
) -> dict[str, CadProvider]:
    """Return one instance of every provider in the closed set.

    ``environ`` is the controlled environment view a Studio app was created
    with, so a test or embedder does not silently fall back to the process
    environment for the FreeCAD/Solid Edge configuration.
    """
    environment = None if environ is None else dict(environ)
    providers: tuple[CadProvider, ...] = (
        OpenCascadeProvider(max_step_bytes=max_step_bytes),
        FreeCadProvider(max_step_bytes=max_step_bytes, environ=environment),
        SolidEdgeProvider(environ=environment),
    )
    return {provider.name: provider for provider in providers}


def probe_providers(
    providers: Mapping[str, CadProvider] | None = None,
) -> tuple[ProviderStatus, ...]:
    """Probe every configured provider and return its honest status.

    A registered provider that raises during its own probe is reported
    unavailable with the exception type as the reason; a missing registration
    is reported unavailable too. Nothing is assumed available.
    """
    source = build_providers() if providers is None else dict(providers)
    statuses: list[ProviderStatus] = []
    for provider_id in DEFAULT_PROVIDER_ORDER:
        provider = source.get(provider_id)
        if provider is None:
            statuses.append(
                ProviderStatus(
                    id=provider_id,
                    name=provider_id,
                    available=False,
                    version=None,
                    reason="the provider is not configured in this Studio build",
                    measured_by=provider_id,
                    mode="unknown",
                )
            )
            continue
        try:
            availability = provider.probe()
        except Exception as error:  # pragma: no cover - a provider probe is defensive
            availability = Availability(
                available=False,
                version=None,
                reason=f"the provider probe failed: {type(error).__name__}",
            )
        statuses.append(
            ProviderStatus(
                id=provider.name,
                name=provider.display_name,
                available=availability.available,
                version=availability.version,
                reason=availability.reason,
                measured_by=provider.measured_by,
                mode=provider.mode,
            )
        )
    return tuple(statuses)


__all__ = (
    "ALLOWED_PROVIDER_IDS",
    "DEFAULT_PROVIDER_ORDER",
    "Availability",
    "CadProvider",
    "CadProviderBuildError",
    "CadProviderError",
    "CadProviderId",
    "CadProviderUnavailableError",
    "FreeCadProvider",
    "MeasuredBounds",
    "MeasuredProperties",
    "OpenCascadeProvider",
    "ProviderBuild",
    "ProviderStatus",
    "SolidEdgeProvider",
    "build_providers",
    "probe_providers",
)
