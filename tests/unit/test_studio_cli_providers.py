"""Tests for the three subscription-CLI chat providers.

Every case runs a **fake** executable that lives in a temporary directory
and is reached through the documented path-override variable or a temporary
``PATH``. No test ever starts the real ``claude``, ``codex`` or
``opencode``, no test opens a network connection, and no test reads a
credential, an auth file or a settings file.

The fake executable records its own argv, its own standard input, its own
environment and its own working directory, so the assertions can prove the
contract instead of assuming it.
"""

from __future__ import annotations

import json
import stat
import sys
from collections.abc import Iterator
from pathlib import Path

import pytest

from kala_agent.studio.providers import cli_runtime
from kala_agent.studio.providers.base import (
    ProviderConfigurationError,
    ProviderProtocolError,
    ProviderSubprocessError,
    ProviderTimeoutError,
    ProviderUnavailableError,
    TooLargeResponseError,
)
from kala_agent.studio.providers.claude_cli import build_argv as claude_argv
from kala_agent.studio.providers.claude_cli import chat as claude_chat
from kala_agent.studio.providers.codex_cli import build_argv as codex_argv
from kala_agent.studio.providers.codex_cli import chat as codex_chat
from kala_agent.studio.providers.dispatch import (
    PROVIDER_IDS,
    list_provider_records,
    probe_subscription_clis,
)
from kala_agent.studio.providers.dispatch import (
    chat as dispatch_chat,
)
from kala_agent.studio.providers.opencode_cli import build_argv as opencode_argv
from kala_agent.studio.providers.opencode_cli import chat as opencode_chat

pytestmark = pytest.mark.skipif(
    sys.platform == "win32",
    reason="the fake CLIs are POSIX executables",
)

#: This module drives a **fake** executable, so the unit-suite guard that
#: keeps the real subscription CLIs from being started is lifted here.
ALLOW_FAKE_CLI_PROBE = True


@pytest.fixture(autouse=True)
def _clear_probe_cache() -> Iterator[None]:
    """Never let one case see another case's probe verdict."""
    cli_runtime.clear_probe_cache()
    yield
    cli_runtime.clear_probe_cache()


def write_fake_cli(directory: Path, name: str, body: str) -> Path:
    """Write a fake CLI into ``directory`` and return its path.

    The script records its argv, its standard input, its environment and its
    working directory, and then runs ``body`` (Python source).
    """
    directory.mkdir(parents=True, exist_ok=True)
    records = directory / "records"
    records.mkdir(exist_ok=True)
    script = directory / name
    script.write_text(
        f"#!{sys.executable}\n"
        "import os, sys\n"
        f"records = {str(records)!r}\n"
        "with open(os.path.join(records, 'argv.txt'), 'w', encoding='utf-8') as handle:\n"
        "    handle.write('\\n'.join(sys.argv[1:]))\n"
        "with open(os.path.join(records, 'stdin.txt'), 'wb') as handle:\n"
        "    handle.write(sys.stdin.buffer.read())\n"
        "with open(os.path.join(records, 'env.txt'), 'w', encoding='utf-8') as handle:\n"
        "    for name in sorted(os.environ):\n"
        "        handle.write(f'{name}={os.environ[name]}\\n')\n"
        "with open(os.path.join(records, 'cwd.txt'), 'w', encoding='utf-8') as handle:\n"
        "    handle.write(os.getcwd())\n"
        f"{body}\n",
        encoding="utf-8",
    )
    script.chmod(script.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return script


def read_records(directory: Path) -> dict[str, str]:
    """Return the records the fake CLI wrote."""
    records = directory / "records"
    result: dict[str, str] = {}
    for name in ("argv.txt", "stdin.txt", "env.txt", "cwd.txt"):
        path = records / name
        result[name] = path.read_text(encoding="utf-8") if path.is_file() else ""
    return result


def probe_body(version: str = "1.2.3", signed_in: bool = True) -> str:
    """Return a fake-CLI body that answers the probe commands."""
    return (
        "import sys\n"
        "if sys.argv[1:2] == ['--version']:\n"
        f"    print({version!r})\n"
        "    raise SystemExit(0)\n"
        "if sys.argv[1:2] and sys.argv[1] in ('auth', 'login'):\n"
        f"    raise SystemExit({0 if signed_in else 1})\n"
        "raise SystemExit(0)\n"
    )


# --- the exact argv ---


def test_claude_turn_uses_the_documented_argv_and_the_prompt_on_stdin(
    tmp_path: Path,
) -> None:
    """The argv is fixed, and the prompt travels on standard input only."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        'print(\'{"result":"hello from claude","modelUsage":{"claude-x":{}}}\')',
    )
    outcome = claude_chat(
        "claude-x",
        "the prompt text",
        {"PATH": str(cli.parent), "HOME": str(tmp_path)},
    )
    assert outcome.text == "hello from claude"
    assert outcome.model == "claude-x"
    records = read_records(tmp_path / "bin")
    assert records["argv.txt"].splitlines() == [
        "--print",
        "--output-format",
        "json",
        "--safe-mode",
        "--tools",
        "",
        "--strict-mcp-config",
        "--permission-mode",
        "dontAsk",
        "--no-session-persistence",
        "--model",
        "claude-x",
    ]
    assert records["stdin.txt"] == "the prompt text"
    assert "the prompt text" not in records["argv.txt"]


def test_claude_turn_without_a_model_leaves_the_default_to_the_cli(
    tmp_path: Path,
) -> None:
    """No ``--model`` flag without a model, and none for the alias ``default``."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        'print(\'{"result":"ok"}\')',
    )
    view = {"PATH": str(cli.parent), "HOME": str(tmp_path)}
    claude_chat(None, "hi", view)
    assert "--model" not in read_records(tmp_path / "bin")["argv.txt"].splitlines()
    claude_chat("default", "hi", view)
    assert "--model" not in read_records(tmp_path / "bin")["argv.txt"].splitlines()
    assert claude_argv("default") == claude_argv(None)


