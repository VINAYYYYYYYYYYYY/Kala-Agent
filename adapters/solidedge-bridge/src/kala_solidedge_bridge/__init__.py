"""The Linux-safe public surface of the Solid Edge bridge.

Only the protocol module is imported here. Runtime, session, server, COM, and
Kala core modules are deliberately absent so importing this package on Linux
can never load Solid Edge or ``pywin32``.
"""

from __future__ import annotations

from .protocol import (
    MAX_OPERATIONS,
    MAX_PARTS,
    PROTOCOL_VERSION,
    Artifact,
    ArtifactFormat,
    AssemblyExportStep,
    AssemblyNew,
    AssemblyPlace,
    AssemblySave,
    BuildAccepted,
    BuildRequest,
    BuildResult,
    BuildStatus,
    ErrorCode,
    ErrorResponse,
    Health,
    JobState,
    PartExportStep,
    PartMeasure,
    PartNew,
    PartResult,
    PartSave,
    Plane,
    RangeBox,
    SketchCircle,
    SketchRectangle,
    SolidCut,
    SolidExtrude,
    SolidRevolve,
    StepTrace,
)

__all__ = (
    "MAX_OPERATIONS",
    "MAX_PARTS",
    "PROTOCOL_VERSION",
    "Artifact",
    "ArtifactFormat",
    "AssemblyExportStep",
    "AssemblyNew",
    "AssemblyPlace",
    "AssemblySave",
    "BuildAccepted",
    "BuildRequest",
    "BuildResult",
    "BuildStatus",
    "ErrorCode",
    "ErrorResponse",
    "Health",
    "JobState",
    "PartExportStep",
    "PartMeasure",
    "PartNew",
    "PartResult",
    "PartSave",
    "Plane",
    "RangeBox",
    "SketchCircle",
    "SketchRectangle",
    "SolidCut",
    "SolidExtrude",
    "SolidRevolve",
    "StepTrace",
)
