"""Availability discovery, exercised without a FreeCAD installation."""

from __future__ import annotations

import dataclasses
import stat
from collections.abc import Sequence
from pathlib import Path
from typing import Any

import pytest

from kala_agent_freecad import availability
from kala_agent_freecad.availability import (
    MAX_DETAIL_LENGTH,
    FreeCADAvailability,
    candidate_lib_roots,
    discover,
    is_safe_calculix,
    parse_version,
    parse_version_payload,
    resolve_calculix,
    unavailable,
)

FAKE_BANNER: str = "FreeCAD 1.1.3, Libs: 1.1.3R44987 (Git)"


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


def _fake_interpreter(tmp_path: Path) -> Path:
    bin_dir = tmp_path / "usr" / "bin"
    bin_dir.mkdir(parents=True)
    for name in ("freecadcmd", "freecad"):
        executable = bin_dir / name
        executable.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        executable.chmod(0o755)
    return bin_dir / "freecadcmd"


def _fake_prefix(tmp_path: Path) -> Path:
    lib_root = tmp_path / "usr" / "lib" / "freecad" / "lib"
    lib_root.mkdir(parents=True)
    (lib_root / "FreeCAD.so").write_bytes(b"\x7fELF not a real library")
    return tmp_path / "usr"


def test_record_is_immutable() -> None:
    record = _record()
    with pytest.raises(dataclasses.FrozenInstanceError):
        record.available = False  # type: ignore[misc]
    assert isinstance(record.import_roots, tuple)
    assert isinstance(record.version_parts, tuple)


def test_record_rejects_a_mutable_sequence() -> None:
    with pytest.raises(TypeError):
        _record(import_roots=["/usr/lib/freecad/lib"])


@pytest.mark.parametrize(
    "overrides",
    (
        {"available": "yes"},
        {"detail": "x" * (MAX_DETAIL_LENGTH + 1)},
        {"import_roots": ("/usr/lib/freecad/lib",), "version_parts": ("1",)},
        {"version_parts": (1, 1, 3, 4, 5, 6, 7)},
        {"interpreter_path": 7},
    ),
)
def test_record_rejects_malformed_fields(overrides: dict[str, object]) -> None:
    with pytest.raises(TypeError):
        _record(**overrides)


def test_unavailable_bounds_its_detail() -> None:
    record = unavailable("x" * 4_000)
    assert record.available is False
    assert len(record.detail) <= MAX_DETAIL_LENGTH
    assert record.solver_available is False
    assert record.import_root_paths == ()


def test_payload_reports_absence_as_null() -> None:
    payload = _record(calculix_path="", version="1.1.3").to_payload()
    assert payload["available"] is True
    assert payload["calculix_path"] is None
    assert payload["version"] == "1.1.3"
    assert payload["version_parts"] == [1, 1, 3]
    assert payload["import_roots"] == ["/usr/lib/freecad/lib"]


@pytest.mark.parametrize(
    ("text", "expected"),
    (
        (FAKE_BANNER, ("1.1.3", (1, 1, 3))),
        ("FreeCAD 1.0.2 Revision: 3210 (Git)", ("1.0.2", (1, 0, 2))),
        ("no version here", ("", ())),
        ("", ("", ())),
    ),
)
def test_parse_version_reads_a_banner(text: str, expected: tuple[str, tuple[int, ...]]) -> None:
    assert parse_version(text) == expected


@pytest.mark.parametrize(
    ("payload", "expected"),
    (
        ('["1", "1", "3", "44987 (Git)"]', ("1.1.3", (1, 1, 3))),
        ('["1","1","3"]', ("1.1.3", (1, 1, 3))),
        ("[1, 1, 3]", ("1.1.3", (1, 1, 3))),
        (FAKE_BANNER, ("1.1.3", (1, 1, 3))),
        ('{"version": "1.1.3"}', ("1.1.3", (1, 1, 3))),
        ("[]", ("", ())),
        ("", ("", ())),
    ),
)
def test_parse_version_payload(
    payload: str,
    expected: tuple[str, tuple[int, ...]],
) -> None:
    assert parse_version_payload(payload) == expected


def test_discover_reports_a_host_without_freecad(tmp_path: Path) -> None:
    record = discover(
        which=lambda _name: None,
        prefixes=(tmp_path,),
        read_version_text=False,
    )
    assert record.available is False
    assert record.interpreter_path == ""
    assert "no freecadcmd or freecad executable" in record.detail


