"""batch_eval treats needs_clarify / part_plan as first-class gate outcomes."""

from __future__ import annotations

import importlib.util
import json
import tempfile
from pathlib import Path

FIXTURES = Path(__file__).resolve().parent
ROOT = FIXTURES.parents[1]
BATCH_EVAL_PATH = ROOT / "scripts" / "batch_eval.py"


def _load_batch_eval():
    import sys

    name = "batch_eval_gate_test_module"
    spec = importlib.util.spec_from_file_location(name, BATCH_EVAL_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def _load_fixture_designs() -> list[dict]:
    path = FIXTURES / "batch_eval_gate.jsonl"
    rows: list[dict] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line:
            rows.append(json.loads(line))
    return rows


def test_gate_outcome_taxonomy_helpers():
    be = _load_batch_eval()
    assert be._row_outcome(state_status="needs_clarify", score_ok=True, err=None) == "needs_clarify"
    assert be._row_outcome(state_status="part_plan", score_ok=True, err=None) == "part_plan"
    assert be._row_outcome(state_status="done", score_ok=True, err=None) == "done"
    assert be._row_outcome(state_status="max_turns", score_ok=False, err=None) == "failed"
    assert be._row_outcome(state_status="done", score_ok=False, err="traceback") == "error"


def test_laptop_needs_clarify_row():
    be = _load_batch_eval()
    design = next(d for d in _load_fixture_designs() if d["id"] == "gate-laptop")
    with tempfile.TemporaryDirectory() as tmp:
        row = be._run_one(design, default_backend="mock", run_dir=Path(tmp) / "gate-laptop")
    assert row["state_status"] == "needs_clarify"
    assert row["outcome"] == "needs_clarify"
    assert row["score"]["ok"] is True
    assert row["error"] is None
    assert "gate=needs_clarify" in row["score"]["reasons"]


def test_gearbox_part_plan_row():
    be = _load_batch_eval()
    design = next(d for d in _load_fixture_designs() if d["id"] == "gate-gearbox")
    with tempfile.TemporaryDirectory() as tmp:
        row = be._run_one(design, default_backend="mock", run_dir=Path(tmp) / "gate-gearbox")
    assert row["state_status"] == "part_plan"
    assert row["outcome"] == "part_plan"
    assert row["score"]["ok"] is True
    assert row["error"] is None
    assert row["score"]["metrics"].get("status") == "part_plan"


def test_l_bracket_still_runs():
    be = _load_batch_eval()
    design = next(d for d in _load_fixture_designs() if d["id"] == "gate-l-bracket")
    with tempfile.TemporaryDirectory() as tmp:
        row = be._run_one(design, default_backend="mock", run_dir=Path(tmp) / "gate-l-bracket")
    assert row["state_status"] not in be.GATE_OUTCOMES
    assert row["outcome"] != "needs_clarify"
    assert row["outcome"] != "part_plan"
    assert row["error"] is None
    assert row["score"]["metrics"]["tools"] >= 1
