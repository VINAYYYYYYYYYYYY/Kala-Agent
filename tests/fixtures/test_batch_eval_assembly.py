"""batch_eval scores all part STEPs when last_export is an assembly manifest."""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

FIXTURES = Path(__file__).resolve().parent
ROOT = FIXTURES.parents[1]
BATCH_EVAL_PATH = ROOT / "scripts" / "batch_eval.py"


def _load_batch_eval():
    name = "batch_eval_assembly_test_module"
    spec = importlib.util.spec_from_file_location(name, BATCH_EVAL_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def _write_manifest(tmp_path: Path, *, include_shaft: bool = True) -> Path:
    parts_dir = tmp_path / "outputs" / "parts"
    parts_dir.mkdir(parents=True)
    housing = parts_dir / "housing.step"
    housing.write_text("housing" * 40, encoding="utf-8")
    parts = [
        {"local_name": "housing", "body_id": "Box_1", "step": "outputs/parts/housing.step"},
    ]
    if include_shaft:
        shaft = parts_dir / "shaft.step"
        shaft.write_text("shaft" * 40, encoding="utf-8")
        parts.append({"local_name": "shaft", "body_id": "Cyl_1", "step": "outputs/parts/shaft.step"})
    manifest = {"kind": "assembly_manifest", "parts": parts}
    manifest_path = tmp_path / "outputs" / "assembly_manifest.json"
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
    return manifest_path


def test_score_run_manifest_aggregates_all_parts(tmp_path: Path, monkeypatch):
    be = _load_batch_eval()
    manifest_path = _write_manifest(tmp_path)
    calls: list[str] = []

    def fake_metrics(path: Path) -> dict:
        calls.append(path.name)
        return {"solids": 1, "volume": 100.0, "bytes": 500, "valid": True}

    monkeypatch.setattr(be, "_step_metrics", fake_metrics)
    monkeypatch.setattr(be, "ROOT", tmp_path)

    state = {
        "status": "done",
        "last_export": "outputs/assembly_manifest.json",
        "history": [{"tool": "export", "ok": True}],
    }
    copied_single = tmp_path / "model.step"
    copied_single.write_text("only-first-part" * 20, encoding="utf-8")

    score = be._score_run(state, {"require_export": True}, export_path=copied_single)

    assert calls == ["housing.step", "shaft.step"]
    geo = score.metrics["geometry"]
    assert geo["parts"] == 2
    assert geo["solids"] == 2
    assert geo["volume"] == 200.0
    assert score.metrics["export_steps"] == [
        str(tmp_path / "outputs/parts/housing.step"),
        str(tmp_path / "outputs/parts/shaft.step"),
    ]
    assert score.ok is True
    assert manifest_path.is_file()


def test_score_run_manifest_includes_later_part_geometry(tmp_path: Path, monkeypatch):
    """First manifest part may be empty; later parts must still contribute to fidelity."""
    be = _load_batch_eval()
    _write_manifest(tmp_path, include_shaft=True)
    monkeypatch.setattr(be, "ROOT", tmp_path)

    def fake_metrics(path: Path) -> dict:
        if path.name == "housing.step":
            return {"solids": 0, "volume": 0.0, "bytes": 500, "valid": False}
        return {"solids": 1, "volume": 100.0, "bytes": 500, "valid": True}

    monkeypatch.setattr(be, "_step_metrics", fake_metrics)

    state = {
        "status": "done",
        "last_export": "outputs/assembly_manifest.json",
        "history": [{"tool": "export", "ok": True}],
    }
    score = be._score_run(state, {"require_export": True}, export_path=None)

    assert score.metrics["geometry"]["solids"] == 1
    assert score.metrics["geometry"]["volume"] == 100.0
    assert "no_solids" not in score.reasons
    assert score.ok is True
