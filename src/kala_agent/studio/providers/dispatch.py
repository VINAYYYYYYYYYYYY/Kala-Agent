"""Dispatch layer for the studio chat providers.

The dispatch layer is the only entry point the server uses. It picks the
provider implementation, enforces the ``allow_remote`` gate globally, and
publishes the static provider list that the registry surface returns.
"""

from __future__ import annotations

import os
from collections.abc import Mapping
from typing import Final

from kala_agent.studio.design.intent import (
    DESIGN_INTENT_INSTRUCTION,
    DesignIntent,
    DesignIntentParseError,
    design_intent_schema_text,
    parse_design_intent_text,
    strip_design_json_block,
)
from kala_agent.studio.providers.base import (
    ChatOutcome,
    ProviderConfigurationError,
)
from kala_agent.studio.providers.codex import codex_chat
from kala_agent.studio.providers.local import (
    DEFAULT_LOCAL_MODEL,
    detect_design_family,
    local_chat,
    parse_design_prompt,
)
from kala_agent.studio.providers.openai_compat import (
    OPENAI_DEFAULT_ENDPOINT,
    OPENROUTER_DEFAULT_ENDPOINT,
    fixed_endpoints,
    openai_compat_chat,
)
from kala_agent.studio.schemas import ProviderRecord

STUDIO_PROVIDER_TIMEOUT_SECONDS: Final[float] = 30.0
STUDIO_RESPONSE_MAX_BYTES: Final[int] = 4096


class StudioChatTurn(ChatOutcome):
    """A provider reply plus any design intent the turn produced.

    The extra fields are metadata for the server, not part of the HTTP
    response. ``design_intent`` is present only when the prompt was parsed or
    validated into a complete intent; ``design_missing`` names the values the
    local helper refused to guess. ``design_error`` records an honest parse
    or repair failure.
    """

    def __init__(
        self,
        *,
        text: str,
        provider: str,
        model: str | None,
        offline: bool,
        design_intent: DesignIntent | None = None,
        design_missing: tuple[str, ...] = (),
        design_notes: tuple[str, ...] = (),
        design_family: str | None = None,
        design_error: str | None = None,
        cad_provider: str | None = None,
    ) -> None:
        super().__init__(text=text, provider=provider, model=model, offline=offline)
        self.design_intent = design_intent
        self.design_missing = design_missing
        self.design_notes = design_notes
        self.design_family = design_family
        self.design_error = design_error
        self.cad_provider = cad_provider


def _design_prompt(prompt: str) -> str:
    schema = design_intent_schema_text()
    return (
        f"{DESIGN_INTENT_INSTRUCTION}\n\n"
        f"DesignIntent JSON Schema:\n{schema}\n\n"
        f"User request:\n{prompt}"
    )


def _repair_prompt(prompt: str, error: str) -> str:
    return (
        "Your previous reply could not be validated as a DesignIntent: "
        f"{error[:512]}\n\n"
        "Return exactly one fenced ```json block containing a corrected "
        "DesignIntent for the same request. Do not invent a value the user "
        "did not state; if a required value is missing, reply in prose with "
        "the missing values instead.\n\n"
        f"User request:\n{prompt}"
    )


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
    cad_provider: str | None = None,
) -> ChatOutcome:
    """Dispatch a chat request to the named provider.

    ``allow_remote`` is honoured globally: ``local`` ignores it; ``openai``
    and ``openrouter`` require it for any non-trivial use; ``codex`` also
    requires it because it would invoke an out-of-process CLI. A caller
    that asks for a provider the registry does not publish is refused with
    :class:`ProviderConfigurationError`.

    Design-intent hook: the local helper's strict parser and the remote
    providers' JSON-Schema prompt both return a :class:`StudioChatTurn` so
    the server can compile the intent without the provider layer ever
    touching OCP. ``allow_remote`` semantics are unchanged.
    """
    if environ is None:
        environ = os.environ
    if provider == "local":
        parsed = parse_design_prompt(prompt)
        outcome = local_chat(prompt, model=model or DEFAULT_LOCAL_MODEL)
        if parsed is None:
            return StudioChatTurn(
                text=outcome.text,
                provider=outcome.provider,
                model=outcome.model,
                offline=outcome.offline,
                cad_provider=cad_provider,
            )
        return StudioChatTurn(
            text=outcome.text,
            provider=outcome.provider,
            model=outcome.model,
            offline=outcome.offline,
            design_intent=parsed.intent,
            design_missing=parsed.missing,
            design_notes=parsed.notes,
            design_family=parsed.family,
            cad_provider=cad_provider,
        )
    if provider == "openai":
        if not model:
            raise ProviderConfigurationError(
                "openai provider requires a model id (e.g. 'gpt-4o-mini')"
            )
        return _remote_turn(
            "openai",
            model=model,
            prompt=prompt,
            allow_remote=allow_remote,
            environ=environ,
            cad_provider=cad_provider,
        )
    if provider == "openrouter":
        if not model:
            raise ProviderConfigurationError("openrouter provider requires a model id")
        return _remote_turn(
            "openrouter",
            model=model,
            prompt=prompt,
            allow_remote=allow_remote,
            environ=environ,
            cad_provider=cad_provider,
        )
    if provider == "codex":
        if not allow_remote:
            raise ProviderConfigurationError(
                "provider=codex requires allow_remote=true at request time"
            )
        return codex_chat(prompt, model=model)
    raise ProviderConfigurationError(f"unknown provider: {provider!r}")


def _remote_turn(
    provider: str,
    *,
    model: str,
    prompt: str,
    allow_remote: bool,
    environ: Mapping[str, str],
    cad_provider: str | None = None,
) -> StudioChatTurn:
    family = detect_design_family(prompt)
    if family is None:
        outcome = openai_compat_chat(
            provider,
            model=model,
            prompt=prompt,
            allow_remote=allow_remote,
            environ=environ,
        )
        return StudioChatTurn(
            text=outcome.text,
            provider=outcome.provider,
            model=outcome.model,
            offline=outcome.offline,
            cad_provider=cad_provider,
        )

    first = openai_compat_chat(
        provider,
        model=model,
        prompt=_design_prompt(prompt),
        allow_remote=allow_remote,
        environ=environ,
    )
    try:
        intent = parse_design_intent_text(first.text)
    except DesignIntentParseError as first_error:
        second = openai_compat_chat(
            provider,
            model=model,
            prompt=_repair_prompt(prompt, str(first_error)),
            allow_remote=allow_remote,
            environ=environ,
        )
        try:
            intent = parse_design_intent_text(second.text)
        except DesignIntentParseError as second_error:
            prose = strip_design_json_block(second.text)
            return StudioChatTurn(
                text=(f"{prose} No validated design was produced: {str(second_error)[:512]}"),
                provider=provider,
                model=second.model or first.model or model,
                offline=False,
                design_family=family,
                design_error=str(second_error),
                cad_provider=cad_provider,
            )
        return StudioChatTurn(
            text=strip_design_json_block(second.text),
            provider=provider,
            model=second.model or first.model or model,
            offline=False,
            design_intent=intent,
            design_family=family,
            cad_provider=cad_provider,
        )
    return StudioChatTurn(
        text=strip_design_json_block(first.text),
        provider=provider,
        model=first.model or model,
        offline=False,
        design_intent=intent,
        design_family=family,
        cad_provider=cad_provider,
    )


__all__ = (
    "STUDIO_PROVIDER_TIMEOUT_SECONDS",
    "STUDIO_RESPONSE_MAX_BYTES",
    "StudioChatTurn",
    "chat",
    "fixed_endpoints",
    "list_provider_records",
)