def test_codex_turn_uses_the_documented_argv_and_reads_the_jsonl(
    tmp_path: Path,
) -> None:
    """The Codex argv is fixed and the reply is the ``agent_message`` event."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "codex",
        'print(\'{"type":"item.completed","item":{"type":"agent_message",'
        '"text":"hello from codex"}}\')',
    )
    outcome = codex_chat(
        "gpt-5",
        "the prompt text",
        {"PATH": str(cli.parent), "HOME": str(tmp_path)},
    )
    assert outcome.text == "hello from codex"
    assert outcome.model == "gpt-5"
    records = read_records(tmp_path / "bin")
    assert records["argv.txt"].splitlines() == [
        "exec",
        "--sandbox",
        "read-only",
        "--skip-git-repo-check",
        "--ephemeral",
        "--ignore-user-config",
        "--json",
        "--model",
        "gpt-5",
    ]
    assert records["stdin.txt"] == "the prompt text"


def test_opencode_turn_uses_the_documented_argv_and_the_sandbox_agent(
    tmp_path: Path,
) -> None:
    """The OpenCode argv, the deny-all agent and the configuration document."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "opencode",
        'print(\'{"type":"text","part":{"text":"hello from opencode"}}\')',
    )
    outcome = opencode_chat(
        "anthropic/claude",
        "the prompt text",
        {"PATH": str(cli.parent), "HOME": str(tmp_path)},
    )
    assert outcome.text == "hello from opencode"
    records = read_records(tmp_path / "bin")
    assert records["argv.txt"].splitlines() == [
        "run",
        "--standalone",
        "--format",
        "json",
        "--agent",
        "kala-chat",
        "--model",
        "anthropic/claude",
    ]
    assert records["stdin.txt"] == "the prompt text"
    assert "OPENCODE_CONFIG_CONTENT=" in records["env.txt"]
    assert "kala-chat" in records["env.txt"]


