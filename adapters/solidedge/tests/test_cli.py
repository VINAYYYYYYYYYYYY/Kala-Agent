"""CLI gating, dry-run compilation, and the non-TTY confirmation rule.

These tests drive the public :func:`kala_agent_solidedge.cli.run` seam with
injected streams and an explicit platform, so no test depends on the host
interpreter's real platform.
"""

from __future__ import annotations

import io
import json
from collections.abc import Sequence
from pathlib import Path

import pytest

from kala_agent_solidedge import cli
from kala_agent_solidedge.journal import JOURNAL_FILENAME, Journal, JournalEntry

EXAMPLES = Path(__file__).resolve().parents[1] / "examples"


def run(
    argv: Sequence[str],
    *,
    platform: str | None = "linux",
    stdin: io.StringIO | None = None,
) -> tuple[int, str, str]:
    stdout = io.StringIO()
    stderr = io.StringIO()
    code = cli.run(
        list(argv),
        stdout=stdout,
        stderr=stderr,
        stdin=stdin if stdin is not None else io.StringIO(""),
        platform=platform,
    )
    return code, stdout.getvalue(), stderr.getvalue()


def test_dry_run_compiles_without_a_token_or_network() -> None:
    code, stdout, stderr = run(
        ["build", "--script", str(EXAMPLES / "plate_with_hole.json"), "--dry-run"]
    )

    assert code == cli.EXIT_OK
    document = json.loads(stdout)
    assert document["components"] == ["plate"]
    assert document["operations"] == 8
    assert document["assembly"] is False
    assert document["request_digest"]
    assert stderr == ""


def test_dry_run_of_positioned_parts_reports_an_assembly() -> None:
    code, stdout, _ = run(
        ["build", "--script", str(EXAMPLES / "positioned_parts.json"), "--dry-run"]
    )

    assert code == cli.EXIT_OK
    assert json.loads(stdout)["assembly"] is True


def test_a_real_build_is_refused_on_a_non_windows_host(tmp_path: Path) -> None:
    code, _, stderr = run(
        [
            "build",
            "--script",
            str(EXAMPLES / "plate_with_hole.json"),
            "--calibration-id",
            "cal-1",
            "--yes",
            "--output-dir",
            str(tmp_path),
        ]
    )

    assert code == cli.EXIT_USAGE
    assert "requires Windows" in stderr
    assert not (tmp_path / JOURNAL_FILENAME).exists()


def test_status_and_cancel_are_refused_off_windows() -> None:
    status_code, _, status_stderr = run(["status", "build-1"])
    cancel_code, _, cancel_stderr = run(["cancel", "build-1"])

    assert status_code == cli.EXIT_USAGE
    assert cancel_code == cli.EXIT_USAGE
    assert "requires Windows" in status_stderr
    assert "requires Windows" in cancel_stderr


def test_a_build_without_yes_on_a_non_tty_fails_fast(tmp_path: Path) -> None:
    code, _, stderr = run(
        [
            "build",
            "--script",
            str(EXAMPLES / "plate_with_hole.json"),
            "--calibration-id",
            "cal-1",
            "--output-dir",
            str(tmp_path),
            "--token-file",
            str(tmp_path / "bridge.token"),
        ],
        platform="win32",
        stdin=io.StringIO("yes\n"),
    )

    assert code == cli.EXIT_ERROR
    assert "not confirmed" in stderr
    assert not (tmp_path / JOURNAL_FILENAME).exists()


def test_a_compiler_refusal_happens_before_the_client_is_built(tmp_path: Path) -> None:
    bad = tmp_path / "helix.json"
    bad.write_text(
        json.dumps(
            {
                "summary": "screw",
                "components": [
                    {
                        "name": "screw",
                        "steps": [
                            {"op": "sketch", "profile": "circle", "diameter": 16},
                            {"op": "extrude", "length": 40},
                            {"op": "helix", "diameter": 16, "pitch": 4, "length": 24},
                        ],
                    }
                ],
                "assembly": [],
            }
        ),
        encoding="utf-8",
    )

    code, stdout, stderr = run(["build", "--script", str(bad), "--dry-run"])

    assert code == cli.EXIT_ERROR
    assert stdout == ""
    assert "helix" in stderr


def test_doctor_without_a_token_path_is_unsupported_not_a_crash() -> None:
    code, stdout, stderr = run(["doctor"], platform="linux")

    assert code == cli.EXIT_ERROR
    document = json.loads(stdout)
    assert document["condition"] == "unsupported"
    assert stderr == ""


def test_doctor_distinguishes_unreachable(tmp_path: Path) -> None:
    token = tmp_path / "bridge.token"
    token.write_text("a-token-that-is-long-enough", encoding="ascii")

    code, stdout, _ = run(
        ["doctor", "--token-file", str(token), "--bridge-url", "http://127.0.0.1:1"],
        platform="win32",
    )

    assert code == cli.EXIT_ERROR
    assert json.loads(stdout)["condition"] == "unreachable"


