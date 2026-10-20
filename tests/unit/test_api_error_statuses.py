"""Regression tests for the statuses the local API error handler answers.

``kala_agent.api.errors.handle_http_exception`` used to compare the status of
an ``HTTPException`` against ``status.HTTP_500_BAD_REQUEST``, a name Starlette
does not define. Every status of 400 or more that is not 404, 405 or 413
therefore raised an ``AttributeError`` inside the handler, the server error
middleware answered a generic 500, and the caller never learned the status it
had asked for.

These tests raise the exception from a real route, so the whole handler path is
exercised: the framework dispatch, the status, the code and the bounded
message. The status of a refused request is part of the contract of this API,
so it is asserted rather than assumed.
"""

from __future__ import annotations

from collections.abc import Iterator

import pytest
from fastapi import FastAPI, HTTPException
from fastapi.testclient import TestClient

from kala_agent.api.errors import (
    INTERNAL_ERROR_MESSAGE,
    METHOD_NOT_ALLOWED_MESSAGE,
    UNKNOWN_ROUTE_MESSAGE,
    install_error_handlers,
)

LOOPBACK_BASE_URL = "http://127.0.0.1"
REJECTED_STATUSES = (400, 401, 403, 409, 418, 429, 499)
SERVER_STATUSES = (500, 501, 502, 503, 504)
PAYLOAD_TOO_LARGE_MESSAGE = "the request body exceeds the configured size limit"
REJECTED_MESSAGE = "the request was rejected"


@pytest.fixture(autouse=True)
def _forbid_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    def blocked(*args: object, **kwargs: object) -> None:
        raise AssertionError("the local API must not open network connections")

    monkeypatch.setattr("socket.socket.connect", blocked)
    monkeypatch.setattr("socket.socket.connect_ex", blocked)
    monkeypatch.setattr("socket.create_connection", blocked)
    yield


def _client_for(status_code: int, detail: str) -> TestClient:
    """Return a client whose only route raises one HTTPException.

    The application carries the real error handlers and nothing else, so the
    assertion is about this module and not about the routes around it.
    """
    app = FastAPI()

    @app.get("/rejected")
    async def rejected() -> dict[str, bool]:
        raise HTTPException(status_code=status_code, detail=detail)

    install_error_handlers(app)
    return TestClient(
        app,
        base_url=LOOPBACK_BASE_URL,
        raise_server_exceptions=False,
    )


@pytest.mark.parametrize("status_code", REJECTED_STATUSES)
def test_a_rejected_request_keeps_its_status(status_code: int) -> None:
    with _client_for(status_code, "the request was refused") as client:
        response = client.get("/rejected")
    assert response.status_code == status_code
    body = response.json()
    assert body["error"]["code"] == "request_rejected"
    assert body["error"]["message"] == "the request was refused"


def test_the_detail_is_truncated_to_128_characters() -> None:
    with _client_for(400, "d" * 200) as client:
        response = client.get("/rejected")
    assert response.status_code == 400
    assert response.json()["error"]["message"] == "d" * 128


def test_an_empty_detail_answers_with_the_generic_message() -> None:
    with _client_for(400, "") as client:
        response = client.get("/rejected")
    assert response.status_code == 400
    assert response.json()["error"]["message"] == REJECTED_MESSAGE


@pytest.mark.parametrize(
    ("status_code", "code", "message"),
    (
        (404, "not_found", UNKNOWN_ROUTE_MESSAGE),
        (405, "method_not_allowed", METHOD_NOT_ALLOWED_MESSAGE),
        (413, "payload_too_large", PAYLOAD_TOO_LARGE_MESSAGE),
    ),
)
def test_the_routing_statuses_keep_their_own_code(
    status_code: int,
    code: str,
    message: str,
) -> None:
    with _client_for(status_code, "a detail the handler must not echo") as client:
        response = client.get("/rejected")
    assert response.status_code == status_code
    body = response.json()
    assert body["error"]["code"] == code
    assert body["error"]["message"] == message


@pytest.mark.parametrize("status_code", SERVER_STATUSES)
def test_a_server_status_answers_with_the_internal_error_code(status_code: int) -> None:
    with _client_for(status_code, "an internal detail") as client:
        response = client.get("/rejected")
    assert response.status_code == status_code
    body = response.json()
    assert body["error"]["code"] == "internal_error"
    assert body["error"]["message"] == INTERNAL_ERROR_MESSAGE
    assert "an internal detail" not in response.text


def test_a_status_below_four_hundred_answers_with_the_internal_error_code() -> None:
    with _client_for(301, "moved") as client:
        response = client.get("/rejected")
    assert response.status_code == 301
    assert response.json()["error"]["code"] == "internal_error"


def test_the_answer_carries_no_traceback_and_no_detail_of_a_server_failure() -> None:
    with _client_for(500, "an internal detail") as client:
        response = client.get("/rejected")
    assert "Traceback" not in response.text
    assert "Exception" not in response.text
    assert response.json()["details"] == []
