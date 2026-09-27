"""The sandbox profile the FreeCAD tools are meant to run under.

CAD work needs a larger artifact and time budget than the shipped default and
it wants the strict namespace profile when the host can enforce it, but it
never widens authority: no network, no egress, and no privileged capability is
ever granted. The shipped profile is only read, never mutated.
"""

from __future__ import annotations

from dataclasses import replace

from kala_agent.tools.executor import SandboxProfile
from kala_agent.tools.sandbox import IsolationProfile, SandboxProbe
from kala_agent_freecad.contracts import (
    CAD_DEFAULT_TIMEOUT_SECONDS,
    CAD_MAX_ARGUMENT_BYTES,
    CAD_MAX_ARTIFACT_BYTES,
    CAD_MAX_OUTPUT_BYTES,
    CAD_MAX_QUEUE_SECONDS,
    CAD_MAX_WORKERS,
    CAD_PROFILE_NAME,
)


def cad_profile(
    *,
    isolation: IsolationProfile = IsolationProfile.STRICT_LINUX,
    probe: SandboxProbe | None = None,
) -> SandboxProfile:
    """Return a CAD tuned profile derived from the shipped defaults.

    The result is a new value: the shipped profile is never edited, and the
    executor may still replace the attached probe with the one it observed.
    """
    if not isinstance(isolation, IsolationProfile):
        raise TypeError("isolation must be an IsolationProfile")
    if probe is not None and not isinstance(probe, SandboxProbe):
        raise TypeError("probe must be a SandboxProbe")
    return replace(
        SandboxProfile(),
        profile_name=CAD_PROFILE_NAME,
        max_output_bytes=CAD_MAX_OUTPUT_BYTES,
        max_argument_bytes=CAD_MAX_ARGUMENT_BYTES,
        max_artifact_bytes=CAD_MAX_ARTIFACT_BYTES,
        default_timeout_seconds=CAD_DEFAULT_TIMEOUT_SECONDS,
        max_workers=CAD_MAX_WORKERS,
        max_queue_seconds=CAD_MAX_QUEUE_SECONDS,
        allow_network=False,
        allow_egress=False,
        allow_privileged_capabilities=False,
        enforce_child_limits=True,
        isolation_profile=isolation,
        sandbox_probe=probe,
    )


def cad_report(profile: SandboxProfile | None = None) -> dict[str, object]:
    """Describe what a CAD profile enforces, for an operator's run report."""
    selected = cad_profile() if profile is None else profile
    return {
        "artifact_bytes": selected.max_artifact_bytes,
        "controls": list(selected.enforced_controls),
        "isolation_profile": selected.isolation_profile.value,
        "network": selected.allow_network,
        "egress": selected.allow_egress,
        "privileged": selected.allow_privileged_capabilities,
        "profile_name": selected.profile_name,
        "timeout_seconds": selected.default_timeout_seconds,
        "workers": selected.max_workers,
    }


__all__ = (
    "cad_profile",
    "cad_report",
)
