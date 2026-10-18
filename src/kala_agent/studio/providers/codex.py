"""Codex CLI provider for the studio chat surface.

The provider is **fail-closed** and, in this build, always unavailable.
The route refuses every request with
:class:`~kala_agent.studio.providers.base.ProviderUnavailableError`, which
the server maps to a sanitised 503. No subprocess is ever launched and no
request leaves the process.

The substantive Codex CLI integration requires a reviewed subprocess
wrapper that enforces the documented flags (``codex exec --sandbox
read-only --skip-git-repo-check --ephemeral --ignore-user-config``),
prompt-on-stdin (never on argv), a minimal sanitised environment, a
bounded deadline, dead child reaping, and a capped concurrent reader
buffer. Implementing that wrapper requires real acceptance testing against
the locally installed CLI; the studio refuses to ship a half-built
subprocess path that could leak prompt content through environment vars,
argv, or buffered stderr.

The module therefore never reads ``KALA_STUDIO_ENABLE_CODEX`` to enable
anything, never inspects or reports the local CLI installation path, and
never claims the provider works. An operator who asks for it gets an
actionable refusal.
"""

from __future__ import annotations

import shutil
from collections.abc import Mapping
from typing import Final

from kala_agent.studio.providers.base import (
    ChatOutcome,
    ProviderConfigurationError,
    ProviderUnavailableError,
)

CODEX_CLI_NAME: Final[str] = "codex"
CODEX_ENABLE_ENV_VAR: Final[str] = "KALA_STUDIO_ENABLE_CODEX"
CODEX_UNAVAILABLE_MESSAGE: Final[str] = (
    "provider=codex is not available in this studio build; the verified "
    "Codex CLI subprocess integration is not shipped yet, so no request "
    "is sent. Use provider=openai or provider=openrouter with an API key "
    "for remote chat."
)


def codex_chat(prompt: str, *, model: str | None = None) -> ChatOutcome:
    """Refuse a codex request and return no reply.

    The provider is unavailable in every configuration. The function only
    validates the prompt shape so a programming error surfaces early.
    """
    del model
    if not isinstance(prompt, str):
        raise ProviderConfigurationError("prompt must be a string")
    raise ProviderUnavailableError(CODEX_UNAVAILABLE_MESSAGE)


def is_codex_enabled(environ: Mapping[str, str]) -> bool:
    """Return whether the (inert) codex opt-in flag is set.

    The flag is retained for diagnostics only: this build refuses the
    route whether or not it is set, so the value never enables anything.
    """
    return environ.get(CODEX_ENABLE_ENV_VAR, "").strip() == "1"


def codex_cli_present() -> bool:
    """Return whether a codex CLI is installed, without disclosing its path."""
    return shutil.which(CODEX_CLI_NAME) is not None


__all__ = (
    "CODEX_CLI_NAME",
    "CODEX_ENABLE_ENV_VAR",
    "CODEX_UNAVAILABLE_MESSAGE",
    "codex_chat",
    "codex_cli_present",
    "is_codex_enabled",
)
