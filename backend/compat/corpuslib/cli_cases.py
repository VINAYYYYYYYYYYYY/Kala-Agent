"""CLI and package-launch contract cases.

The inventory is read from the live ``argparse`` parsers. A few commands are
executed in-process against a temporary data directory with a fixed clock so
their JSON shape and exit codes are frozen without touching the operator's
data. Help prose is summarized, not copied wholesale.
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
from pathlib import Path
from typing import Final

from corpuslib.common import FixedClock, call_or_error, fixed_journal_clock, sha256_text

MAX_HELP_SNAPSHOT: Final[int] = 0


def _json_default(value: object) -> object:
    if isinstance(value, Path):
        return str(value)
    return repr(value)


def _action_record(action: argparse.Action) -> dict[str, object]:
    choices = getattr(action, "choices", None)
    return {
        "option_strings": list(action.option_strings),
        "dest": action.dest,
        "required": bool(action.required),
        "nargs": action.nargs,
        "default": action.default if _is_json_value(action.default) else repr(action.default),
        "choices": None
        if choices is None
        else [str(choice) for choice in choices],
        "type": None if action.type is None else getattr(action.type, "__name__", str(action.type)),
        "help": action.help,
    }


def _is_json_value(value: object) -> bool:
    try:
        json.dumps(value)
    except (TypeError, ValueError):
        return False
    return True


def _parser_inventory(parser: argparse.ArgumentParser) -> dict[str, object]:
    top_level: list[dict[str, object]] = []
    commands: list[dict[str, object]] = []
    for action in parser._actions:
        if isinstance(action, argparse._SubParsersAction):
            for name, subparser in action.choices.items():
                commands.append(
                    {
                        "name": name,
                        "help": next(
                            (
                                subaction.help
                                for subaction in action._choices_actions
                                if subaction.dest == name
                            ),
                            None,
                        ),
                        "options": [
                            _action_record(subaction)
                            for subaction in subparser._actions
                            if not isinstance(subaction, argparse._SubParsersAction)
                        ],
                    }
                )
            continue
        top_level.append(_action_record(action))
    usage = parser.format_usage().strip()
    help_text = parser.format_help()
    return {
        "prog": parser.prog,
        "description": parser.description,
        "epilog": parser.epilog,
        "usage_first_line": usage.splitlines()[0] if usage else "",
        "help_length": len(help_text),
        "help_sha256": sha256_text(help_text),
        "help_first_line": help_text.splitlines()[0] if help_text else "",
        "top_level_options": top_level,
        "commands": sorted(commands, key=lambda item: str(item["name"])),
    }


def _normalize_paths(text: str, root: Path) -> str:
    return text.replace(str(root), "<DATA_DIR>").replace(str(root.resolve()), "<DATA_DIR>")


def _run_main(
    argv: list[str],
    application: object,
    *,
    root: Path,
) -> dict[str, object]:
    from kala_agent.cli.main import main

    stdout = io.StringIO()
    stderr = io.StringIO()
    try:
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            exit_code = main(
                argv,
                application=application,  # type: ignore[arg-type]
                stdout=stdout,
                stderr=stderr,
            )
    except SystemExit as error:
        return {
            "argv": argv,
            "exit_code": int(error.code or 0),
            "stdout_text": _normalize_paths(stdout.getvalue(), root),
            "stderr_text": _normalize_paths(stderr.getvalue(), root),
            "system_exit": True,
        }
    return {
        "argv": argv,
        "exit_code": exit_code,
        "stdout_text": _normalize_paths(stdout.getvalue(), root),
        "stderr_text": _normalize_paths(stderr.getvalue(), root),
        "system_exit": False,
    }


def _core_command_cases(root: Path) -> list[dict[str, object]]:
    from kala_agent.application import KalaApplication
    from kala_agent.config import Settings

    settings = Settings(data_dir=root / "data")
    clock = FixedClock()
    application = KalaApplication(settings, clock=clock)
    journal_clock = fixed_journal_clock(clock)
    journal_clock.__enter__()
    try:
        executions = [
            _run_main(["init"], application, root=root),
            _run_main(["domains"], application, root=root),
            _run_main(
                ["evaluate", "--domain", "mechanical", "--param", "length_mm=140"],
                application,
                root=root,
            ),
            _run_main(
                [
                    "evaluate",
                    "--domain",
                    "mechanical",
                    "--param",
                    "thickness_mm=0.5",
                    "--param",
                    "minimum_thickness_mm=5.0",
                ],
                application,
                root=root,
            ),
            _run_main(["events", "--limit", "10"], application, root=root),
            _run_main(
                ["workflow-status", "--workflow-id", "missing-workflow"],
                application,
                root=root,
            ),
            _run_main(
                ["commit-revision", "--request-id", "missing-commit"],
                application,
                root=root,
            ),
            _run_main([], application, root=root),
            _run_main(["--help"], application, root=root),
        ]
        help_run = executions[-1]
        help_text = str(help_run.get("stdout_text", ""))
        help_lines = help_text.splitlines()
        help_run["stdout_full_length"] = len(help_text)
        help_run["stdout_full_sha256"] = sha256_text(help_text)
        help_run["stdout_text"] = "\n".join(help_lines[:20]) + (
            "\n... (truncated; see stdout_full_sha256)" if len(help_lines) > 20 else ""
        )
    finally:
        journal_clock.__exit__(None, None, None)
        application.close()
    return executions


def _target_record(target: object) -> dict[str, object]:
    return {
        "host": target.host,  # type: ignore[attr-defined]
        "port": target.port,  # type: ignore[attr-defined]
        "url": target.url,  # type: ignore[attr-defined]
    }


def _core_inventory() -> dict[str, object]:
    from kala_agent import cli
    from kala_agent.cli.main import (
        DEFAULT_EVENT_LIMIT,
        EXIT_ERROR,
        EXIT_FAILED_OUTCOME,
        EXIT_OK,
        EXIT_USAGE,
        MAX_EVENT_LIMIT,
        PROGRAM_NAME,
        build_parser,
        build_serve_target,
    )

    targets: list[dict[str, object]] = []
    for host, port in (
        (None, None),
        ("127.0.0.1", 8000),
        ("::1", 9000),
        ("localhost", 8123),
        ("0.0.0.0", 8000),
        ("192.168.1.5", 8000),
        ("127.0.0.1", 0),
        ("127.0.0.1", 70_000),
    ):
        result = call_or_error(lambda host=host, port=port: _target_record(build_serve_target(host, port)))
        targets.append({"host": host, "port": port, "expected": result})
    return {
        "program_name": PROGRAM_NAME,
        "module": getattr(cli, "__name__", "kala_agent.cli"),
        "exit_codes": {
            "ok": EXIT_OK,
            "error": EXIT_ERROR,
            "usage": EXIT_USAGE,
            "failed_outcome": EXIT_FAILED_OUTCOME,
        },
        "default_event_limit": DEFAULT_EVENT_LIMIT,
        "max_event_limit": MAX_EVENT_LIMIT,
        "serve_targets": targets,
        "parser": _parser_inventory(build_parser()),
    }


def _studio_inventory() -> dict[str, object]:
    from kala_agent.studio.boundary import (
        STUDIO_TOKEN_ENV_VAR,
        STUDIO_TOKEN_FILE_NAME,
        validate_studio_token,
    )
    from kala_agent.studio.cli import (
        DEFAULT_HOST,
        DEFAULT_PORT,
        EXIT_ERROR,
        EXIT_OK,
        EXIT_USAGE,
        PROGRAM_NAME,
        build_parser,
        build_target,
    )

    targets: list[dict[str, object]] = []
    for host, port in (
        (None, None),
        ("127.0.0.1", 8765),
        ("::1", 9000),
        ("localhost", 8123),
        ("0.0.0.0", 8765),
        ("192.168.1.5", 8765),
        ("127.0.0.1", 0),
        ("127.0.0.1", 70_000),
    ):
        result = call_or_error(lambda host=host, port=port: _target_record(build_target(host, port)))
        targets.append({"host": host, "port": port, "expected": result})

    token_cases = []
    for token in (
        "corpus-studio-token-0123456789abcdef",
        "short",
        "a" * 129,
        "contains a space 0123456789",
        "",
    ):
        token_cases.append(
            {"token_length": len(token), "expected": call_or_error(lambda token=token: validate_studio_token(token))}
        )
    return {
        "program_name": PROGRAM_NAME,
        "default_host": DEFAULT_HOST,
        "default_port": DEFAULT_PORT,
        "exit_codes": {"ok": EXIT_OK, "error": EXIT_ERROR, "usage": EXIT_USAGE},
        "token_environment_variable": STUDIO_TOKEN_ENV_VAR,
        "token_file_name": STUDIO_TOKEN_FILE_NAME,
        "targets": targets,
        "token_validation": token_cases,
        "parser": _parser_inventory(build_parser()),
        "electron_launch_contract": {
            "source": "migration audit inventory; not re-measured by this corpus",
            "arguments": ["--host", "127.0.0.1", "--port", "<port>"],
            "authorization_environment_variable": "KALA_STUDIO_TOKEN",
            "readiness_timeout_seconds": 15,
            "termination": "SIGTERM, then SIGKILL after 3 seconds",
            "packaged_backend_path": "resources/backend/kala-studio[.exe]",
            "loopback_only": True,
            "stdout": "one JSON launch banner before serving",
        },
    }


def build(root: Path) -> dict[str, object]:
    return {
        "corpus_schema_version": 1,
        "kind": "cli_launch",
        "core": _core_inventory(),
        "studio": _studio_inventory(),
        "executions": _core_command_cases(root),
        "normalizations": [
            {
                "field": "temporary data directory paths in stdout/stderr",
                "value": "<DATA_DIR>",
                "reason": "the fixture never records the operator's filesystem paths",
            }
        ],
        "notes": [
            "Commands run in-process with an injected fixed clock and a temporary data directory.",
            "No serve command is started; only the resolved launch contract is recorded.",
            "Help output is summarized by usage line, length, and digest; prose is not frozen.",
        ],
        "deterministic_overrides": {
            "application_clock": "fixed one-second sequence starting 2026-01-01T00:00:00Z",
            "journal_clock": (
                "EventJournal uses datetime.now(UTC) in production; the fixture harness "
                "substitutes a fixed clock for reproducible event timestamps and digests"
            ),
            "temporary_paths": "replaced with <DATA_DIR>",
        },
    }


__all__ = ("build",)
