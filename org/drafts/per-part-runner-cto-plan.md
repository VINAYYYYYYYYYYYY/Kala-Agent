# P1+ Per-Part BOM Runner — POST-#29 Delta Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Baseline (merged #29):** `assess_goal` → `PartPlan` is **consumed by default** — `Agent.run` delegates to `_run_part_plan` (no early-return `status=part_plan` stop). Each bindable `PartSpec` runs sequentially on a **shared CAD backend** via a child `Agent._execute_playbook(..., enrich=False)`; null/unknown `procedure_id` → `part_skip` (never invent). `keep_separate=True` → assembly `export body_id=ALL` (no fuse-all). `keep_separate=False` → per-part fuse allowed via `_skip_silent_fuse`. `part_runs` metadata + `part_bind`/`part_skip` events land on parent `SessionState`. CLI exits 3 **only** for `ClarifyNeeded`. Desktop shows BOM sketch then starts `RunWorker`.

**This doc:** delta work **after** #29 only. Do not re-introduce stop-at-`part_plan` or `KALA_EXECUTE_PART_PLAN=1` opt-in as the base story.

**Tech Stack:** Python 3.11+, `kala.agent.loop.Agent` (`_run_part_plan`, `library_procedure_id`, `_skip_silent_fuse`), `kala.procedures.gate`, `kala.session.state.SessionState`, `kala.parts.catalog.PartsCatalog`, `kala.cad` backends, `pytest`.

**Spec:** #29 behavior + frozen contracts below + CTO assembly rule (`org/grok/03_CTO.txt`: assemblies keep separate + export ALL; single parts fuse to one solid).

---

## Frozen contracts (do not break)

| Contract | Rule |
|----------|------|
| `PartSpec` / `PartPlan` / `ClarifyNeeded` shapes | Fields unchanged (`local_name`, `brief`, `procedure_id`, `keep_separate`; `PartPlan.to_dict()` `kind=part_plan`). |
| `assess_goal` | **Unchanged.** No new gearbox BOM lines, no laptop→PartPlan. Parent `ClarifyNeeded` never bypassed. |
| Procedure / catalog ids | **Never invent.** Bind only `procedure_id ∈ list_procedure_ids()`; catalog only via `PartsCatalog` (+ aliases). |
| `keep_separate` | `True` → no assembly fuse-all; export **ALL** bodies. `False` → fuse inside that part's playbook only. |
| `id_aliases` remap | Existing `removed → body_id` chain resolution stays. Additive `part:<local_name>` keys OK; must follow remaps. |
| `DesignContextModel.enrich` | Part sub-runs use `_procedure_step_context` only — **no enrich** in per-part playbooks. Parent single-playbook path still enriches. |
| CAD Protocol | No new Protocol fields; `part_runs` / `part_body_map` are session metadata only. |

---

## #29 shipped (reference — not re-implement)

- `library_procedure_id()` — null or unknown → skip/clarify path
- `_run_part_plan()` — bindable filter, shared `_open_registry()`, child playbook per part
- `_skip_silent_fuse()` — honors per-part `keep_separate` + `machine_assembly`
- `SessionState.part_runs` — per-part status in `to_dict()`
- `tests/agent/test_part_runner.py` — gearbox bind/skip, keep_separate export ALL, enrich=0 on parts, laptop clarify
- CLI / desktop — PartPlan flows into `Agent.run` (no exit 3 for PartPlan)

---

## Delta: remaining work

### Task 1: Frozen `part:<local_name>` aliases + `part_body_map`

**Why:** #29 merges child `id_aliases` but does not label which live body belongs to which BOM part. Downstream tools / UI / eval need stable part labels without re-parsing history.

**Files:**
- Modify: `kala/agent/loop.py` (`_run_part_plan` after each part completes)
- Modify: `kala/session/state.py` (add `part_body_map`)
- Test: `tests/agent/test_part_runner.py`

**Interfaces:**
- `part_body_map: dict[str, str]` — `local_name → resolved live body_id`
- `state.id_aliases[f"part:{local_name}"] = live_id` — additive; remapped when part bodies fuse/cut (reuse `_resolve` chain from main loop)
- `freeze_part_alias(state, local_name, body_id)` helper in `loop.py` (or small `kala/agent/part_aliases.py` if loop grows)

- [ ] **Step 1: Failing tests**

```python
def test_freeze_part_alias_after_bindable_part():
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=32)
    result = agent.run("planetary gearbox with shaft and housing")
    st = result.state
    for r in st.part_runs:
        if r.get("status") in {"done", "max_turns"} and r.get("procedure_id"):
            name = r["local_name"]
            assert name in st.part_body_map
            assert st.id_aliases.get(f"part:{name}") == st.part_body_map[name]
    data = st.to_dict()
    assert "part_body_map" in data
```

- [ ] **Step 2: Implement** — after each successful part playbook, pick primary body (last ok `create_*` / `boolean_*` / `insert_part` `body_id` for that part, resolved through merged `id_aliases`), call `freeze_part_alias`.
- [ ] **Step 3: Verify** — `.venv/bin/pytest tests/agent/test_part_runner.py -v`
- [ ] **Step 4: Commit** — `feat(agent): freeze part:<local_name> aliases + part_body_map after #29 runner`

---

### Task 2: Catalog bind for null `procedure_id` (skip → insert_part)

**Why:** #29 skips all null `procedure_id` parts. When `PartsCatalog.resolve(local_name|brief)` returns a **real** catalog id, insert instead of skip — still never invent ids.

