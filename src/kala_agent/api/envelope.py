"""The portable error envelope shared by the local adapters.

Every failure the local HTTP surfaces report leaves the process in this one
small, stable JSON shape: an error code, a bounded human message, and optional
sanitized details. Nothing here imports the composition root, the tool plane,
or a platform-specific component, so the envelope can be loaded by a transport
that only needs to answer a refusal.

:mod:`kala_agent.api.schemas` re-exports these names, so the public surface and
the object identities are unchanged for existing callers.
"""

from __future__ import annotations

from typing import Final

from pydantic import BaseModel, ConfigDict

MAX_ERROR_DETAILS: Final[int] = 16
MAX_ERROR_MESSAGE_LENGTH: Final[int] = 256


class _StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)


class ErrorDetail(_StrictModel):
    code: str
    message: str


class ErrorResponse(_StrictModel):
    """The only error shape this API returns. It never carries a traceback."""

    error: ErrorDetail
    details: tuple[str, ...] = ()


def error_response(code: str, message: str, details: tuple[str, ...] = ()) -> ErrorResponse:
    """Build a bounded, secret-free error body."""
    bounded_details = tuple(
        detail[:MAX_ERROR_MESSAGE_LENGTH] for detail in details[:MAX_ERROR_DETAILS]
    )
    return ErrorResponse(
        error=ErrorDetail(
            code=code,
            message=message[:MAX_ERROR_MESSAGE_LENGTH],
        ),
        details=bounded_details,
    )


__all__ = (
    "MAX_ERROR_DETAILS",
    "MAX_ERROR_MESSAGE_LENGTH",
    "ErrorDetail",
    "ErrorResponse",
    "error_response",
)
