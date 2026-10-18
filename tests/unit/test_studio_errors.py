"""Regression tests for the studio HTTP error handlers.

The handlers are the last line of defence for the studio envelope: every
framework failure must leave a flat ``{code,message,details}`` body and
must never echo a submitted credential. These tests call the handlers
directly for status codes that the route tests do not exercise, and assert
the sanitised shape.
"""

from __future__ import annotations

import asyncio

import pytest
from fastapi import HTTPException
from fastapi.responses import JSONResponse

from kala_agent.studio.errors import (
    INTERNAL_ERROR_MESSAGE,
    REQUEST_REJECTED_MESSAGE,
    _safe_detail,
    handle_http_exception,
)

CREDENTIAL_SAMPLE = "sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"


def _run(error: Exception) -> JSONResponse:
    return asyncio.run(handle_http_exception(None, error))  # type: ignore[arg-type]


@pytest.mark.parametrize("status_code", (400, 401, 403, 409, 418))
def test_generic_4xx_uses_sanitized_message(status_code: int) -> None:
    response = _run(HTTPException(status_code, "some internal detail"))
    assert response.status_code == status_code
    body = response.body.decode("utf-8")
    assert REQUEST_REJECTED_MESSAGE in body
    assert "code" in body and "message" in body and "details" in body


@pytest.mark.parametrize("status_code", (500, 501, 502, 503, 504))
def test_5xx_never_echoes_framework_detail(status_code: int) -> None:
    response = _run(HTTPException(status_code, "traceback-worthy internal detail"))
    assert response.status_code == status_code
    body = response.body.decode("utf-8")
    assert "internal detail" not in body
    assert INTERNAL_ERROR_MESSAGE in body


def test_4xx_detail_carrying_a_credential_is_dropped() -> None:
    response = _run(HTTPException(400, f"rejected token {CREDENTIAL_SAMPLE}"))
    body = response.body.decode("utf-8")
    assert CREDENTIAL_SAMPLE not in body
    assert "rejected token" not in body


def test_404_uses_not_found_message() -> None:
    response = _run(HTTPException(404, "no such route"))
    assert response.status_code == 404
    body = response.body.decode("utf-8")
    assert "not_found" in body
    assert "no such route" not in body


def test_405_uses_method_not_allowed_message() -> None:
    response = _run(HTTPException(405, "wrong verb"))
    assert response.status_code == 405
    body = response.body.decode("utf-8")
    assert "method_not_allowed" in body


def test_413_uses_payload_too_large_message() -> None:
    response = _run(HTTPException(413, "too big"))
    assert response.status_code == 413
    body = response.body.decode("utf-8")
    assert "payload_too_large" in body


def test_non_http_exception_is_internal_error() -> None:
    response = _run(RuntimeError("boom"))
    assert response.status_code == 500
    assert INTERNAL_ERROR_MESSAGE in response.body.decode("utf-8")


def test_safe_detail_rejects_credentials_and_long_text() -> None:
    assert _safe_detail("a short harmless note") == "a short harmless note"
    assert _safe_detail(CREDENTIAL_SAMPLE) is None
    assert _safe_detail("Bearer " + "a" * 40) is None
    assert _safe_detail("x" * 200) is None
    assert _safe_detail(None) is None
    assert _safe_detail(1234) is None