def test_opencode_writes_the_deny_all_configuration_into_the_working_directory(
    tmp_path: Path,
) -> None:
    """The private working directory holds the sandboxing document."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "opencode",
        "import shutil\n"
        "shutil.copyfile('opencode.json', 'copied.json')\n"
        'print(\'{"type":"text","part":{"text":"ok"}}\')',
    )
    opencode_chat(None, "hi", {"PATH": str(cli.parent), "HOME": str(tmp_path)})
    records = read_records(tmp_path / "bin")
    cwd = Path(records["cwd.txt"].strip())
    assert cwd.is_absolute()
    assert cwd != Path.cwd()
    # The working directory is deleted with the turn; the copied document is
    # the witness that it was there.
    assert not cwd.exists()


# --- the environment ---


def test_the_child_environment_carries_no_kala_secret(tmp_path: Path) -> None:
    """The allowlist reaches the child; nothing of Kala's does."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        'print(\'{"result":"ok"}\')',
    )
    view = {
        "PATH": str(cli.parent),
        "HOME": str(tmp_path),
        "OPENAI_API_KEY": "sk-not-a-real-key",
        "OPENROUTER_API_KEY": "sk-or-not-a-real-key",
        "KALA_STUDIO_TOKEN": "not-a-real-token",
        "KALA_AGENT_DATA_DIR": "/not/a/real/dir",
    }
    claude_chat(None, "hi", view)
    records = read_records(tmp_path / "bin")
    assert "PATH=" in records["env.txt"]
    assert "HOME=" in records["env.txt"]
    assert "OPENAI_API_KEY" not in records["env.txt"]
    assert "OPENROUTER_API_KEY" not in records["env.txt"]
    assert "KALA_STUDIO_TOKEN" not in records["env.txt"]
    assert "KALA_AGENT_DATA_DIR" not in records["env.txt"]


def test_the_claude_login_directory_is_forwarded(tmp_path: Path) -> None:
    """The one name Claude Code needs to find its own login is forwarded."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        'print(\'{"result":"ok"}\')',
    )
    view = {
        "PATH": str(cli.parent),
        "HOME": str(tmp_path),
        "CLAUDE_CONFIG_DIR": str(tmp_path / "claude-config"),
    }
    claude_chat(None, "hi", view)
    records = read_records(tmp_path / "bin")
    assert "CLAUDE_CONFIG_DIR=" in records["env.txt"]


# --- the budget ---


def test_timeout_seconds_falls_back_and_clamps() -> None:
    """An unreadable budget falls back; a valid one is clamped."""
    assert cli_runtime.timeout_seconds({}) == cli_runtime.DEFAULT_CLI_TIMEOUT_SECONDS
    assert (
        cli_runtime.timeout_seconds({cli_runtime.CLI_TIMEOUT_ENV: "nonsense"})
        == cli_runtime.DEFAULT_CLI_TIMEOUT_SECONDS
    )
    assert (
        cli_runtime.timeout_seconds({cli_runtime.CLI_TIMEOUT_ENV: "9000"})
        == cli_runtime.MAX_CLI_TIMEOUT_SECONDS
    )
    assert (
        cli_runtime.timeout_seconds({cli_runtime.CLI_TIMEOUT_ENV: "0"})
        == cli_runtime.MIN_CLI_TIMEOUT_SECONDS
    )
    assert cli_runtime.timeout_seconds({cli_runtime.CLI_TIMEOUT_ENV: "42"}) == 42


def test_a_cli_that_overruns_is_killed_and_reported_as_a_timeout(
    tmp_path: Path,
) -> None:
    """A child that overruns the budget is killed and the turn times out."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        "import time\ntime.sleep(30)",
    )
    view = {
        "PATH": str(cli.parent),
        "HOME": str(tmp_path),
        cli_runtime.CLI_TIMEOUT_ENV: "1",
    }
    with pytest.raises(ProviderTimeoutError, match="bounded timeout"):
        claude_chat(None, "hi", view)


# --- the failures ---


def test_a_missing_cli_is_reported_as_unavailable(tmp_path: Path) -> None:
    """A missing binary is a configuration failure, not a crash."""
    view = {"PATH": str(tmp_path / "empty"), "HOME": str(tmp_path)}
    with pytest.raises(ProviderUnavailableError) as failure:
        claude_chat(None, "hi", view)
    assert "the Claude Code CLI was not found on PATH or at" in str(failure.value)
    assert "KALA_STUDIO_CLAUDE_CLI_PATH" in str(failure.value)


def test_a_failing_cli_reports_its_own_bounded_text(tmp_path: Path) -> None:
    """A non-zero exit is a subprocess failure carrying the CLI's text."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        "import sys\nsys.stderr.write('the cli is not signed in\\n')\nraise SystemExit(3)",
    )
    with pytest.raises(ProviderSubprocessError, match="not signed in"):
        claude_chat(None, "hi", {"PATH": str(cli.parent), "HOME": str(tmp_path)})


def test_a_failure_the_cli_prints_as_its_result_is_reported(tmp_path: Path) -> None:
    """A failure printed as the JSON result is reported with that text."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        'print(\'{"is_error":true,"result":"the session expired"}\')\nraise SystemExit(1)',
    )
    with pytest.raises(ProviderSubprocessError, match="the session expired"):
        claude_chat(None, "hi", {"PATH": str(cli.parent), "HOME": str(tmp_path)})


