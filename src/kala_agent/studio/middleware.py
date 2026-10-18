"""Transport-level middleware for the studio HTTP adapter.

Three controls live here: the loopback Host guard, the per-launch bearer
token guard for mutating methods, and the same-origin guard that accepts an
exact match between the request ``Origin`` and the loopback authority in
``Host`` and refuses everything else. The same-origin policy is the only one
that lets a browser in the same origin drive the API; the other choices
(every-origin, wildcard, host-list) would either relinquish the boundary or
introduce a footgun a future change could trip on. The studio also enforces
a fixed security header set and serves a static-file directory without path
traversal. The ``/api/*`` prefix is reserved; the static fallback never
answers an API request.
"""

from __future__ import annotations

import json
import logging
import mimetypes
import posixpath
import secrets
import urllib.parse
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Final

from starlette.types import ASGIApp, Message, Receive, Scope, Send

from kala_agent.studio.boundary import (
    HOST_REFUSED_MESSAGE,
    STUDIO_MUTATING_METHODS,
    TOKEN_REQUIRED_MESSAGE,
    is_loopback_host,
    is_permitted_authority,
)
from kala_agent.studio.schemas import error_response

LOGGER: Final[logging.Logger] = logging.getLogger("kala_agent.studio")

MAX_REQUEST_BODY_BYTES: Final[int] = 262_144
MIN_REQUEST_BODY_BYTES: Final[int] = 1_024
MAX_HEADER_VALUE_LENGTH: Final[int] = 512
STUDIO_BEARER_REALM: Final[bytes] = b'Bearer realm="kala-studio"'

SECURITY_HEADERS: Final[Mapping[str, str]] = {
    "cache-control": "no-store",
    "content-security-policy": (
        "default-src 'self'; "
        "script-src 'self'; "
        "connect-src 'self'; "
        "style-src 'self' 'unsafe-inline'; "
        "img-src 'self' data: blob:; "
        "font-src 'self'; "
        "object-src 'none'; "
        "frame-ancestors 'none'; "
        "base-uri 'none'; "
        "form-action 'self'"
    ),
    "cross-origin-opener-policy": "same-origin",
    "cross-origin-resource-policy": "same-origin",
    "referrer-policy": "no-referrer",
    "x-content-type-options": "nosniff",
    "x-frame-options": "DENY",
}
STRIPPED_RESPONSE_HEADERS: Final[frozenset[bytes]] = frozenset({b"server", b"x-powered-by"})
STUDIO_API_PATH_PREFIX: Final[str] = "/api"
STUDIO_SPA_FALLBACK_NAME: Final[str] = "index.html"
ALLOWED_ORIGIN_SCHEMES: Final[frozenset[str]] = frozenset({"http", "https"})


class StudioAPIError(Exception):
    """An HTTP-bound error with a status code, error code, and message.

    Studio routes raise this error with a sanitized message; the error
    handler installed by :mod:`kala_agent.studio.errors` translates it into
    a JSON response that carries the security header set. The base class
    is generic; subclasses attach a status code and an error code so the
    route layer reads the type rather than re-parsing a string.
    """

    status_code: int = 500
    code: str = "internal_error"
    message: str = "the studio request could not be completed"

    def __init__(self, message: str | None = None) -> None:
        super().__init__(message or self.message)
        if message is not None:
            self.message = message


class StudioValidationError(StudioAPIError):
    status_code = 422
    code = "invalid_request"
    message = "the request body or parameters are invalid"


class StudioUnsupportedError(StudioAPIError):
    status_code = 503
    code = "unsupported"
    message = "this capability is not available in the current studio configuration"


class StudioUnauthorizedError(StudioAPIError):
    status_code = 401
    code = "unauthorized"
    message = TOKEN_REQUIRED_MESSAGE


def validate_max_body_bytes(value: int) -> int:
    """Return a usable studio request body limit."""
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError("max_body_bytes must be an integer")
    if not MIN_REQUEST_BODY_BYTES <= value <= MAX_REQUEST_BODY_BYTES:
        raise ValueError(
            f"max_body_bytes must be between {MIN_REQUEST_BODY_BYTES} and {MAX_REQUEST_BODY_BYTES}"
        )
    return value


def is_loopback_authority(authority: object) -> bool:
    """Return whether the supplied authority names the loopback interface."""
    return is_permitted_authority(authority)


