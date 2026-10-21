"""Provider implementations for the studio chat surface.

The package exposes one ``chat`` function per registered provider id. The
``local`` provider is the deterministic offline helper; the ``openai`` and
``openrouter`` providers hit a fixed HTTPS endpoint via ``urllib`` with
``allow_remote`` as an opt-in gate; the ``codex``, ``claude_code``,
``codex_cli`` and ``opencode_cli`` providers run the operator's own,
already-authenticated command-line tools as sandboxed subprocesses with a
bounded timeout, a private working directory and a sanitized environment.

None of the providers read files outside the studio data directory; none
cache credentials; none run paid traffic at import time. The CLI providers
never read a credential file either: the probe uses only the exit status of
the CLI's own status command.
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
    PROVIDER_IDS,
    STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS,
    STUDIO_PROVIDER_TIMEOUT_SECONDS,
    STUDIO_RESPONSE_MAX_BYTES,
    SUBSCRIPTION_SOURCE,
    chat,
    list_provider_records,
    probe_subscription_clis,
)

__all__ = (
    "PROVIDER_IDS",
    "STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS",
    "STUDIO_PROVIDER_TIMEOUT_SECONDS",
    "STUDIO_RESPONSE_MAX_BYTES",
    "SUBSCRIPTION_SOURCE",
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
    "probe_subscription_clis",
)
