"""The FreeCAD design box is a mod link, not a browser page."""

from __future__ import annotations

from pathlib import Path

import pytest

from kala_agent_freecad.panel import install, source_mod, user_mod_dir


def test_the_mod_is_the_design_box() -> None:
    source = source_mod()
    text = (source / "kala_gui.py").read_text(encoding="utf-8")
    loader = (source / "InitGui.py").read_text(encoding="utf-8")
    assert "QDockWidget" in text
    assert "Build in this window" in text
    assert "Run analysis" in text
    assert "design_request" in text
    assert "RunLog" in text
    assert "kala_gui" in loader


def test_install_links_the_mod_into_freecad(tmp_path: Path) -> None:
    freecad = tmp_path / ".local" / "share" / "FreeCAD" / "v1-1"
    freecad.mkdir(parents=True)
    link = install(home=tmp_path)
    assert link == user_mod_dir(tmp_path) / "Kala"
    assert link.is_symlink()
    assert link.resolve() == source_mod().resolve()
    assert install(home=tmp_path) == link


def test_install_refuses_a_different_existing_mod(tmp_path: Path) -> None:
    freecad = tmp_path / ".local" / "share" / "FreeCAD" / "v1-1" / "Mod" / "Kala"
    freecad.mkdir(parents=True)
    with pytest.raises(FileExistsError):
        install(home=tmp_path)
