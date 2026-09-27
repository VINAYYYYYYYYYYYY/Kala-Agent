"""The request boundary of the local Kala HTTP adapter.

Three controls live here and nothing else: the loopback Host guard that refuses a
rebound name, the per-launch bearer token, and the cross-origin guard that
refuses every browser-originated request. None of them reaches the network,
inspects a payload for a business rule, or writes a row.

The launch token is minted once per launch, stored in the local data directory
with owner-only permissions, and required on every mutating method. It is never
logged, never returned in a response, and never echoed in an error, and a
refusal says only that a valid token is required. Reads stay open to the caller
that already has the data directory, which is the trust model this process has
always claimed: one local operator, no multi-user isolation.
"""

from __future__ import annotations

import ipaddress
import logging
import os
import re
import secrets
from pathlib import Path
from typing import Final

from starlette.types import ASGIApp, Receive, Scope, Send

from kala_agent.api.middleware import (
    header_value,
    header_values,
    send_refusal,
)

LOGGER: Final[logging.Logger] = logging.getLogger("kala_agent.api")
API_TOKEN_FILE_NAME: Final[str] = "api-token"
API_TOKEN_BYTES: Final[int] = 32
API_TOKEN_FILE_MODE: Final[int] = 0o600
MIN_API_TOKEN_LENGTH: Final[int] = 16
MAX_API_TOKEN_LENGTH: Final[int] = 128
API_TOKEN_PATTERN: Final[str] = r"^[A-Za-z0-9._~-]+$"
_API_TOKEN_RE: Final[re.Pattern[str]] = re.compile(API_TOKEN_PATTERN)
MUTATING_METHODS: Final[frozenset[str]] = frozenset({"DELETE", "PATCH", "POST", "PUT"})
AUTH_SCHEME: Final[str] = "bearer"
AUTH_CHALLENGE: Final[tuple[bytes, bytes]] = (b"www-authenticate", b'Bearer realm="kala-agent"')
MAX_AUTHORITY_LENGTH: Final[int] = 256
LOOPBACK_HOSTNAMES: Final[frozenset[str]] = frozenset({"localhost", "127.0.0.1", "::1"})
WILDCARD_AUTHORITIES: Final[frozenset[str]] = frozenset(
    {"0.0.0.0", "::", "[::]", "[::0]", "0:0:0:0:0:0:0:0", "*"}
)
TOKEN_REQUIRED_MESSAGE: Final[str] = "a valid bearer token is required for this request"
HOST_REFUSED_MESSAGE: Final[str] = "the request host is not a loopback address of this process"
ORIGIN_REFUSED_MESSAGE: Final[str] = "this api serves no cross-origin client"


def is_loopback_host(host: object) -> bool:
    """Return whether a bare hostname or address names the loopback interface.

    A wildcard name is not loopback. ``0.0.0.0`` and ``::`` name this machine
    without naming the interface a request arrived on, so they are refused
    explicitly rather than by accident, and a name that is not a loopback
    address at all is refused as well.
    """
    if not isinstance(host, str):
        return False
    candidate = host.lower()
    if not candidate or len(candidate) > MAX_AUTHORITY_LENGTH:
        return False
    if candidate in WILDCARD_AUTHORITIES:
        return False
    if candidate in LOOPBACK_HOSTNAMES:
        return True
    try:
        address = ipaddress.ip_address(candidate)
    except ValueError:
        return False
    return address.is_loopback and not address.is_unspecified


def _split_authority(authority: str) -> tuple[str, str | None] | None:
    if not authority or len(authority) > MAX_AUTHORITY_LENGTH:
        return None
    if any(character.isspace() for character in authority):
        return None
    port: str | None = None
    if authority.startswith("["):
        closing = authority.find("]")
        if closing < 0:
            return None
        host = authority[1:closing]
        remainder = authority[closing + 1 :]
        if remainder:
            if not remainder.startswith(":"):
                return None
            port = remainder[1:]
    elif ":" in authority:
        host, separator, port = authority.rpartition(":")
        if not separator or ":" in host:
            return None
    else:
        host = authority
    if not host:
        return None
    if port is not None and not (port.isascii() and port.isdigit() and 1 <= int(port) <= 65_535):
        return None
    return host, port


def is_permitted_authority(authority: object) -> bool:
    """Return whether a Host header names this process on the loopback interface.

    The optional port is accepted and ignored, because the interface is what the
    trust model is about. Everything else is refused: a userinfo section, a path,
    a query, a fragment, a non-numeric or out-of-range port, an empty authority,
    a trailing dot, and any name that is not loopback.
    """
    if not isinstance(authority, str):
        return False
    parts = _split_authority(authority)
    if parts is None:
        return False
    return is_loopback_host(parts[0])


class HostGuardMiddleware:
    """Refuse a request whose Host header does not name the loopback interface.

    A browser resolves the name it sends, so a rebound name reaches this process
    carrying an attacker-chosen header. Comparing that header against loopback
    is what stops it, and it is why the check happens before a body is read, a
    route is resolved, or the application is touched. A request that declares
    more than one Host is refused too, because the browser sends exactly one.
    """

    def __init__(self, app: ASGIApp) -> None:
        self._app = app

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            await self._app(scope, receive, send)
            return
        hosts = header_values(scope, b"host")
        if len(hosts) != 1 or not is_permitted_authority(hosts[0]):
            await self._refuse(send)
            return
        await self._app(scope, receive, send)

    async def _refuse(self, send: Send) -> None:
        await send_refusal(
            send,
            status_code=421,
            code="invalid_host",
            message=HOST_REFUSED_MESSAGE,
        )


