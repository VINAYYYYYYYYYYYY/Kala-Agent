"""Dispatch layer for the studio chat providers.

The dispatch layer is the only entry point the server uses. It picks the
provider implementation, enforces the ``allow_remote`` gate globally, and
publishes the static provider list that the registry surface returns.
"""

from __future__ import annotations

import os
from collections.abc import Mapping
from typing import Final

from kala_agent.studio.providers.base import (
    ChatOutcome,
    ProviderConfigurationError,
)
from kala_agent.studio.providers.codex import codex_chat
from kala_agent.studio.providers.local import DEFAULT_LOCAL_MODEL, local_chat
from kala_agent.studio.providers.openai_compat import (
    OPENAI_DEFAULT_ENDPOINT,
    OPENROUTER_DEFAULT_ENDPOINT,
    fixed_endpoints,
    openai_compat_chat,
)
from kala_agent.studio.schemas import ProviderRecord

STUDIO_PROVIDER_TIMEOUT_SECONDS: Final[float] = 30.0
STUDIO_RESPONSE_MAX_BYTES: Final[int] = 4096


def list_provider_records(
    environ: Mapping[str, str] | None = None,
) -> tuple[ProviderRecord, ...]:
    """Return the sanitised provider list for ``/api/studio/providers``.

    ``configured`` is reported by inspecting the controlled environment
    view the caller passes; the studio never leaks the keys themselves.

    The codex row is always reported as ``configured=False``: this build
    deliberately refuses the codex route, so a CLI on ``PATH`` must not
    turn the row into a claim that the provider works. The note does not
    disclose the local CLI installation path.
    """
    if environ is None:
        environ = os.environ
    openai_configured = bool(_peek_credential("openai", environ))
    openrouter_configured = bool(_peek_credential("openrouter", environ))
    codex_notes = (
        "Codex CLI (ChatGPT subscription). This build does not ship the "
        "verified codex subprocess integration, so the provider is always "
        "unavailable and a request returns a 503. No request is sent."
    )

    return (
        ProviderRecord(
            id="local",
            name="Local Deterministic Helper",
            configured=True,
            notes=(
                "Offline deterministic mechanical-engineering helper. Extracts "
                "dimensions and returns nominal analytic volume/surface-area "
                "estimates, and documents the built-in OpenCascade (OCP) "
                "STEP/STL estimate, neutral CAD/CAE handoff, and standalone "
                "build workflow. Opens no network connection and runs no solver."
            ),
        ),
        ProviderRecord(
            id="openai",
            name="OpenAI (Chat Completions)",
            configured=openai_configured,
            notes=(
                "API-key route; uses the standard Chat Completions endpoint at "
                + OPENAI_DEFAULT_ENDPOINT
                + ". Requires allow_remote=true at request time."
            ),
        ),
        ProviderRecord(
            id="openrouter",
            name="OpenRouter (Chat Completions)",
            configured=openrouter_configured,
            notes=(
                "API-key route; uses the public OpenRouter endpoint at "
                + OPENROUTER_DEFAULT_ENDPOINT
                + ". Requires allow_remote=true at request time."
            ),
        ),
        ProviderRecord(
            id="codex",
            name="Codex CLI (ChatGPT subscription)",
            configured=False,
            notes=codex_notes,
        ),
    )


def _peek_credential(provider: str, environ: Mapping[str, str]) -> str:
    candidates = {
        "openai": ("OPENAI_API_KEY",),
        "openrouter": ("OPENROUTER_API_KEY",),
    }
    for name in candidates.get(provider, ()):
        value = environ.get(name)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return ""


def chat(
    *,
    provider: str,
    model: str | None,
    prompt: str,
    allow_remote: bool,
    environ: Mapping[str, str] | None = None,
) -> ChatOutcome:
    """Dispatch a chat request to the named provider.

    ``allow_remote`` is honoured globally: ``local`` ignores it; ``openai``
    and ``openrouter`` require it for any non-trivial use; ``codex`` also
    requires it because it would invoke an out-of-process CLI. A caller
    that asks for a provider the registry does not publish is refused with
    :class:`ProviderConfigurationError`.
    """
    if environ is None:
        environ = os.environ
    if provider == "local":
        return local_chat(prompt, model=model or DEFAULT_LOCAL_MODEL)
    if provider == "openai":
        if not model:
            raise ProviderConfigurationError(
                "openai provider requires a model id (e.g. 'gpt-4o-mini')"
            )
        return openai_compat_chat(
            "openai",
            model=model,
            prompt=prompt,
            allow_remote=allow_remote,
            environ=environ,
        )
    if provider == "openrouter":
        if not model:
            raise ProviderConfigurationError("openrouter provider requires a model id")
        return openai_compat_chat(
            "openrouter",
            model=model,
            prompt=prompt,
            allow_remote=allow_remote,
            environ=environ,
        )
    if provider == "codex":
        if not allow_remote:
            raise ProviderConfigurationError(
                "provider=codex requires allow_remote=true at request time"
            )
        return codex_chat(prompt, model=model)
    raise ProviderConfigurationError(f"unknown provider: {provider!r}")


__all__ = (
    "STUDIO_PROVIDER_TIMEOUT_SECONDS",
    "STUDIO_RESPONSE_MAX_BYTES",
    "chat",
    "fixed_endpoints",
    "list_provider_records",
)
