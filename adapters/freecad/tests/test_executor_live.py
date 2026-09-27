"""End to end dispatch of the adapter tools through the kernel executor.

The executor owns the worker process, the workspace, and artifact
verification, so these tests prove the two halves fit: the tools register,
dispatch out of process, and the artifacts the kernel verifies are the ones
FreeCAD really wrote.
"""

from __future__ import annotations

import json
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Final, cast

import pytest

from conftest import BOX_ARGUMENTS, FEM_BOX_ARGUMENTS
from kala_agent.contracts import ArtifactClassification, SideEffectLevel, ToolCall, ToolRunStatus
from kala_agent.tools.executor import SandboxProfile, ToolExecutor
from kala_agent.tools.protocol import ToolErrorCode, canonical_json_dumps, sha256_hex
from kala_agent.tools.registry import ToolRegistry
from kala_agent.tools.sandbox import IsolationProfile
from kala_agent_freecad.availability import FreeCADAvailability
from kala_agent_freecad.contracts import (
    FEM_TOOL_ID,
    FEM_TOOL_VERSION,
    GEOMETRY_TOOL_ID,
    GEOMETRY_TOOL_VERSION,
    TOOL_CAPABILITIES,
)
from kala_agent_freecad.policy import cad_profile
from kala_agent_freecad.registration import register, worker_import_roots

pytestmark = pytest.mark.freecad


@pytest.fixture(autouse=True)
def _live_freecad(require_freecad: FreeCADAvailability) -> None:
    del require_freecad


CAD_TOOL_IDS: Final[tuple[str, ...]] = (GEOMETRY_TOOL_ID, FEM_TOOL_ID)
RESULT_DETAIL: Final[int] = 4_096


def _executor(
    run_directory: Path,
    availability: FreeCADAvailability,
    profile: SandboxProfile | None = None,
) -> ToolExecutor:
    registry = ToolRegistry()
    register(registry, availability)
    return ToolExecutor(
        registry,
        run_directory=run_directory,
        granted_capabilities=TOOL_CAPABILITIES,
        profile=profile,
        import_roots=worker_import_roots(availability),
    )


def _call(
    tool_id: str,
    version: str,
    arguments: dict[str, Any],
    call_id: str,
    timeout_seconds: float,
) -> ToolCall:
    return ToolCall(
        call_id=call_id,
        tool_id=tool_id,
        tool_version=version,
        arguments_json=canonical_json_dumps(arguments),
        requested_capabilities=TOOL_CAPABILITIES,
        side_effect_level=SideEffectLevel.LOCAL_WRITE,
        timeout_seconds=timeout_seconds,
        issued_at=datetime.now(UTC),
    )


def test_both_tools_register_and_dispatch(
    tmp_path: Path,
    availability: FreeCADAvailability,
) -> None:
    executor = _executor(tmp_path / "runs", availability)
    assert executor.registry.tool_ids() == tuple(sorted(CAD_TOOL_IDS))
    geometry = executor.execute(
        _call(
            GEOMETRY_TOOL_ID,
            GEOMETRY_TOOL_VERSION,
            dict(BOX_ARGUMENTS),
            "dispatch-geometry",
            180.0,
        )
    )
    assert geometry.status is ToolRunStatus.SUCCEEDED, geometry.error_message
    assert len(geometry.artifacts) == 3
    for artifact in geometry.artifacts:
        target = Path(artifact.uri.removeprefix("file://"))
        assert sha256_hex(target.read_bytes()) == artifact.digest
        assert artifact.classification is ArtifactClassification.DESIGN_BINARY
    fem = executor.execute(
        _call(
            FEM_TOOL_ID,
            FEM_TOOL_VERSION,
            dict(FEM_BOX_ARGUMENTS),
            "dispatch-fem",
            300.0,
        )
    )
    assert fem.status is ToolRunStatus.SUCCEEDED, fem.error_message
    assert {artifact.artifact_id for artifact in fem.artifacts} == {
        "dispatch-fem-fcstd",
        "dispatch-fem-step",
    }
    result = json.loads(cast(str, fem.result_json))
    assert result["objects"]["analysis"] == "KalaAnalysis"
    if not availability.solver_available:
        assert result["solver"]["status"] == "solver_unavailable"
        assert result["solver"]["executed"] is False
        assert result["results"]["available"] is False


