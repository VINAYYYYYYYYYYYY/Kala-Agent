"""The CAD sandbox profile, checked without a FreeCAD installation."""

from __future__ import annotations

from typing import Any, cast

import pytest

from kala_agent.tools.executor import SandboxProfile
from kala_agent.tools.sandbox import (
    STRICT_OPTIONAL_CONTROLS,
    STRICT_REQUIRED_CONTROLS,
    IsolationProfile,
    SandboxProbe,
    judge_sandbox,
)
from kala_agent_freecad.contracts import (
    CAD_DEFAULT_TIMEOUT_SECONDS,
    CAD_MAX_ARGUMENT_BYTES,
    CAD_MAX_ARTIFACT_BYTES,
    CAD_MAX_OUTPUT_BYTES,
    CAD_MAX_QUEUE_SECONDS,
    CAD_MAX_WORKERS,
    CAD_PROFILE_NAME,
)
from kala_agent_freecad.policy import cad_profile, cad_report

SHIPPED = SandboxProfile()


def test_profile_is_derived_from_the_shipped_defaults() -> None:
    profile = cad_profile()
    assert profile.profile_name == CAD_PROFILE_NAME
    assert profile.max_output_bytes == CAD_MAX_OUTPUT_BYTES
    assert profile.max_argument_bytes == CAD_MAX_ARGUMENT_BYTES
    assert profile.max_artifact_bytes == CAD_MAX_ARTIFACT_BYTES
    assert profile.default_timeout_seconds == CAD_DEFAULT_TIMEOUT_SECONDS
    assert profile.max_workers == CAD_MAX_WORKERS
    assert profile.max_queue_seconds == CAD_MAX_QUEUE_SECONDS


def test_profile_budgets_exceed_the_shipped_ones() -> None:
    profile = cad_profile()
    assert profile.max_artifact_bytes > SHIPPED.max_artifact_bytes
    assert profile.max_output_bytes > SHIPPED.max_output_bytes
    assert profile.default_timeout_seconds > SHIPPED.default_timeout_seconds
    assert 1 <= profile.max_workers <= 8
    assert 0.0 < profile.max_queue_seconds <= 300.0


def test_profile_never_grants_authority() -> None:
    profile = cad_profile()
    assert profile.allow_network is False
    assert profile.allow_egress is False
    assert profile.allow_privileged_capabilities is False
    assert profile.enforce_child_limits is True
    assert dict(profile.extra_environment) == {}


def test_profile_is_isolated_where_the_host_can_enforce_it() -> None:
    profile = cad_profile()
    assert profile.isolation_profile is IsolationProfile.STRICT_LINUX
    assert profile.sandbox_probe is None
    assert "wall_clock_timeout" in profile.enforced_controls
    assert "cwd_confined" in profile.enforced_controls
    assert profile.enforced_controls == SHIPPED.enforced_controls


def test_profile_keeps_the_portable_option() -> None:
    profile = cad_profile(isolation=IsolationProfile.PORTABLE)
    assert profile.isolation_profile is IsolationProfile.PORTABLE
    assert judge_sandbox(profile.isolation_profile, None).allowed is True


def test_profile_accepts_a_probe_and_nothing_else() -> None:
    probe = SandboxProbe(
        available=True,
        platform="linux-x86_64",
        controls=tuple(sorted(STRICT_REQUIRED_CONTROLS | STRICT_OPTIONAL_CONTROLS)),
    )
    profile = cad_profile(probe=probe)
    assert profile.sandbox_probe is probe
    assert judge_sandbox(profile.isolation_profile, profile.sandbox_probe).allowed is True
    assert set(profile.enforced_controls) >= STRICT_REQUIRED_CONTROLS
    with pytest.raises(TypeError):
        cad_profile(probe=cast("Any", "probe"))
    with pytest.raises(TypeError):
        cad_profile(isolation=cast("Any", "portable"))


def test_profile_does_not_mutate_the_shipped_profile() -> None:
    first = cad_profile()
    second = cad_profile()
    assert first is not second
    assert first == second
    assert SHIPPED.profile_name == "portable-minimum"
    assert SHIPPED.max_artifact_bytes == 8_388_608
    assert SHIPPED.default_timeout_seconds == 30.0
    assert SHIPPED.max_workers == 4
    assert SHIPPED.isolation_profile is IsolationProfile.PORTABLE
    assert SHIPPED.sandbox_probe is None


def test_report_describes_what_is_enforced() -> None:
    report = cad_report()
    assert report["profile_name"] == CAD_PROFILE_NAME
    assert report["artifact_bytes"] == CAD_MAX_ARTIFACT_BYTES
    assert report["isolation_profile"] == IsolationProfile.STRICT_LINUX.value
    assert report["network"] is False
    assert report["egress"] is False
    assert report["privileged"] is False
    assert report["workers"] == CAD_MAX_WORKERS
    assert report["timeout_seconds"] == CAD_DEFAULT_TIMEOUT_SECONDS
    assert isinstance(report["controls"], list)


def test_strict_profile_refuses_side_effects_when_it_is_unavailable() -> None:
    refused = judge_sandbox(IsolationProfile.STRICT_LINUX, None)
    assert refused.allowed is False
    assert "not available on this host" in refused.reason
    unavailable_probe = SandboxProbe(
        available=False,
        platform="linux-x86_64",
        detail="namespaces are refused",
    )
    refused_with_detail = judge_sandbox(IsolationProfile.STRICT_LINUX, unavailable_probe)
    assert refused_with_detail.allowed is False
    assert "namespaces are refused" in refused_with_detail.reason
