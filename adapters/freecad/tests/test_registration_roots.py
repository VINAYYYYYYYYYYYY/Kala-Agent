"""The loader roots a worker is handed.

A worker imports this adapter and FreeCAD and nothing else the host happens
to have on its path, so a root that is not a directory, or the same root
reached twice, must collapse rather than reach the child interpreter.
"""

from __future__ import annotations

from pathlib import Path
from typing import Final

import kala_agent_freecad
from kala_agent_freecad.availability import FreeCADAvailability
from kala_agent_freecad.registration import worker_import_roots

ADAPTER_PARENT: Final[Path] = Path(kala_agent_freecad.__file__).resolve().parent.parent


def _available(*roots: str) -> FreeCADAvailability:
    return FreeCADAvailability(
        available=True,
        interpreter_path="/usr/bin/freecadcmd",
        executable_path="/usr/bin/freecad",
        import_roots=roots,
        version="1.1.3",
        version_parts=(1, 1, 3),
        calculix_path="",
        detail="FreeCAD 1.1.3 is available for import",
    )


def test_a_root_that_is_not_a_directory_is_left_out(tmp_path: Path) -> None:
    missing = tmp_path / "opt" / "freecad" / "lib"
    assert worker_import_roots(_available(missing.as_posix())) == (ADAPTER_PARENT,)


def test_the_same_root_is_never_listed_twice(tmp_path: Path) -> None:
    lib_root = tmp_path / "opt" / "freecad" / "lib"
    lib_root.mkdir(parents=True)
    roots = worker_import_roots(_available(lib_root.as_posix(), lib_root.as_posix()))
    assert roots == (lib_root, ADAPTER_PARENT)
    assert len(roots) == len(set(roots))