def test_a_cli_that_reports_an_error_event_is_a_subprocess_failure(
    tmp_path: Path,
) -> None:
    """The Codex JSONL error event is a subprocess failure."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "codex",
        'print(\'{"type":"error","message":"the model is unavailable"}\')',
    )
    with pytest.raises(ProviderSubprocessError, match="the model is unavailable"):
        codex_chat(None, "hi", {"PATH": str(cli.parent), "HOME": str(tmp_path)})


def test_a_cli_that_prints_no_json_is_a_protocol_failure(tmp_path: Path) -> None:
    """A reply that is not the documented shape is a protocol failure."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        "print('not json at all')",
    )
    with pytest.raises(ProviderProtocolError, match="not valid JSON"):
        claude_chat(None, "hi", {"PATH": str(cli.parent), "HOME": str(tmp_path)})


def test_a_cli_that_prints_no_event_is_a_protocol_failure(tmp_path: Path) -> None:
    """A Codex reply with no JSON event is a protocol failure."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "codex",
        "print('not json at all')",
    )
    with pytest.raises(ProviderProtocolError, match="no JSON event"):
        codex_chat(None, "hi", {"PATH": str(cli.parent), "HOME": str(tmp_path)})


def test_output_beyond_the_bound_is_refused(tmp_path: Path) -> None:
    """More standard output than the bound allows is refused."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        "import sys\nsys.stdout.write('x' * 1_200_000)",
    )
    with pytest.raises(TooLargeResponseError, match="bounded size"):
        claude_chat(None, "hi", {"PATH": str(cli.parent), "HOME": str(tmp_path)})


# --- the probe ---


def _claude_spec() -> cli_runtime.CliSpec:
    return cli_runtime.CliSpec(
        id="claude_code",
        name="Claude Code CLI",
        binary="claude",
        path_override_env="KALA_STUDIO_CLAUDE_CLI_PATH",
        version_args=("--version",),
        auth_args=("auth", "status"),
    )


def test_the_probe_reports_a_missing_cli_without_running_it(tmp_path: Path) -> None:
    """A missing binary is reported without running anything."""
    verdict = cli_runtime.probe(_claude_spec(), {"PATH": str(tmp_path / "empty")})
    assert verdict.installed is False
    assert verdict.authenticated is False
    assert verdict.usable() is False
    assert "was not found on PATH" in verdict.reason


def test_the_probe_reports_the_version_and_the_login_state(tmp_path: Path) -> None:
    """The version and the login state come from the exit statuses."""
    cli = write_fake_cli(tmp_path / "bin", "claude", probe_body(version="2.1.303"))
    verdict = cli_runtime.probe(
        _claude_spec(),
        {"PATH": str(cli.parent), "HOME": str(tmp_path)},
    )
    assert verdict.installed is True
    assert verdict.authenticated is True
    assert verdict.usable() is True
    assert verdict.reason == "Claude Code CLI 2.1.303 is installed and signed in"


def test_the_probe_reports_a_cli_that_is_not_signed_in(tmp_path: Path) -> None:
    """A CLI that reports "not signed in" is installed but not usable."""
    cli = write_fake_cli(tmp_path / "bin", "claude", probe_body(signed_in=False))
    verdict = cli_runtime.probe(
        _claude_spec(),
        {"PATH": str(cli.parent), "HOME": str(tmp_path)},
    )
    assert verdict.installed is True
    assert verdict.authenticated is False
    assert verdict.usable() is False
    assert "not signed in" in verdict.reason


