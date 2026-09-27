from __future__ import annotations

from typing import Final

from kala_agent.tools.protocol import ToolErrorCode

MAX_ERROR_MESSAGE_LENGTH: Final[int] = 1_024


class FreeCADAdapterError(RuntimeError):
    """An adapter failure that already knows the code the kernel must see."""

    error_code: ToolErrorCode = ToolErrorCode.INTERNAL_ERROR


class FreeCADInputError(FreeCADAdapterError):
    """The request was refused before FreeCAD was ever loaded."""

    error_code: ToolErrorCode = ToolErrorCode.INVALID_ARGUMENTS


class FreeCADDependencyError(FreeCADAdapterError):
    """A required system dependency is absent or could not be loaded."""

    error_code: ToolErrorCode = ToolErrorCode.SANDBOX_UNAVAILABLE


class FreeCADWorkspaceError(FreeCADAdapterError):
    """A requested path left the worker workspace."""

    error_code: ToolErrorCode = ToolErrorCode.WORKSPACE_VIOLATION


class FreeCADArtifactError(FreeCADAdapterError):
    """A produced file could not be published as a verified artifact."""

    error_code: ToolErrorCode = ToolErrorCode.ARTIFACT_REJECTED


class FreeCADOutputTooLargeError(FreeCADAdapterError):
    """A produced file exceeded the bounded artifact budget."""

    error_code: ToolErrorCode = ToolErrorCode.OUTPUT_TOO_LARGE


class FreeCADOperationError(FreeCADAdapterError):
    """FreeCAD itself refused or crashed while doing the requested work."""

    error_code: ToolErrorCode = ToolErrorCode.INTERNAL_ERROR


def error_code_for(error: BaseException) -> ToolErrorCode:
    """Return the code a failure must be reported under."""
    if isinstance(error, FreeCADAdapterError):
        return error.error_code
    return ToolErrorCode.INTERNAL_ERROR


def error_message_for(error: BaseException, *, replacement: str = "") -> str:
    """Return a short, single line, workspace free message for a failure."""
    name = type(error).__name__
    detail = " ".join(str(error).split())
    if replacement:
        detail = detail.replace(replacement, "<workspace>")
    message = f"{name}: {detail}" if detail else name
    return message[:MAX_ERROR_MESSAGE_LENGTH]


__all__ = (
    "MAX_ERROR_MESSAGE_LENGTH",
    "FreeCADAdapterError",
    "FreeCADArtifactError",
    "FreeCADDependencyError",
    "FreeCADInputError",
    "FreeCADOperationError",
    "FreeCADOutputTooLargeError",
    "FreeCADWorkspaceError",
    "error_code_for",
    "error_message_for",
)
