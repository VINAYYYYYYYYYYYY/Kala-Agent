"""The OpenAI Codex CLI chat provider.

The Codex CLI is the operator's own, already-authenticated command-line
client for their ChatGPT subscription. Kala never holds the credential: the
CLI reads its own login from its own ``CODEX_HOME``, and Kala only runs
the CLI.

The exact argv
--------------

.. code-block:: text

    codex exec --sandbox read-only --skip-git-repo-check --ephemeral \\
           --ignore-user-config --json [--model <model>]

* ``exec`` is the CLI's non-interactive mode; it reads the prompt from
  standard input when no prompt argument is given.
* ``--sandbox read-only`` confines every command the model may run to a
  read-only sandbox, so the turn cannot change anything on the host.
* ``--skip-git-repo-check`` lets the turn run in the private temporary
  directory, which is not a repository.
* ``--ephemeral`` keeps the turn out of the CLI's session store, so a chat
  turn leaves no transcript behind.
* ``--ignore-user-config`` does not load ``$CODEX_HOME/config.toml``. The
  operator's MCP servers, hooks and custom instructions therefore stay out
  of the turn, and the CLI falls back to its own default model. The login
  still comes from ``CODEX_HOME``.
* ``--json`` prints the turn's events as JSON lines. The assistant's final
  message is the ``item.completed`` event whose item is an
  ``agent_message``.
* ``--model`` is passed through only when the request names a model; the
  alias ``default`` means the CLI's own default model and is therefore not
  passed at all.

**Honest limit:** the Codex CLI has no flag that removes its shell tool.
``--sandbox read-only`` plus ``--ignore-user-config`` is the strongest
"read-only, no agentic extras" posture the CLI offers.

The prompt is written to the child's standard input and is never placed on
the argv. The child runs in a private temporary directory with a sanitized
environment; see :mod:`kala_agent.studio.providers.cli_runtime` for the
shared contract.
"""

from __future__ import annotations

import json
import tempfile
from collections.abc import Mapping
from pathlib import Path
from typing import Final

from kala_agent.studio.providers.base import (
    ChatOutcome,
    ProviderProtocolError,
    ProviderSubprocessError,
)
from kala_agent.studio.providers.cli_runtime import (
    CliInvocation,
    CliRun,
    CliSpec,
    consent_message,
    failure_excerpt,
    failure_text,
    program_or_reason,
    run,
    timeout_seconds,
)

#: The provider id of the Codex CLI route.
PROVIDER_ID: Final[str] = "codex_cli"

#: The historical provider id of the Codex CLI route.
#:
#: The Studio published ``codex`` before the subscription routes existed.
#: The id stays as an alias so an old saved preference still resolves.
LEGACY_PROVIDER_ID: Final[str] = "codex"

#: The identity and the fixed probe commands of the Codex CLI.
SPEC: Final[CliSpec] = CliSpec(
    id=PROVIDER_ID,
    name="Codex CLI",
    binary="codex",
    path_override_env="KALA_STUDIO_CODEX_CLI_PATH",
    version_args=("--version",),
    auth_args=("login", "status"),
    env_passthrough=("CODEX_HOME",),
)

#: The model alias that means "the CLI's own default model".
DEFAULT_MODEL: Final[str] = "default"


def build_argv(model: str | None = None) -> tuple[str, ...]:
    """Return the exact argv of one Codex CLI turn."""
    argv: list[str] = [
        "exec",
        "--sandbox",
        "read-only",
        "--skip-git-repo-check",
        "--ephemeral",
        "--ignore-user-config",
        "--json",
    ]
    if model and model != DEFAULT_MODEL:
        argv.append("--model")
        argv.append(model)
    return tuple(argv)


def chat(
    model: str | None,
    prompt: str,
    environ: Mapping[str, str],
) -> ChatOutcome:
    """Run one Codex CLI turn.

    Raises the typed provider failure the Studio route maps onto a
    sanitized status. The CLI's own text is bounded and never carries a
    credential.
    """
    program = program_or_reason(SPEC, environ)
    with tempfile.TemporaryDirectory(prefix="kala-codex-") as workdir:
        argv = build_argv(model)
        outcome = run(
            CliInvocation(
                spec=SPEC,
                program=program,
                argv=argv,
                stdin=prompt.encode("utf-8"),
                timeout=float(timeout_seconds(environ)),
                environ=environ,
                workdir=Path(workdir),
            )
        )
    if not outcome.succeeded():
        raise ProviderSubprocessError(failure_message(outcome))
    text, failure = parse_events(outcome.stdout)
    if failure is not None:
        raise ProviderSubprocessError(failure)
    if text is None or not text.strip():
        raise ProviderProtocolError("the Codex CLI produced no assistant message")
    return ChatOutcome(
        text=text,
        provider=PROVIDER_ID,
        model=(model if model else None) or DEFAULT_MODEL,
        offline=False,
    )


def failure_message(outcome: CliRun) -> str:
    """Return the bounded failure text of a CLI run that did not exit zero."""
    detail = failure_excerpt(outcome.stderr)
    if detail:
        return detail
    detail = failure_excerpt(outcome.stdout)
    if detail:
        return detail
    status = outcome.status
    return f"the Codex CLI exited with status {status if status is not None else -1}"


def parse_events(stdout: bytes) -> tuple[str | None, str | None]:
    """Read the JSON lines the CLI prints with ``--json``.

    The assistant's final message is the last ``item.completed`` event
    whose item is an ``agent_message``. A ``turn.failed`` or ``error``
    event is the failure the operator sees.
    """
    text = stdout.decode("utf-8", errors="replace")
    answer: str | None = None
    failure: str | None = None
    parsed_any = False
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if not isinstance(event, dict):
            continue
        parsed_any = True
        kind = event.get("type")
        kind = kind if isinstance(kind, str) else ""
        candidate: str | None = None
        if kind == "item.completed":
            item = event.get("item")
            if isinstance(item, dict) and item.get("type") == "agent_message":
                message = item.get("text")
                if isinstance(message, str):
                    answer = message
        elif kind == "turn.failed":
            candidate = event_message(event.get("error"))
        elif kind == "error":
            candidate = event_message(event.get("message"))
        if candidate is not None and failure is None:
            failure = candidate
    if not parsed_any:
        raise ProviderProtocolError("the Codex CLI output carried no JSON event")
    return answer, failure


def event_message(value: object) -> str:
    """Return the bounded text of a failure event."""
    if isinstance(value, str):
        return failure_text(value)
    if isinstance(value, dict):
        message = value.get("message")
        if isinstance(message, str):
            return failure_text(message)
    return ""


def consent_refusal() -> str:
    """Return the refusal the route shows when the operator did not consent."""
    return consent_message(PROVIDER_ID)


__all__ = (
    "DEFAULT_MODEL",
    "LEGACY_PROVIDER_ID",
    "PROVIDER_ID",
    "SPEC",
    "build_argv",
    "chat",
    "consent_refusal",
    "event_message",
    "failure_message",
    "parse_events",
)
