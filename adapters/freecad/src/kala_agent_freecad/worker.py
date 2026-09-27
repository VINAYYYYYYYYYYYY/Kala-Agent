"""Worker entry points for the FreeCAD adapter.

These are the only callables the executor dispatches, and each one runs in a
subprocess worker. Every one of them validates the request against the shipped
schema, refuses anything unexpected, and returns exactly one response
envelope: nothing escapes as an exception, and nothing is written to the
protocol channel other than the envelope the executor writes itself.
"""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path
from typing import Final, cast

from kala_agent.contracts import ArtifactRef, ToolRunStatus
from kala_agent.tools.protocol import (
    JSONValue,
    SchemaValidationError,
    ToolErrorCode,
    ToolRequestEnvelope,
    ToolResponseEnvelope,
    validate_against_schema,
)
from kala_agent_freecad.availability import resolve_calculix
from kala_agent_freecad.cad import fem, geometry
from kala_agent_freecad.cad.runtime import import_roots_from_path
from kala_agent_freecad.contracts import (
    FEM_INPUT_SCHEMA,
    FEM_OUTPUT_SCHEMA,
    GEOMETRY_INPUT_SCHEMA,
    GEOMETRY_OUTPUT_SCHEMA,
)
from kala_agent_freecad.errors import (
    MAX_ERROR_MESSAGE_LENGTH,
    error_code_for,
    error_message_for,
)

MAX_WORKSPACE_LENGTH: Final[int] = 1_024


def invoke_geometry(request: ToolRequestEnvelope) -> ToolResponseEnvelope:
    """Build one primitive solid and publish its exports."""
    arguments, rejection = _validated(request, GEOMETRY_INPUT_SCHEMA)
    if rejection is not None:
        return rejection
    workspace, rejection = _workspace(request)
    if rejection is not None:
        return rejection
    try:
        outcome = geometry.run(
            geometry.parse_request(arguments),
            workspace=workspace,
            import_roots=import_roots_from_path(),
            max_artifact_bytes=request.max_artifact_bytes,
            call_id=request.call_id,
        )
    except Exception as error:
        return _failed(request, error)
    return _succeeded(request, outcome.result, outcome.artifacts, GEOMETRY_OUTPUT_SCHEMA)


def invoke_fem(request: ToolRequestEnvelope) -> ToolResponseEnvelope:
    """Build one Fem analysis and report an honest solver status."""
    arguments, rejection = _validated(request, FEM_INPUT_SCHEMA)
    if rejection is not None:
        return rejection
    workspace, rejection = _workspace(request)
    if rejection is not None:
        return rejection
    try:
        outcome = fem.run(
            fem.parse_request(arguments),
            workspace=workspace,
            import_roots=import_roots_from_path(),
            calculix_path=resolve_calculix(),
            max_artifact_bytes=request.max_artifact_bytes,
            call_id=request.call_id,
        )
    except Exception as error:
        return _failed(request, error)
    return _succeeded(request, outcome.result, outcome.artifacts, FEM_OUTPUT_SCHEMA)


def _validated(
    request: ToolRequestEnvelope,
    schema: Mapping[str, object],
) -> tuple[Mapping[str, JSONValue], ToolResponseEnvelope | None]:
    arguments = cast("Mapping[str, JSONValue]", dict(request.arguments))
    try:
        validate_against_schema(arguments, schema, path="$")
    except SchemaValidationError as error:
        return arguments, _failed_message(
            request,
            ToolErrorCode.INVALID_ARGUMENTS,
            str(error),
        )
    return arguments, None


def _workspace(request: ToolRequestEnvelope) -> tuple[Path, ToolResponseEnvelope | None]:
    if not request.workspace.startswith("/") or len(request.workspace) > MAX_WORKSPACE_LENGTH:
        return Path("."), _failed_message(
            request,
            ToolErrorCode.WORKSPACE_VIOLATION,
            "the call workspace is not an absolute bounded path",
        )
    return Path(request.workspace), None


def _failed(request: ToolRequestEnvelope, error: BaseException) -> ToolResponseEnvelope:
    return _failed_message(
        request,
        error_code_for(error),
        error_message_for(error, replacement=request.workspace),
    )


def _failed_message(
    request: ToolRequestEnvelope,
    code: ToolErrorCode,
    message: str,
) -> ToolResponseEnvelope:
    return ToolResponseEnvelope.failed_envelope(
        call_id=request.call_id,
        tool_id=request.tool_id,
        tool_version=request.tool_version,
        status=ToolRunStatus.FAILED,
        error_code=code,
        error_message=message[:MAX_ERROR_MESSAGE_LENGTH],
    )


def _succeeded(
    request: ToolRequestEnvelope,
    result: Mapping[str, JSONValue],
    artifacts: tuple[ArtifactRef, ...],
    schema: Mapping[str, object],
) -> ToolResponseEnvelope:
    try:
        validate_against_schema(result, schema, path="$")
    except SchemaValidationError as error:
        return _failed_message(request, ToolErrorCode.INVALID_RESULT, str(error))
    return ToolResponseEnvelope.succeeded_envelope(
        call_id=request.call_id,
        tool_id=request.tool_id,
        tool_version=request.tool_version,
        result=result,
        artifacts=artifacts,
    )


__all__ = (
    "invoke_fem",
    "invoke_geometry",
)
