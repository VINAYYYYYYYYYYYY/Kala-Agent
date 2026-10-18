"""Error handling for the studio HTTP adapter.

Every failure leaves the studio as a small, stable JSON body: an error
code, a bounded human message, and optional sanitized details. No
traceback, no exception chain, no submitted value, and no environment
content is ever echoed back, so a request that carries credential-shaped
text cannot leak it through an error.

The handler here maps framework ``HTTPException`` instances onto the
studio envelope. The 4xx status codes other than the dedicated
404/405/413 paths use a generic sanitized message; the original detail
becomes a bounded ``details`` entry only when it is short and contains
no credential-shaped text. The 5xx path never echoes the framework
detail at all.
"""

from __future__ import annotations

import logging
import re
from typing import Final

from fastapi import FastAPI, Request, status
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse
from starlette.exceptions import HTTPException as StarletteHTTPException

from kala_agent.studio.middleware import SECURITY_HEADERS, StudioAPIError
from kala_agent.studio.schemas import ErrorResponse, error_response

LOGGER: Final[logging.Logger] = logging.getLogger("kala_agent.studio.errors")
UNPROCESSABLE_CONTENT: Final[int] = 422
INTERNAL_ERROR_MESSAGE: Final[str] = "the studio request could not be completed"
VALIDATION_MESSAGE: Final[str] = "the request body or parameters are invalid"
UNKNOWN_ROUTE_MESSAGE: Final[str] = "the requested resource does not exist"
METHOD_NOT_ALLOWED_MESSAGE: Final[str] = "the requested method is not allowed for this resource"
PAYLOAD_TOO_LARGE_MESSAGE: Final[str] = "the request body exceeds the configured size limit"
REQUEST_REJECTED_MESSAGE: Final[str] = "the request was rejected"
# Starlette renamed ``HTTP_413_REQUEST_ENTITY_TOO_LARGE`` in newer releases;
# the numeric status is stable across versions.
PAYLOAD_TOO_LARGE_STATUS: Final[int] = 413
MAX_VALIDATION_DETAILS: Final[int] = 8
MAX_DETAIL_ENTRY_LENGTH: Final[int] = 128
# A loose heuristic for credential-shaped substrings. The studio never
# forwards a request detail that matches this shape to a client.
_CREDENTIAL_RE: Final[re.Pattern[str]] = re.compile(
    r"sk-[A-Za-z0-9]{16,}|sk_(?:live|test)_[A-Za-z0-9]{8,}|"
    r"AIza[0-9A-Za-z_-]{16,}|Bearer\s+[A-Za-z0-9._-]{16,}|"
    r"AKIA[0-9A-Z]{8,}",
    re.IGNORECASE,
)


def _safe_detail(value: object) -> str | None:
    """Return a sanitized detail string, or ``None`` when it is unsafe.

    The studio only forwards a detail to the client when it is a short
    string with no credential-shaped substring; this avoids leaking
    anything an upstream framework echoed back from the request body.
    """
    if not isinstance(value, str):
        return None
    text = value.strip()
    if not text or len(text) > MAX_DETAIL_ENTRY_LENGTH:
        return None
    if _CREDENTIAL_RE.search(text):
        return None
    return text


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
    for failure in error.errors()[:MAX_VALIDATION_DETAILS]:
        location = ".".join(str(item) for item in failure.get("loc", ())) or "body"
        raw_message = str(failure.get("msg", "invalid value"))
        message = _safe_detail(raw_message) or "value rejected by validator"
        kind = str(failure.get("type", "invalid"))[:64]
        details.append(f"{location}: {kind}: {message[:96]}")
    return tuple(details)


async def handle_request_validation(request: Request, error: Exception) -> JSONResponse:
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


async def handle_studio_api_error(request: Request, error: Exception) -> JSONResponse:
    """Map a studio route error onto a sanitized HTTP status."""
    if not isinstance(error, StudioAPIError):
        return _json(
            status.HTTP_500_INTERNAL_SERVER_ERROR,
            error_response("internal_error", INTERNAL_ERROR_MESSAGE),
        )
    return _json(error.status_code, error_response(error.code, error.message))


async def handle_http_exception(request: Request, error: Exception) -> JSONResponse:
    """Return a sanitized body for framework and routing errors.

    The handler maps FastAPI / Starlette ``HTTPException`` onto the
    flat studio envelope. The 4xx paths use a generic sanitized message;
    the framework ``detail`` only ever lands in ``details`` when it is
    short and contains no credential-shaped text. The 5xx path never
    echoes the framework detail at all.
    """
    if not isinstance(error, StarletteHTTPException):
        return _json(
            status.HTTP_500_INTERNAL_SERVER_ERROR,
            error_response("internal_error", INTERNAL_ERROR_MESSAGE),
        )
    code = "internal_error"
    message = INTERNAL_ERROR_MESSAGE
    details: tuple[str, ...] = ()
    if error.status_code == status.HTTP_404_NOT_FOUND:
        code = "not_found"
        message = UNKNOWN_ROUTE_MESSAGE
    elif error.status_code == status.HTTP_405_METHOD_NOT_ALLOWED:
        code = "method_not_allowed"
        message = METHOD_NOT_ALLOWED_MESSAGE
    elif error.status_code == PAYLOAD_TOO_LARGE_STATUS:
        code = "payload_too_large"
        message = PAYLOAD_TOO_LARGE_MESSAGE
    elif status.HTTP_400_BAD_REQUEST <= error.status_code < status.HTTP_500_INTERNAL_SERVER_ERROR:
        code = "request_rejected"
        message = REQUEST_REJECTED_MESSAGE
        safe = _safe_detail(error.detail)
        if safe is not None:
            details = (safe,)
    elif error.status_code >= status.HTTP_500_INTERNAL_SERVER_ERROR:
        code = "internal_error"
        message = INTERNAL_ERROR_MESSAGE
    return _json(error.status_code, error_response(code, message, details))


async def handle_unexpected_error(request: Request, error: Exception) -> JSONResponse:
    """Fail closed: report the failure type locally and return a bare 500."""
    LOGGER.error("unhandled studio failure: %s", type(error).__name__)
    return _json(
        status.HTTP_500_INTERNAL_SERVER_ERROR,
        error_response("internal_error", INTERNAL_ERROR_MESSAGE),
    )


def install_error_handlers(app: FastAPI) -> None:
    """Install every error handler the studio relies on."""
    app.add_exception_handler(RequestValidationError, handle_request_validation)
    app.add_exception_handler(StudioAPIError, handle_studio_api_error)
    app.add_exception_handler(StarletteHTTPException, handle_http_exception)
    app.add_exception_handler(Exception, handle_unexpected_error)


__all__ = (
    "INTERNAL_ERROR_MESSAGE",
    "MAX_DETAIL_ENTRY_LENGTH",
    "MAX_VALIDATION_DETAILS",
    "METHOD_NOT_ALLOWED_MESSAGE",
    "PAYLOAD_TOO_LARGE_MESSAGE",
    "PAYLOAD_TOO_LARGE_STATUS",
    "REQUEST_REJECTED_MESSAGE",
    "UNKNOWN_ROUTE_MESSAGE",
    "VALIDATION_MESSAGE",
    "_CREDENTIAL_RE",
    "_safe_detail",
    "handle_http_exception",
    "handle_request_validation",
    "handle_studio_api_error",
    "handle_unexpected_error",
    "install_error_handlers",
)
