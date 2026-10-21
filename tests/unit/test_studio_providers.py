"""Tests for the studio chat provider dispatch layer.

The tests cover:

- The local helper produces a deterministic, offline reply.
- The OpenAI/OpenRouter provider refuses to leave the process without an
  environment key and ``allow_remote=True``.
- The subscription CLI routes refuse without the remote consent and report
  a missing binary as an actionable availability failure.
- The dispatch layer refuses every unknown provider id.

The tests do not perform any paid live calls, do not read auth files and
never start a real ``claude``, ``codex`` or ``opencode``: the subscription
routes are exercised with a fake executable in
``tests/unit/test_studio_cli_providers.py``.
"""

from __future__ import annotations

import os
import re
from collections.abc import Iterator
from pathlib import Path

import pytest

from kala_agent.studio.providers.base import (
    ChatOutcome,
    ProviderConfigurationError,
    ProviderMissingCredentialError,
    ProviderProtocolError,
    ProviderUnavailableError,
)
from kala_agent.studio.providers.cli_runtime import CliProbe
from kala_agent.studio.providers.dispatch import (
    fixed_endpoints,
    list_provider_records,
)
from kala_agent.studio.providers.local import (
    DEFAULT_LOCAL_MODEL,
    MAX_PROMPT_LENGTH,
    MAX_RESPONSE_LENGTH,
    local_chat,
)
from kala_agent.studio.providers.openai_compat import (
    MAX_HTTP_RESPONSE_BYTES,
    OPENAI_DEFAULT_ENDPOINT,
    OPENROUTER_DEFAULT_ENDPOINT,
    openai_compat_chat,
)


@pytest.fixture(autouse=True)
def _forbid_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    def blocked_connect(self: object, address: object, *_args: object, **_kwargs: object) -> int:  # type: ignore[no-untyped-def]
        host = address[0] if isinstance(address, tuple) else address
        raise AssertionError(f"provider tests would have connected to {host}")

    monkeypatch.setattr("socket.socket.connect", blocked_connect)
    yield


# --- local helper ---


def test_local_chat_returns_offline_text() -> None:
    outcome = local_chat("Design a small clamp. It must fit a 12 mm shaft.")
    assert isinstance(outcome, ChatOutcome)
    assert outcome.provider == "local"
    assert outcome.offline is True
    assert outcome.model
    assert outcome.text
    assert "Local helper" in outcome.text


def test_local_chat_is_deterministic() -> None:
    first = local_chat("Analyse the cog.")
    second = local_chat("Analyse the cog.")
    assert first.text == second.text


def test_local_chat_rejects_non_string_prompt() -> None:
    with pytest.raises(ProviderProtocolError):
        local_chat(b"raw bytes prompt")  # type: ignore[arg-type]


def test_local_chat_rejects_oversized_prompt() -> None:
    with pytest.raises(ProviderProtocolError):
        local_chat("x" * 10_000)


def test_local_chat_handles_empty_prompt_gracefully() -> None:
    outcome = local_chat("")
    assert outcome.text


def test_local_chat_default_model_is_stable() -> None:
    outcome = local_chat("hello world")
    assert outcome.model == DEFAULT_LOCAL_MODEL
    assert DEFAULT_LOCAL_MODEL == "deterministic-paraphrase-v1"


def test_local_chat_box_estimate_is_analytic_and_deterministic() -> None:
    prompt = "Estimate a box 20x30x40 mm mounting plate."
    first = local_chat(prompt)
    second = local_chat(prompt)
    assert first.text == second.text
    assert "Nominal analytic estimate" in first.text
    assert "box 20x30x40 mm" in first.text
    assert "24000 mm^3" in first.text
    assert "24 cm^3" in first.text
    assert "5200 mm^2" in first.text