**Files:**
- Modify: `kala/agent/loop.py` (`_run_part_plan` skip branch)
- Modify: `kala/parts/catalog.py` (use existing `resolve` / `lookup` API — read actual symbol)
- Test: `tests/agent/test_part_runner.py`, `tests/parts/test_catalog_aliases.py`

**Interfaces:**
- New bind mode in `part_runs`: `status="catalog"`, `catalog_part_id=<known id>`
- `registry.call("insert_part", ...)` with catalog id; then `freeze_part_alias`
- Unknown catalog miss → keep current `part_skip` behavior

- [ ] **Step 1: Failing test** — brief with bearing keyword maps to real `bearing_608` (or fixture id), `procedure_id=None`, part runs via catalog not skip.
- [ ] **Step 2: Implement** — only when `library_procedure_id` is None **and** catalog resolves.
- [ ] **Step 3: Verify** — `.venv/bin/pytest tests/agent/test_part_runner.py tests/parts/ -v`
- [ ] **Step 4: Commit** — `feat(agent): catalog bind for null procedure_id parts (no invent)`

---

### Task 3: Alias remap propagation for frozen part keys

**Why:** When a part's bodies fuse inside `keep_separate=False`, `id_aliases` remap donors but `part:<name>` may point at stale ids.

**Files:**
- Modify: `kala/agent/loop.py` (after merging child aliases, re-resolve `part_body_map` entries)
- Test: `tests/agent/test_part_runner.py`, extend `tests/agent/test_alias_persistence.py`

- [ ] **Step 1: Test** — part with `keep_separate=False` + `simple_bracket`; after fuse, `part:bracket` resolves to live fused id.
- [ ] **Step 2: Implement** — `def _resolve_alias(state, bid) -> str` shared; loop `part_body_map` values after each part.
- [ ] **Step 3: Commit** — `fix(agent): remap frozen part:<name> aliases after boolean fuse`

---

### Task 4: Assembly export polish (manifest fallback)

**Why:** #29 uses `export body_id=ALL` when any `keep_separate=True` (#31 soft compound). If backend lacks ALL export, fall back per-part STEP + manifest (no fuse-all).

**Files:**
- Modify: `kala/agent/loop.py` (`_run_part_plan` tail export block)
- Test: `tests/agent/test_part_runner.py`

- [ ] **Step 1: Test** — mock backend without ALL: each `part_body_map` entry exports to `outputs/parts/<local_name>.step`; `last_export` → `outputs/assembly_manifest.json`.
- [ ] **Step 2: Implement** — prefer ALL when `registry.has("export")` and ALL works; else manifest path.
- [ ] **Step 3: Commit** — `feat(agent): per-part STEP manifest when ALL export unavailable`

---

### Task 5: Desktop / CLI surfacing of frozen part ids

**Files:**
- Modify: `kala/ui/desktop.py` (`_done` rail — show `part_body_map` next to `part_runs`)
- Modify: `kala/cli.py` (human output: list frozen ids when `--json` off)
- Test: smoke via existing agent tests (no UI harness required)

- [ ] Show `housing→Box_3` style lines when `part_body_map` populated.
- [ ] Commit — `feat(ui,cli): show frozen part body ids after assembly run`

---

### Task 6: FreeCAD eval smoke (optional gate)

**Files:**
- Create: `tests/agent/test_part_runner_freecad_smoke.py` (mark `@pytest.mark.freecad`, skip when backend unavailable)

- [ ] `planetary gearbox with shaft and housing` on FreeCAD mock-or-real: `done`, `last_export`, ≥2 distinct bodies when both bindable.
- [ ] Document in commit message; do not block mock CI.

---

## Future additive (not in scope for next implement PR)

- **Kill-switch:** `KALA_EXECUTE_PART_PLAN=0` could restore BOM-sketch-only behavior for debugging. Default remains execute (#29). Add only if a concrete regression needs it — not the product story.
- Mating / constraint solver, per-part FEA, new procedure JSON without Founder approval.
- Turning assembly `keep_separate=False` into whole-assembly single brick (forbidden).

---

## Non-goals (P2)

- Change `assess_goal` heuristics or `PartSpec` fields.
- Call `DesignContextModel.enrich` inside per-part playbooks.
- Invent procedure or catalog ids.
- Fuse-all across assembly when any part has `keep_separate=True`.
- Extract `PartPlanRunner` class unless `loop.py` exceeds maintainability threshold (YAGNI — #29 inline is fine for now).

---

## Patch order

1. Task 1 — frozen `part:<local_name>` + `part_body_map`
2. Task 3 — alias remap (depends on Task 1)
3. Task 2 — catalog bind for null procedure_id
4. Task 4 — export manifest fallback
5. Task 5 — UI/CLI surfacing
6. Task 6 — FreeCAD smoke (optional)

---

## Re-run

```bash
.venv/bin/pytest tests/agent/test_part_runner.py tests/agent/test_decompose_gate.py tests/procedures/test_gate.py -v
.venv/bin/pytest tests/agent/test_alias_persistence.py -v
# After Task 6 (optional):
.venv/bin/kala run -b freecad "planetary gearbox with shaft and housing" --json
```

---

## Self-review

1. **POST-#29 only:** Base story is consume/run PartPlan (#29); no opt-in flag as default narrative.
2. **Freezes:** shapes ✓, assess_goal ✓, no invent ✓, keep_separate→ALL ✓, enrich frozen ✓, `part:<name>` additive ✓.
3. **Concrete:** Tasks name real symbols (`_run_part_plan`, `library_procedure_id`, `part_runs`, `freeze_part_alias`).
4. **Docs-only PR:** This file; product code lands on feature branches.

---

*Draft for Founder / Eng Lead. Planning artifact — implement deltas on `feat/*` branches off `develop`, not this docs PR.*
