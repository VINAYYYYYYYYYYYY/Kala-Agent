"""The OpenCode CLI chat provider.

OpenCode is the operator's own command-line agent. Kala never holds a
credential: OpenCode reads its own provider logins from its own data
directory, and Kala only runs the CLI.

The exact argv
--------------

.. code-block:: text

    opencode run --standalone --format json --agent kala-chat [--model <provider/model>]

* ``run`` is OpenCode's non-interactive mode. It reads the prompt from
  standard input when no message argument is given.
* ``--standalone`` runs a private server for the turn instead of joining
  the operator's background service, so a chat turn cannot disturb or
  inherit a running session.
* ``--format json`` prints the turn's events as JSON lines. The
  assistant's text is the ``text`` event's ``part.text``.
* ``--agent kala-chat`` selects the agent the turn writes into the private
  working directory. That agent denies every tool, so the model cannot
  read a file, run a command or fetch a URL.
* ``--model`` is passed through only when the request names a model. The
  alias ``default`` means OpenCode's own default model and is therefore
  not passed at all. OpenCode names its models ``provider/model``.

The sandboxing configuration
----------------------------

The turn runs in a private temporary directory that holds an
``opencode.json`` (and the same document in ``OPENCODE_CONFIG_CONTENT``).
The document gives the ``kala-chat`` agent a permission rule that denies
every action on every resource, and the same rule at the top level so it
reaches every agent. The deprecated ``permission`` and ``tools`` spellings
carry the same intent, so the rule holds whichever configuration shape the
installed OpenCode reads.

The prompt is written to the child's standard input and is never placed on
the argv. The child runs with a sanitized environment; see
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

#: The provider id of the OpenCode CLI route.
PROVIDER_ID: Final[str] = "opencode_cli"

#: The identity and the fixed probe commands of the OpenCode CLI.
SPEC: Final[CliSpec] = CliSpec(
    id=PROVIDER_ID,
    name="OpenCode CLI",
    binary="opencode",
    path_override_env="KALA_STUDIO_OPENCODE_CLI_PATH",
    version_args=("--version",),
    auth_args=("auth", "list", "--format", "json"),
    env_passthrough=(),
)

#: The model alias that means "the CLI's own default model".
DEFAULT_MODEL: Final[str] = "default"

#: The name of the agent the turn runs as.
AGENT_NAME: Final[str] = "kala-chat"

#: The name of the configuration document written into the working directory.
CONFIG_FILE_NAME: Final[str] = "opencode.json"

#: The environment variable that carries the same document inline.
CONFIG_CONTENT_ENV: Final[str] = "OPENCODE_CONFIG_CONTENT"

#: The sandboxing configuration of the turn.
#:
#: Every action is denied on every resource. The document is written both
#: as the deprecated ``permission`` object and as the current
#: ``permissions`` rule list, and the ``tools`` map denies every built-in
#: tool by name, so the intent survives whichever shape the installed
#: OpenCode reads.
SANDBOX_CONFIG: Final[str] = """{
  "$schema": "https://opencode.ai/config.json",
  "permission": { "*": "deny" },
  "permissions": [ { "action": "*", "resource": "*", "effect": "deny" } ],
  "agent": {
    "kala-chat": {
      "description": "Kala Studio chat turn: no tools, no file access, no network fetch.",
      "mode": "primary",
      "permission": { "*": "deny" },
      "permissions": [ { "action": "*", "resource": "*", "effect": "deny" } ],
      "tools": {
        "bash": false,
        "edit": false,
        "write": false,
        "patch": false,
        "read": false,
        "grep": false,
        "glob": false,
        "list": false,
        "todowrite": false,
        "webfetch": false,
        "websearch": false,
        "task": false,
        "question": false,
        "lsp": false,
        "skill": false,
        "external_directory": false,
        "doom_loop": false
      }
    }
  }
}"""


def build_argv(model: str | None = None) -> tuple[str, ...]:
    """Return the exact argv of one OpenCode turn."""
    argv: list[str] = [
        "run",
        "--standalone",
        "--format",
        "json",
        "--agent",
        AGENT_NAME,
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
    """Run one OpenCode turn.

    Raises the typed provider failure the Studio route maps onto a
    sanitized status. The CLI's own text is bounded and never carries a
    credential.
    """
    program = program_or_reason(SPEC, environ)
    with tempfile.TemporaryDirectory(prefix="kala-opencode-") as workdir:
        write_config(Path(workdir))
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
                extra_env=((CONFIG_CONTENT_ENV, SANDBOX_CONFIG),),
            )
        )
    if not outcome.succeeded():
        raise ProviderSubprocessError(failure_message(outcome))
    text, failure = parse_events(outcome.stdout)
    if failure is not None:
        raise ProviderSubprocessError(failure)
    if text is None or not text.strip():
        raise ProviderProtocolError("the OpenCode CLI produced no assistant text")
    return ChatOutcome(
        text=text,
        provider=PROVIDER_ID,
        model=(model if model else None) or DEFAULT_MODEL,
        offline=False,
    )


def write_config(workdir: Path) -> None:
    """Write the sandboxing configuration into the private working directory."""
    (workdir / CONFIG_FILE_NAME).write_text(SANDBOX_CONFIG, encoding="utf-8")


def failure_message(outcome: CliRun) -> str:
    """Return the bounded failure text of a CLI run that did not exit zero."""
    detail = failure_excerpt(outcome.stderr)
    if detail:
        return detail
    detail = failure_excerpt(outcome.stdout)
    if detail:
        return detail
    status = outcome.status
    return f"the OpenCode CLI exited with status {status if status is not None else -1}"


def parse_events(stdout: bytes) -> tuple[str | None, str | None]:
    """Read the JSON lines the CLI prints with ``--format json``.

    The assistant's text is the concatenation of the ``text`` events' text.
    An ``error`` event is the failure the operator sees.
    """
    text = stdout.decode("utf-8", errors="replace")
    answer: list[str] = []
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
        if kind == "text":
            part = event.get("part")
            if isinstance(part, dict):
                chunk = part.get("text")
                if isinstance(chunk, str):
                    answer.append(chunk)
        elif kind == "error":
            candidate = event_message(event.get("error"))
        if candidate is not None and failure is None:
            failure = candidate
    if not parsed_any:
        raise ProviderProtocolError("the OpenCode CLI output carried no JSON event")
    joined = "".join(answer)
    return (joined if joined else None), failure


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
    "AGENT_NAME",
    "CONFIG_CONTENT_ENV",
    "CONFIG_FILE_NAME",
    "DEFAULT_MODEL",
    "PROVIDER_ID",
    "SANDBOX_CONFIG",
    "SPEC",
    "build_argv",
    "chat",
    "consent_refusal",
    "event_message",
    "failure_message",
    "parse_events",
    "write_config",
)