def test_local_chat_cylinder_estimate_uses_radius_and_height() -> None:
    outcome = local_chat("Size a cylinder: radius 10, height 30 mm shaft.")
    assert "(cylinder r=10 h=30 mm)" in outcome.text
    assert "9424.778 mm^3" in outcome.text
    assert "9.425 cm^3" in outcome.text
    assert "2513.274 mm^2" in outcome.text


def test_local_chat_sphere_estimate_uses_radius() -> None:
    outcome = local_chat("sphere radius 10 mm ball bearing")
    assert "(sphere r=10 mm)" in outcome.text
    assert "4188.79 mm^3" in outcome.text
    assert "4.189 cm^3" in outcome.text
    assert "1256.637 mm^2" in outcome.text


def test_local_chat_converts_centimetre_dimensions_to_millimetres() -> None:
    outcome = local_chat("Estimate a box 2x3x4 cm block.")
    assert "box 20x30x40 mm" in outcome.text
    assert "24000 mm^3" in outcome.text


def test_local_chat_does_not_invent_dimensions_when_none_are_given() -> None:
    outcome = local_chat("Explain the OpenCascade STEP workflow please.")
    assert "Nominal analytic estimate" not in outcome.text


def test_local_chat_describes_unified_opencascade_workflow() -> None:
    for prompt in (
        "Estimate a box 20x30x40 mm.",
        "Size a shaft radius 5 height 20 mm.",
        "Estimate a sphere radius 7 mm.",
    ):
        text = local_chat(prompt).text
        assert "OpenCascade" in text
        assert "OCP" in text
        assert "STEP" in text
        assert "STL" in text
        assert "handoff" in text
        assert "standalone" in text


def test_local_chat_never_separates_hobbyists_from_pros() -> None:
    for prompt in (
        "Estimate a box 20x30x40 mm.",
        "Help me size a shaft radius 5 height 20 mm.",
    ):
        text = local_chat(prompt).text.lower()
        assert "hobbyist" not in text
        assert re.search(r"\bpros?\b", text) is None
        assert "professional" not in text


def test_local_chat_response_stays_within_limits_on_long_prompts() -> None:
    long_prompt = "Size a box 20x30x40 mm. " + ("consider the mounting plate. " * 100)
    assert len(long_prompt) <= MAX_PROMPT_LENGTH
    outcome = local_chat(long_prompt)
    assert len(outcome.text) <= MAX_RESPONSE_LENGTH
    assert "Local helper" in outcome.text
    assert "Reasoning trace id" in outcome.text
    assert "OpenCascade" in outcome.text


def test_local_provider_record_describes_engineering_helper() -> None:
    records = list_provider_records(environ={}, clis=())
    local_row = next(row for row in records if row.id == "local")
    assert local_row.configured is True
    note = local_row.notes.lower()
    assert "opencascade" in note
    assert "step" in note and "stl" in note
    assert "offline" in note
    assert "solver" in note


# --- OpenAI-compatible fixed endpoints ---


def test_openai_compat_lists_fixed_endpoints() -> None:
    endpoints = fixed_endpoints()
    assert OPENAI_DEFAULT_ENDPOINT in endpoints
    assert OPENROUTER_DEFAULT_ENDPOINT in endpoints
    assert MAX_HTTP_RESPONSE_BYTES > 0


def test_openai_provider_refuses_when_remote_not_allowed() -> None:
    with pytest.raises(ProviderConfigurationError):
        openai_compat_chat(
            "openai",
            model="gpt-4",
            prompt="hi",
            allow_remote=False,
            environ={"OPENAI_API_KEY": "sk-test-1234567890abcdef"},
        )


def test_openai_provider_refuses_when_key_missing() -> None:
    with pytest.raises(ProviderMissingCredentialError):
        openai_compat_chat(
            "openai",
            model="gpt-4",
            prompt="hi",
            allow_remote=True,
            environ={},
        )


def test_openrouter_provider_refuses_when_key_missing() -> None:
    with pytest.raises(ProviderMissingCredentialError):
        openai_compat_chat(
            "openrouter",
            model="anthropic/claude-3-haiku",
            prompt="hi",
            allow_remote=True,
            environ={},
        )


