"""Shared fixtures for the FreeCAD adapter suite.

Nothing in this suite reaches the network, writes outside ``tmp_path``, or
needs a FreeCAD installation: the tests that do exercise a real installation
are marked ``freecad`` and skip cleanly when discovery cannot find one.
"""

from __future__ import annotations

import os
from collections.abc import Mapping
from pathlib import Path
from typing import Final

import pytest

from kala_agent.tools.protocol import (
    PROTOCOL_VERSION,
    JSONValue,
    ToolEnvelopeKind,
    ToolRequestEnvelope,
)
from kala_agent_freecad.availability import FreeCADAvailability, discover
from kala_agent_freecad.cad.runtime import LIB_ROOT_ENVIRONMENT_KEY
from kala_agent_freecad.contracts import (
    FEM_TOOL_ID,
    FEM_TOOL_VERSION,
    GEOMETRY_TOOL_ID,
    GEOMETRY_TOOL_VERSION,
    TOOL_CAPABILITIES,
)

BOX_ARGUMENTS: Final[Mapping[str, JSONValue]] = {
    "export_formats": ["step", "iges", "fcstd"],
    "height_mm": 20.0,
    "length_mm": 120.0,
    "shape": "box",
    "width_mm": 40.0,
}
CYLINDER_ARGUMENTS: Final[Mapping[str, JSONValue]] = {
    "export_formats": ["step"],
    "height_mm": 60.0,
    "radius_mm": 12.5,
    "shape": "cylinder",
}
FEM_BOX_ARGUMENTS: Final[Mapping[str, JSONValue]] = {
    "height_mm": 20.0,
    "length_mm": 120.0,
    "run_solver": True,
    "shape": "box",
    "width_mm": 40.0,
}
MAX_TEST_ARTIFACT_BYTES: Final[int] = 8_388_608


def make_request(
    arguments: Mapping[str, JSONValue],
    *,
    workspace: Path,
    call_id: str = "kala-call-1",
    tool_id: str = GEOMETRY_TOOL_ID,
    tool_version: str = GEOMETRY_TOOL_VERSION,
    max_artifact_bytes: int = MAX_TEST_ARTIFACT_BYTES,
) -> ToolRequestEnvelope:
    """Build a request envelope the way the executor would."""
    return ToolRequestEnvelope(
        protocol_version=PROTOCOL_VERSION,
        kind=ToolEnvelopeKind.REQUEST,
        call_id=call_id,
        tool_id=tool_id,
        tool_version=tool_version,
        arguments=arguments,
        workspace=workspace.resolve(strict=False).as_posix(),
        deadline_seconds=120.0,
        max_output_bytes=1_048_576,
        max_artifact_bytes=max_artifact_bytes,
        granted_capabilities=TOOL_CAPABILITIES,
    )


def make_fem_request(
    arguments: Mapping[str, JSONValue],
    *,
    workspace: Path,
    call_id: str = "kala-fem-call-1",
    max_artifact_bytes: int = MAX_TEST_ARTIFACT_BYTES,
) -> ToolRequestEnvelope:
    return make_request(
        arguments,
        workspace=workspace,
        call_id=call_id,
        tool_id=FEM_TOOL_ID,
        tool_version=FEM_TOOL_VERSION,
        max_artifact_bytes=max_artifact_bytes,
    )


@pytest.fixture(scope="session")
def availability() -> FreeCADAvailability:
    """Discover FreeCAD once for the whole session."""
    return discover()


@pytest.fixture
def require_freecad(availability: FreeCADAvailability) -> FreeCADAvailability:
    if not availability.available:
        pytest.skip(f"FreeCAD is not available: {availability.detail}")
    return availability


@pytest.fixture
def import_roots(
    require_freecad: FreeCADAvailability,
    monkeypatch: pytest.MonkeyPatch,
) -> tuple[Path, ...]:
    """Publish the discovered root the way a worker's loader path would."""
    roots = require_freecad.import_root_paths
    monkeypatch.setenv(LIB_ROOT_ENVIRONMENT_KEY, os.pathsep.join(str(root) for root in roots))
    return roots
