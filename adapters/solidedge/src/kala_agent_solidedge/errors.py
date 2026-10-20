"""Typed failures for the Windows-local Solid Edge client.

Nothing here is a wire type: the bridge owns the wire. These exceptions exist so
the compiler, the HTTP client, and the CLI can each fail with a message that
says which stage refused the work. A refusal from the compiler happens before a
socket is opened.
"""

from __future__ import annotations

from typing import Final

MAX_MESSAGE_CHARS: Final[int] = 500


class SolidEdgeClientError(Exception):
    """The base failure of this package. Messages are bounded."""

    def __init__(self, message: str) -> None:
        text = " ".join(message.replace("\r", " ").replace("\n", " ").split())
        super().__init__(text[:MAX_MESSAGE_CHARS] or "solid edge client failure")


class CompileError(SolidEdgeClientError):
    """The construction script is outside the supported Solid Edge subset."""


class ClientError(SolidEdgeClientError):
    """The local bridge could not be reached or refused the request."""


class AuthenticationError(ClientError):
    """The bearer token was missing, unreadable, or rejected."""


class ProtocolError(ClientError):
    """The bridge answered with something outside protocol v1."""


class NetworkError(ClientError):
    """A socket, timeout, or transfer failure. Submission may be ambiguous."""


class ArtifactError(ClientError):
    """A downloaded artifact did not match its declared size or digest."""


__all__ = (
    "ArtifactError",
    "AuthenticationError",
    "ClientError",
    "CompileError",
    "NetworkError",
    "ProtocolError",
    "SolidEdgeClientError",
)
