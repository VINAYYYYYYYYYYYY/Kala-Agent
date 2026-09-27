"""Availability records and probes that a broken host can produce.

A host without FreeCAD, a probe that cannot start, an interpreter that exits
nonzero, a discovered solver that is really a directory: none of these need an
installation to reproduce, and each one has to be refused rather than
propagated as a usable tool.
"""

from __future__ import annotations

import subprocess
import sys
from collections.abc import Sequence
from pathlib import Path
from typing import Any, cast

import pytest

from freecad_fakes import calculator
from kala_agent_freecad.availability import (
    LIB_ROOT_RELATIVE_CANDIDATES,
    MAX_PATH_LENGTH,
    MAX_SOLVER_BYTES,
    MAX_VERSION_PARTS,
    SEARCH_PREFIXES,
    FreeCADAvailability,
    candidate_lib_roots,
    discover,
    is_safe_calculix,
    parse_version,
    parse_version_payload,
    read_version,
    resolve_calculix,
)


def _record(**overrides: object) -> FreeCADAvailability:
    values: dict[str, Any] = {
        "available": True,
        "interpreter_path": "/usr/bin/freecadcmd",
        "executable_path": "/usr/bin/freecad",
        "import_roots": ("/usr/lib/freecad/lib",),
        "version": "1.1.3",
        "version_parts": (1, 1, 3),
        "calculix_path": "",
        "detail": "FreeCAD 1.1.3 is available for import",
    }
    values.update(overrides)
    return FreeCADAvailability(**values)


def _fake_interpreter(tmp_path: Path, *, exit_code: int = 0) -> Path:
    bin_dir = tmp_path / "usr" / "bin"
    bin_dir.mkdir(parents=True)
    program = bin_dir / "freecadcmd"
    program.write_text(f"#!/bin/sh\nexit {exit_code}\n", encoding="utf-8")
    program.chmod(0o755)
    return program


def _fake_prefix(tmp_path: Path) -> Path:
    lib_root = tmp_path / "usr" / "lib" / "freecad" / "lib"
    lib_root.mkdir(parents=True)
    (lib_root / "FreeCAD.so").write_bytes(b"\x7fELF not a real library")
    return tmp_path / "usr"


def test_a_path_field_longer_than_the_bound_is_refused() -> None:
    with pytest.raises(TypeError, match="exceeds the bounded length"):
        _record(interpreter_path="/" + "a" * MAX_PATH_LENGTH)


def test_more_import_roots_than_any_prefix_can_hold_is_refused() -> None:
    bound = len(SEARCH_PREFIXES) * len(LIB_ROOT_RELATIVE_CANDIDATES)
    assert (
        len(_record(import_roots=tuple(f"/root/{n}" for n in range(bound))).import_roots) == bound
    )
    with pytest.raises(TypeError, match="import_roots exceeds the bounded count"):
        _record(import_roots=tuple(f"/root/{n}" for n in range(bound + 1)))


def test_a_version_of_exactly_the_bound_is_accepted() -> None:
    parts = tuple(range(MAX_VERSION_PARTS))
    assert _record(version_parts=parts).version_parts == parts


def test_a_version_part_beyond_the_bound_is_refused() -> None:
    with pytest.raises(TypeError, match="bounded integers"):
        _record(version_parts=(1, 1, 1_000_001))
    with pytest.raises(TypeError, match="bounded integers"):
        _record(version_parts=(1, True))


def test_parse_version_refuses_a_non_string_banner() -> None:
    assert parse_version(None) == ("", ())
    assert parse_version(1_000_003) == ("", ())
    assert parse_version(["1", "1", "3"]) == ("", ())


def test_a_boolean_leads_no_version_to_the_digits_behind_it() -> None:
    assert parse_version_payload("[true, 1, 1, 3]") == ("", ())
    assert parse_version_payload('["1", false, "1", "3"]') == ("", ())


def test_a_version_part_out_of_the_bounded_range_ends_the_scan() -> None:
    assert parse_version_payload("[1, 1, 1000001, 3]") == ("", ())
    assert parse_version_payload('["1", "1", "12345678"]') == ("", ())


