"""HTTPS chat provider for OpenAI-compatible endpoints.

The provider uses only the Python standard library: ``urllib`` for the
HTTP request, ``json`` for the payload. It enforces four constraints so
that a misconfigured operator cannot accidentally exfiltrate data or
ship credentials:

- The endpoint URL must be one of a hard-coded set (``FIXED_ENDPOINTS``).
  Any other value raises :class:`ProviderConfigurationError`.
- Redirects are denied at the request level so a clever DNS record
  cannot move the call onto a different host.
- The response body is bounded to :data:`STUDIO_RESPONSE_MAX_BYTES`; a
  larger body raises :class:`TooLargeResponseError` before the studio
  accepts the payload.
- The timeout is bounded and never configurable at request time.

A caller that has not set ``allow_remote=True`` cannot reach this code:
the dispatch layer refuses the request with a sanitised message before
calling the provider.
"""

from __future__ import annotations

import json
import socket
import ssl
import urllib.error
import urllib.request
from collections.abc import Mapping
from typing import Final

from kala_agent.studio.providers.base import (
    ChatOutcome,
    ProviderConfigurationError,
    ProviderMissingCredentialError,
    ProviderNetworkError,
    ProviderProtocolError,
    ProviderTimeoutError,
    TooLargeResponseError,
)

OPENAI_DEFAULT_ENDPOINT: Final[str] = "https://api.openai.com/v1/chat/completions"
OPENROUTER_DEFAULT_ENDPOINT: Final[str] = "https://openrouter.ai/api/v1/chat/completions"
STUDIO_REQUEST_TIMEOUT_SECONDS: Final[float] = 30.0
MAX_HTTP_RESPONSE_BYTES: Final[int] = 1_000_000
MAX_KEY_LENGTH: Final[int] = 256


_FIXED_ENDPOINTS: Final[tuple[str, ...]] = (
    OPENAI_DEFAULT_ENDPOINT,
    OPENROUTER_DEFAULT_ENDPOINT,
)


def fixed_endpoints() -> tuple[str, ...]:
    """Return the hard-coded endpoint URLs this provider trusts."""
    return _FIXED_ENDPOINTS


def _endpoint_for(provider: str, environ: Mapping[str, str]) -> str:
    """Return the fixed endpoint URL for the provider.

    The function reads nothing from the environment that could redirect a
    request: the URL is hard-coded per provider. A caller asking for a
    route that is not in the fixed set gets a sanitised refusal.
    """
    if provider == "openai":
        return OPENAI_DEFAULT_ENDPOINT
    if provider == "openrouter":
        return OPENROUTER_DEFAULT_ENDPOINT
    raise ProviderConfigurationError(f"no fixed endpoint registered for provider {provider!r}")


def _credential(provider: str, environ: Mapping[str, str]) -> str:
    """Return the API key for ``provider`` from the controlled environment.

    Only the documented environment names are read. Anything that looks
    credential-shaped but does not match a documented name is silently
    ignored. The empty string is treated as "no key".
    """
    candidates = {
        "openai": ("OPENAI_API_KEY",),
        "openrouter": ("OPENROUTER_API_KEY",),
    }
    names = candidates.get(provider, ())
    for name in names:
        value = environ.get(name)
        if isinstance(value, str) and value.strip():
            stripped = value.strip()
            if len(stripped) > MAX_KEY_LENGTH:
                raise ProviderConfigurationError(f"{name} is longer than the bounded length")
            return stripped
    raise ProviderMissingCredentialError(
        f"provider {provider!r} is not configured (set the documented env key)"
    )


def _bearer_headers(key: str, *, referer: str | None = None) -> dict[str, str]:
    headers = {
        "Authorization": "Bearer " + key,
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "kala-studio/0.1",
    }
    if referer:
        headers["HTTP-Referer"] = referer
        headers["X-Title"] = "kala-studio"
    return headers


def _build_ssl_context() -> ssl.SSLContext:
    return ssl.create_default_context()