def test_dispatch_refuses_arguments_the_schema_forbids(
    tmp_path: Path,
    availability: FreeCADAvailability,
) -> None:
    executor = _executor(tmp_path / "runs", availability)
    result = executor.execute(
        _call(
            GEOMETRY_TOOL_ID,
            GEOMETRY_TOOL_VERSION,
            {"shape": "box", "export_formats": ["step"], "escape": True},
            "dispatch-invalid",
            60.0,
        )
    )
    assert result.status is ToolRunStatus.FAILED
    assert result.error_code == ToolErrorCode.INVALID_ARGUMENTS.value
    assert result.result_json is None


def test_dispatch_refuses_an_ungranted_capability(
    tmp_path: Path,
    availability: FreeCADAvailability,
) -> None:
    registry = ToolRegistry()
    register(registry, availability)
    executor = ToolExecutor(
        registry,
        run_directory=tmp_path / "runs",
        granted_capabilities=(),
        import_roots=worker_import_roots(availability),
    )
    result = executor.execute(
        _call(
            GEOMETRY_TOOL_ID,
            GEOMETRY_TOOL_VERSION,
            dict(BOX_ARGUMENTS),
            "dispatch-denied",
            60.0,
        )
    )
    assert result.status is ToolRunStatus.DENIED
    assert result.error_code == ToolErrorCode.CAPABILITY_DENIED.value


def test_cad_profile_budget_reaches_the_executor(
    tmp_path: Path,
    availability: FreeCADAvailability,
) -> None:
    profile = cad_profile(isolation=IsolationProfile.PORTABLE)
    executor = _executor(tmp_path / "runs", availability, profile)
    assert executor.profile.profile_name == profile.profile_name
    assert executor.profile.max_artifact_bytes == profile.max_artifact_bytes
    assert executor.profile.default_timeout_seconds == profile.default_timeout_seconds
    assert executor.profile.allow_network is False
    assert executor.profile.allow_egress is False
    assert executor.profile.allow_privileged_capabilities is False
    report = executor.execute(
        _call(
            GEOMETRY_TOOL_ID,
            GEOMETRY_TOOL_VERSION,
            dict(BOX_ARGUMENTS),
            "dispatch-profile",
            180.0,
        )
    )
    assert report.status is ToolRunStatus.SUCCEEDED, report.error_message
    assert executor.reports[-1].sandbox_profile == profile.profile_name
    assert "process_isolation" in executor.reports[-1].enforced_controls


def test_strict_profile_is_enforced_or_refused(
    tmp_path: Path,
    availability: FreeCADAvailability,
) -> None:
    executor = _executor(tmp_path / "runs", availability, cad_profile())
    assert executor.profile.isolation_profile.value == "strict_linux"
    if executor.sandbox_probe.available:
        result = executor.execute(
            _call(
                GEOMETRY_TOOL_ID,
                GEOMETRY_TOOL_VERSION,
                dict(BOX_ARGUMENTS),
                "dispatch-strict",
                180.0,
            )
        )
        assert result.status is ToolRunStatus.SUCCEEDED, result.error_message
        assert set(executor.sandbox_probe.controls) <= set(executor.reports[-1].enforced_controls)
    else:
        result = executor.execute(
            _call(
                GEOMETRY_TOOL_ID,
                GEOMETRY_TOOL_VERSION,
                dict(BOX_ARGUMENTS),
                "dispatch-strict",
                180.0,
            )
        )
        assert result.status is ToolRunStatus.DENIED
        assert result.error_code == ToolErrorCode.SANDBOX_UNAVAILABLE.value