def test_the_probe_never_echoes_the_cli_output(tmp_path: Path) -> None:
    """The probe reads the exit status only, never the CLI's own text."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        "import sys\n"
        "if sys.argv[1:2] == ['--version']:\n"
        "    print('1.0.0')\n"
        "    raise SystemExit(0)\n"
        "print('signed in as somebody@example.com')\n"
        "raise SystemExit(0)",
    )
    verdict = cli_runtime.probe(
        _claude_spec(),
        {"PATH": str(cli.parent), "HOME": str(tmp_path)},
    )
    assert "somebody@example.com" not in verdict.reason


# --- the dispatch ---


def test_the_route_refuses_a_cli_turn_without_remote_consent(tmp_path: Path) -> None:
    """The consent gate refuses the turn and the CLI is never started."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        'print(\'{"result":"ok"}\')',
    )
    with pytest.raises(ProviderConfigurationError, match="allow_remote"):
        dispatch_chat(
            provider="claude_code",
            model=None,
            prompt="hi",
            allow_remote=False,
            environ={"PATH": str(cli.parent), "HOME": str(tmp_path)},
        )
    assert not (tmp_path / "bin" / "records" / "argv.txt").exists()


def test_the_route_refuses_the_codex_alias_without_remote_consent(
    tmp_path: Path,
) -> None:
    """The historical ``codex`` id keeps its own refusal wording."""
    with pytest.raises(ProviderConfigurationError) as failure:
        dispatch_chat(
            provider="codex",
            model=None,
            prompt="hi",
            allow_remote=False,
            environ={"PATH": str(tmp_path / "empty"), "HOME": str(tmp_path)},
        )
    assert str(failure.value) == "provider=codex requires allow_remote=true at request time"


def test_the_route_runs_the_cli_when_the_operator_consented(tmp_path: Path) -> None:
    """With the consent, the CLI answers the turn."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        'print(\'{"result":"the cli answered"}\')',
    )
    outcome = dispatch_chat(
        provider="claude_code",
        model=None,
        prompt="hi",
        allow_remote=True,
        environ={"PATH": str(cli.parent), "HOME": str(tmp_path)},
    )
    assert outcome.text == "the cli answered"
    assert outcome.model == "default"


def test_the_codex_alias_runs_the_same_cli_as_the_codex_cli_id(tmp_path: Path) -> None:
    """``codex`` and ``codex_cli`` are two names for one CLI."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "codex",
        'print(\'{"type":"item.completed","item":{"type":"agent_message","text":"answered"}}\')',
    )
    view = {"PATH": str(cli.parent), "HOME": str(tmp_path)}
    alias = dispatch_chat(
        provider="codex",
        model=None,
        prompt="hi",
        allow_remote=True,
        environ=view,
    )
    current = dispatch_chat(
        provider="codex_cli",
        model=None,
        prompt="hi",
        allow_remote=True,
        environ=view,
    )
    assert alias.text == current.text == "answered"


def test_a_design_turn_goes_through_the_subscription_cli(tmp_path: Path) -> None:
    """A design turn asks the CLI for the DesignIntent, like the API routes."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        'print(\'{"result":"which material should it be?"}\')',
    )
    outcome = dispatch_chat(
        provider="claude_code",
        model=None,
        prompt="Design a 40 x 30 x 5 mm base with a 12 mm boss.",
        allow_remote=True,
        environ={"PATH": str(cli.parent), "HOME": str(tmp_path)},
        cad_provider="opencascade",
    )
    records = read_records(tmp_path / "bin")
    assert "DesignIntent" in records["stdin.txt"]
    assert "Design a 40 x 30 x 5 mm base" in records["stdin.txt"]
    assert "No part was built" in outcome.text


# --- the provider list ---


def test_the_provider_list_names_the_subscription_routes(tmp_path: Path) -> None:
    """The seven ids, the order, the source and the probe verdict."""
    cli = write_fake_cli(tmp_path / "bin", "claude", probe_body(version="2.1.303"))
    view = {"PATH": str(cli.parent), "HOME": str(tmp_path)}
    clis = probe_subscription_clis(view)
    records = list_provider_records(view, clis)
    assert [row.id for row in records] == list(PROVIDER_IDS)
    claude = next(row for row in records if row.id == "claude_code")
    assert claude.name == "Claude Code CLI (Anthropic subscription)"
    assert claude.configured is True
    assert claude.available is True
    assert claude.reason is None
    assert claude.source == "subscription (CLI)"
    codex = next(row for row in records if row.id == "codex_cli")
    assert codex.configured is False
    assert codex.available is False
    assert codex.reason is not None
    assert "the Codex CLI was not found on PATH" in codex.reason
    local = next(row for row in records if row.id == "local")
    assert local.available is None
    assert local.source is None


def test_the_provider_list_never_discloses_the_cli_path(tmp_path: Path) -> None:
    """The rows name the environment variable, never the installation path."""
    cli = write_fake_cli(tmp_path / "bin", "claude", probe_body())
    view = {"PATH": str(cli.parent), "HOME": str(tmp_path)}
    records = list_provider_records(view, probe_subscription_clis(view))
    rendered = json.dumps([row.model_dump(mode="json") for row in records])
    assert str(cli) not in rendered
    assert str(tmp_path) not in rendered


def test_the_working_directory_is_private(tmp_path: Path) -> None:
    """The CLI runs in a private temporary directory, not in the project."""
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        'print(\'{"result":"ok"}\')',
    )
    claude_chat(None, "hi", {"PATH": str(cli.parent), "HOME": str(tmp_path)})
    cwd = Path(read_records(tmp_path / "bin")["cwd.txt"].strip())
    assert cwd.is_absolute()
    assert cwd != Path.cwd()
    assert not cwd.exists()


def test_the_sanitized_environment_starts_empty() -> None:
    """The child environment is the allowlist, never the studio's own."""
    spec = cli_runtime.CliSpec(
        id="claude_code",
        name="Claude Code CLI",
        binary="claude",
        path_override_env="KALA_STUDIO_CLAUDE_CLI_PATH",
        version_args=("--version",),
        auth_args=("auth", "status"),
        env_passthrough=("CLAUDE_CONFIG_DIR",),
    )
    environ = {
        "PATH": "/usr/bin",
        "HOME": "/home/somebody",
        "LANG": "en_US.UTF-8",
        "OPENAI_API_KEY": "sk-secret",
        "KALA_STUDIO_TOKEN": "secret",
    }
    child = cli_runtime.child_env(spec, environ)
    assert child == {
        "PATH": "/usr/bin",
        "HOME": "/home/somebody",
        "LANG": "en_US.UTF-8",
    }