def test_a_short_leading_scan_falls_back_to_the_joined_text() -> None:
    assert parse_version_payload('["1", "1.0.2"]') == ("1.0.2", (1, 0, 2))
    assert parse_version_payload("[1, 2, 3.4.5]") == ("3.4.5", (3, 4, 5))


def test_a_non_list_payload_is_read_as_a_banner() -> None:
    assert parse_version_payload("FreeCAD 0.21.2 (Git)") == ("0.21.2", (0, 21, 2))
    assert parse_version_payload(cast("str", 42)) == ("", ())


def test_a_solver_name_with_a_separator_is_refused(tmp_path: Path) -> None:
    program = calculator(tmp_path)
    assert is_safe_calculix(f"{tmp_path.as_posix()}\\ccx") is False
    assert is_safe_calculix(f"{tmp_path.as_posix()}\x00/ccx") is False
    assert is_safe_calculix(program.as_posix()) is True


def test_a_solver_that_cannot_be_inspected_is_refused(tmp_path: Path) -> None:
    missing = tmp_path / "missing" / "ccx"
    assert is_safe_calculix(missing.as_posix()) is False


def test_a_solver_that_is_a_directory_is_refused(tmp_path: Path) -> None:
    directory = tmp_path / "ccx"
    directory.mkdir()
    assert is_safe_calculix(directory.as_posix()) is False


def test_an_empty_solver_is_refused(tmp_path: Path) -> None:
    program = tmp_path / "ccx"
    program.write_bytes(b"")
    program.chmod(0o755)
    assert is_safe_calculix(program.as_posix()) is False


def test_a_solver_larger_than_a_program_is_refused(tmp_path: Path) -> None:
    program = tmp_path / "ccx_2.22"
    with program.open("wb") as handle:
        handle.truncate(MAX_SOLVER_BYTES + 1)
    program.chmod(0o755)
    assert program.stat().st_size > MAX_SOLVER_BYTES
    assert is_safe_calculix(program.as_posix()) is False


def test_a_name_that_does_not_match_the_solver_pattern_is_refused(tmp_path: Path) -> None:
    program = tmp_path / "CalculiX;reboot"
    program.write_bytes(b"#!/bin/sh\n")
    program.chmod(0o755)
    assert is_safe_calculix(program.as_posix()) is False


def test_resolve_calculix_ignores_a_locator_that_answers_nonsense() -> None:
    answers: dict[str, object] = {
        "ccx": None,
        "CalculiX": 7,
        "ccx_2.22": "",
        "ccx_2.21": "not a path with a name",
        "ccx_2.20": "relative/ccx",
    }

    def locate(name: str) -> str | None:
        return cast("str | None", answers.get(name))

    assert resolve_calculix(locate) == ""


def test_candidate_roots_can_come_from_prefixes_alone(tmp_path: Path) -> None:
    prefix = _fake_prefix(tmp_path)
    expected = prefix / "lib" / "freecad" / "lib"
    roots = candidate_lib_roots("", (prefix, prefix, prefix))
    assert roots == (expected,)
    assert candidate_lib_roots("") == ()


def test_read_version_without_an_interpreter_reports_nothing() -> None:
    assert read_version("", (), python_executable="") == ""
    assert read_version("", (), python_executable="/nowhere/at/all/python") == ""