def validate_api_token(value: object) -> str:
    """Return a usable bearer token, or refuse one that is malformed."""
    if not isinstance(value, str):
        raise ValueError("the api token must be a string")
    if not MIN_API_TOKEN_LENGTH <= len(value) <= MAX_API_TOKEN_LENGTH:
        raise ValueError(
            f"the api token must be between {MIN_API_TOKEN_LENGTH} and {MAX_API_TOKEN_LENGTH} "
            "characters"
        )
    if _API_TOKEN_RE.fullmatch(value) is None:
        raise ValueError("the api token contains unsupported characters")
    return value


def _store_api_token(data_dir: Path, token: str) -> None:
    path = data_dir / API_TOKEN_FILE_NAME
    flags = os.O_WRONLY | os.O_CREAT | os.O_TRUNC | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags, API_TOKEN_FILE_MODE)
    with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
        os.fchmod(handle.fileno(), API_TOKEN_FILE_MODE)
        handle.write(f"{token}\n")


def issue_api_token(data_dir: Path) -> str:
    """Return a fresh launch token and store it in the local data directory.

    A launch invalidates the previous token by replacing the file. When the
    directory is unusable the token still guards this process, held in memory
    only, and the failure is reported by type. The token itself is never
    written to a log, a response, or an error.
    """
    token = secrets.token_urlsafe(API_TOKEN_BYTES)
    try:
        _store_api_token(data_dir, token)
    except OSError as error:
        LOGGER.warning(
            "the launch token was not stored in the local data directory: %s",
            type(error).__name__,
        )
    return token


def read_api_token(data_dir: Path) -> str | None:
    """Return the token stored for the running launch, or None when there is none.

    This is the retrieval path for the local operator: the CLI reads the file
    this process wrote. Only a bounded prefix is read, and a missing, unreadable,
    over-long, or malformed file reads as None rather than raising, so a corrupt
    store is reported as an absence.
    """
    try:
        with (data_dir / API_TOKEN_FILE_NAME).open("rb") as handle:
            raw = handle.read(MAX_API_TOKEN_LENGTH + 2)
    except OSError:
        return None
    try:
        candidate = raw.decode("utf-8").strip()
    except UnicodeDecodeError:
        return None
    try:
        return validate_api_token(candidate)
    except ValueError:
        return None


def _presented_credential(scope: Scope) -> bytes | None:
    values = header_values(scope, b"authorization")
    if len(values) != 1:
        return None
    scheme, separator, credential = values[0].partition(" ")
    if not separator or scheme.lower() != AUTH_SCHEME:
        return None
    token = credential.strip()
    if not token or len(token) > MAX_API_TOKEN_LENGTH:
        return None
    return token.encode("utf-8")


class BearerTokenMiddleware:
    """Require the launch token on every mutating method.

    Reads stay open to the loopback caller, because a local process can read the
    data directory directly. Anything that writes, commits a revision, or records
    a checkpoint decision needs the token, and the check is a constant-time
    comparison of a bounded header value against a value this process holds. The
    refusal carries no hint about the expected token and the presented value is
    never logged or echoed.
    """

    def __init__(self, app: ASGIApp, *, api_token: str) -> None:
        self._app = app
        self._expected = validate_api_token(api_token).encode("utf-8")

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        method = scope.get("method") if scope["type"] == "http" else None
        if not isinstance(method, str) or method.upper() not in MUTATING_METHODS:
            await self._app(scope, receive, send)
            return
        presented = _presented_credential(scope)
        if presented is None or not secrets.compare_digest(presented, self._expected):
            await send_refusal(
                send,
                status_code=401,
                code="unauthorized",
                message=TOKEN_REQUIRED_MESSAGE,
                extra_headers=(AUTH_CHALLENGE,),
            )
            return
        await self._app(scope, receive, send)


class CrossOriginGuardMiddleware:
    """Refuse every request that carries an Origin header.

    This process serves one local operator and no browser, so nothing legitimate
    sends Origin: the only client that does is a page on another origin trying
    to drive this API from a browser it controls. No CORS header is granted to
    anyone, which is why there is no allowlist to configure and no origin that
    can be reflected back: a refusal is also unreadable to the page that caused
    it.
    """

    def __init__(self, app: ASGIApp) -> None:
        self._app = app

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] == "http" and header_value(scope, b"origin") is not None:
            await send_refusal(
                send,
                status_code=403,
                code="cross_origin_denied",
                message=ORIGIN_REFUSED_MESSAGE,
            )
            return
        await self._app(scope, receive, send)


__all__ = (
    "API_TOKEN_BYTES",
    "API_TOKEN_FILE_MODE",
    "API_TOKEN_FILE_NAME",
    "LOOPBACK_HOSTNAMES",
    "MAX_API_TOKEN_LENGTH",
    "MIN_API_TOKEN_LENGTH",
    "MUTATING_METHODS",
    "WILDCARD_AUTHORITIES",
    "BearerTokenMiddleware",
    "CrossOriginGuardMiddleware",
    "HostGuardMiddleware",
    "is_loopback_host",
    "is_permitted_authority",
    "issue_api_token",
    "read_api_token",
    "validate_api_token",
)