def test_the_documented_argv_is_stable() -> None:
    """The three argv vectors are part of the contract."""
    assert claude_argv(None) == (
        "--print",
        "--output-format",
        "json",
        "--safe-mode",
        "--tools",
        "",
        "--strict-mcp-config",
        "--permission-mode",
        "dontAsk",
        "--no-session-persistence",
    )
    assert codex_argv(None) == (
        "exec",
        "--sandbox",
        "read-only",
        "--skip-git-repo-check",
        "--ephemeral",
        "--ignore-user-config",
        "--json",
    )
    assert opencode_argv(None) == (
        "run",
        "--standalone",
        "--format",
        "json",
        "--agent",
        "kala-chat",
    )
    assert claude_argv("m")[0] == "--print"
    assert codex_argv("m")[-2:] == ("--model", "m")
    assert opencode_argv("m")[-2:] == ("--model", "m")


def test_the_environment_allowlist_is_the_documented_one() -> None:
    """The allowlist carries no credential name."""
    assert "PATH" in cli_runtime.ENV_ALLOWLIST
    assert "HOME" in cli_runtime.ENV_ALLOWLIST
    assert not any("API_KEY" in name for name in cli_runtime.ENV_ALLOWLIST)
    assert not any(name.startswith("KALA_") for name in cli_runtime.ENV_ALLOWLIST)


def test_the_sandbox_configuration_denies_everything() -> None:
    """The OpenCode document denies every action on every resource."""
    from kala_agent.studio.providers.opencode_cli import SANDBOX_CONFIG

    document = json.loads(SANDBOX_CONFIG)
    assert document["permission"] == {"*": "deny"}
    assert document["permissions"] == [{"action": "*", "resource": "*", "effect": "deny"}]
    agent = document["agent"]["kala-chat"]
    assert agent["permission"] == {"*": "deny"}
    assert all(value is False for value in agent["tools"].values())
    assert len(agent["tools"]) >= 17


def test_the_probe_cache_reuses_the_verdict(tmp_path: Path) -> None:
    """The verdict is cached, so a polled list does not launch the CLI."""
    counter = tmp_path / "counter"
    cli = write_fake_cli(
        tmp_path / "bin",
        "claude",
        "with open("
        f"{str(counter)!r}, 'a', encoding='utf-8') as handle:\n"
        "    handle.write('a')\n" + probe_body(),
    )
    view = {"PATH": str(cli.parent), "HOME": str(tmp_path)}
    first = cli_runtime.probe(_claude_spec(), view)
    after_first = counter.read_text(encoding="utf-8")
    second = cli_runtime.probe(_claude_spec(), view)
    assert first == second
    # The CLI ran for the version and the login check; the second probe is
    # answered from the cache and launches nothing.
    assert after_first
    assert counter.read_text(encoding="utf-8") == after_first


