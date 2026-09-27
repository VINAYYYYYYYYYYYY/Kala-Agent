"""The FreeCAD loader, driven with a fake import table.

The session is the only place the adapter imports the native modules, so every
failure a real installation produces (a module that will not import, a version
that is not a triple, a bridge that is absent, a document that stays open) can
be reproduced here without FreeCAD and without a subprocess.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path
from types import ModuleType
from typing import Any, Final, cast

import pytest

from freecad_fakes import FakeFreeCAD
from kala_agent_freecad.cad import runtime
from kala_agent_freecad.cad.runtime import (
    LIB_ROOT_ENVIRONMENT_KEY,
    OPTIONAL_MODULE,
    freecad_session,
    import_roots_from_path,
    is_import_root,
    native_output_to_stderr,
)
from kala_agent_freecad.errors import FreeCADDependencyError

BROKEN_ROOT_TEXT: Final[str] = "not an import root"


def _root(tmp_path: Path, name: str = "lib") -> Path:
    root = tmp_path / name
    root.mkdir(parents=True)
    (root / "FreeCAD.py").write_text("# a stand-in marker\n", encoding="utf-8")
    return root.resolve()


def _import_root_table(
    *,
    freecad: object,
    with_bridge: bool = True,
    absent: tuple[str, ...] = (),
) -> dict[str, ModuleType]:
    application = ModuleType("FreeCAD")
    application.Version = freecad.Version  # type: ignore[attr-defined]
    application.newDocument = freecad.newDocument  # type: ignore[attr-defined]
    application.listDocuments = freecad.listDocuments  # type: ignore[attr-defined]
    application.closeDocument = freecad.closeDocument  # type: ignore[attr-defined]
    table: dict[str, ModuleType] = {
        "FreeCAD": application,
        "Part": ModuleType("Part"),
        "ObjectsFem": ModuleType("ObjectsFem"),
    }
    if with_bridge:
        table[OPTIONAL_MODULE] = ModuleType(OPTIONAL_MODULE)
    for name in absent:
        table.pop(name, None)
    return table


def _loader(
    monkeypatch: pytest.MonkeyPatch,
    table: dict[str, ModuleType],
) -> None:
    def import_module(name: str, package: str | None = None) -> ModuleType:
        module = table.get(name)
        if module is None:
            raise ImportError(f"no module named {name}")
        return module

    stand_in = ModuleType("fake_importlib")
    stand_in.import_module = import_module  # type: ignore[attr-defined]
    monkeypatch.setattr(runtime, "importlib", stand_in)


def test_loader_entries_that_are_not_paths_are_skipped(tmp_path: Path) -> None:
    root = _root(tmp_path)
    entries = cast("Any", (None, "", 7, root.as_posix(), root.as_posix()))
    assert import_roots_from_path(entries) == (root,)


def test_a_declared_root_that_is_not_a_root_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(LIB_ROOT_ENVIRONMENT_KEY, str(tmp_path / "missing"))
    assert import_roots_from_path(()) == ()


def test_an_unbounded_declared_root_is_ignored(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(LIB_ROOT_ENVIRONMENT_KEY, os.pathsep.join([BROKEN_ROOT_TEXT] * 2_000))
    assert import_roots_from_path(()) == ()


def test_is_import_root_needs_a_directory_with_a_marker(tmp_path: Path) -> None:
    root = _root(tmp_path)
    assert is_import_root(root) is True
    assert is_import_root(tmp_path) is False
    assert is_import_root(tmp_path / "missing") is False


def test_the_session_loads_every_module_in_order(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = _root(tmp_path)
    freecad = FakeFreeCAD()
    _loader(monkeypatch, _import_root_table(freecad=freecad))
    with freecad_session((root,)) as modules:
        assert modules.version == "1.1.3"
        assert modules.import_root == root.as_posix()
        assert modules.import_order == ("FreeCAD", "Part", "ObjectsFem", OPTIONAL_MODULE)
        assert modules.solver_bridge_available is True
        assert modules.solver_bridge_detail == ""
        assert root.as_posix() in sys.path
    assert freecad.closed == []
    assert root.as_posix() not in sys.path


def test_the_session_closes_the_documents_it_opened(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = _root(tmp_path)
    freecad = FakeFreeCAD()
    _loader(monkeypatch, _import_root_table(freecad=freecad))
    with freecad_session((root,)):
        freecad.newDocument("left-open")
        assert freecad.listDocuments() == ["left-open"]
    assert freecad.closed == ["left-open"]
    assert freecad.listDocuments() == []


def test_a_root_already_on_the_loader_path_is_not_added_again(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = _root(tmp_path)
    text = root.as_posix()
    monkeypatch.syspath_prepend(text)
    _loader(monkeypatch, _import_root_table(freecad=FakeFreeCAD()))
    with freecad_session((root,)):
        assert sys.path.count(text) == 1
    assert sys.path.count(text) == 1


def test_a_version_that_is_not_a_triple_is_reported_as_unknown(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = _root(tmp_path)
    _loader(monkeypatch, _import_root_table(freecad=FakeFreeCAD(version="1.1.3")))
    with freecad_session((root,)) as modules:
        assert modules.version == ""


def test_a_short_version_is_padded_rather_than_guessed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = _root(tmp_path)
    _loader(monkeypatch, _import_root_table(freecad=FakeFreeCAD(version=["1", "1"])))
    with freecad_session((root,)) as modules:
        assert modules.version == "1.1.0"


def test_a_missing_module_is_reported_as_a_dependency_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = _root(tmp_path)
    _loader(
        monkeypatch,
        _import_root_table(freecad=FakeFreeCAD(), absent=("ObjectsFem",)),
    )
    with pytest.raises(FreeCADDependencyError, match="ObjectsFem") as failure:
        with freecad_session((root,)):
            pytest.fail("the session must not start without its modules")
    assert isinstance(failure.value.__cause__, ImportError)
    assert root.as_posix() not in sys.path


def test_a_failed_freecad_import_opens_no_document(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = _root(tmp_path)
    freecad = FakeFreeCAD()
    _loader(
        monkeypatch,
        _import_root_table(freecad=freecad, absent=("FreeCAD",)),
    )
    with pytest.raises(FreeCADDependencyError, match="FreeCAD"):
        with freecad_session((root,)):
            pytest.fail("the session must not start without FreeCAD")
    assert freecad.listDocuments() == []
    assert freecad.closed == []


def test_an_optional_bridge_may_be_absent(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    root = _root(tmp_path)
    _loader(
        monkeypatch,
        _import_root_table(freecad=FakeFreeCAD(), with_bridge=False),
    )
    with freecad_session((root,)) as modules:
        assert modules.solver_bridge_available is False
        assert modules.ccx_tools is None
        assert OPTIONAL_MODULE in modules.solver_bridge_detail
        assert modules.import_order == ("FreeCAD", "Part", "ObjectsFem")


def test_a_required_bridge_is_never_tolerated(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = _root(tmp_path)
    _loader(
        monkeypatch,
        _import_root_table(freecad=FakeFreeCAD(), with_bridge=False),
    )
    with pytest.raises(FreeCADDependencyError, match=OPTIONAL_MODULE):
        with freecad_session((root,), require_solver_bridge=True):
            pytest.fail("a required bridge may not be missing")


def test_a_session_without_an_import_root_fails_closed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(LIB_ROOT_ENVIRONMENT_KEY, str(tmp_path / "absent"))
    with pytest.raises(FreeCADDependencyError, match="no FreeCAD import root"):
        with freecad_session(()):
            pytest.fail("the session must not start without an import root")


def test_the_session_tears_down_after_a_failed_body(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = _root(tmp_path)
    freecad = FakeFreeCAD()
    _loader(monkeypatch, _import_root_table(freecad=freecad))
    with pytest.raises(ZeroDivisionError):
        with freecad_session((root,)):
            freecad.newDocument("half-built")
            raise ZeroDivisionError("a native failure inside the call")
    assert freecad.closed == ["half-built"]
    assert root.as_posix() not in sys.path


def test_a_worker_without_a_usable_stdout_descriptor_still_runs(
    capfd: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    def refuse(*_descriptor: int) -> int:
        raise OSError(24, "too many open files")

    monkeypatch.setattr("os.dup", refuse)
    with native_output_to_stderr():
        pass
    monkeypatch.undo()
    os.write(1, b"protocol-line\n")
    assert "protocol-line" in capfd.readouterr().out
