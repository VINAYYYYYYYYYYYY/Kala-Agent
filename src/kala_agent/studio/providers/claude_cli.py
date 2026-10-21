"""The Claude Code CLI chat provider.

Claude Code is the operator's own, already-authenticated command-line
client for their Claude subscription. Kala never holds the credential: the
CLI reads its own login from its own configuration directory, and Kala only
runs the CLI.

The exact argv
--------------

.. code-block:: text

    claude --print --output-format json --safe-mode --tools "" \\
           --strict-mcp-config --permission-mode dontAsk \\
           --no-session-persistence [--model <model>]

* ``--print`` runs the non-interactive mode the CLI documents for scripted
  and SDK callers.
* ``--output-format json`` makes the CLI answer with one JSON object whose
  ``result`` field holds the assistant's final text.
* ``--safe-mode`` switches off the customizations a normal session loads -
  ``CLAUDE.md``, skills, plugins, hooks, MCP servers, custom commands and
  agents - while leaving the login, the model selection and the
  permissions alone. A chat turn must not run the operator's hooks.
* ``--tools ""`` removes every built-in tool, so the model cannot read a
  file, run a command or fetch a URL. This is the hard guarantee that the
  turn is a plain question and answer.
* ``--strict-mcp-config`` makes sure no MCP server is connected, even when
  the operator configured one.
* ``--permission-mode dontAsk`` denies anything that would still ask for a
  permission, so the turn can never block on a question.
* ``--no-session-persistence`` keeps the turn out of the CLI's session
  store, so a chat turn leaves no transcript behind.
* ``--model`` is passed through only when the request names a model; the
  alias ``default`` means the CLI's own default model and is therefore not
  passed at all.

The prompt is written to the child's standard input and is never placed on
the argv, so it cannot leak through the process table. The child runs in a
private temporary directory with a sanitized environment; see
:mod:`kala_agent.studio.providers.cli_runtime` for the shared contract.
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
    CLI_TIMEOUT_ENV,
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

#: The provider id of the Claude Code route.
PROVIDER_ID: Final[str] = "claude_code"

#: The identity and the fixed probe commands of the Claude Code CLI.
SPEC: Final[CliSpec] = CliSpec(
    id=PROVIDER_ID,
    name="Claude Code CLI",
    binary="claude",
    path_override_env="KALA_STUDIO_CLAUDE_CLI_PATH",
    version_args=("--version",),
    auth_args=("auth", "status"),
    env_passthrough=("CLAUDE_CONFIG_DIR",),
)

#: The model alias that means "the CLI's own default model".
DEFAULT_MODEL: Final[str] = "default"


def build_argv(model: str | None = None) -> tuple[str, ...]:
    """Return the exact argv of one Claude Code turn."""
    argv: list[str] = [
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
    """Run one Claude Code turn.

    Raises the typed provider failure the Studio route maps onto a
    sanitized status. The CLI's own text is bounded and never carries a
    credential.
    """
    program = program_or_reason(SPEC, environ)
    with tempfile.TemporaryDirectory(prefix="kala-claude-") as workdir:
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
    text, observed = parse_result(outcome.stdout)
    if not text.strip():
        raise ProviderProtocolError("the Claude Code CLI produced no assistant text")
    return ChatOutcome(
        text=text,
        provider=PROVIDER_ID,
        model=observed or (model if model else None) or DEFAULT_MODEL,
        offline=False,
    )


def failure_message(outcome: CliRun) -> str:
    """Return the bounded failure text of a CLI run that did not exit zero.

    The CLI's own documentation says that a failure inside a run - a
    missing login, for example - is printed as the result on standard
    output, so the JSON result is read first when it is there.
    """
    stdout = outcome.stdout
    stderr = outcome.stderr
    status = outcome.status
    try:
        payload = json.loads(stdout.decode("utf-8", errors="replace").strip())
    except ValueError:
        payload = None
    if isinstance(payload, dict):
        result = payload.get("result")
        if isinstance(result, str):
            detail = failure_text(result)
            if detail:
                return detail
    detail = failure_excerpt(stderr)
    if detail:
        return detail
    detail = failure_excerpt(stdout)
    if detail:
        return detail
    return f"the Claude Code CLI exited with status {status if status is not None else -1}"


def parse_result(stdout: bytes) -> tuple[str, str | None]:
    """Parse the JSON object the CLI prints with ``--output-format json``.

    The object carries the assistant's final text in ``result``. A run that
    failed reports ``is_error``, and its text is the failure the operator
    sees. The model that answered is read from ``modelUsage``, the
    per-model cost breakdown the CLI publishes.
    """
    text = stdout.decode("utf-8", errors="replace").strip()
    try:
        payload = json.loads(text)
    except ValueError as error:
        raise ProviderProtocolError("the Claude Code CLI output was not valid JSON") from error
    if not isinstance(payload, dict):
        raise ProviderProtocolError("the Claude Code CLI output was not a JSON object")
    if payload.get("is_error") is True:
        result = payload.get("result")
        detail = failure_text(result) if isinstance(result, str) else ""
        if not detail:
            detail = "the Claude Code CLI reported a failed turn"
        raise ProviderSubprocessError(detail)
    result = payload.get("result")
    if not isinstance(result, str):
        raise ProviderProtocolError("the Claude Code CLI output carried no result")
    observed: str | None = None
    usage = payload.get("modelUsage")
    if isinstance(usage, dict):
        for key in usage:
            observed = key
            break
    if observed is None:
        model = payload.get("model")
        if isinstance(model, str):
            observed = model
    return result, observed


#: The environment variable that configures the wall-clock budget.
TIMEOUT_ENV: Final[str] = CLI_TIMEOUT_ENV


def consent_refusal() -> str:
    """Return the refusal the route shows when the operator did not consent."""
    return consent_message(PROVIDER_ID)


__all__ = (
    "DEFAULT_MODEL",
    "PROVIDER_ID",
    "SPEC",
    "TIMEOUT_ENV",
    "build_argv",
    "chat",
    "consent_refusal",
    "failure_message",
    "parse_result",
)
