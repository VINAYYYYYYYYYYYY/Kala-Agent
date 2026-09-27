from __future__ import annotations

import ast
from pathlib import Path

PROJECT_ROOT = Path(__file__).parents[2]
CORE_ROOT = PROJECT_ROOT / "src" / "kala_agent"
ADAPTER_ROOT = PROJECT_ROOT / "adapters" / "freecad" / "src" / "kala_agent_freecad"
FORBIDDEN_CORE_IMPORTS = ("FreeCAD", "femtools", "kala_agent_freecad")
FORBIDDEN_ADAPTER_IMPORTS = (
    "kala_agent.application",
    "kala_agent.api",
    "kala_agent.cli",
    "kala_agent.domains",
    "kala_agent.kernel",
    "kala_agent.persistence",
    "kala_agent.runtime",
)


def _imports(path: Path) -> tuple[str, ...]:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    names: list[str] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            names.extend(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module is not None:
            names.append(node.module)
    return tuple(names)


def test_core_modules_do_not_import_freecad_or_the_adapter() -> None:
    for path in CORE_ROOT.rglob("*.py"):
        for name in _imports(path):
            assert not name.startswith(FORBIDDEN_CORE_IMPORTS), path


def test_freecad_adapter_does_not_import_core_application_layers() -> None:
    for path in ADAPTER_ROOT.rglob("*.py"):
        for name in _imports(path):
            assert not name.startswith(FORBIDDEN_ADAPTER_IMPORTS), path
