"""The CLI process tree, the Windows shim search and the cmd quoting.

The three cases mirror the native product's
``providers/cli_runtime.rs`` unit tests, so the Python reference and the
shipped Rust service agree on how a CLI is found and how its tree is killed.

No real ``claude``/``codex``/``opencode`` is started: every program here is a
small temporary shell script.
"""

from __future__ import annotations

import os
import time
from pathlib import Path

import pytest

from kala_agent.studio.providers import cli_runtime
from kala_agent.studio.providers.base import ProviderSubprocessError, ProviderTimeoutError
from kala_agent.studio.providers.claude_cli import SPEC as CLAUDE_SPEC


def _fake(directory: Path, name: str, body: str) -> Path:
    program = directory / name
    program.write_text(f"#!/bin/sh\n{body}\n", encoding="utf-8")
    program.chmod(0o755)
    return program


def _environment(directory: Path, program: Path) -> dict[str, str]:
    return {
        "PATH": "/usr/bin:/bin",
        "HOME": str(directory / "home"),
        CLAUDE_SPEC.path_override_env: str(program),
    }


def _children_gone(pids: list[int]) -> bool:
    return all(not Path(f"/proc/{pid}").exists() for pid in pids)


@pytest.mark.skipif(not Path("/proc").is_dir(), reason="needs /proc to observe a pid")
def test_a_timeout_kills_the_whole_cli_process_tree(tmp_path: Path) -> None:
    """A cancelled or overrunning turn leaves no surviving grandchild."""
    pid_file = tmp_path / "pids.txt"
    program = _fake(
        tmp_path,
        "claude",
        f"sleep 300 & echo $! >> '{pid_file}'\n"
        f"sleep 300 & echo $! >> '{pid_file}'\n"
        "wait\n",
    )
    environ = _environment(tmp_path, program)
    environ[cli_runtime.CLI_TIMEOUT_ENV] = "1"

    started = time.monotonic()
    with pytest.raises(ProviderTimeoutError):
        cli_runtime.run(
            cli_runtime.CliInvocation(
                spec=CLAUDE_SPEC,
                program=str(program),
                argv=(),
                environ=environ,
                workdir=tmp_path,
                timeout=1.0,
            )
        )
    assert time.monotonic() - started < 10

    pids = [int(line) for line in pid_file.read_text().split() if line.strip()]
    assert len(pids) == 2, pids
    deadline = time.monotonic() + 2.0
    while time.monotonic() < deadline and not _children_gone(pids):
        time.sleep(0.02)
    assert _children_gone(pids), f"surviving CLI children after the timeout: {pids}"


@pytest.mark.skipif(not Path("/proc").is_dir(), reason="needs /proc to observe a pid")
def test_a_finished_turn_still_reaps_its_child(tmp_path: Path) -> None:
    program = _fake(tmp_path, "claude", "echo done")
    run = cli_runtime.run(
        cli_runtime.CliInvocation(
            spec=CLAUDE_SPEC,
            program=str(program),
            argv=(),
            environ=_environment(tmp_path, program),
            workdir=tmp_path,
            timeout=5.0,
        )
    )
    assert run.succeeded()
    assert run.stdout.strip() == b"done"


def test_pathext_defaults_and_normalises() -> None:
    assert cli_runtime.parse_pathext(None) == (".COM", ".EXE", ".BAT", ".CMD")
    assert cli_runtime.parse_pathext("") == (".COM", ".EXE", ".BAT", ".CMD")
    assert cli_runtime.parse_pathext("exe; .CMD ;exe;;.Bat") == (".exe", ".CMD", ".Bat")


def test_a_windows_search_prefers_the_cmd_shim_over_the_bare_file(tmp_path: Path) -> None:
    """npm writes ``codex`` (a POSIX helper) beside ``codex.cmd``."""
    shims = tmp_path / "shims"
    shims.mkdir()
    for name in ("probe-test", "probe-test.CMD"):
        (shims / name).write_text("", encoding="utf-8")
    search = cli_runtime.PathSearch(
        extensions=cli_runtime.parse_pathext(".COM;.EXE;.BAT;.CMD"),
        allow_bare=False,
    )
    spec = cli_runtime.CliSpec(
        id="probe_test",
        name="Probe Test CLI",
        binary="probe-test",
        path_override_env="KALA_PROBE_TEST_PATH",
        version_args=("--version",),
        auth_args=(),
    )
    found = cli_runtime.resolve_program_with(
        spec,
        {"PATH": str(shims)},
        search,
        is_exec=os.path.isfile,
    )
    assert found == str(shims / "probe-test.CMD")


def test_a_unix_search_accepts_a_bare_program(tmp_path: Path) -> None:
    program = _fake(tmp_path, "probe-test", "exit 0")
    search = cli_runtime.PathSearch(extensions=(".exe",), allow_bare=True)
    spec = cli_runtime.CliSpec(
        id="probe_test",
        name="Probe Test CLI",
        binary="probe-test",
        path_override_env="KALA_PROBE_TEST_PATH",
        version_args=("--version",),
        auth_args=(),
    )
    assert (
        cli_runtime.resolve_program_with(spec, {"PATH": str(tmp_path)}, search)
        == str(program)
    )


def test_batch_shim_detection_is_case_insensitive() -> None:
    assert cli_runtime.is_batch_shim("C:/npm/codex.CMD")
    assert cli_runtime.is_batch_shim("C:/npm/codex.bat")
    assert not cli_runtime.is_batch_shim("C:/npm/codex.exe")
    assert not cli_runtime.is_batch_shim("C:/npm/codex")


def test_cmd_quoting_neutralises_metacharacters() -> None:
    assert cli_runtime._escape_cmd_argument("plain") == '^"plain^"'
    assert cli_runtime._escape_cmd_argument("two words") == '^"two^ words^"'
    assert cli_runtime._escape_cmd_argument("a&b|c") == '^"a^&b^|c^"'
    assert cli_runtime._escape_cmd_argument('a"b') == '^"a\\^"b^"'
    assert cli_runtime._escape_cmd_argument("C:\\dir\\") == '^"C:\\dir\\\\^"'
    assert cli_runtime._escape_cmd_argument("100%") == '^"100^%^"'


def test_cmd_shim_argv_is_an_argument_list() -> None:
    argv = cli_runtime.cmd_shim_argv(
        "C:/Users/me/AppData/Roaming/npm/codex.cmd",
        ["exec", "--model", "gpt-5"],
    )
    assert argv[:3] == ["/d", "/s", "/c"]
    assert argv[3].startswith('"^"C:/Users/me/AppData/Roaming/npm/codex.cmd^"')
    assert '^"exec^"' in argv[3]
    assert len(argv) == 4


def test_cmd_shim_argv_refuses_a_control_character() -> None:
    with pytest.raises(ProviderSubprocessError) as error:
        cli_runtime.cmd_shim_argv("C:/npm/codex.cmd", ["a\nb"])
    assert "control character" in str(error.value)
