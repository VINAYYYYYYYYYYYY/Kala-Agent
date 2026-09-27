"""Transport-level middleware for the local Kala HTTP adapter.

Two controls live here and nothing else: a request body size limit that answers
every method, and a fixed set of security headers on every response. The rest of
the request boundary, loopback addressing, the launch token, and cross-origin
refusals, lives in :mod:`kala_agent.api.boundary`. No middleware here
authenticates a caller, inspects a payload for a business rule, or reaches the
network.
"""

from __future__ import annotations

import json
from collections.abc import Mapping, Sequence
from typing import Final

from starlette.types import ASGIApp, Message, Receive, Scope, Send

from kala_agent.api.schemas import error_response

MAX_REQUEST_BODY_BYTES: Final[int] = 262_144
MIN_REQUEST_BODY_BYTES: Final[int] = 1_024
MAX_HEADER_VALUE_LENGTH: Final[int] = 512
SECURITY_HEADERS: Final[Mapping[str, str]] = {
    "cache-control": "no-store",
    "content-security-policy": "default-src 'none'; frame-ancestors 'none'; base-uri 'none'",
    "cross-origin-opener-policy": "same-origin",
    "cross-origin-resource-policy": "same-origin",
    "referrer-policy": "no-referrer",
    "x-content-type-options": "nosniff",
    "x-frame-options": "DENY",
}
STRIPPED_RESPONSE_HEADERS: Final[frozenset[bytes]] = frozenset({b"server", b"x-powered-by"})


def validate_max_body_bytes(value: int) -> int:
    """Return a usable request body limit."""
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError("max_body_bytes must be an integer")
    if not MIN_REQUEST_BODY_BYTES <= value <= MAX_REQUEST_BODY_BYTES:
        raise ValueError(
            f"max_body_bytes must be between {MIN_REQUEST_BODY_BYTES} and {MAX_REQUEST_BODY_BYTES}"
        )
    return value


def refusal_body(code: str, message: str, details: Sequence[str] = ()) -> bytes:
    """Return the one sanitized JSON body every transport refusal uses."""
    payload = error_response(code, message, tuple(details)).model_dump(mode="json")
    return json.dumps(payload, separators=(",", ":"), sort_keys=True).encode("utf-8")


async def send_refusal(
    send: Send,
    *,
    status_code: int,
    code: str,
    message: str,
    details: Sequence[str] = (),
    extra_headers: Sequence[tuple[bytes, bytes]] = (),
) -> None:
    """Answer from the transport with a bounded body and the full header set.

    A refusal raised below the router never reaches the error handlers, so it
    carries the security headers itself. The body is the same sanitized shape
    every other error uses, and it echoes nothing that was submitted.
    """
    body = refusal_body(code, message, details)
    headers: list[tuple[bytes, bytes]] = [
        (b"content-length", str(len(body)).encode("latin-1")),
        (b"content-type", b"application/json"),
    ]
    headers.extend(extra_headers)
    headers.extend(
        (name.encode("latin-1"), value.encode("latin-1"))
        for name, value in SECURITY_HEADERS.items()
    )
    await send({"type": "http.response.start", "status": status_code, "headers": headers})
    await send({"type": "http.response.body", "body": body})


def header_values(scope: Scope, name: bytes) -> tuple[str, ...]:
    """Return every value sent for one header, bounded and safely decoded."""
    found: list[str] = []
    for key, value in scope.get("headers", ()):
        if key != name:
            continue
        try:
            found.append(value.decode("latin-1")[:MAX_HEADER_VALUE_LENGTH])
        except UnicodeDecodeError:
            return ()
    return tuple(found)


def header_value(scope: Scope, name: bytes) -> str | None:
    """Return the first value sent for one header, or None when it is absent."""
    values = header_values(scope, name)
    return values[0] if values else None


def _declared_length(scope: Scope) -> int | None:
    raw = header_value(scope, b"content-length")
    if raw is None:
        return None
    candidate = raw.strip()
    if not candidate.isascii() or not candidate.isdigit():
        return None
    return int(candidate)


async def _buffer_body(
    receive: Receive,
    limit: int,
) -> tuple[bytes | None, bool]:
    collected = bytearray()
    while True:
        message = await receive()
        if message["type"] == "http.disconnect":
            return None, False
        chunk = message.get("body", b"")
        if len(collected) + len(chunk) > limit:
            return None, True
        collected.extend(chunk)
        if not message.get("more_body", False):
            return bytes(collected), False


class MaxBodySizeMiddleware:
    """Refuse an oversized request body before any route handler runs.

    The limit answers every method, not only the ones that carry a payload, and
    it does not depend on a declared length: a declared content-length is
    checked first, then the body is buffered up to the limit and replayed, so a
    chunked request without a declared length cannot bypass it either. The bound
    is small enough that buffering is cheaper than trusting the framing.
    """

    def __init__(self, app: ASGIApp, *, max_body_bytes: int = MAX_REQUEST_BODY_BYTES) -> None:
        self._app = app
        self._max_body_bytes = validate_max_body_bytes(max_body_bytes)

    @property
    def max_body_bytes(self) -> int:
        """Return the enforced request body limit in bytes."""
        return self._max_body_bytes

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            await self._app(scope, receive, send)
            return
        limit = self._max_body_bytes
        declared = _declared_length(scope)
        if declared is not None and declared > limit:
            await self._too_large(send, limit)
            return
        body, exceeded = await _buffer_body(receive, limit)
        if exceeded:
            await self._too_large(send, limit)
            return
        if body is None:
            return

        async def replayed_receive() -> Message:
            return {"type": "http.request", "body": body, "more_body": False}

        await self._app(scope, replayed_receive, send)

    async def _too_large(self, send: Send, limit: int) -> None:
        await send_refusal(
            send,
            status_code=413,
            code="payload_too_large",
            message="the request body exceeds the configured size limit",
            details=(f"the request body must not exceed {limit} bytes",),
        )


class SecurityHeadersMiddleware:
    """Add a fixed set of security headers to every response.

    Headers the application already set are preserved, and headers that
    advertise the implementation are removed rather than echoed.
    """

    def __init__(self, app: ASGIApp, *, headers: Mapping[str, str] | None = None) -> None:
        self._app = app
        source = SECURITY_HEADERS if headers is None else headers
        self._headers: tuple[tuple[bytes, bytes], ...] = tuple(
            (
                name.lower().encode("latin-1"),
                value[:MAX_HEADER_VALUE_LENGTH].encode("latin-1"),
            )
            for name, value in source.items()
        )

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            await self._app(scope, receive, send)
            return

        async def send_with_headers(message: Message) -> None:
            if message["type"] != "http.response.start":
                await send(message)
                return
            retained = [
                (key, value)
                for key, value in message.get("headers", ())
                if key.lower() not in STRIPPED_RESPONSE_HEADERS
            ]
            present = {key.lower() for key, _ in retained}
            await send(
                {
                    **message,
                    "headers": retained
                    + [(key, value) for key, value in self._headers if key not in present],
                }
            )

        await self._app(scope, receive, send_with_headers)


__all__ = (
    "MAX_REQUEST_BODY_BYTES",
    "MIN_REQUEST_BODY_BYTES",
    "SECURITY_HEADERS",
    "MaxBodySizeMiddleware",
    "SecurityHeadersMiddleware",
    "header_value",
    "header_values",
    "refusal_body",
    "send_refusal",
    "validate_max_body_bytes",
)