def _header_values(scope: Scope, name: bytes) -> tuple[str, ...]:
    found: list[str] = []
    for key, value in scope.get("headers", ()):
        if key != name:
            continue
        try:
            found.append(value.decode("latin-1")[:MAX_HEADER_VALUE_LENGTH])
        except UnicodeDecodeError:
            return ()
    return tuple(found)


def _header_value(scope: Scope, name: bytes) -> str | None:
    values = _header_values(scope, name)
    return values[0] if values else None


def _declared_length(scope: Scope) -> int | None:
    raw = _header_value(scope, b"content-length")
    if raw is None:
        return None
    candidate = raw.strip()
    if not candidate.isascii() or not candidate.isdigit():
        return None
    return int(candidate)


async def _buffer_body(receive: Receive, limit: int) -> tuple[bytes | None, bool]:
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


async def _send_refusal(
    send: Send,
    *,
    status_code: int,
    code: str,
    message: str,
    details: Sequence[str] = (),
    extra_headers: Sequence[tuple[bytes, bytes]] = (),
) -> None:
    body_text = json.dumps(
        error_response(code, message, tuple(details)).model_dump(mode="json"),
        separators=(",", ":"),
        sort_keys=True,
    )
    body = body_text.encode("utf-8")
    headers: list[tuple[bytes, bytes]] = [
        (b"content-length", str(len(body)).encode("latin-1")),
        (b"content-type", b"application/json"),
    ]
    headers.extend(extra_headers)
    for name, value in SECURITY_HEADERS.items():
        headers.append((name.encode("latin-1"), value[:MAX_HEADER_VALUE_LENGTH].encode("latin-1")))
    await send({"type": "http.response.start", "status": status_code, "headers": headers})
    await send({"type": "http.response.body", "body": body})


class HostGuardMiddleware:
    """Refuse a request whose Host header does not name the loopback interface."""

    def __init__(self, app: ASGIApp) -> None:
        self._app = app

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            await self._app(scope, receive, send)
            return
        hosts = _header_values(scope, b"host")
        if len(hosts) != 1 or not is_loopback_authority(hosts[0]):
            await self._refuse(send)
            return
        await self._app(scope, receive, send)

    async def _refuse(self, send: Send) -> None:
        await _send_refusal(
            send,
            status_code=421,
            code="invalid_host",
            message=HOST_REFUSED_MESSAGE,
        )


def _parse_origin_header(raw: str) -> tuple[str, str, int] | None:
    """Parse an ``Origin`` header into ``(scheme, host, port)``.

    The function refuses anything that is not exactly a triple of scheme,
    host, and port: no path, no query, no fragment, no userinfo, no scheme
    other than HTTP/HTTPS. Null origins (``"null"``) and origins with more
    than one slash pair are explicitly rejected, which prevents a browser
    sandbox from faking same-origin.
    """
    candidate = raw.strip()
    if not candidate or candidate.lower() == "null":
        return None
    parsed = urllib.parse.urlparse(candidate)
    if parsed.params or parsed.query or parsed.fragment or parsed.username or parsed.password:
        return None
    if parsed.scheme.lower() not in ALLOWED_ORIGIN_SCHEMES:
        return None
    if not parsed.hostname:
        return None
    if parsed.path not in ("", "/"):
        return None
    host = parsed.hostname.lower()
    if not host or len(host) > MAX_HEADER_VALUE_LENGTH:
        return None
    port = parsed.port
    if port is None:
        port = 443 if parsed.scheme.lower() == "https" else 80
    return parsed.scheme.lower(), host, port


def origins_match(origin: tuple[str, str, int], authority: tuple[str, str, int]) -> bool:
    """Return whether a parsed origin and a parsed authority match exactly."""
    return origin[0] == authority[0] and origin[1] == authority[1] and origin[2] == authority[2]


def _parse_authority(authority: str) -> tuple[str, str, int] | None:
    """Parse an HTTP authority (Host header) into ``(scheme, host, port)``.

    The Host header carries no scheme, so this function assumes ``http`` for
    the loopback case the studio serves. The port defaults to 80 only when
    no explicit port is present; the studio serves 127.0.0.1:8765 by
    default.
    """
    raw = authority.strip()
    parsed = urllib.parse.urlparse("//" + raw)
    host = parsed.hostname
    if not host or not is_loopback_host(host):
        return None
    if parsed.username or parsed.password or parsed.path not in ("", "/"):
        return None
    port = parsed.port
    if port is None:
        port = 80
    return "http", host.lower(), port


