"""Boundary checks for the Solid Edge bridge and the Windows-local client.

These tests are static or import-only. They never execute Windows-specific
code, never open a socket, and never call Solid Edge. They encode the rules the
integration must keep:

* Kala core never imports the new adapters or any shared CAD package.
* The bridge never imports ``kala_agent``; the client imports core contracts
  only (no Studio, no application, no kernel, no tool plane).
* Neither package couples to FreeCAD.
* ``pywin32`` is imported lazily, only from ``session.py``.
* Importing the bridge protocol on a Linux development host loads no
  ``pywin32`` module and no Kala core module.
"""

from __future__ import annotations

import ast
import json
import os
import subprocess
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).parents[2]
CORE_SRC = PROJECT_ROOT / "src" / "kala_agent"
ROOT_SRC = PROJECT_ROOT / "src"
BRIDGE_SRC = PROJECT_ROOT / "adapters" / "solidedge-bridge" / "src"
CLIENT_SRC = PROJECT_ROOT / "adapters" / "solidedge" / "src"
BRIDGE_PACKAGE = BRIDGE_SRC / "kala_solidedge_bridge"
CLIENT_PACKAGE = CLIENT_SRC / "kala_agent_solidedge"

_CAD_PACKAGES = ("kala_agent_solidedge", "kala_solidedge_bridge", "FreeCAD", "FreeCADGui")
_WINDOWS_MODULES = ("pythoncom", "win32com", "pywintypes", "win32api", "wintypes")


def _python_files(root: Path) -> list[Path]:
    return sorted(path for path in root.rglob("*.py") if "__pycache__" not in path.parts)


def _imported_modules(path: Path) -> tuple[str, ...]:
    """Return every module named by an import statement in ``path``."""
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    modules: list[str] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            modules.extend(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module is not None:
            modules.append(node.module)
    return tuple(modules)


def _import_module_literals(path: Path) -> tuple[tuple[str, str], ...]:
    """Return ``(module, innermost enclosing function)`` for ``import_module`` calls."""
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    parents: dict[ast.AST, ast.AST] = {}
    for parent in ast.walk(tree):
        for child in ast.iter_child_nodes(parent):
            parents[child] = parent

    found: list[tuple[str, str]] = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        function = node.func
        if not isinstance(function, ast.Attribute) or function.attr != "import_module":
            continue
        if not node.args:
            continue
        argument = node.args[0]
        if not isinstance(argument, ast.Constant) or not isinstance(argument.value, str):
            continue
        holder = "<module>"
        current = parents.get(node)
        while current is not None:
            if isinstance(current, (ast.FunctionDef, ast.AsyncFunctionDef)):
                holder = current.name
                break
            current = parents.get(current)
        found.append((argument.value, holder))
    return tuple(found)


def test_core_never_imports_the_solidedge_packages_or_shared_cad() -> None:
    for path in _python_files(CORE_SRC):
        for module in _imported_modules(path):
            assert not module.startswith(_CAD_PACKAGES), f"{path}: {module}"


def test_the_bridge_never_imports_kala_core() -> None:
    for path in _python_files(BRIDGE_PACKAGE):
        for module in _imported_modules(path):
            assert module != "kala_agent" and not module.startswith("kala_agent."), (
                f"{path}: {module}"
            )


def test_the_client_imports_core_contracts_only() -> None:
    """The Windows-local client may use the shared construction contract only."""
    for path in _python_files(CLIENT_PACKAGE):
        for module in _imported_modules(path):
            if module == "kala_agent" or module.startswith("kala_agent."):
                assert module == "kala_agent.contracts" or module.startswith(
                    "kala_agent.contracts."
                ), f"{path}: {module}"


def test_neither_package_couples_to_freecad() -> None:
    for root in (BRIDGE_PACKAGE, CLIENT_PACKAGE):
        for path in _python_files(root):
            for module in _imported_modules(path):
                assert not module.startswith(("FreeCAD", "FreeCADGui", "femtools", "Part")), (
                    f"{path}: {module}"
                )


def test_pywin32_is_imported_only_lazily_from_session() -> None:
    """No static pywin32 import exists, and the lazy calls live in session.py."""
    for path in _python_files(BRIDGE_PACKAGE):
        for module in _imported_modules(path):
            assert not module.startswith(_WINDOWS_MODULES), f"{path}: {module} imports {module}"
        for module, holder in _import_module_literals(path):
            if module.startswith(_WINDOWS_MODULES):
                assert path.name == "session.py", f"{path}: lazy {module}"
                assert holder != "<module>", (
                    f"{path}: pywin32 must be imported inside a function body"
                )


def _run_isolated(code: str) -> dict[str, object]:
    environment = dict(os.environ)
    environment["PYTHONPATH"] = os.pathsep.join((str(ROOT_SRC), str(BRIDGE_SRC), str(CLIENT_SRC)))
    environment["PYTHONDONTWRITEBYTECODE"] = "1"
    completed = subprocess.run(
        [sys.executable, "-c", code],
        capture_output=True,
        text=True,
        env=environment,
        timeout=120,
        check=False,
    )
    assert completed.returncode == 0, completed.stderr
    return json.loads(completed.stdout.strip().splitlines()[-1])  # type: ignore[no-any-return]


def test_importing_the_bridge_protocol_loads_no_windows_module_or_core() -> None:
    result = _run_isolated(
        """
import json
import sys

import kala_solidedge_bridge
import kala_solidedge_bridge.protocol

blocked = ("pythoncom", "win32com", "pywintypes", "win32api")
loaded_windows = sorted(name for name in blocked if name in sys.modules)
loaded_core = sorted(name for name in sys.modules if name.startswith("kala_agent"))
print(json.dumps({
    "loaded_windows": loaded_windows,
    "loaded_core": loaded_core,
    "protocol_version": kala_solidedge_bridge.PROTOCOL_VERSION,
}))
"""
    )

    assert result["loaded_windows"] == []
    assert result["loaded_core"] == []
    assert result["protocol_version"] == 1


def test_importing_the_client_compiler_loads_no_windows_module_or_linux_runtime() -> None:
    result = _run_isolated(
        """
import json
import sys

from kala_agent_solidedge import compile, oracle, paths

blocked = ("pythoncom", "win32com", "pywintypes", "win32api")
loaded_windows = sorted(name for name in blocked if name in sys.modules)
loaded_core = sorted(name for name in sys.modules if name.startswith("kala_agent"))
print(json.dumps({
    "loaded_windows": loaded_windows,
    "loaded_core": loaded_core,
    "token_filename": paths.TOKEN_FILENAME,
}))
"""
    )

    assert result["loaded_windows"] == []
    assert "kala_agent.application" not in result["loaded_core"]  # type: ignore[operator]
    assert "kala_agent.runtime" not in result["loaded_core"]  # type: ignore[operator]
    assert "kala_agent.tools" not in result["loaded_core"]  # type: ignore[operator]
    assert result["token_filename"] == "bridge.token"