def _post_json(
    endpoint: str,
    payload: dict[str, object],
    headers: dict[str, str],
    *,
    timeout: float,
) -> bytes:
    """POST ``payload`` to ``endpoint`` and return the bounded body bytes.

    The function refuses redirects at the request level, so a configured
    DNS record cannot move the call onto a different host. The response
    body is read into memory in one bounded chunk via ``Content-Length``
    when present; otherwise the read loop is bounded by
    :data:`MAX_HTTP_RESPONSE_BYTES`.
    """
    request = urllib.request.Request(
        endpoint,
        data=json.dumps(payload).encode("utf-8"),
        headers=headers,
        method="POST",
    )
    opener = urllib.request.build_opener(NoRedirectHandler())
    try:
        with opener.open(request, timeout=timeout) as response:
            content_length = response.headers.get("Content-Length")
            if content_length is not None:
                try:
                    declared = int(content_length)
                except ValueError:
                    declared = -1
                if declared > MAX_HTTP_RESPONSE_BYTES:
                    raise TooLargeResponseError(
                        "the provider response is larger than the bounded size"
                    )
            chunks: list[bytes] = []
            total = 0
            while True:
                chunk = response.read(64 * 1024)
                if not chunk:
                    break
                total += len(chunk)
                if total > MAX_HTTP_RESPONSE_BYTES:
                    raise TooLargeResponseError(
                        "the provider response exceeded the bounded size during read"
                    )
                chunks.append(chunk)
            return b"".join(chunks)
    except urllib.error.HTTPError as error:
        raise ProviderProtocolError(
            f"the provider returned HTTP {error.code}: {error.reason!s}"
        ) from error
    except urllib.error.URLError as error:
        reason = getattr(error, "reason", None)
        if isinstance(reason, (socket.timeout, TimeoutError)):
            raise ProviderTimeoutError(
                "the provider request exceeded its bounded timeout"
            ) from error
        raise ProviderNetworkError(
            f"the provider network call failed: {type(reason).__name__ if reason else 'unknown'}"
        ) from error
    except TimeoutError as error:
        raise ProviderTimeoutError("the provider request exceeded its bounded timeout") from error


class NoRedirectHandler(urllib.request.HTTPRedirectHandler):
    """A redirect handler that raises on any redirect attempt."""

    def redirect_request(
        self,
        *_args: object,
        **_kwargs: object,
    ) -> None:
        raise ProviderNetworkError("redirects to a different origin are refused")


def _read_response(data: bytes) -> tuple[str, str | None]:
    """Return ``(text, model)`` parsed from a provider response.

    ``model`` is ``None`` when the provider did not name one, so the
    caller can fall back to the model the operator requested rather than
    silently substituting the provider id.
    """
    try:
        payload = json.loads(data)
    except json.JSONDecodeError as error:
        raise ProviderProtocolError("the provider response was not valid JSON") from error
    if not isinstance(payload, dict):
        raise ProviderProtocolError("the provider response was not a JSON object")
    choices = payload.get("choices")
    if not isinstance(choices, list) or not choices:
        raise ProviderProtocolError("the provider response did not declare any choices")
    first = choices[0]
    if not isinstance(first, dict):
        raise ProviderProtocolError("the provider response had a malformed choice")
    message = first.get("message")
    text = ""
    if isinstance(message, dict):
        content = message.get("content")
        if isinstance(content, str):
            text = content
    if not text:
        raise ProviderProtocolError("the provider choice had no readable message content")
    model_value = payload.get("model")
    if not isinstance(model_value, str) or not model_value:
        model_value = None
    if len(text) > 4096:
        text = text[:4096]
    return text, model_value


def openai_compat_chat(
    provider: str,
    *,
    model: str,
    prompt: str,
    allow_remote: bool,
    environ: Mapping[str, str],
) -> ChatOutcome:
    """Send ``prompt`` to the fixed endpoint for ``provider`` and parse the reply.

    ``allow_remote`` must be ``True`` for the request to leave the process;
    the dispatch layer enforces this invariant upstream so the studio never
    sends a chat on the operator's behalf without explicit consent.
    """
    if not allow_remote:
        raise ProviderConfigurationError("remote calls require explicit allow_remote=true")
    endpoint = _endpoint_for(provider, environ)
    if endpoint not in _FIXED_ENDPOINTS:
        raise ProviderConfigurationError("the endpoint is not in the hard-coded allowlist")
    key = _credential(provider, environ)
    payload = {
        "model": model,
        "messages": ({"role": "user", "content": prompt},),
        "temperature": 0.2,
        "stream": False,
    }
    referer = "https://kala-agent.local/studio"
    headers = _bearer_headers(key, referer=referer)
    data = _post_json(
        endpoint,
        payload,
        headers,
        timeout=STUDIO_REQUEST_TIMEOUT_SECONDS,
    )
    text, observed_model = _read_response(data)
    return ChatOutcome(
        text=text,
        provider=provider,
        model=observed_model or model,
        offline=False,
    )


__all__ = (
    "MAX_HTTP_RESPONSE_BYTES",
    "OPENAI_DEFAULT_ENDPOINT",
    "OPENROUTER_DEFAULT_ENDPOINT",
    "STUDIO_REQUEST_TIMEOUT_SECONDS",
    "fixed_endpoints",
    "openai_compat_chat",
)
