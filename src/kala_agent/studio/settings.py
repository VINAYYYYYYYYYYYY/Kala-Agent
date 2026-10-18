"""Persistent, owner-only API key settings store for Kala Studio.

The settings file lives inside the studio data directory alongside
``studio-token`` and is written atomically with ``0o600`` permissions via
:func:`kala_agent.studio.boundary._atomic_write_text`.

Only ``openai`` and ``openrouter`` credentials can be stored here. Stored
secrets are merged into an effective environment dictionary at request time
and are **never** echoed back in any HTTP response or error envelope.
"""

from __future__ import annotations

import json
from collections.abc import Mapping
from pathlib import Path
from typing import Final

from kala_agent.studio.boundary import _atomic_write_text
from kala_agent.studio.schemas import (
    CredentialProviderId,
    CredentialSource,
    ProviderCredentialStatus,
)

STUDIO_SETTINGS_FILE_NAME: Final[str] = "studio-settings.json"
SUPPORTED_CREDENTIAL_PROVIDERS: Final[tuple[str, ...]] = ("openai", "openrouter")
PROVIDER_ENV_VARS: Final[dict[str, str]] = {
    "openai": "OPENAI_API_KEY",
    "openrouter": "OPENROUTER_API_KEY",
}
MAX_API_KEY_LENGTH: Final[int] = 256


def validate_api_key(value: str | None) -> str:
    """Validate and normalise an API key, or return ``""`` to clear it.

    Error messages intentionally never include the submitted key value so
    an invalid submission cannot echo a secret in a 422 response.
    """
    if value is None:
        return ""
    if not isinstance(value, str):
        raise ValueError("API key must be a string or null")
    stripped = value.strip()
    if not stripped:
        return ""
    if len(stripped) > MAX_API_KEY_LENGTH:
        raise ValueError("API key exceeds maximum length")
    if any(ord(ch) < 32 or ord(ch) == 127 for ch in stripped):
        raise ValueError("API key contains unsupported control characters")
    if any(ch.isspace() for ch in stripped):
        raise ValueError("API key must not contain whitespace")
    return stripped


def read_stored_keys(data_dir: Path | None) -> dict[str, str]:
    """Read validated stored provider keys from ``data_dir / studio-settings.json``.

    Missing, unreadable, or malformed files return ``{}`` rather than raising.
    """
    if data_dir is None:
        return {}
    path = data_dir / STUDIO_SETTINGS_FILE_NAME
    try:
        raw = path.read_text(encoding="utf-8")
    except OSError:
        return {}
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError:
        return {}
    if not isinstance(parsed, dict):
        return {}
    api_keys = parsed.get("api_keys")
    if not isinstance(api_keys, dict):
        return {}
    result: dict[str, str] = {}
    for provider in SUPPORTED_CREDENTIAL_PROVIDERS:
        candidate = api_keys.get(provider)
        if isinstance(candidate, str):
            try:
                cleaned = validate_api_key(candidate)
            except ValueError:
                continue
            if cleaned:
                result[provider] = cleaned
    return result


def write_stored_key(data_dir: Path, provider: str, api_key: str | None) -> dict[str, str]:
    """Store or clear an API key for ``provider`` in ``data_dir``."""
    if provider not in SUPPORTED_CREDENTIAL_PROVIDERS:
        raise ValueError(f"unsupported provider for API key settings: {provider!r}")
    cleaned = validate_api_key(api_key)
    current = read_stored_keys(data_dir)
    if cleaned:
        current[provider] = cleaned
    else:
        current.pop(provider, None)
    payload = json.dumps({"api_keys": current}, indent=2, sort_keys=True) + "\n"
    _atomic_write_text(data_dir / STUDIO_SETTINGS_FILE_NAME, payload)
    return current


def effective_environ(
    base_environ: Mapping[str, str],
    data_dir: Path | None = None,
) -> dict[str, str]:
    """Return an environment mapping with stored studio API keys overlaid."""
    merged = dict(base_environ)
    stored = read_stored_keys(data_dir)
    for provider, env_name in PROVIDER_ENV_VARS.items():
        key = stored.get(provider, "")
        if key:
            merged[env_name] = key
    return merged


def describe_provider_credentials(
    base_environ: Mapping[str, str],
    data_dir: Path | None = None,
) -> tuple[ProviderCredentialStatus, ...]:
    """Return non-secret credential status records for each configurable provider."""
    stored = read_stored_keys(data_dir)
    rows: list[ProviderCredentialStatus] = []
    for provider in SUPPORTED_CREDENTIAL_PROVIDERS:
        env_var = PROVIDER_ENV_VARS[provider]
        stored_key = stored.get(provider, "")
        env_raw = base_environ.get(env_var, "")
        env_key = env_raw.strip() if isinstance(env_raw, str) else ""
        if stored_key:
            source = CredentialSource.STORED
            configured = True
        elif env_key:
            source = CredentialSource.ENVIRONMENT
            configured = True
        else:
            source = CredentialSource.NONE
            configured = False
        rows.append(
            ProviderCredentialStatus(
                provider=CredentialProviderId(provider),
                env_var=env_var,
                configured=configured,
                source=source,
            )
        )
    return tuple(rows)


__all__ = (
    "MAX_API_KEY_LENGTH",
    "PROVIDER_ENV_VARS",
    "STUDIO_SETTINGS_FILE_NAME",
    "SUPPORTED_CREDENTIAL_PROVIDERS",
    "describe_provider_credentials",
    "effective_environ",
    "read_stored_keys",
    "validate_api_key",
    "write_stored_key",
)
