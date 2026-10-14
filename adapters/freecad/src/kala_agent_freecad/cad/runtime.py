"""Lazy FreeCAD loading for the adapter.

Every FreeCAD import in this package lives inside :func:`load_freecad` and is
performed at call time, never at module import time, so importing the adapter
never touches the native library. The order is fixed and must not be changed:
``FreeCAD`` is imported before ``Part``, ``ObjectsFem``, and
``femtools.ccxtools`` because the FreeCAD workbench modules bind to the
application library that only ``FreeCAD`` initialises. Importing a workbench
first can abort the process inside the native library, which no Python level
try/except can recover from.
"""

from __future__ import annotations

import importlib
import os
import sys
from collections.abc import Iterator, Sequence
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import Any, Final, TypeAlias

from kala_agent_freecad.errors import FreeCADDependencyError

FreeCADObject: TypeAlias = Any
"""A FreeCAD document, feature, or analysis proxy.

FreeCAD hands out C++ backed proxies whose attributes are created on demand,
so their shape is only knowable at run time and is deliberately untyped here.
"""

FREECAD_MODULE: Final[str] = "FreeCAD"
FREECAD_IMPORT_ORDER: Final[tuple[str, ...]] = (
    "FreeCAD",
    "Part",
    "ObjectsFem",
    "femtools.ccxtools",
)
REQUIRED_MODULE_ORDER: Final[tuple[str, ...]] = ("FreeCAD", "Part", "ObjectsFem")
OPTIONAL_MODULE: Final[str] = "femtools.ccxtools"
MODULE_MARKERS: Final[tuple[str, ...]] = ("FreeCAD.so", "FreeCAD.dylib", "FreeCAD.py")
LIB_ROOT_ENVIRONMENT_KEY: Final[str] = "KALA_FREECAD_LIB_ROOT"
MAX_MESSAGE_LENGTH: Final[int] = 512


@dataclass(frozen=True, slots=True)
class FreeCADModules:
    """The FreeCAD modules a single worker call is allowed to use."""

    freecad: FreeCADObject
    part: FreeCADObject
    objects_fem: FreeCADObject
    ccx_tools: FreeCADObject | None
    version: str
    import_root: str
    import_order: tuple[str, ...]
    solver_bridge_detail: str

    @property
    def solver_bridge_available(self) -> bool:
        return self.ccx_tools is not None


def is_import_root(candidate: Path) -> bool:
    try:
        if not candidate.is_dir():
            return False
    except OSError:  # pragma: no cover - defensive
        return False
    return any((candidate / marker).exists() for marker in MODULE_MARKERS)


def import_roots_from_path(entries: Sequence[str] = ()) -> tuple[Path, ...]:
    """Return the FreeCAD import roots already reachable from the loader path."""
    candidates: list[Path] = []
    seen: set[str] = set()
    for value in (*entries, *_environment_roots(), *sys.path):
        if not isinstance(value, str) or not value:
            continue
        candidate = Path(value)
        try:
            resolved = candidate.resolve(strict=False)
        except OSError:  # pragma: no cover - defensive
            continue
        text = resolved.as_posix()
        if text in seen or not is_import_root(resolved):
            continue
        seen.add(text)
        candidates.append(resolved)
    return tuple(candidates)


def _environment_roots() -> tuple[str, ...]:
    declared = os.environ.get(LIB_ROOT_ENVIRONMENT_KEY, "")
    if not declared or len(declared) > 4_096:
        return ()
    return tuple(part for part in declared.split(os.pathsep) if part)


@contextmanager
def native_output_to_stderr() -> Iterator[None]:
    """Point the native file descriptor stdout at stderr for the duration.

    The tool protocol owns the worker's standard output. FreeCAD writes to
    file descriptor one from native code, which bypasses ``sys.stdout``, so
    the descriptor is redirected around every FreeCAD call and restored
    afterwards, and a native banner can never corrupt a response envelope.
    """
    try:
        sys.stdout.flush()
    except (AttributeError, OSError, ValueError):  # pragma: no cover - defensive
        pass
    saved: int | None = None
    try:
        saved = os.dup(1)
        os.dup2(2, 1)
    except (AttributeError, OSError, ValueError):  # pragma: no cover - defensive
        saved = None
    try:
        yield
    finally:
        if saved is not None:
            try:
                sys.stdout.flush()
                os.dup2(saved, 1)
            except (OSError, ValueError):  # pragma: no cover - defensive
                pass
            finally:
                os.close(saved)


