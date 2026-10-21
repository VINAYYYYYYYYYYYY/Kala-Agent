"""Typed provider errors and outcomes for the studio chat surface.

Every provider raises one of the typed exceptions below when it cannot
fulfil a request. The server maps these to sanitised HTTP responses with
codes that mirror what the UI renders. None of the errors carry
credential-shaped text or the server-side stack trace; the message is a
short sentence the operator can read without opening the log.
"""

from __future__ import annotations

from typing import ClassVar


class ProviderProtocolError(RuntimeError):
    """Raised when a provider returns an unparseable payload."""


class ProviderNetworkError(RuntimeError):
    """Raised when the studio cannot reach a provider over the network."""


class ProviderTimeoutError(RuntimeError):
    """Raised when a provider exceeds its bounded timeout."""


class ProviderConfigurationError(RuntimeError):
    """Raised when the environment is not configured for a provider."""

    code: ClassVar[str] = "provider_not_configured"


class ProviderMissingCredentialError(ProviderConfigurationError):
    """Raised when a provider is configured but its credential is absent."""

    code: ClassVar[str] = "missing_api_key"


class ProviderUnavailableError(ProviderConfigurationError):
    """Raised when a documented provider is deliberately not enabled.

    The error is distinct from a missing credential: the provider route
    exists but the studio refuses to run it in this build. The server maps
    it to a 503 with an actionable message instead of the 400 it returns
    for a fixable configuration gap.
    """

    code: ClassVar[str] = "provider_unavailable"


class ProviderSubprocessError(RuntimeError):
    """Raised when a CLI-backed provider exits non-zero or stalls."""


class TooLargeResponseError(RuntimeError):
    """Raised when a provider returns more bytes than the studio will accept."""


class ChatOutcome:
    """Bundles the chat reply with the model identity and offline flag.

    ``text`` is the display text; ``reply`` is the model's full reply before
    the display bound. A design inside a reply can be longer than the prose
    that is displayed, so design extraction reads ``reply`` while the API
    response shows ``text``.
    """

    def __init__(
        self,
        *,
        text: str,
        provider: str,
        model: str | None,
        offline: bool,
        reply: str | None = None,
    ) -> None:
        self.text = text
        self.reply = text if reply is None else reply
        self.provider = provider
        self.model = model
        self.offline = offline

    def __repr__(self) -> str:
        return (
            f"ChatOutcome(provider={self.provider!r}, model={self.model!r}, "
            f"offline={self.offline!r}, text_len={len(self.text)})"
        )


__all__ = (
    "ChatOutcome",
    "ProviderConfigurationError",
    "ProviderMissingCredentialError",
    "ProviderNetworkError",
    "ProviderProtocolError",
    "ProviderSubprocessError",
    "ProviderTimeoutError",
    "ProviderUnavailableError",
    "TooLargeResponseError",
)
