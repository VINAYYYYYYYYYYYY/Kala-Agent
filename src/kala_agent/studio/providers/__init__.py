"""Provider implementations for the studio chat surface.

The package exposes one ``chat`` function per registered provider id. The
``local`` provider is the deterministic offline helper; the ``openai`` and
``openrouter`` providers hit a fixed HTTPS endpoint via ``urllib`` with
``allow_remote`` as an opt-in gate; the ``codex`` provider runs the
official Codex CLI as a subprocess with bounded timeout and an isolated
temporary cwd. None of the providers read files outside the studio data
directory; none cache credentials; none run paid traffic at import time.
"""

from kala_agent.studio.providers.base import (
    ChatOutcome,
    ProviderConfigurationError,
    ProviderMissingCredentialError,
    ProviderNetworkError,
    ProviderProtocolError,
    ProviderSubprocessError,
    ProviderTimeoutError,
    TooLargeResponseError,
)
from kala_agent.studio.providers.dispatch import (
    STUDIO_PROVIDER_TIMEOUT_SECONDS,
    STUDIO_RESPONSE_MAX_BYTES,
    chat,
    list_provider_records,
)

__all__ = (
    "STUDIO_PROVIDER_TIMEOUT_SECONDS",
    "STUDIO_RESPONSE_MAX_BYTES",
    "ChatOutcome",
    "ProviderConfigurationError",
    "ProviderMissingCredentialError",
    "ProviderNetworkError",
    "ProviderProtocolError",
    "ProviderSubprocessError",
    "ProviderTimeoutError",
    "TooLargeResponseError",
    "chat",
    "list_provider_records",
)
