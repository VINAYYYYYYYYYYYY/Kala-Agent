"""Error handling for the local Kala HTTP adapter.

Every failure leaves this process as a small, stable JSON body: an error code, a
bounded human message, and optional sanitized details. No traceback, no exception
chain, no submitted value, and no environment content is ever echoed back, so a
request that carries credential-shaped text cannot leak it through an error.

The security headers are attached here as well as in the middleware, because the
server error middleware answers a failed request from outside the middleware
stack.
"""

from __future__ import annotations

import logging
from typing import Final

from fastapi import FastAPI, Request, status
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse
from starlette.exceptions import HTTPException as StarletteHTTPException

from kala_agent.api.middleware import SECURITY_HEADERS
from kala_agent.api.schemas import MAX_ERROR_DETAILS, ErrorResponse, error_response
from kala_agent.application import (
    ApplicationError,
    CheckpointDecisionDeniedError,
    CheckpointDigestMismatchError,
    CheckpointNotPendingError,
    ClosedApplicationError,
    CorruptKernelStateError,
    InvalidIdentifierError,
    InvalidParametersError,
    ProvenanceUnavailableError,
    RevisionCommitRejectedError,
    RevisionForkRefusedError,
    RevisionHeadMovedError,
    UnknownDomainError,
    UnknownPreparedRevisionError,
    UnknownRevisionError,
    UnknownWorkflowError,
    WorkflowFinishedError,
)

LOGGER: Final[logging.Logger] = logging.getLogger("kala_agent.api")
UNPROCESSABLE_CONTENT: Final[int] = 422
UNKNOWN_ROUTE_MESSAGE: Final[str] = "the requested resource does not exist"
METHOD_NOT_ALLOWED_MESSAGE: Final[str] = "the requested method is not allowed for this resource"
INTERNAL_ERROR_MESSAGE: Final[str] = "the request could not be completed"
VALIDATION_MESSAGE: Final[str] = "the request body or parameters are invalid"

_STATUS_BY_ERROR: Final[tuple[tuple[type[ApplicationError], int], ...]] = (
    (UnknownDomainError, status.HTTP_404_NOT_FOUND),
    (UnknownRevisionError, status.HTTP_404_NOT_FOUND),
    (UnknownPreparedRevisionError, status.HTTP_404_NOT_FOUND),
    (UnknownWorkflowError, status.HTTP_404_NOT_FOUND),
    (ProvenanceUnavailableError, status.HTTP_404_NOT_FOUND),
    (
        CheckpointNotPendingError,
        status.HTTP_409_CONFLICT,
    ),
    (RevisionCommitRejectedError, status.HTTP_409_CONFLICT),
    (RevisionForkRefusedError, status.HTTP_409_CONFLICT),
    (RevisionHeadMovedError, status.HTTP_409_CONFLICT),
    (CheckpointDecisionDeniedError, status.HTTP_409_CONFLICT),
    (CheckpointDigestMismatchError, status.HTTP_409_CONFLICT),
    (WorkflowFinishedError, status.HTTP_409_CONFLICT),
    (CorruptKernelStateError, status.HTTP_503_SERVICE_UNAVAILABLE),
    (ClosedApplicationError, status.HTTP_503_SERVICE_UNAVAILABLE),
    (InvalidIdentifierError, UNPROCESSABLE_CONTENT),
    (InvalidParametersError, UNPROCESSABLE_CONTENT),
)


def _status_for(error: ApplicationError) -> int:
    for error_type, code in _STATUS_BY_ERROR:
        if isinstance(error, error_type):
            return code
    return status.HTTP_400_BAD_REQUEST


def _json(status_code: int, body: ErrorResponse) -> JSONResponse:
    response = JSONResponse(
        status_code=status_code,
        content=body.model_dump(mode="json"),
        media_type="application/json",
    )
    for name, value in SECURITY_HEADERS.items():
        response.headers[name] = value
    return response


def _validation_details(error: RequestValidationError) -> tuple[str, ...]:
    details: list[str] = []
    for failure in error.errors()[:MAX_ERROR_DETAILS]:
        location = ".".join(str(item) for item in failure.get("loc", ())) or "body"
        message = str(failure.get("msg", "invalid value"))[:128]
        kind = str(failure.get("type", "invalid"))[:64]
        details.append(f"{location}: {kind}: {message}")
    return tuple(details)


async def handle_request_validation(
    request: Request,
    error: Exception,
) -> JSONResponse:
    """Return a sanitized 422 for a rejected request payload."""
    if not isinstance(error, RequestValidationError):
        return _json(
            UNPROCESSABLE_CONTENT,
            error_response("invalid_request", VALIDATION_MESSAGE),
        )
    return _json(
        UNPROCESSABLE_CONTENT,
        error_response("invalid_request", VALIDATION_MESSAGE, _validation_details(error)),
    )


async def handle_application_error(request: Request, error: Exception) -> JSONResponse:
    """Map a composition root failure onto a bounded HTTP status."""
    if not isinstance(error, ApplicationError):
        return _json(
            status.HTTP_500_INTERNAL_SERVER_ERROR,
            error_response("internal_error", INTERNAL_ERROR_MESSAGE),
        )
    return _json(_status_for(error), error_response(error.code, error.message))


async def handle_http_exception(request: Request, error: Exception) -> JSONResponse:
    """Return a sanitized body for framework and routing errors."""
    if not isinstance(error, StarletteHTTPException):
        return _json(
            status.HTTP_500_INTERNAL_SERVER_ERROR,
            error_response("internal_error", INTERNAL_ERROR_MESSAGE),
        )
    code = "internal_error"
    if error.status_code == status.HTTP_404_NOT_FOUND:
        code = "not_found"
        message = UNKNOWN_ROUTE_MESSAGE
    elif error.status_code == status.HTTP_405_METHOD_NOT_ALLOWED:
        code = "method_not_allowed"
        message = METHOD_NOT_ALLOWED_MESSAGE
    elif error.status_code == status.HTTP_413_REQUEST_ENTITY_TOO_LARGE:
        code = "payload_too_large"
        message = "the request body exceeds the configured size limit"
    elif status.HTTP_400_BAD_REQUEST <= error.status_code < status.HTTP_500_BAD_REQUEST:
        code = "request_rejected"
        message = str(error.detail)[:128] or "the request was rejected"
    else:
        code = "internal_error"
        message = INTERNAL_ERROR_MESSAGE
    return _json(error.status_code, error_response(code, message))


async def handle_unexpected_error(request: Request, error: Exception) -> JSONResponse:
    """Fail closed: report the failure type locally and return a bare 500."""
    LOGGER.error("unhandled local failure: %s", type(error).__name__)
    return _json(
        status.HTTP_500_INTERNAL_SERVER_ERROR,
        error_response("internal_error", INTERNAL_ERROR_MESSAGE),
    )


def install_error_handlers(app: FastAPI) -> None:
    """Install every error handler this API relies on."""
    app.add_exception_handler(RequestValidationError, handle_request_validation)
    app.add_exception_handler(ApplicationError, handle_application_error)
    app.add_exception_handler(StarletteHTTPException, handle_http_exception)
    app.add_exception_handler(Exception, handle_unexpected_error)


__all__ = (
    "INTERNAL_ERROR_MESSAGE",
    "METHOD_NOT_ALLOWED_MESSAGE",
    "UNKNOWN_ROUTE_MESSAGE",
    "VALIDATION_MESSAGE",
    "handle_application_error",
    "handle_http_exception",
    "handle_request_validation",
    "handle_unexpected_error",
    "install_error_handlers",
)
