"""Persistent, owner-only API key settings store for Kala Studio.

The settings file lives inside the studio data directory alongside
``studio-token`` and is written atomically with ``0o600`` permissions via
:func:`kala_agent.studio.boundary._atomic_write_text`.

Only ``openai`` and ``openrouter`` credentials can be stored here. Stored
secrets are merged into an effective environment dictionary at request time
and are **never** echoed back in any HTTP response or error envelope.

The subscription routes have no stored credential at all: their credential
is the operator's own CLI login, so their rows report the CLI's own
availability probe and never name an environment variable.
"""

from __future__ import annotations

import json
import os
import tempfile
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Final

from kala_agent.studio.boundary import (
    _atomic_write_text,
    _tighten_directory_mode,
    resolve_studio_data_dir,
)
from kala_agent.studio.providers.cli_runtime import CliProbe
from kala_agent.studio.schemas import (
    CredentialSource,
    ProviderCredentialStatus,
)

STUDIO_SETTINGS_FILE_NAME: Final[str] = "studio-settings.json"
#: The environment variable that names the studio data directory itself.
#:
#: Unlike ``KALA_AGENT_DATA_DIR`` it is not a Kala data directory: the value
#: *is* the studio directory, so no ``studio`` child is appended.
STUDIO_DATA_DIR_ENV_VAR: Final[str] = "KALA_STUDIO_DATA_DIR"
#: The environment variable that names the shared Kala data directory.
AGENT_DATA_DIR_ENV_VAR: Final[str] = "KALA_AGENT_DATA_DIR"
#: The environment variable that names the XDG data home.
XDG_DATA_HOME_ENV_VAR: Final[str] = "XDG_DATA_HOME"
#: The directory name the studio child carries under a Kala data directory.
AGENT_DIRECTORY_NAME: Final[str] = "kala-agent"
SUPPORTED_CREDENTIAL_PROVIDERS: Final[tuple[str, ...]] = ("openai", "openrouter")
PROVIDER_ENV_VARS: Final[dict[str, str]] = {
    "openai": "OPENAI_API_KEY",
    "openrouter": "OPENROUTER_API_KEY",
}
#: The subscription providers whose credential is the operator's own CLI
#: login.
#:
#: The rows come after the API-key rows so the stored-key indices of the
#: settings route do not move.
SUBSCRIPTION_CREDENTIAL_PROVIDERS: Final[tuple[str, ...]] = (
    "codex",
    "claude_code",
    "codex_cli",
    "opencode_cli",
)
#: The provider id whose probe answers for the historical ``codex`` alias.
CODEX_CLI_PROVIDER_ID: Final[str] = "codex_cli"
MAX_API_KEY_LENGTH: Final[int] = 256


def _home_directory(environ: Mapping[str, str]) -> Path | None:
    """Return the home directory named by an environment view.

    The process environment is never consulted, so the resolution is
    reproducible from the mapping the caller passes; the native product's
    ``boundary::home_directory_from`` reads the same mapping.
    """
    home = environ.get("HOME")
    if isinstance(home, str) and home:
        return Path(home)
    profile = environ.get("USERPROFILE")
    if isinstance(profile, str) and profile:
        return Path(profile)
    drive = environ.get("HOMEDRIVE")
    path = environ.get("HOMEPATH")
    if isinstance(drive, str) and drive and isinstance(path, str) and path:
        return Path(drive + path)
    return None


def _expand_home(value: str, home: Path | None) -> Path | None:
    """Expand a leading ``~`` against ``home``; a relative path is refused."""
    trimmed = value.strip()
    if trimmed == "~":
        return home
    if trimmed.startswith("~/"):
        return None if home is None else home / trimmed[2:]
    candidate = Path(trimmed)
    if candidate.is_absolute():
        return candidate
    return None


def _valid_studio_data_dir(value: str, home: Path | None) -> Path | None:
    """Return a usable studio data directory, or ``None``.

    The rules are the ones the ``Settings`` validator applies to
    ``data_dir``: absolute, not a filesystem root, and not an existing
    non-directory. The native product's ``valid_studio_data_dir`` applies
    the same rules.
    """
    candidate = _expand_home(value, home)
    if candidate is None:
        return None
    if not candidate.is_absolute() or candidate.parent == candidate:
        return None
    try:
        if candidate.exists() and not candidate.is_dir():
            return None
    except (OSError, ValueError):
        # An unrepresentable path (for example an embedded NUL) is unusable.
        return None
    return candidate