def origin_matches_host(scope: Scope) -> bool:
    """Return whether the ``Origin`` header matches the ``Host`` exactly.

    The function tolerates requests that carry no ``Origin`` (they reach
    the bearer-token guard, which still demands a token for mutating
    methods). Browsers always send an Origin on POST, so the rule remains
    strict for the request body the studio actually serves.
    """
    origin_values = _header_values(scope, b"origin")
    if not origin_values:
        return True
    if len(origin_values) != 1:
        return False
    parsed_origin = _parse_origin_header(origin_values[0])
    if parsed_origin is None:
        return False
    host_values = _header_values(scope, b"host")
    if len(host_values) != 1:
        return False
    parsed_authority = _parse_authority(host_values[0])
    if parsed_authority is None:
        return False
    return origins_match(parsed_origin, parsed_authority)


class SameOriginGuardMiddleware:
    """Refuse a request whose ``Origin`` is not an exact same-origin match.

    The middleware runs *after* :class:`HostGuardMiddleware` so a request
    with a non-loopback host is already refused; this layer compares the
    loopback authority in ``Host`` to the parsed ``Origin`` triple (scheme,
    host, port) and accepts only an exact match. Anything else — including
    null origins, multiple Origin headers, mismatched ports, and https/http
    mixing — is refused without a CORS-style preflight, because the studio
    has no cross-origin clients to support.
    """

    def __init__(self, app: ASGIApp) -> None:
        self._app = app

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            await self._app(scope, receive, send)
            return
        if not origin_matches_host(scope):
            await self._refuse(send)
            return
        await self._app(scope, receive, send)

    async def _refuse(self, send: Send) -> None:
        await _send_refusal(
            send,
            status_code=403,
            code="cross_origin_denied",
            message="the studio serves no cross-origin client",
        )


class MaxBodySizeMiddleware:
    """Refuse an oversized request body before the router runs."""

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
        await _send_refusal(
            send,
            status_code=413,
            code="payload_too_large",
            message="the request body exceeds the configured size limit",
            details=(f"the request body must not exceed {limit} bytes",),
        )


class SecurityHeadersMiddleware:
    """Add the studio security header set to every response."""

    def __init__(
        self,
        app: ASGIApp,
        *,
        headers: Mapping[str, str] | None = None,
    ) -> None:
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


class BearerTokenMiddleware:
    """Require the studio token on every mutating method."""

    def __init__(self, app: ASGIApp, *, api_token: str) -> None:
        from kala_agent.studio.boundary import validate_studio_token

        self._app = app
        self._expected = validate_studio_token(api_token).encode("utf-8")

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        method = scope.get("method") if scope["type"] == "http" else None
        if not isinstance(method, str) or method.upper() not in STUDIO_MUTATING_METHODS:
            await self._app(scope, receive, send)
            return
        presented = _presented_credential(scope)
        if presented is None or not secrets.compare_digest(presented, self._expected):
            await _send_refusal(
                send,
                status_code=401,
                code="unauthorized",
                message=TOKEN_REQUIRED_MESSAGE,
                extra_headers=((b"www-authenticate", STUDIO_BEARER_REALM),),
            )
            return
        await self._app(scope, receive, send)


def _presented_credential(scope: Scope) -> bytes | None:
    values = _header_values(scope, b"authorization")
    if len(values) != 1:
        return None
    scheme, separator, credential = values[0].partition(" ")
    if not separator or scheme.lower() != "bearer":
        return None
    token = credential.strip()
    if not token or len(token) > MAX_HEADER_VALUE_LENGTH:
        return None
    return token.encode("utf-8")