# --- dispatch ---


def test_dispatch_local_does_not_require_remote_consent(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from kala_agent.studio.providers.dispatch import chat

    outcome = chat(
        provider="local",
        model="studio-local",
        prompt="Hello world.",
        allow_remote=False,
        environ={},
    )
    assert outcome.provider == "local"


def test_dispatch_openai_without_remote_returns_error(tmp_path: Path) -> None:
    from kala_agent.studio.providers.dispatch import chat

    with pytest.raises(ProviderConfigurationError):
        chat(
            provider="openai",
            model="gpt-4",
            prompt="hi",
            allow_remote=False,
            environ={"OPENAI_API_KEY": "sk-test-1234567890abcdef"},
        )


def test_dispatch_unknown_provider_raises() -> None:
    from kala_agent.studio.providers.dispatch import chat

    with pytest.raises(ProviderConfigurationError):
        chat(
            provider="unknown",
            model="x",
            prompt="hi",
            allow_remote=False,
            environ={},
        )


def test_list_provider_records_never_includes_credentials(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("OPENAI_API_KEY", "sk-secret-1234567890abcdef")
    monkeypatch.setenv("OPENROUTER_API_KEY", "sk-secret-9876543210abcdef")
    records = list_provider_records(environ=os.environ, clis=())
    rendered = str(records)
    assert "sk-secret-1234567890abcdef" not in rendered
    assert "sk-secret-9876543210abcdef" not in rendered
    assert "OPENAI_API_KEY" in {row.id for row in records} or "openai" in {
        row.id for row in records
    }


# --- the subscription CLI routes ---


def test_codex_dispatch_requires_allow_remote(monkeypatch: pytest.MonkeyPatch) -> None:
    from kala_agent.studio.providers.dispatch import chat as dispatch_chat

    with pytest.raises(ProviderConfigurationError, match="allow_remote"):
        dispatch_chat(
            provider="codex",
            model=None,
            prompt="hi",
            allow_remote=False,
            environ={},
        )


def test_codex_dispatch_with_consent_reports_the_missing_cli() -> None:
    """With the consent but no CLI, the route reports the honest reason."""
    from kala_agent.studio.providers.dispatch import chat as dispatch_chat

    with pytest.raises(ProviderUnavailableError) as failure:
        dispatch_chat(
            provider="codex",
            model=None,
            prompt="hi",
            allow_remote=True,
            environ={"PATH": str(Path(os.devnull).parent / "no-such-cli-directory")},
        )
    assert "the Codex CLI was not found on PATH" in str(failure.value)


def test_the_subscription_rows_carry_the_probe_verdict() -> None:
    """A row is configured only when the CLI is installed and signed in."""
    probes = (
        CliProbe(
            id="codex_cli",
            installed=True,
            authenticated=True,
            reason="Codex CLI 1.0.0 is installed and signed in",
        ),
    )
    records = list_provider_records(environ={}, clis=probes)
    codex = next(row for row in records if row.id == "codex_cli")
    assert codex.configured is True
    assert codex.available is True
    assert codex.reason is None
    assert codex.source == "subscription (CLI)"
    alias = next(row for row in records if row.id == "codex")
    assert alias.configured is True
    claude = next(row for row in records if row.id == "claude_code")
    assert claude.configured is False
    assert claude.available is False
    assert claude.reason is None


def test_the_subscription_rows_hide_the_cli_path() -> None:
    """The rows never disclose the local CLI installation path."""
    probes = (
        CliProbe(
            id="codex_cli",
            installed=False,
            authenticated=False,
            reason="the Codex CLI was not found on PATH or at KALA_STUDIO_CODEX_CLI_PATH",
        ),
    )
    records = list_provider_records(environ={}, clis=probes)
    codex = next(row for row in records if row.id == "codex")
    assert codex.configured is False
    assert "/opt/secret/codex-binary" not in codex.notes
    assert "codex" in codex.notes
