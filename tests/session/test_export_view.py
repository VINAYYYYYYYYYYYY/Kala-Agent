"""Unit tests for export / part-body display helpers."""

from __future__ import annotations

import json
from pathlib import Path

from kala.session.export_view import (
    export_label,
    format_export_human,
    format_part_body_lines,
    part_steps_from_manifest,
    read_assembly_manifest,
    resolve_assembly_step_paths,
    resolve_step_export,
)


def test_format_part_body_lines_sorted():
    lines = format_part_body_lines({"housing": "Box_3", "shaft": "Cylinder_1"})
    assert lines == ["housing→Box_3", "shaft→Cylinder_1"]


def test_format_part_body_lines_empty():
    assert format_part_body_lines({}) == []
    assert format_part_body_lines(None) == []


def test_read_assembly_manifest(tmp_path: Path):
    manifest = {
        "kind": "assembly_manifest",
        "parts": [
            {"local_name": "housing", "body_id": "Box_3", "step": "outputs/parts/housing.step"},
        ],
    }
    path = tmp_path / "assembly_manifest.json"
    path.write_text(json.dumps(manifest), encoding="utf-8")
    assert read_assembly_manifest(path) == manifest
    assert read_assembly_manifest(tmp_path / "other.json") is None


def test_format_export_human_manifest(tmp_path: Path):
    step = tmp_path / "housing.step"
    step.write_text("step", encoding="utf-8")
    manifest = {
        "kind": "assembly_manifest",
        "parts": [
            {"local_name": "housing", "body_id": "Box_3", "step": str(step)},
        ],
    }
    manifest_path = tmp_path / "assembly_manifest.json"
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

    lines = format_export_human(str(manifest_path), root=tmp_path)
    assert lines[0] == "export: assembly_manifest.json (assembly_manifest)"
    assert lines[1] == "  housing: housing.step"
    assert export_label(str(manifest_path), root=tmp_path) == "assembly_manifest.json (1 parts)"


def test_format_export_human_step():
    lines = format_export_human("outputs/model.step")
    assert lines == ["export: outputs/model.step"]


def test_resolve_step_export_manifest(tmp_path: Path):
    step = tmp_path / "shaft.step"
    step.write_text("step", encoding="utf-8")
    manifest = {
        "kind": "assembly_manifest",
        "parts": [
            {"local_name": "shaft", "body_id": "Cyl_1", "step": str(step)},
        ],
    }
    manifest_path = tmp_path / "assembly_manifest.json"
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

    resolved = resolve_step_export(str(manifest_path), root=tmp_path)
    assert resolved == step


def test_part_steps_from_manifest_relative_under_root(tmp_path: Path):
    parts_dir = tmp_path / "outputs" / "parts"
    parts_dir.mkdir(parents=True)
    housing = parts_dir / "housing.step"
    shaft = parts_dir / "shaft.step"
    housing.write_text("housing", encoding="utf-8")
    shaft.write_text("shaft", encoding="utf-8")
    manifest = {
        "kind": "assembly_manifest",
        "parts": [
            {"local_name": "housing", "body_id": "Box_1", "step": "outputs/parts/housing.step"},
            {"local_name": "shaft", "body_id": "Cyl_1", "step": "outputs/parts/shaft.step"},
        ],
    }
    steps = part_steps_from_manifest(manifest, root=tmp_path)
    assert steps == [housing, shaft]


def test_resolve_assembly_step_paths_multi_part(tmp_path: Path):
    parts_dir = tmp_path / "outputs" / "parts"
    parts_dir.mkdir(parents=True)
    housing = parts_dir / "housing.step"
    shaft = parts_dir / "shaft.step"
    housing.write_text("housing", encoding="utf-8")
    shaft.write_text("shaft", encoding="utf-8")
    manifest = {
        "kind": "assembly_manifest",
        "parts": [
            {"local_name": "housing", "body_id": "Box_1", "step": "outputs/parts/housing.step"},
            {"local_name": "shaft", "body_id": "Cyl_1", "step": "outputs/parts/shaft.step"},
        ],
    }
    manifest_path = tmp_path / "outputs" / "assembly_manifest.json"
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

    resolved = resolve_assembly_step_paths("outputs/assembly_manifest.json", root=tmp_path)
    assert resolved == [housing, shaft]