@contextmanager
def freecad_session(
    roots: Sequence[Path] = (),
    *,
    require_solver_bridge: bool = False,
) -> Iterator[FreeCADModules]:
    """Import FreeCAD for the duration of one call and clean up afterwards.

    The documents opened during the call are closed on the way out whether the
    body succeeded or failed, and every import root this call introduced is
    removed from the loader path again.
    """
    candidates = import_roots_from_path(tuple(root.as_posix() for root in roots))
    if not candidates:
        raise FreeCADDependencyError("no FreeCAD import root is on the loader path of this worker")
    introduced = _extend_loader_path(candidates)
    freecad: FreeCADObject = None
    order: list[str] = []
    try:
        with native_output_to_stderr():
            freecad = _import_module(FREECAD_MODULE, order)
            part = _import_module("Part", order)
            objects_fem = _import_module("ObjectsFem", order)
            ccx_tools, bridge_detail = _import_solver_bridge(order, require_solver_bridge)
            yield FreeCADModules(
                freecad=freecad,
                part=part,
                objects_fem=objects_fem,
                ccx_tools=ccx_tools,
                version=_version_text(freecad),
                import_root=candidates[0].as_posix(),
                import_order=tuple(order),
                solver_bridge_detail=bridge_detail,
            )
    finally:
        if freecad is not None:
            close_documents(freecad)
        _restore_loader_path(introduced)


def load_sketcher() -> FreeCADObject:
    """Import Sketcher after FreeCAD itself has already been loaded."""
    return _import_module("Sketcher", [])


def _extend_loader_path(roots: Sequence[Path]) -> tuple[str, ...]:
    introduced: list[str] = []
    for root in roots:
        text = root.as_posix()
        if text in sys.path:
            continue
        sys.path.insert(0, text)
        introduced.append(text)
    return tuple(introduced)


def _restore_loader_path(introduced: Sequence[str]) -> None:
    for text in introduced:
        while text in sys.path:
            sys.path.remove(text)


def _import_module(name: str, order: list[str]) -> ModuleType:
    try:
        module = importlib.import_module(name)
    except Exception as error:
        raise FreeCADDependencyError(
            f"the FreeCAD module {name} could not be imported: "
            f"{type(error).__name__}: {str(error)[:MAX_MESSAGE_LENGTH]}"
        ) from error
    order.append(name)
    return module


def _import_solver_bridge(
    order: list[str],
    require: bool,
) -> tuple[ModuleType | None, str]:
    try:
        module = importlib.import_module(OPTIONAL_MODULE)
    except Exception as error:
        detail = (
            f"the CalculiX bridge {OPTIONAL_MODULE} is unavailable: "
            f"{type(error).__name__}: {str(error)[:MAX_MESSAGE_LENGTH]}"
        )
        if require:
            raise FreeCADDependencyError(detail) from error
        return None, detail
    order.append(OPTIONAL_MODULE)
    return module, ""


def _version_text(freecad: FreeCADObject) -> str:
    try:
        version = freecad.Version()
    except Exception:  # pragma: no cover - defensive
        return ""
    if not isinstance(version, (list, tuple)):
        return ""
    parts = [str(part) for part in version[:3]]
    while len(parts) < 3:
        parts.append("0")
    return ".".join(parts)


def close_documents(freecad: FreeCADObject) -> None:
    """Close every document this worker opened, ignoring teardown noise."""
    try:
        names = list(freecad.listDocuments())
    except Exception:  # pragma: no cover - defensive
        return
    for name in names:
        try:
            freecad.closeDocument(name)
        except Exception:  # pragma: no cover - defensive
            continue


__all__ = (
    "FREECAD_IMPORT_ORDER",
    "FREECAD_MODULE",
    "LIB_ROOT_ENVIRONMENT_KEY",
    "MAX_MESSAGE_LENGTH",
    "OPTIONAL_MODULE",
    "REQUIRED_MODULE_ORDER",
    "FreeCADModules",
    "FreeCADObject",
    "close_documents",
    "freecad_session",
    "import_roots_from_path",
    "is_import_root",
    "load_sketcher",
    "native_output_to_stderr",
)
