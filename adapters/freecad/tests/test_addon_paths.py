"""The add-on has to find the adapter and its dependencies by itself.

The Kala design box is installed in three shapes: copied out of the AppImage
into FreeCAD's ``Mod`` directory, linked into it (the developer flow), and
pointed at an explicit ``KALA_FREECAD_SYS_PATH``. All three are resolved by
``mod/Kala/kala_paths.py``, and none of them may fall back to a guessed
repository root or to a virtualenv.

These tests exercise the resolver without FreeCAD: the module only needs
``sys.implementation.cache_tag`` and the file system.
"""

from __future__ import annotations

import importlib.util
import re
import sys
from collections.abc import Iterator
from pathlib import Path
from types import ModuleType

import pytest

ADDON_DIR = Path(__file__).resolve().parents[1] / "mod" / "Kala"
ADDON_MODULE = ADDON_DIR / "kala_paths.py"


def _load_addon_paths() -> ModuleType:
    spec = importlib.util.spec_from_file_location("kala_addon_paths", ADDON_MODULE)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


kala_paths = _load_addon_paths()


def _make_package(root: Path, name: str) -> Path:
    package = root / name
    package.mkdir(parents=True, exist_ok=True)
    (package / "__init__.py").write_text("", encoding="utf8")
    return package


@pytest.fixture
def clean_import_path(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    """Restore ``sys.path`` and the environment after every test."""
    original = list(sys.path)
    monkeypatch.delenv("KALA_FREECAD_SYS_PATH", raising=False)
    yield
    sys.path[:] = original


def test_the_addon_module_has_no_repository_or_virtualenv_guess() -> None:
    """The old `parents[4]` layout and the repository `.venv` are gone."""
    source = ADDON_MODULE.read_text(encoding="utf8")
    assert re.search(r"\.parents\[\s*4\s*\]", source) is None
    assert re.search(r"""["']\.venv["']""", source) is None
    assert "glob(" not in source


def test_abi_tag_follows_the_running_interpreter() -> None:
    tag = kala_paths.abi_tag()
    cache_tag = sys.implementation.cache_tag
    if cache_tag.startswith("cpython"):
        assert tag == f"cp{cache_tag[len('cpython-'):]}"
        assert tag is not None and tag.startswith("cp")
    else:
        assert tag is None


def test_the_explicit_adapter_path_is_used(
    clean_import_path: Iterator[None],
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    root = tmp_path / "adapter"
    sources = root / "src"
    _make_package(sources, "kala_agent_freecad")
    _make_package(sources, "kala_agent")
    packages = root / f"site-packages-{kala_paths.abi_tag()}"
    packages.mkdir(parents=True)

    monkeypatch.setenv("KALA_FREECAD_SYS_PATH", str(sources))
    state = kala_paths.prepare()

    assert state["adapter"] == str(sources)
    assert state["shared"] == str(sources)
    assert state["sitePackages"] == str(packages)
    assert kala_paths.problems() == []
    assert str(sources) in sys.path
    assert str(packages) in sys.path


def test_the_vendored_dependencies_are_matched_to_the_interpreter(
    clean_import_path: Iterator[None],
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Only the directory for the running ABI is used."""
    root = tmp_path / "adapter"
    sources = root / "src"
    _make_package(sources, "kala_agent_freecad")
    for abi in ("cp311", "cp312", "cp313", "cp314"):
        (root / f"site-packages-{abi}").mkdir(parents=True)

    monkeypatch.setenv("KALA_FREECAD_SYS_PATH", str(sources))
    state = kala_paths.prepare()

    abi = kala_paths.abi_tag()
    if abi is None:
        assert state["sitePackages"] is None
        return
    assert state["sitePackages"] == str(root / f"site-packages-{abi}")
    others = [
        f"site-packages-{other}"
        for other in ("cp311", "cp312", "cp313", "cp314")
        if other != abi
    ]
    for other in others:
        assert str(root / other) not in sys.path


def test_a_missing_payload_is_reported_honestly(
    clean_import_path: Iterator[None],
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The add-on says what is missing instead of failing with a traceback."""
    root = tmp_path / "empty"
    root.mkdir()
    monkeypatch.setenv("KALA_FREECAD_SYS_PATH", str(root))
    kala_paths.prepare()
    problems = kala_paths.problems()

    abi = kala_paths.abi_tag()
    if abi is None:
        assert problems
    else:
        # The vendored dependencies for this interpreter are not next to the
        # add-on in this fixture, so the add-on has to say so.
        assert any(abi in problem for problem in problems)
    summary = kala_paths.describe()
    assert "KALA_FREECAD_SYS_PATH" in summary or abi in summary


def test_the_summary_names_the_adapter_and_the_dependencies(
    clean_import_path: Iterator[None],
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    root = tmp_path / "adapter"
    sources = root / "src"
    _make_package(sources, "kala_agent_freecad")
    _make_package(sources, "kala_agent")
    packages = root / f"site-packages-{kala_paths.abi_tag()}"
    packages.mkdir(parents=True)

    monkeypatch.setenv("KALA_FREECAD_SYS_PATH", str(sources))
    kala_paths.prepare()
    summary = kala_paths.describe()
    assert str(sources) in summary
    assert "cp" in summary
