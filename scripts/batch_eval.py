"""
Batch-eval Kala on many design briefs.

Each design gets a fresh run folder:
  outputs/eval/<batch>/runs/<id>/
    model.step      — copied export (isolated per design)
    state.json      — full agent state
    score.json      — score + geometry
    analysis.md     — human-readable check
    goal.txt        — exact goal used

Usage:
  uv run python scripts/batch_eval.py designs/batch_cad1000_150.jsonl --resume
  uv run python scripts/batch_eval.py designs/batch_cad1000_150.jsonl --limit 10 --backend mock
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import subprocess
import time
import traceback
from collections import Counter
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]

# First-class gate exits — row outcome taxonomy, not hard Agent errors.
# Post #29: part_plan is executed via _run_part_plan; terminal status is
# needs_clarify (no bindable playbooks) or done/max_turns, with part_plan set.
GATE_OUTCOMES = frozenset({"needs_clarify", "part_plan"})


def _row_outcome(
    *,
    state_status: str | None,
    score_ok: bool,
    err: str | None,
    expect: dict[str, Any] | None = None,
    part_plan: dict[str, Any] | None = None,
) -> str:
    """Exit taxonomy for one batch_eval row."""
    if err:
        return "error"
    if part_plan is not None:
        return "part_plan"
    status = state_status or ""
    if status == "needs_clarify":
        return "needs_clarify"
    if status == "done" and score_ok:
        return "done"
    return "failed"


def _is_gate_success(
    status: str,
    expect: dict[str, Any] | None,
    *,
    part_plan: dict[str, Any] | None = None,
) -> bool:
    """True when the agent exited on the expected (or any valid) gate route."""
    expected = (expect or {}).get("gate")
    if part_plan is not None:
        if expected == "part_plan":
            return status in {"needs_clarify", "done", "max_turns"}
        if expected == "needs_clarify":
            return False
        return status in {"needs_clarify", "done", "max_turns"}
    if status != "needs_clarify":
        return False
    if expected == "needs_clarify":
        return True
    if expected == "part_plan":
        return False
    return True


@dataclass
class Score:
    ok: bool
    score: float  # 0..1
    reasons: list[str] = field(default_factory=list)
    metrics: dict[str, Any] = field(default_factory=dict)


def _load_designs(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for i, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError as exc:
            raise SystemExit(f"{path}:{i}: invalid JSON: {exc}") from exc
        if "goal" not in obj:
            raise SystemExit(f"{path}:{i}: missing 'goal'")
        obj.setdefault("id", f"design-{i:04d}")
        rows.append(obj)
    return rows


def _compute_step_hash(path: Path) -> str | None:
    """Compute SHA256 hash of STEP file for deduplication."""
    try:
        if not path.is_file():
            return None
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except Exception:  # noqa: BLE001
        return None


def _step_metrics(path: Path) -> dict[str, Any]:
    """Read STEP via freecadcmd (avoids Part.so SIGSEGV in plain python).
    
    Retries up to 3 times on crashes/timeouts to handle crashy freecadcmd.
    """
    if not path.is_file():
        return {"error": "missing_file"}
    size = path.stat().st_size
    if size < 200:
        return {"error": "stub_or_empty_step", "bytes": size}
    
    file_hash = _compute_step_hash(path)
    if not file_hash:
        file_hash = "hash_error"
    
    # Escape path for FreeCAD -c string
    p = str(path).replace("\\", "\\\\").replace('"', '\\"')
    code = (
        "import Part, json\n"
        f'path = "{p}"\n'
        "shape = Part.Shape()\n"
        "shape.read(path)\n"
        "if not shape.isValid():\n"
        "    try:\n"
        "        shape.fix()\n"
        "    except Exception:\n"
        "        pass\n"
        "if (not shape.isValid()) and getattr(shape, 'Solids', None):\n"
        "    solids = list(shape.Solids)\n"
        "    if solids:\n"
        "        shape = solids[0]\n"
        "        for s in solids[1:]:\n"
        "            shape = shape.fuse(s)\n"
        "        if not shape.isValid():\n"
        "            try: shape.fix()\n"
        "            except Exception: pass\n"
        "bb = shape.BoundBox\n"
        "print('METRICS', json.dumps({"
        "'solids': len(getattr(shape, 'Solids', []) or ([shape] if shape.Volume else [])), "
        "'volume': float(shape.Volume), "
        "'size': [float(bb.XLength), float(bb.YLength), float(bb.ZLength)], "
        f"'bytes': {size}, "
        "'valid': bool(shape.isValid())"
        "}))\n"
    )
    
    # Retry up to 3 times on crashes/timeouts (freecadcmd can SIGSEGV on malformed STEP)
    last_error = None
    for attempt in range(3):
        try:
            proc = subprocess.run(
                ["freecadcmd", "-c", code],
                capture_output=True,
                text=True,
                timeout=90,
                check=False,
            )
            for line in (proc.stdout or "").splitlines():
                if line.startswith("METRICS "):
                    metrics = json.loads(line[len("METRICS ") :])
                    metrics["hash"] = file_hash
                    return metrics
            if proc.returncode != 0:
                err = (proc.stderr or proc.stdout or "")[-200:]
                last_error = f"freecadcmd_exit={proc.returncode}"
                # Retry on crashes (negative returncodes = signal, e.g. -11=SIGSEGV)
                if proc.returncode < 0 and attempt < 2:
                    time.sleep(0.5)
                    continue
                return {"error": last_error, "detail": err, "hash": file_hash}
            last_error = "no_metrics_line"
            return {"error": last_error, "bytes": size, "hash": file_hash}
        except subprocess.TimeoutExpired:
            last_error = "timeout_90s"
            if attempt < 2:
                time.sleep(0.5)
                continue
            return {"error": last_error, "hash": file_hash}
        except Exception as exc:  # noqa: BLE001
            last_error = str(exc)[:160]
            if attempt < 2:
                time.sleep(0.5)
                continue
    
    return {"error": last_error or "unknown", "hash": file_hash}


def _resolve_export(export: Any) -> Path | None:
    if not export:
        return None
    export_path = Path(str(export))
    if export_path.is_file():
        return export_path
    cand = ROOT / "outputs" / export_path.name
    if cand.is_file():
        return cand
    # Relative under project
    cand2 = ROOT / export_path
    if cand2.is_file():
        return cand2
    return export_path if export_path.exists() else None


def _score_run(
    state: dict[str, Any],
    expect: dict[str, Any] | None,
    *,
    export_path: Path | None = None,
) -> Score:
    expect = expect or {}
    hist = list(state.get("history") or [])
    tools = [e.get("tool") for e in hist]
    fails = [e for e in hist if not e.get("ok")]
    n = max(len(hist), 1)
    fail_rate = len(fails) / n
    searches = sum(1 for t in tools if t == "search_parts")
    unknown = sum(
        1
        for e in fails
        if e.get("tool") == "insert_part"
        and "Unknown part_id" in (e.get("message") or "")
    )
    export = state.get("last_export")
    status = state.get("status") or ""
    part_plan = state.get("part_plan")
    is_clarify_gate = status == "needs_clarify" and not part_plan
    is_part_plan_route = bool(part_plan)
    is_gate = is_clarify_gate or is_part_plan_route
    gate_ok = _is_gate_success(status, expect, part_plan=part_plan)

    reasons: list[str] = []
    points = 1.0

    if is_gate:
        gate_label = "part_plan" if is_part_plan_route else "needs_clarify"
        if gate_ok:
            reasons.append(f"gate={gate_label}")
        else:
            points -= 0.35
            reasons.append(f"unexpected_gate={gate_label}")
    elif status != "done":
        points -= 0.35
        reasons.append(f"status={status}")
    if not is_gate and expect.get("require_export", True) and not (export_path or export):
        points -= 0.25
        reasons.append("missing_export")
    if fail_rate > 0.15:
        points -= min(0.3, fail_rate)
        reasons.append(f"fail_rate={fail_rate:.2f}")
    if searches > 12:
        points -= 0.15
        reasons.append(f"search_spam={searches}")
    if unknown:
        points -= min(0.25, 0.05 * unknown)
        reasons.append(f"unknown_part_id×{unknown}")

    min_tools = int(expect.get("min_tools") or 0)
    real_tools = [t for t in tools if t not in {"list_bodies", "show_in_freecad", "search_parts"}]
    if len(real_tools) < min_tools:
        points -= 0.1
        reasons.append(f"too_few_tools={len(real_tools)}<{min_tools}")

    geo: dict[str, Any] = {}
    path = export_path or _resolve_export(export)
    if not is_gate and path and path.is_file() and str(path).lower().endswith((".step", ".stp")):
        geo = _step_metrics(path)
        if geo.get("error"):
            reasons.append(f"step_read={geo['error']}")
            points -= 0.05
        else:
            if geo.get("solids", 0) < 1:
                points -= 0.2
                reasons.append("no_solids")
            if float(geo.get("volume") or 0) <= 0:
                points -= 0.1
                reasons.append("zero_volume")
            # Tiny exports often mean incomplete models
            if path.stat().st_size < 1500 and float(geo.get("volume") or 0) < 50:
                points -= 0.1
                reasons.append("tiny_geometry")

    points = max(0.0, min(1.0, points))
    has_export = bool(path and path.is_file()) or bool(export)
    auto_ok = is_gate and gate_ok and (is_clarify_gate or (is_part_plan_route and status == "needs_clarify"))
    if auto_ok:
        ok = True
        score_val = 1.0
    else:
        ok = points >= 0.7 and status == "done" and bool(has_export or not expect.get("require_export", True))
        score_val = round(points, 3)
    return Score(
        ok=ok,
        score=score_val,
        reasons=reasons,
        metrics={
            "status": status,
            "tools": len(hist),
            "fails": len(fails),
            "search_parts": searches,
            "unknown_part_ids": unknown,
            "export": str(path) if path else export,
            "usage": state.get("usage"),
            "geometry": geo,
            "fail_messages": [
                f"{e.get('tool')}:{(e.get('message') or '').splitlines()[0][:120]}"
                for e in fails[:8]
            ],
        },
    )


def _rubric_hits(rubrics: list[str], hist: list[dict[str, Any]], goal: str) -> dict[str, Any]:
    """Weak heuristic: which rubric keywords appear in tool args / messages."""
    blob = goal.lower()
    for e in hist:
        blob += " " + json.dumps(e.get("args") or {}, ensure_ascii=False).lower()
        blob += " " + str(e.get("message") or "").lower()
    hits = []
    misses = []
    for r in rubrics[:10]:
        # pick a few content words > 4 chars
        words = [w for w in re.findall(r"[a-zA-Z]{5,}", r.lower()) if w not in {
            "shall", "include", "circular", "through", "central", "upper", "lower",
            "mounting", "structure", "completed", "model", "part", "with", "from",
        }]
        key_words = words[:4]
        if not key_words:
            hits.append(r[:80])
            continue
        if sum(1 for w in key_words if w in blob) >= max(1, len(key_words) // 2):
            hits.append(r[:80])
        else:
            misses.append(r[:80])
    return {"hits": hits, "misses": misses, "hit_rate": round(len(hits) / max(len(hits) + len(misses), 1), 2)}


def _learn_unknown_parts(hist: list[dict[str, Any]]) -> None:
    """Append unknown part_ids to learned_aliases.json for later catalog merge."""
    learned_path = ROOT / "kala" / "parts" / "learned_aliases.json"
    unknown: list[str] = []
    for e in hist:
        msg = e.get("message") or ""
        if e.get("tool") == "insert_part" and "Unknown part_id" in msg:
            m = re.search(r"Unknown part_id:\s*([^\s.]+)", msg)
            if m:
                unknown.append(m.group(1))
    if not unknown:
        return
    data: dict[str, Any] = {"pending": [], "aliases": {}}
    if learned_path.is_file():
        try:
            data = json.loads(learned_path.read_text(encoding="utf-8"))
        except Exception:
            pass
    pending = list(data.get("pending") or [])
    for u in unknown:
        if u not in pending and u not in (data.get("aliases") or {}):
            pending.append(u)
    data["pending"] = pending[:200]
    data["updated"] = datetime.now(timezone.utc).isoformat()
    learned_path.parent.mkdir(parents=True, exist_ok=True)
    learned_path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def _write_analysis(
    run_dir: Path,
    design: dict[str, Any],
    row: dict[str, Any],
    state: dict[str, Any],
    rubric_info: dict[str, Any],
) -> None:
    geo = row.get("score", {}).get("metrics", {}).get("geometry") or {}
    lines = [
        f"# Analysis — `{design['id']}`",
        "",
        f"- **tier**: {design.get('tier', '?')}",
        f"- **ok**: {row['score']['ok']}",
        f"- **score**: {row['score']['score']}",
        f"- **status**: {row.get('state_status')}",
        f"- **elapsed_s**: {row.get('elapsed_s')}",
        f"- **reasons**: {row['score'].get('reasons')}",
        f"- **export**: `{row.get('saved_export') or row.get('export')}`",
        f"- **geometry**: `{json.dumps(geo)}`",
        f"- **usage**: `{json.dumps(row.get('usage'))}`",
        "",
        "## Source task",
        "",
        f"- {((design.get('source') or {}).get('task') or design.get('goal', '')[:120])}",
        "",
        "## Rubric heuristic",
        "",
        f"- hit_rate: **{rubric_info.get('hit_rate')}**",
        "",
        "### Likely covered",
        "",
    ]
    for h in rubric_info.get("hits") or []:
        lines.append(f"- {h}")
    lines += ["", "### Possibly missing", ""]
    for m in rubric_info.get("misses") or []:
        lines.append(f"- {m}")
    fails = row.get("score", {}).get("metrics", {}).get("fail_messages") or []
    if fails:
        lines += ["", "## Tool failures", ""]
        for f in fails:
            lines.append(f"- `{f}`")
    tools = [e.get("tool") for e in (state.get("history") or [])]
    lines += ["", "## Tool sequence", "", "```", " → ".join(str(t) for t in tools[:40]), "```", ""]
    (run_dir / "analysis.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def _run_one(
    design: dict[str, Any],
    *,
    default_backend: str,
    run_dir: Path,
    min_tools_override: int | None = None,
) -> dict[str, Any]:
    from kala.agent.loop import Agent

    run_dir.mkdir(parents=True, exist_ok=True)
    # Wipe previous attempt for this id (fresh files)
    for p in run_dir.iterdir():
        if p.is_file():
            p.unlink()
        elif p.is_dir():
            shutil.rmtree(p)

    export_rel = f"outputs/eval/{run_dir.parent.parent.name}/runs/{design['id']}/model.step"
    export_abs = ROOT / export_rel
    export_abs.parent.mkdir(parents=True, exist_ok=True)

    base_goal = str(design["goal"])
    goal = (
        f"{base_goal}\n\n"
        f"=== KALA EVAL CONTRACT ===\n"
        f"- This is a FRESH design. Do not reuse geometry from other designs.\n"
        f"- When finished, export STEP to this exact path: {export_rel}\n"
        f"- Prefer fuse into one solid for single parts; body_id=ALL only for assemblies.\n"
    )
    (run_dir / "goal.txt").write_text(goal, encoding="utf-8")
    (run_dir / "design.json").write_text(json.dumps(design, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    backend = str(design.get("backend") or default_backend)
    parts = bool(design.get("standard_parts", True))
    procedure = str(design.get("procedure") or "machine_assembly")
    from kala.procedures import require_known_procedure
    t0 = time.time()
    try:
        require_known_procedure(procedure)
        result = Agent(
            backend_name=backend,
            standard_parts=parts,
            procedure_id=procedure,
        ).run(goal)
        state = result.state.to_dict()
        err = None
    except Exception as exc:  # noqa: BLE001
        state = {
            "goal": goal,
            "status": "error",
            "error": str(exc),
            "history": [],
            "last_export": None,
            "usage": None,
        }
        err = traceback.format_exc()[-1500:]
    elapsed = time.time() - t0

    # Capture export into isolated model.step
    saved_export: str | None = None
    src = _resolve_export(state.get("last_export"))
    if src and src.is_file():
        try:
            if src.resolve() != export_abs.resolve():
                shutil.copy2(src, export_abs)
            saved_export = str(export_abs)
        except Exception:
            saved_export = str(src)
    elif export_abs.is_file():
        saved_export = str(export_abs)
        state["last_export"] = saved_export

    (run_dir / "state.json").write_text(json.dumps(state, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    score = _score_run(state, design.get("expect"), export_path=export_abs if export_abs.is_file() else src)
    rubric_info = _rubric_hits(
        list(design.get("rubrics") or []),
        list(state.get("history") or []),
        goal,
    )
    _learn_unknown_parts(list(state.get("history") or []))

    row = {
        "id": design["id"],
        "tier": design.get("tier"),
        "goal": goal[:400],
        "backend": backend,
        "procedure": procedure,
        "standard_parts": parts,
        "elapsed_s": round(elapsed, 2),
        "score": asdict(score),
        "state_status": state.get("status"),
        "outcome": _row_outcome(
            state_status=state.get("status"),
            score_ok=score.ok,
            err=err,
            expect=design.get("expect"),
            part_plan=state.get("part_plan"),
        ),
        "export": state.get("last_export"),
        "saved_export": saved_export,
        "run_dir": str(run_dir),
        "rubric_heuristic": rubric_info,
        "usage": state.get("usage"),
        "error": err,
        "ts": datetime.now(timezone.utc).isoformat(),
    }
    (run_dir / "score.json").write_text(json.dumps(row, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    _write_analysis(run_dir, design, row, state, rubric_info)
    return row


def _batch_summary(rows: list[dict[str, Any]], *, name: str, out_dir: Path, done: bool = False) -> dict[str, Any]:
    passed = sum(1 for r in rows if r.get("score", {}).get("ok"))
    gate_needs_clarify = sum(1 for r in rows if r.get("outcome") == "needs_clarify")
    gate_part_plan = sum(1 for r in rows if r.get("outcome") == "part_plan")
    failed = sum(
        1
        for r in rows
        if r.get("outcome") == "failed" or (not r.get("score", {}).get("ok") and r.get("outcome") not in GATE_OUTCOMES)
    )
    crashed = sum(1 for r in rows if r.get("outcome") == "error")
    summary = {
        "batch": name,
        "n": len(rows),
        "passed": passed,
        "pass_rate": round(passed / max(len(rows), 1), 3),
        "gate_needs_clarify": gate_needs_clarify,
        "gate_part_plan": gate_part_plan,
        "failed": failed,
        "crashed": crashed,
        "avg_score": round(
            sum(float(r.get("score", {}).get("score") or 0) for r in rows) / max(len(rows), 1),
            3,
        ),
        "out_dir": str(out_dir),
        "updated": datetime.now(timezone.utc).isoformat(),
    }
    if done:
        summary["done"] = True
    return summary


def _write_rectify_queue(out_dir: Path, rows: list[dict[str, Any]]) -> None:
    reason_counts: Counter[str] = Counter()
    fail_msg_counts: Counter[str] = Counter()
    weak_rubric: list[str] = []
    hash_collisions: Counter[str] = Counter()
    
    for row in rows:
        for r in row.get("score", {}).get("reasons") or []:
            key = r.split("=")[0].split("×")[0]
            reason_counts[key] += 1
        for m in row.get("score", {}).get("metrics", {}).get("fail_messages") or []:
            fail_msg_counts[
                m.split(":")[0] + ":" + m.split(":")[1][:40] if ":" in m else m[:60]
            ] += 1
        rh = row.get("rubric_heuristic") or {}
        if float(rh.get("hit_rate") or 1) < 0.4 and not row.get("score", {}).get("ok"):
            if row.get("outcome") not in GATE_OUTCOMES:
                weak_rubric.append(row["id"])
        
        # Track STEP hash collisions (duplicate exports)
        geo = row.get("score", {}).get("metrics", {}).get("geometry") or {}
        step_hash = geo.get("hash")
        if step_hash and step_hash != "hash_error":
            hash_collisions[step_hash] += 1

    # Detect high-frequency STEP hash collisions (stub/catalog reuse)
    stub_hashes = [h for h, count in hash_collisions.items() if count >= 3]
    duplicate_exports = {h: count for h, count in hash_collisions.items() if count > 1}
    
    lines = [
        "# Rectify queue (auto)",
        "",
        f"Generated: {datetime.now(timezone.utc).isoformat()}",
        f"Designs scored: {len(rows)}",
        f"Unique STEP hashes: {len(hash_collisions)}",
        f"Duplicate exports (hash collisions): {len(duplicate_exports)}",
        "",
        "## STEP hash collisions (duplicate exports)",
        "",
    ]
    if duplicate_exports:
        for h, count in sorted(duplicate_exports.items(), key=lambda x: -x[1])[:10]:
            # Find IDs with this hash
            ids = [
                row["id"]
                for row in rows
                if row.get("score", {}).get("metrics", {}).get("geometry", {}).get("hash") == h
            ]
            lines.append(f"- `{h[:16]}...` × {count} — {', '.join(ids[:5])}")
        if stub_hashes:
            lines += ["", "### High-frequency hashes (likely stubs/catalog):", ""]
            for h in stub_hashes[:5]:
                lines.append(f"- `{h}`")
    else:
        lines.append("- No duplicate STEP hashes detected ✓")
    
    lines += ["", "## Top failure reasons", ""]
    for k, n in reason_counts.most_common(20):
        lines.append(f"- **{k}** × {n}")
    lines += ["", "## Top tool failure messages", ""]
    for k, n in fail_msg_counts.most_common(20):
        lines.append(f"- `{k}` × {n}")
    if weak_rubric:
        lines += ["", "## Low rubric coverage (failed)", ""]
        for i in weak_rubric[:30]:
            lines.append(f"- `{i}`")
    lines += [
        "",
        "## Gate outcomes (expected non-crash exits)",
        "",
    ]
    gate_rows = [r for r in rows if r.get("outcome") in GATE_OUTCOMES]
    if gate_rows:
        for r in gate_rows:
            lines.append(f"- `{r['id']}` → **{r['outcome']}**")
    else:
        lines.append("- None")
    lines += [
        "",
        "## Suggested agent fixes (priority)",
        "",
        "1. Whatever tops **unknown_part_id** → extend `PartsCatalog.ALIASES` / `learned_aliases.json`",
        "2. **search_spam** → tighten stall breaker / force insert after 1 search",
        "3. **missing_export** / **status=max_turns** → raise turns or advance playbook sooner",
        "4. **fail_rate** → remap body ids / overlap rules / tool allowlists",
        "5. **no_solids** / **zero_volume** / **tiny_geometry** → placement + overlap before fuse/export",
        "",
        "## Failed ids (re-run)",
        "",
    ]
    failed = [
        r["id"]
        for r in rows
        if not r.get("score", {}).get("ok") and r.get("outcome") not in GATE_OUTCOMES
    ]
    for fid in failed:
        lines.append(f"- `{fid}`")
    (out_dir / "rectify_queue.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    (out_dir / "failed_ids.json").write_text(json.dumps(failed, indent=2) + "\n", encoding="utf-8")
    
    # Write stub hashes file for KALA_KNOWN_STUB_HASHES env var
    if stub_hashes:
        stub_data = {
            "hashes": stub_hashes,
            "threshold": "3+ collisions",
            "updated": datetime.now(timezone.utc).isoformat(),
            "env_var": "KALA_KNOWN_STUB_HASHES=" + ",".join(stub_hashes[:10]),
        }
        (out_dir / "stub_hashes.json").write_text(
            json.dumps(stub_data, indent=2) + "\n", encoding="utf-8"
        )


def main() -> None:
    ap = argparse.ArgumentParser(description="Batch-eval Kala designs")
    ap.add_argument("designs", type=Path, help="JSONL designs file")
    ap.add_argument("--limit", type=int, default=0, help="Only first N designs")
    ap.add_argument("--offset", type=int, default=0, help="Skip first N designs")
    ap.add_argument("--backend", default="freecad", choices=["freecad", "mock"])
    ap.add_argument("--out", type=Path, default=None, help="Output dir")
    ap.add_argument("--resume", action="store_true", help="Skip ids already in results.jsonl")
    ap.add_argument("--min-tools", type=int, default=None, help="Override KALA_MIN_TOOLS (default 4)")
    args = ap.parse_args()

    designs = _load_designs(args.designs)
    if args.offset:
        designs = designs[args.offset :]
    if args.limit and args.limit > 0:
        designs = designs[: args.limit]

    name = args.designs.stem
    out_dir = args.out or (ROOT / "outputs" / "eval" / name)
    runs_dir = out_dir / "runs"
    out_dir.mkdir(parents=True, exist_ok=True)
    runs_dir.mkdir(parents=True, exist_ok=True)
    results_path = out_dir / "results.jsonl"
    
    # Wire fidelity env vars for Eval Ops:
    # - KALA_MIN_TOOLS: default 4, extreme batches use 8
    # - KALA_KNOWN_STUB_HASHES: seed from stub_hashes.json when present
    import os
    if args.min_tools is not None:
        os.environ["KALA_MIN_TOOLS"] = str(args.min_tools)
        print(f"KALA_MIN_TOOLS override: {args.min_tools}", flush=True)
    else:
        min_tools_env = os.environ.get("KALA_MIN_TOOLS", "4")
        print(f"KALA_MIN_TOOLS: {min_tools_env} (use --min-tools or KALA_MIN_TOOLS env to override)", flush=True)
    
    # Auto-load stub hashes from prior runs if stub_hashes.json exists
    stub_hashes_path = out_dir / "stub_hashes.json"
    if stub_hashes_path.is_file() and not os.environ.get("KALA_KNOWN_STUB_HASHES"):
        try:
            stub_data = json.loads(stub_hashes_path.read_text(encoding="utf-8"))
            hashes = stub_data.get("hashes") or []
            if hashes:
                os.environ["KALA_KNOWN_STUB_HASHES"] = ",".join(hashes[:10])
                print(f"KALA_KNOWN_STUB_HASHES seeded from {stub_hashes_path}: {len(hashes)} hashes", flush=True)
        except Exception:
            pass

    done_ids: set[str] = set()
    if args.resume and results_path.is_file():
        for line in results_path.read_text(encoding="utf-8").splitlines():
            try:
                done_ids.add(json.loads(line)["id"])
            except Exception:
                continue

    print(f"designs={len(designs)} out={out_dir} resume_skip={len(done_ids)}", flush=True)
    rows: list[dict[str, Any]] = []
    if args.resume and results_path.is_file():
        for line in results_path.read_text(encoding="utf-8").splitlines():
            try:
                rows.append(json.loads(line))
            except Exception:
                continue

    with results_path.open("a" if args.resume else "w", encoding="utf-8") as fh:
        for i, design in enumerate(designs, 1):
            did = design["id"]
            if did in done_ids:
                print(f"[{i}/{len(designs)}] skip {did}", flush=True)
                continue
            print(f"[{i}/{len(designs)}] run {did} …", flush=True)
            run_dir = runs_dir / did
            row = _run_one(design, default_backend=args.backend, run_dir=run_dir, min_tools_override=args.min_tools)
            fh.write(json.dumps(row, ensure_ascii=False) + "\n")
            fh.flush()
            rows.append(row)
            geo = row["score"]["metrics"].get("geometry") or {}
            print(
                f"  → score={row['score']['score']} ok={row['score']['ok']} "
                f"outcome={row.get('outcome')} status={row['state_status']} geo={geo} "
                f"saved={row.get('saved_export')} reasons={row['score']['reasons']}",
                flush=True,
            )
            # Refresh summary after every design so overnight progress is visible
            summary = _batch_summary(rows, name=name, out_dir=out_dir)
            (out_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
            _write_rectify_queue(out_dir, rows)

    summary = _batch_summary(rows, name=name, out_dir=out_dir, done=True)
    (out_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    _write_rectify_queue(out_dir, rows)
    print(json.dumps(summary, indent=2), flush=True)


if __name__ == "__main__":
    main()
