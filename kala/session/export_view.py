"""Human-facing export and frozen part-body display helpers."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any


def format_part_body_lines(part_body_map: dict[str, str] | None) -> list[str]:
    """Return lines like ``housing→Box_3`` for populated part_body_map."""
    if not part_body_map:
        return []
    return [f"{name}→{body_id}" for name, body_id in sorted(part_body_map.items())]


def read_assembly_manifest(path: str | Path) -> dict[str, Any] | None:
    """Return manifest dict when file content has ``kind=assembly_manifest``."""
    try:
        p = Path(path)
        if not p.is_file() or p.suffix.lower() != ".json":
            return None
        data = json.loads(p.read_text(encoding="utf-8"))
        if isinstance(data, dict) and data.get("kind") == "assembly_manifest":
            return data
    except (OSError, json.JSONDecodeError, TypeError):
        pass
    return None


def resolve_export_path(export: str | Path | None, *, root: Path | None = None) -> Path | None:
    """Resolve last_export to an existing file path."""
    if not export:
        return None
    export_path = Path(str(export))
    if export_path.is_file():
        return export_path
    if root is not None:
        for cand in (root / "outputs" / export_path.name, root / export_path):
            if cand.is_file():
                return cand
    return export_path if export_path.exists() else None


def primary_step_from_manifest(manifest: dict[str, Any]) -> Path | None:
    """First on-disk part STEP path from an assembly manifest."""
    for entry in manifest.get("parts") or []:
        step = entry.get("step")
        if not step:
            continue
        p = Path(str(step))
        if p.is_file():
            return p
    return None


def format_export_human(last_export: str | None, *, root: Path | None = None) -> list[str]:
    """Human-readable export lines; assembly manifests list part STEP paths."""
    if not last_export:
        return []
    path = resolve_export_path(last_export, root=root)
    if path is None:
        return [f"export: {last_export}"]
    manifest = read_assembly_manifest(path)
    if manifest is not None:
        lines = [f"export: {path.name} (assembly_manifest)"]
        for entry in manifest.get("parts") or []:
            name = entry.get("local_name") or "?"
            step = entry.get("step") or "?"
            lines.append(f"  {name}: {Path(str(step)).name}")
        return lines
    return [f"export: {last_export}"]


def export_label(last_export: str | None, *, root: Path | None = None) -> str:
    """Short export label for desktop rail."""
    if not last_export:
        return "—"
    path = resolve_export_path(last_export, root=root)
    if path is None:
        return Path(last_export).name
    manifest = read_assembly_manifest(path)
    if manifest is not None:
        n = len(manifest.get("parts") or [])
        return f"{path.name} ({n} parts)"
    return path.name


def resolve_step_export(export: str | Path | None, *, root: Path | None = None) -> Path | None:
    """Resolve last_export to a STEP file (manifest → first part step)."""
    path = resolve_export_path(export, root=root)
    if path is None:
        return None
    manifest = read_assembly_manifest(path)
    if manifest is not None:
        return primary_step_from_manifest(manifest) or path
    return path