def test_discover_reports_a_host_without_an_import_root(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    interpreter = _fake_interpreter(tmp_path)
    monkeypatch.setattr(availability, "candidate_lib_roots", lambda *_a, **_k: ())
    record = discover(
        which=lambda name: interpreter.as_posix() if name == "freecadcmd" else None,
        prefixes=(tmp_path,),
        read_version_text=False,
    )
    assert record.available is False
    assert record.import_roots == ()
    assert record.interpreter_path == interpreter.as_posix()
    assert "no FreeCAD import root" in record.detail


def test_discover_resolves_version_and_roots(tmp_path: Path) -> None:
    interpreter = _fake_interpreter(tmp_path)
    prefix = _fake_prefix(tmp_path)
    expected = prefix / "lib" / "freecad" / "lib"
    calls: list[tuple[str, tuple[Path, ...]]] = []

    def reader(path: str, roots: Sequence[Path]) -> str:
        calls.append((path, tuple(roots)))
        return '["1", "1", "3", "44987 (Git)"]'

    record = discover(
        which=lambda name: interpreter.as_posix() if name.startswith("freecad") else None,
        version_reader=reader,
        prefixes=(prefix,),
    )
    assert record.available is True
    assert record.version == "1.1.3"
    assert record.version_parts == (1, 1, 3)
    assert expected in record.import_root_paths
    assert all(root.is_dir() for root in record.import_root_paths)
    assert calls and calls[0][0] == interpreter.as_posix()
    assert expected in calls[0][1]
    assert "1.1.3" in record.detail


def test_discover_refuses_an_unusable_version(tmp_path: Path) -> None:
    interpreter = _fake_interpreter(tmp_path)
    prefix = _fake_prefix(tmp_path)
    record = discover(
        which=lambda _name: interpreter.as_posix(),
        version_reader=lambda _path, _roots: "",
        prefixes=(prefix,),
    )
    assert record.available is False
    assert record.version == ""
    assert "did not report a usable version" in record.detail


def test_candidate_lib_roots_are_deduplicated(tmp_path: Path) -> None:
    interpreter = _fake_interpreter(tmp_path)
    prefix = _fake_prefix(tmp_path)
    expected = prefix / "lib" / "freecad" / "lib"
    roots = candidate_lib_roots(interpreter.as_posix(), (prefix, prefix))
    assert expected in roots
    assert len(roots) == len(set(roots))
    assert all(is_safe_root(root) for root in roots)


def is_safe_root(root: Path) -> bool:
    return root.is_dir() and (root / "FreeCAD.so").exists()


def test_safe_calculix_accepts_only_a_plausible_binary(tmp_path: Path) -> None:
    program = tmp_path / "ccx"
    program.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    program.chmod(0o755)
    assert is_safe_calculix(program.as_posix()) is True
    assert is_safe_calculix((tmp_path / "missing").as_posix()) is False
    assert is_safe_calculix(tmp_path.as_posix()) is False
    assert is_safe_calculix("") is False
    assert is_safe_calculix("ccx; rm -rf /") is False
    assert is_safe_calculix("x" * 4_096) is False


def test_safe_calculix_refuses_a_non_executable(tmp_path: Path) -> None:
    program = tmp_path / "ccx"
    program.write_text("not executable", encoding="utf-8")
    program.chmod(0o600)
    assert is_safe_calculix(program.as_posix()) is False
    assert stat.S_ISREG(program.stat().st_mode)


def test_resolve_calculix_skips_unsafe_candidates(tmp_path: Path) -> None:
    program = tmp_path / "ccx"
    program.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    program.chmod(0o755)
    decoy = tmp_path / "ccx_backup"
    decoy.write_text("not a program", encoding="utf-8")
    decoy.chmod(0o600)
    calls: list[str] = []

    def locate(name: str) -> str:
        calls.append(name)
        return decoy.as_posix() if name == "ccx" else program.as_posix()

    assert resolve_calculix(locate) == program.as_posix()
    assert calls[0] == "ccx"


def test_resolve_calculix_survives_a_broken_locator(tmp_path: Path) -> None:
    def locate(name: str) -> str:
        if name == "ccx":
            raise OSError("no search path")
        return ""

    assert resolve_calculix(locate) == ""


def test_unavailable_record_has_no_solver() -> None:
    record = unavailable("nothing here")
    assert record.solver_available is False
    assert record.to_payload()["calculix_path"] is None


def test_payload_keys_are_stable() -> None:
    assert tuple(_record().to_payload()) == (
        "available",
        "calculix_path",
        "detail",
        "executable_path",
        "import_roots",
        "interpreter_path",
        "version",
        "version_parts",
    )