def test_doctor_distinguishes_an_authentication_failure(tmp_path: Path) -> None:
    token = tmp_path / "bridge.token"
    token.write_text("a-token-that-is-long-enough", encoding="ascii")

    # A non-loopback URL is refused by the client's own URL guard, which is a
    # typed client failure rather than an unhandled traceback.
    code, _, stderr = run(
        ["doctor", "--token-file", str(token), "--bridge-url", "http://192.168.1.2:8797"],
        platform="win32",
    )

    assert code == cli.EXIT_ERROR
    assert "loopback" in stderr


def test_the_parser_help_states_the_current_behavior() -> None:
    text = " ".join(cli._parser().format_help().split())

    assert "kala-solidedge" in text
    assert "--dry-run" in text
    assert "require Windows" in text
    assert "Prompt and Studio modes are not implemented" in text


def test_every_known_command_parses_with_its_required_arguments() -> None:
    parser = cli._parser()
    assert parser.parse_args(["doctor"]).command == "doctor"
    assert parser.parse_args(["build", "--script", "s.json"]).command == "build"
    assert parser.parse_args(["status", "build-1"]).command == "status"
    assert parser.parse_args(["cancel", "build-1"]).command == "cancel"


def test_connection_flags_parse_before_and_after_the_subcommand() -> None:
    parser = cli._parser()
    before = parser.parse_args(["--token-file", "/tmp/a", "status", "build-1"])
    after = parser.parse_args(["status", "build-1", "--token-file", "/tmp/a"])

    assert before.token_file == after.token_file == "/tmp/a"


def test_a_journal_entry_records_no_credentials(tmp_path: Path) -> None:
    journal = Journal(tmp_path / JOURNAL_FILENAME)
    journal.append(
        JournalEntry(
            script_digest="a" * 64,
            request_id="req-1",
            build_id="build-1",
            status="queued",
            summary="one part",
        )
    )

    text = (tmp_path / JOURNAL_FILENAME).read_text(encoding="utf-8")
    assert "a" * 64 in text
    assert "Bearer" not in text
    assert "token" not in text.casefold()


def test_the_journal_reuses_only_an_exact_pending_request(tmp_path: Path) -> None:
    journal = Journal(tmp_path / JOURNAL_FILENAME)
    journal.append(
        JournalEntry(
            script_digest="b" * 64,
            request_id="pending-id",
            build_id=None,
            status="pending",
            summary="lost",
            request_digest="d" * 64,
            bridge_target="http://127.0.0.1:8797|cal-1",
        )
    )

    assert (
        cli._reusable_request_id(
            journal,
            "b" * 64,
            "d" * 64,
            "http://127.0.0.1:8797|cal-1",
            "fresh",
        )
        == "pending-id"
    )
    journal.update("pending-id", build_id="build-1", status="succeeded")
    assert (
        cli._reusable_request_id(
            journal,
            "b" * 64,
            "d" * 64,
            "http://127.0.0.1:8797|cal-1",
            "fresh",
        )
        == "fresh"
    )
    assert cli._reusable_request_id(journal, "c" * 64, "e" * 64, "target", "fresh") == "fresh"


def test_the_journal_does_not_reuse_a_changed_calibration_or_legacy_entry(
    tmp_path: Path,
) -> None:
    journal = Journal(tmp_path / JOURNAL_FILENAME)
    journal.append(
        JournalEntry(
            script_digest="b" * 64,
            request_id="old-cal-id",
            build_id=None,
            status="pending",
            summary="lost",
            request_digest="d" * 64,
            bridge_target="http://127.0.0.1:8797|cal-old",
        )
    )
    journal.append(
        JournalEntry(
            script_digest="c" * 64,
            request_id="legacy-id",
            build_id=None,
            status="pending",
            summary="legacy",
        )
    )

    # Same script, new calibration: the request digest and target differ.
    assert (
        cli._reusable_request_id(
            journal,
            "b" * 64,
            "e" * 64,
            "http://127.0.0.1:8797|cal-new",
            "fresh",
        )
        == "fresh"
    )
    # A legacy entry has no exact identity; it must not be inherited silently.
    assert (
        cli._reusable_request_id(
            journal,
            "c" * 64,
            "f" * 64,
            "http://127.0.0.1:8797|cal-new",
            "fresh",
        )
        == "fresh"
    )


@pytest.mark.parametrize(
    "arguments",
    [
        ["build"],
        ["status"],
        ["cancel"],
    ],
)
def test_missing_arguments_are_a_usage_error(arguments: list[str]) -> None:
    with pytest.raises(SystemExit) as caught:
        cli._parser().parse_args(arguments)
    assert caught.value.code == 2


def test_the_journal_records_the_normalized_script(tmp_path: Path) -> None:
    from kala_agent_solidedge.cli import _load_script, _normalized_script, _script_digest

    loaded = _load_script(EXAMPLES / "plate_with_hole.json")
    normalized = _normalized_script(loaded)
    journal = Journal(tmp_path / JOURNAL_FILENAME)
    journal.append(
        JournalEntry(
            script_digest=_script_digest(loaded),
            request_id="req-1",
            build_id=None,
            status="pending",
            summary="one part",
            script=normalized,
        )
    )

    entry = Journal(tmp_path / JOURNAL_FILENAME).read()[0]
    assert entry.script == normalized
    assert '"plate"' in normalized
    assert "token" not in normalized.casefold()