def resolve_studio_data_dir_from_env(environ: Mapping[str, str] | None = None) -> Path:
    """Resolve the studio data directory from an environment view.

    The precedence is identical in the native product
    (``boundary::default_studio_data_dir``):

    1. ``--data-dir`` is used as-is by the caller; it never reaches this
       function.
    2. ``KALA_STUDIO_DATA_DIR`` is used as-is, because it *is* the studio
       directory. A value that is not an absolute, non-root directory path
       is ignored and the next row is tried.
    3. ``KALA_AGENT_DATA_DIR`` is the shared Kala data directory: the
       ``studio`` child is appended. An unusable value ends the search with
       the temporary fallback, exactly like the ``Settings`` validator's
       failure path today.
    4. ``$XDG_DATA_HOME/kala-agent/studio``.
    5. ``$HOME/.local/share/kala-agent/studio`` (the historical default; a
       launch with none of these variables set resolves here unchanged).
    6. ``$TMPDIR/kala-studio``.
    """
    source: Mapping[str, str] = os.environ if environ is None else environ
    fallback = Path(tempfile.gettempdir()) / "kala-studio"
    home = _home_directory(source)

    raw = source.get(STUDIO_DATA_DIR_ENV_VAR)
    if isinstance(raw, str):
        explicit = _valid_studio_data_dir(raw, home)
        if explicit is not None:
            try:
                explicit.mkdir(parents=True, exist_ok=True)
                _tighten_directory_mode(explicit)
                return explicit
            except (OSError, ValueError):
                pass

    raw = source.get(AGENT_DATA_DIR_ENV_VAR)
    if isinstance(raw, str):
        parent = _expand_home(raw, home)
        if parent is None or parent.parent == parent:
            return fallback
        try:
            if parent.exists() and not parent.is_dir():
                return fallback
            return resolve_studio_data_dir(parent)
        except (OSError, ValueError):
            return fallback

    raw = source.get(XDG_DATA_HOME_ENV_VAR)
    if isinstance(raw, str):
        data_home = _valid_studio_data_dir(raw, home)
        if data_home is not None:
            try:
                return resolve_studio_data_dir(data_home / AGENT_DIRECTORY_NAME)
            except (OSError, ValueError):
                pass

    if home is not None:
        parent = home / ".local" / "share" / AGENT_DIRECTORY_NAME
        try:
            return resolve_studio_data_dir(parent)
        except (OSError, ValueError):
            pass
    return fallback


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
    subscription_probes: Sequence[CliProbe] = (),
) -> tuple[ProviderCredentialStatus, ...]:
    """Return non-secret credential status records for each configurable provider.

    The API-key rows report where the key comes from. The subscription rows
    report the CLI's own probe: the row is ``configured`` when the CLI is
    installed and signed in, and its source is ``subscription`` because the
    credential lives in the CLI's own store, never in Kala's.
    """
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
                provider=provider,
                env_var=env_var,
                configured=configured,
                source=source,
            )
        )
    for provider in SUBSCRIPTION_CREDENTIAL_PROVIDERS:
        probe = _subscription_probe(subscription_probes, provider)
        rows.append(
            ProviderCredentialStatus(
                provider=provider,
                env_var="",
                configured=probe is not None and probe.usable(),
                source=CredentialSource.SUBSCRIPTION,
            )
        )
    return tuple(rows)


def _subscription_probe(
    subscription_probes: Sequence[CliProbe],
    provider: str,
) -> CliProbe | None:
    """Return the probe verdict of one subscription provider.

    The historical ``codex`` id names the same CLI as ``codex_cli``, so it
    answers with that CLI's verdict.
    """
    for probe in subscription_probes:
        if probe.id == provider:
            return probe
    if provider == "codex":
        for probe in subscription_probes:
            if probe.id == CODEX_CLI_PROVIDER_ID:
                return probe
    return None


__all__ = (
    "CODEX_CLI_PROVIDER_ID",
    "MAX_API_KEY_LENGTH",
    "PROVIDER_ENV_VARS",
    "STUDIO_SETTINGS_FILE_NAME",
    "SUBSCRIPTION_CREDENTIAL_PROVIDERS",
    "SUPPORTED_CREDENTIAL_PROVIDERS",
    "describe_provider_credentials",
    "effective_environ",
    "read_stored_keys",
    "validate_api_key",
    "write_stored_key",
)