class StaticFileMiddleware:
    """Serve a server-controlled static directory without path traversal."""

    def __init__(
        self,
        app: ASGIApp,
        *,
        root: Path | None,
        fallback_filename: str = STUDIO_SPA_FALLBACK_NAME,
        api_prefix: str = STUDIO_API_PATH_PREFIX,
    ) -> None:
        self._app = app
        self._api_prefix = api_prefix.rstrip("/")
        self._fallback_filename = fallback_filename
        self._root: Path | None = None
        self._root_resolved: Path | None = None
        if root is not None:
            resolved = root.resolve()
            if resolved.is_dir():
                self._root = resolved
                self._root_resolved = resolved

    @property
    def root(self) -> Path | None:
        """Return the configured static root, or None when serving is off."""
        return self._root

    def _safe_join(self, requested: str) -> Path | None:
        """Resolve ``requested`` under the static root, refusing traversal."""
        if self._root is None or self._root_resolved is None:
            return None
        relative = requested.lstrip("/")
        if not relative:
            return self._root / self._fallback_filename
        candidate = (self._root / relative).resolve()
        try:
            candidate.relative_to(self._root_resolved)
        except ValueError:
            return None
        return candidate

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            await self._app(scope, receive, send)
            return
        if self._root is None:
            await self._app(scope, receive, send)
            return
        method = scope.get("method")
        if method not in {"GET", "HEAD"}:
            await self._app(scope, receive, send)
            return
        path = scope.get("path", "")
        if path == self._api_prefix or path.startswith(f"{self._api_prefix}/"):
            await self._app(scope, receive, send)
            return
        raw = scope.get("raw_path") or path.encode("utf-8")
        try:
            decoded = raw.decode("utf-8") if isinstance(raw, bytes) else str(raw)
        except UnicodeDecodeError:
            await self._not_found(send)
            return
        clean_path, _, _ = decoded.partition("?")
        normalised = posixpath.normpath(clean_path)
        if normalised == ".." or normalised.startswith("../") or "/../" in normalised:
            await self._not_found(send)
            return
        target = self._safe_join(normalised)
        if target is None:
            await self._not_found(send)
            return
        if target.is_dir():
            target = target / self._fallback_filename
        if not target.is_file():
            fallback = self._root / self._fallback_filename if self._root else None
            if fallback is None or not fallback.is_file():
                await self._not_found(send)
                return
            target = fallback
        await self._serve_file(target, method=method, send=send)

    async def _serve_file(self, path: Path, *, method: str, send: Send) -> None:
        import asyncio

        try:
            data = await asyncio.to_thread(path.read_bytes)
        except OSError:
            await self._not_found(send)
            return
        media_type, _ = mimetypes.guess_type(str(path))
        if media_type is None:
            media_type = "application/octet-stream"
        if not media_type.startswith("text/") and not media_type.startswith("application/"):
            media_type = "application/octet-stream"
        headers: list[tuple[bytes, bytes]] = [
            (b"content-length", str(len(data)).encode("latin-1")),
            (b"content-type", media_type.encode("latin-1")),
        ]
        for name, value in SECURITY_HEADERS.items():
            headers.append(
                (name.encode("latin-1"), value[:MAX_HEADER_VALUE_LENGTH].encode("latin-1"))
            )
        await send({"type": "http.response.start", "status": 200, "headers": headers})
        if method == "HEAD":
            await send({"type": "http.response.body", "body": b""})
        else:
            await send({"type": "http.response.body", "body": data})

    async def _not_found(self, send: Send) -> None:
        body_text = json.dumps(
            error_response("not_found", "the requested resource does not exist").model_dump(
                mode="json"
            ),
            separators=(",", ":"),
            sort_keys=True,
        )
        body = body_text.encode("utf-8")
        headers: list[tuple[bytes, bytes]] = [
            (b"content-length", str(len(body)).encode("latin-1")),
            (b"content-type", b"application/json"),
        ]
        for name, value in SECURITY_HEADERS.items():
            headers.append(
                (name.encode("latin-1"), value[:MAX_HEADER_VALUE_LENGTH].encode("latin-1"))
            )
        await send({"type": "http.response.start", "status": 404, "headers": headers})
        await send({"type": "http.response.body", "body": body})


def build_security_headers() -> tuple[tuple[bytes, bytes], ...]:
    """Return the studio security header pairs for response tweaking."""
    return tuple(
        (name.encode("latin-1"), value[:MAX_HEADER_VALUE_LENGTH].encode("latin-1"))
        for name, value in SECURITY_HEADERS.items()
    )


__all__ = (
    "ALLOWED_ORIGIN_SCHEMES",
    "LOGGER",
    "MAX_HEADER_VALUE_LENGTH",
    "MAX_REQUEST_BODY_BYTES",
    "MIN_REQUEST_BODY_BYTES",
    "SECURITY_HEADERS",
    "STUDIO_API_PATH_PREFIX",
    "STUDIO_BEARER_REALM",
    "STUDIO_SPA_FALLBACK_NAME",
    "BearerTokenMiddleware",
    "HostGuardMiddleware",
    "MaxBodySizeMiddleware",
    "SameOriginGuardMiddleware",
    "SecurityHeadersMiddleware",
    "StaticFileMiddleware",
    "StudioAPIError",
    "StudioUnauthorizedError",
    "StudioUnsupportedError",
    "StudioValidationError",
    "build_security_headers",
    "is_loopback_authority",
    "is_loopback_host",
    "origin_matches_host",
    "origins_match",
    "validate_max_body_bytes",
)