def test_read_version_survives_a_probe_that_cannot_start(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def refuse(*_command: object, **_options: object) -> subprocess.CompletedProcess[bytes]:
        raise OSError("no such interpreter")

    monkeypatch.setattr("subprocess.run", refuse)
    assert read_version("", (), python_executable=sys.executable) == ""


def test_read_version_refuses_a_probe_that_exits_nonzero(tmp_path: Path) -> None:
    failing = _fake_interpreter(tmp_path, exit_code=3)
    assert read_version(failing.as_posix(), ()) == ""


def test_a_probe_interpreter_that_answers_is_trusted(tmp_path: Path) -> None:
    probe = tmp_path / "probe-python"
    probe.write_text('#!/bin/sh\necho \'["1", "1", "3"]\'\n', encoding="utf-8")
    probe.chmod(0o755)
    payload = read_version("", (), python_executable=probe.as_posix())
    assert parse_version_payload(payload) == ("1.1.3", (1, 1, 3))


def test_read_version_falls_back_to_the_interpreter_banner(tmp_path: Path) -> None:
    banner = tmp_path / "banner"
    banner.write_text("#!/bin/sh\necho 'FreeCAD 1.0.1, Libs 1.0.1'\n", encoding="utf-8")
    banner.chmod(0o755)
    assert read_version(banner.as_posix(), ()) == "FreeCAD 1.0.1, Libs 1.0.1"
    assert parse_version(read_version(banner.as_posix(), ())) == ("1.0.1", (1, 0, 1))


def test_discover_survives_a_broken_locator(tmp_path: Path) -> None:
    def locate(name: str) -> str | None:
        if name == "freecadcmd":
            raise OSError("the search path is unreadable")
        raise ValueError("an unusable name")

    record = discover(which=locate, prefixes=(tmp_path,), read_version_text=False)
    assert record.available is False
    assert record.interpreter_path == ""


def test_discover_reports_a_version_it_never_read(tmp_path: Path) -> None:
    interpreter = _fake_interpreter(tmp_path)
    prefix = _fake_prefix(tmp_path)
    record = discover(
        which=lambda _name: interpreter.as_posix(),
        prefixes=(prefix,),
        read_version_text=False,
    )
    assert record.available is False
    assert record.version == ""
    assert record.version_parts == ()
    assert record.import_root_paths
    assert "did not report a usable version" in record.detail


def test_discover_records_a_solver_next_to_the_analysis(tmp_path: Path) -> None:
    interpreter = _fake_interpreter(tmp_path)
    prefix = _fake_prefix(tmp_path)
    program = calculator(tmp_path)
    record = discover(
        which=lambda name: (
            interpreter.as_posix() if name.startswith("freecad") else program.as_posix()
        ),
        version_reader=lambda _path, _roots: '["1", "1", "3"]',
        prefixes=(prefix,),
    )
    assert record.available is True
    assert record.calculix_path == program.as_posix()
    assert record.solver_available is True
    assert record.to_payload()["calculix_path"] == program.as_posix()


def test_a_missing_executable_leaves_the_gui_command_as_the_interpreter(
    tmp_path: Path,
) -> None:
    interpreter = _fake_interpreter(tmp_path)
    prefix = _fake_prefix(tmp_path)

    def locate(name: str) -> str | None:
        return interpreter.as_posix() if name == "freecadcmd" else None

    record = discover(
        which=locate,
        version_reader=lambda _path, _roots: '["1", "1", "3"]',
        prefixes=(prefix,),
    )
    assert record.available is True
    assert record.executable_path == ""
    assert record.to_payload()["executable_path"] is None
    assert record.to_payload()["interpreter_path"] == interpreter.as_posix()


def test_an_unusable_executable_is_not_treated_as_a_discovery(tmp_path: Path) -> None:
    not_executable = tmp_path / "freecadcmd"
    not_executable.write_text("#!/bin/sh\n", encoding="utf-8")
    not_executable.chmod(0o600)
    record = discover(
        which=lambda _name: not_executable.as_posix(),
        prefixes=(tmp_path,),
        read_version_text=False,
    )
    assert record.available is False
    assert "no freecadcmd or freecad executable" in record.detail


def test_a_declared_version_is_read_from_the_probe_output(tmp_path: Path) -> None:
    interpreter = _fake_interpreter(tmp_path)
    prefix = _fake_prefix(tmp_path)
    seen: list[Sequence[Path]] = []

    def reader(_path: str, roots: Sequence[Path]) -> str:
        seen.append(tuple(roots))
        return "[1, 1, 3]"

    record = discover(
        which=lambda _name: interpreter.as_posix(),
        version_reader=reader,
        prefixes=(prefix,),
    )
    assert record.version_parts == (1, 1, 3)
    assert record.version == "1.1.3"
    assert len(seen) == 1
