"""Where the Kala adapter and its dependencies live.

The Kala design box runs inside FreeCAD's own interpreter. Three things have
to be on ``sys.path`` before the adapter can be imported:

* ``kala_agent_freecad`` — the adapter modules,
* ``kala_agent`` — the shared pure-Python contracts package,
* the vendored dependencies for the ABI of the running interpreter
  (``site-packages-cp314`` for CPython 3.14, ``site-packages-cp311`` for
  CPython 3.11, and so on).

The add-on is installed in three shapes, and all three have to work:

1. **Copied out of the AppImage into FreeCAD's ``Mod`` directory.** The
   install copies the add-on *and* its payload (``src/`` and the matching
   ``site-packages-<abi>/``) next to each other, so everything the add-on
   needs sits beside it.
2. **Linked into FreeCAD's ``Mod`` directory** (the developer flow, see
   ``kala_agent_freecad.panel``). The link points at the adapter tree, so the
   payload sits one level above the add-on folder.
3. **An explicit override.** ``KALA_FREECAD_SYS_PATH`` names the directory
   that holds the adapter sources; the Rust provider sets it to
   ``<adapter root>/src``.

Every candidate is checked for the file it has to contain. The resolver never
counts parents to guess a repository root and never adds a virtualenv: the
old ``parents[4]`` layout and the repository ``.venv`` are gone.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

#: The prefix of the per-ABI vendored dependency directories.
SITE_PACKAGES_PREFIX = "site-packages-"

#: The environment variable that overrides the adapter sources.
ADAPTER_PATH_ENV = "KALA_FREECAD_SYS_PATH"

#: The package that carries the adapter modules.
ADAPTER_PACKAGE = "kala_agent_freecad"

#: The package that carries the shared contracts.
SHARED_PACKAGE = "kala_agent"

_state: dict[str, object] = {}


def cache_tag() -> str:
    """The cache tag of the running interpreter, for example ``cpython-314``."""
    return str(getattr(sys.implementation, "cache_tag", "") or "")


def abi_tag() -> str | None:
    """The ABI directory name of the running interpreter, for example ``cp314``.

    ``None`` means the interpreter is not a CPython this project ships
    vendored dependencies for.
    """
    tag = cache_tag()
    if not tag.startswith("cpython"):
        return None
    digits = "".join(character for character in tag if character.isdigit())
    if len(digits) < 2:
        return None
    return f"cp{digits}"


def _is_package(directory: Path, package: str) -> bool:
    return (directory / package / "__init__.py").is_file()


def _candidate_roots() -> list[Path]:
    """The directories to probe, most specific first."""
    roots: list[Path] = []
    for entry in (os.environ.get(ADAPTER_PATH_ENV) or "").split(os.pathsep):
        entry = entry.strip()
        if entry:
            roots.append(Path(entry))
    here = Path(__file__).resolve().parent
    # The payload beside the add-on (the copied install) and inside it.
    roots.append(here)
    roots.append(here / "payload")
    # The adapter root of the staged tree: <root>/mod/Kala -> <root>.
    roots.append(here.parent.parent)
    return roots


def _source_directories(roots: list[Path]) -> list[Path]:
    """Every directory that holds the adapter package."""
    sources: list[Path] = []
    for root in roots:
        for candidate in (root, root / "src"):
            if _is_package(candidate, ADAPTER_PACKAGE) and candidate not in sources:
                sources.append(candidate)
    return sources


def _shared_directory(sources: list[Path]) -> Path | None:
    """The directory that holds the shared contracts package.

    In the packaged add-on it sits beside the adapter package. In a checked
    out repository it lives in the repository's own ``src`` directory, so the
    directory that really contains it is looked for instead of a repository
    root guessed from the depth of the add-on folder.
    """
    for source in sources:
        if _is_package(source, SHARED_PACKAGE):
            return source
    here = Path(__file__).resolve().parent
    for directory in (here, *here.parents):
        candidate = directory / "src"
        if _is_package(candidate, SHARED_PACKAGE):
            return candidate
    return None


def _site_packages_directory(roots: list[Path], abi: str | None) -> Path | None:
    """The vendored dependency directory for this interpreter's ABI."""
    if abi is None:
        return None
    name = f"{SITE_PACKAGES_PREFIX}{abi}"
    for root in roots:
        for candidate in (root / name, root.parent / name):
            if candidate.is_dir():
                return candidate
    return None


def prepare() -> dict[str, object]:
    """Put the adapter and its dependencies on FreeCAD's import path.

    The call is idempotent: FreeCAD runs ``Init.py`` in every session and the
    graphical add-on asks for the same paths again.
    """
    roots = _candidate_roots()
    abi = abi_tag()
    sources = _source_directories(roots)
    shared = _shared_directory(sources)
    site_packages = _site_packages_directory(roots, abi)

    ordered: list[Path] = list(sources)
    if shared is not None and shared not in ordered:
        ordered.append(shared)
    if site_packages is not None:
        ordered.append(site_packages)

    added: list[str] = []
    for directory in reversed(ordered):
        text = str(directory)
        if text in sys.path:
            continue
        sys.path.insert(0, text)
        added.append(text)

    state: dict[str, object] = {
        "abi": abi,
        "cacheTag": cache_tag(),
        "adapter": str(sources[0]) if sources else None,
        "shared": str(shared) if shared is not None else None,
        "sitePackages": str(site_packages) if site_packages is not None else None,
        "added": added,
    }
    _state.update(state)
    return state


def describe() -> str:
    """A short, honest summary of where the adapter was found."""
    state = _state or prepare()
    abi = state.get("abi") or state.get("cacheTag") or "this interpreter"
    adapter = state.get("adapter")
    packages = state.get("sitePackages")
    if not adapter:
        return (
            "The Kala adapter was not found. Install the add-on from the AppImage "
            "or set KALA_FREECAD_SYS_PATH."
        )
    if not packages:
        return (
            f"The Kala adapter is at {adapter}, but the vendored dependencies for "
            f"{abi} are not next to it. Install the add-on with its payload."
        )
    return f"The Kala adapter is at {adapter} with the {abi} dependencies."


def problems() -> list[str]:
    """What is missing for the add-on to work, in plain words."""
    state = _state or prepare()
    issues: list[str] = []
    if not state.get("adapter"):
        issues.append(
            "the Kala adapter sources were not found next to the add-on "
            "(install it from the AppImage or set KALA_FREECAD_SYS_PATH)"
        )
    if not state.get("sitePackages"):
        abi = state.get("abi") or "this interpreter"
        issues.append(
            f"the vendored dependencies for {abi} were not found next to the add-on "
            "(install it with its payload)"
        )
    return issues