def test_the_path_override_wins_over_the_path(tmp_path: Path) -> None:
    """A named executable wins even when another copy sits earlier on PATH."""
    named = write_fake_cli(
        tmp_path / "named",
        "claude",
        'print(\'{"result":"the named copy"}\')',
    )
    other = write_fake_cli(
        tmp_path / "other",
        "claude",
        'print(\'{"result":"the other copy"}\')',
    )
    outcome = claude_chat(
        None,
        "hi",
        {
            "PATH": str(other.parent),
            "HOME": str(tmp_path),
            "KALA_STUDIO_CLAUDE_CLI_PATH": str(named),
        },
    )
    assert outcome.text == "the named copy"


def test_the_executable_must_be_executable(tmp_path: Path) -> None:
    """A file that cannot be executed is not a CLI."""
    script = tmp_path / "bin" / "claude"
    script.parent.mkdir(parents=True, exist_ok=True)
    script.write_text("#!/bin/sh\necho hi\n", encoding="utf-8")
    script.chmod(0o644)
    assert not cli_runtime.is_executable(script)
    assert cli_runtime.resolve_program(_claude_spec(), {"PATH": str(script.parent)}) is None


def test_the_probe_cache_is_not_shared_between_hosts(tmp_path: Path) -> None:
    """Two different executables never share one verdict."""
    first = write_fake_cli(tmp_path / "one", "claude", probe_body(version="1.0.0"))
    second = write_fake_cli(tmp_path / "two", "claude", probe_body(version="2.0.0"))
    one = cli_runtime.probe(_claude_spec(), {"PATH": str(first.parent)})
    two = cli_runtime.probe(_claude_spec(), {"PATH": str(second.parent)})
    assert one.reason != two.reason


def test_the_probe_does_not_run_a_cli_that_is_not_installed(tmp_path: Path) -> None:
    """The probe of a missing CLI never starts anything."""
    spec = cli_runtime.CliSpec(
        id="opencode_cli",
        name="OpenCode CLI",
        binary="opencode",
        path_override_env="KALA_STUDIO_OPENCODE_CLI_PATH",
        version_args=("--version",),
        auth_args=("auth", "list", "--format", "json"),
    )
    verdict = cli_runtime.probe(spec, {"PATH": str(tmp_path / "empty")})
    assert verdict.reason == (
        "the OpenCode CLI was not found on PATH or at KALA_STUDIO_OPENCODE_CLI_PATH"
    )


def test_the_version_token_is_bounded_and_safe() -> None:
    """Only a version-shaped token survives the sanitiser."""
    assert cli_runtime.sanitize_version(b"2.1.303 (Claude Code)\n") == "2.1.303"
    assert cli_runtime.sanitize_version(b"no version here\n") == "no"
    assert cli_runtime.sanitize_version(b"\n") == ""
    assert cli_runtime.sanitize_version(b"9" * 200) == "9" * 64


def test_the_failure_excerpt_is_bounded_and_control_free() -> None:
    """A failure excerpt is bounded and carries no control character."""
    excerpt = cli_runtime.failure_excerpt(b"a" * 900 + b"\x00tail")
    assert len(excerpt) <= cli_runtime.MAX_CLI_MESSAGE_LENGTH
    assert "\x00" not in excerpt


def test_the_studio_environment_is_never_inherited(tmp_path: Path) -> None:
    """The child environment is built from the request's view, not the process."""
    spec = cli_runtime.CliSpec(
        id="codex_cli",
        name="Codex CLI",
        binary="codex",
        path_override_env="KALA_STUDIO_CODEX_CLI_PATH",
        version_args=("--version",),
        auth_args=("login", "status"),
        env_passthrough=("CODEX_HOME",),
    )
    view = {"PATH": "/usr/bin", "CODEX_HOME": "/home/somebody/.codex"}
    child = cli_runtime.child_env(spec, view)
    assert child["CODEX_HOME"] == "/home/somebody/.codex"
    assert set(child) <= set(cli_runtime.ENV_ALLOWLIST) | {"CODEX_HOME"}
