# P1+ Per-Part BOM Runner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** After the P0 decompose-or-clarify gate emits a `PartPlan`, actually bind each `PartSpec` to a known procedure (or catalog/primitive path), run modeling per part on a shared backend, freeze `local_name → body_id` aliases, honor `keep_separate`, and export an assembly compound — instead of stopping at `status=part_plan`.

**Architecture:** Keep `assess_goal` / `PartSpec` / `PartPlan` as the BOM contract (P0). Add a `PartPlanRunner` that owns a single CAD backend + `SessionState` for the whole assembly. For each part: bind `procedure_id` when set (never invent), run a scoped planner loop with that procedure’s steps, register a frozen alias `part:<local_name>` → live `body_id` in `state.id_aliases`, and fuse only when `keep_separate is False`. Finish with multi-body export (compound / all solids). Opt-in via `KALA_EXECUTE_PART_PLAN=1` (default off preserves P0 stop behavior until tests are green).

**Tech Stack:** Python 3.11+, existing `kala.agent.loop.Agent`, `kala.procedures.gate` (`PartSpec`/`PartPlan`), `kala.session.state.SessionState.id_aliases`, `kala.cad` MockBackend + FreeCAD backend, `pytest`, packaged procedures under `kala/procedures/library/`.

**Spec:** This plan + P0 gate contract in `kala/procedures/gate.py` (PartSpec fields, “never invent procedure_id”, keep_separate default True) + CTO hard limit “Assemblies: keep separate + export ALL; parts: fuse to one solid” (`org/grok/03_CTO.txt`).

## Global Constraints

- Never invent `procedure_id` values — only `list_procedure_ids()` / `require_known_procedure`, else leave `None` and take catalog/primitive/clarify path.
- Never invent catalog `part_id`s outside `PartsCatalog` (+ aliases).
- Default `keep_separate=True` (P0); fuse across a part’s own solids only when that part’s flag is False; never fuse the whole assembly into one brick by default.
- Frozen ids live in `SessionState.id_aliases` as `part:<local_name>` → live FreeCAD/mock `body_id`; remap after fuse/cut the same way the main loop already remaps `removed → body_id`.
- Do not load `Part.so` into the UI process; FreeCAD stays out-of-process / backend process.
- Opt-in execution: `KALA_EXECUTE_PART_PLAN=1` (or explicit API flag). Default remains P0 early-return so desktop/CLI do not regress.
- YAGNI: no mating constraint solver, no per-part FEA, no new procedure JSON without Founder approval (P2 non-goals).
- Docs-only commits for this planning PR; product code lands in follow-up implement PRs.

---

## File map (create / modify)

| Path | Responsibility |
|------|----------------|
| `kala/agent/part_runner.py` | **Create.** `PartPlanRunner`, `PartRunResult`, bind + per-part loop + freeze aliases + assembly export. |
| `kala/agent/loop.py` | **Modify.** When `PartPlan` and execute flag on: delegate to `PartPlanRunner` instead of early-return; share `_resolve` / remap patterns. |
| `kala/session/state.py` | **Modify (small).** Optional `part_body_map: dict[str, str]` (local_name → frozen body_id) serialized in `to_dict` for UI/CLI; keep `id_aliases` as source of truth for remaps. |
| `kala/cli.py` | **Modify.** Honor execute flag; print per-part status + frozen ids + export; exit 0 on successful assembly (not exit 3). |
| `kala/ui/desktop.py` | **Modify.** When execute flag on, continue into runner after showing BOM sketch; rail shows per-part progress. |
| `kala/procedures/__init__.py` | **Modify.** Re-export runner types if public. |
| `tests/agent/test_part_runner.py` | **Create.** TDD for bind, keep_separate, frozen aliases, opt-in flag. |
| `tests/agent/test_decompose_gate.py` | **Modify.** Assert default still early-returns; execute path runs. |
| `tests/procedures/test_gate.py` | **Unchanged contract** — PartSpec fields stay frozen unless a later task explicitly extends. |

---

### Task 1: PartPlanRunner skeleton + opt-in gate hook

**Files:**
- Create: `kala/agent/part_runner.py`
- Modify: `kala/agent/loop.py:388-414` (PartPlan branch)
- Test: `tests/agent/test_part_runner.py`

**Interfaces:**
- Consumes: `PartPlan`, `PartSpec` from `kala.procedures.gate`; `Agent` construction knobs (`backend_name`, `planner`, `standard_parts`, `max_turns`); `os.environ["KALA_EXECUTE_PART_PLAN"]`
- Produces:
  - `def execute_part_plan_enabled() -> bool`
  - `@dataclass class PartRunResult: state: SessionState; contexts: list; part_results: list[dict]`
  - `class PartPlanRunner: def run(self, goal: str, plan: PartPlan) -> PartRunResult`
  - `Agent.run` still returns `RunResult`; on execute path `status` becomes `"running"`/`"done"`/`"error"`, never stuck at `"part_plan"`

- [ ] **Step 1: Write the failing tests**

```python
# tests/agent/test_part_runner.py
from __future__ import annotations

import os

import pytest

from kala.agent.loop import Agent
from kala.agent.part_runner import PartPlanRunner, execute_part_plan_enabled
from kala.llm.stub import StubPlanner
from kala.procedures.gate import PartPlan, PartSpec, assess_goal


def test_execute_flag_default_off(monkeypatch):
    monkeypatch.delenv("KALA_EXECUTE_PART_PLAN", raising=False)
    assert execute_part_plan_enabled() is False


def test_execute_flag_on(monkeypatch):
    monkeypatch.setenv("KALA_EXECUTE_PART_PLAN", "1")
    assert execute_part_plan_enabled() is True


def test_agent_part_plan_still_stops_when_flag_off(monkeypatch):
    monkeypatch.delenv("KALA_EXECUTE_PART_PLAN", raising=False)
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=4)
    result = agent.run("planetary gearbox with sun and planets")
    assert result.state.status == "part_plan"
    assert result.state.part_plan is not None
    assert result.state.part_plan["kind"] == "part_plan"
    assert not result.state.history


def test_runner_accepts_part_plan_object():
    plan = PartPlan(
        parts=[
            PartSpec(local_name="housing", brief="housing envelope", procedure_id="housing_cover", keep_separate=True),
            PartSpec(local_name="shaft", brief="stepped shaft", procedure_id="stepped_shaft", keep_separate=True),
        ],
        notes="test",
    )
    runner = PartPlanRunner(backend_name="mock", planner=StubPlanner(), max_turns=6)
    out = runner.run("multi-part housing and shaft assembly", plan)
    assert out.state.part_plan is not None
    assert out.state.status in {"done", "error", "running", "needs_clarify"}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/v9i/Kala-Agent && .venv/bin/pytest tests/agent/test_part_runner.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'kala.agent.part_runner'` (or import error for `PartPlanRunner`).

- [ ] **Step 3: Write minimal implementation**

```python
# kala/agent/part_runner.py
"""P1+ per-part BOM runner — consume PartPlan, bind procedures, freeze body ids."""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from typing import Any

from kala.llm.base import PlannerProtocol
from kala.ml.base import DesignContextModel, DynamicContext
from kala.procedures.gate import PartPlan, PartSpec
from kala.session.state import SessionState


def execute_part_plan_enabled() -> bool:
    return os.environ.get("KALA_EXECUTE_PART_PLAN", "").strip().lower() in {
        "1",
        "true",
        "yes",
        "on",
    }


@dataclass
class PartRunResult:
    state: SessionState
    contexts: list[DynamicContext] = field(default_factory=list)
    part_results: list[dict[str, Any]] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return {
            "state": self.state.to_dict(),
            "contexts": [c.to_dict() for c in self.contexts],
            "part_results": list(self.part_results),
        }


class PartPlanRunner:
    """Run each PartSpec on a shared backend; freeze part:<name> aliases."""

    def __init__(
        self,
        *,
        backend_name: str = "mock",
        standard_parts: bool = False,
        planner: PlannerProtocol | None = None,
        context_model: DesignContextModel | None = None,
        max_turns: int = 32,
    ) -> None:
        self.backend_name = backend_name
        self.standard_parts = standard_parts
        self.planner = planner
        self.context_model = context_model
        self.max_turns = max_turns

    def run(self, goal: str, plan: PartPlan) -> PartRunResult:
        # Task 1 stub: record plan, do not model yet (Task 2 fills bind+loop).
        from kala.agent.loop import Agent

        agent = Agent(
            backend_name=self.backend_name,
            standard_parts=self.standard_parts,
            procedure_id="machine_assembly",
            planner=self.planner,
            context_model=self.context_model,
            max_turns=self.max_turns,
        )
        # Build empty assembly state without re-entering assess_goal PartPlan stop:
        state, _registry = agent._build(goal)
        state.status = "part_plan"
        state.part_plan = plan.to_dict()
        state.error = "PartPlanRunner stub — bind/run lands in Task 2"
        return PartRunResult(state=state, contexts=[], part_results=[])
```

Wire `Agent.run` PartPlan branch:

```python
# in kala/agent/loop.py run(), replace early-return body when flag on:
        if isinstance(gate, PartPlan):
            from kala.agent.part_runner import PartPlanRunner, execute_part_plan_enabled

            if not execute_part_plan_enabled():
                procedure = load_default_procedure(self.procedure_id)
                state = SessionState(
                    goal=goal,
                    backend_name=self.backend_name,
                    standard_parts=self.standard_parts,
                    procedure=procedure,
                    status="part_plan",
                    part_plan=gate.to_dict(),
                    error=gate.notes or "Part plan required before binding procedures.",
                )
                return RunResult(state=state, contexts=contexts)
            runner = PartPlanRunner(
                backend_name=self.backend_name,
                standard_parts=self.standard_parts,
                planner=self.planner,
                context_model=self.context_model,
                max_turns=self.max_turns,
            )
            out = runner.run(goal, gate)
            return RunResult(state=out.state, contexts=out.contexts)
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `.venv/bin/pytest tests/agent/test_part_runner.py::test_execute_flag_default_off tests/agent/test_part_runner.py::test_execute_flag_on tests/agent/test_part_runner.py::test_agent_part_plan_still_stops_when_flag_off tests/agent/test_part_runner.py::test_runner_accepts_part_plan_object tests/agent/test_decompose_gate.py -v`
Expected: PASS (decompose gate unchanged; runner stub returns a state with `part_plan` set).

- [ ] **Step 5: Commit**

```bash
git add kala/agent/part_runner.py kala/agent/loop.py tests/agent/test_part_runner.py
git commit -m "$(cat <<'EOF'
feat(agent): PartPlanRunner stub + KALA_EXECUTE_PART_PLAN opt-in

P0 part_plan early-return stays default; execute path delegates to runner.
EOF
)"
```

---

### Task 2: Bind PartSpec → procedure / catalog / clarify

**Files:**
- Modify: `kala/agent/part_runner.py`
- Test: `tests/agent/test_part_runner.py`

**Interfaces:**
- Consumes: `PartSpec.procedure_id`, `require_known_procedure`, `PartsCatalog.default().resolve`
- Produces:
  - `def bind_part(spec: PartSpec, catalog: PartsCatalog) -> BoundPart`
  - `@dataclass class BoundPart: spec: PartSpec; mode: Literal["procedure","catalog","primitive","clarify"]; procedure_id: str | None; catalog_part_id: str | None; clarify: ClarifyNeeded | None`

- [ ] **Step 1: Write the failing tests**

```python
from kala.agent.part_runner import BoundPart, bind_part
from kala.parts.catalog import PartsCatalog
from kala.procedures.gate import PartSpec


def test_bind_known_procedure():
    cat = PartsCatalog.default()
    b = bind_part(PartSpec("housing", "box", "housing_cover", True), cat)
    assert b.mode == "procedure"
    assert b.procedure_id == "housing_cover"
    assert b.clarify is None


def test_bind_rejects_invented_procedure():
    cat = PartsCatalog.default()
    b = bind_part(PartSpec("widget", "thing", "not_a_real_proc", True), cat)
    # invented id must not run — clarify or strip to null path
    assert b.mode == "clarify" or (b.mode != "procedure" and b.procedure_id is None)
    if b.mode == "clarify":
        assert b.clarify is not None


def test_bind_null_procedure_tries_catalog():
    cat = PartsCatalog.default()
    b = bind_part(PartSpec("bearing", "608 skate bearing", None, True), cat)
    assert b.mode in {"catalog", "primitive", "clarify"}
    if b.mode == "catalog":
        assert b.catalog_part_id  # real catalog id only
```

- [ ] **Step 2: Run test to verify it fails**

Run: `.venv/bin/pytest tests/agent/test_part_runner.py::test_bind_known_procedure tests/agent/test_part_runner.py::test_bind_rejects_invented_procedure tests/agent/test_part_runner.py::test_bind_null_procedure_tries_catalog -v`
Expected: FAIL with `ImportError` / `bind_part` not defined.

- [ ] **Step 3: Write minimal implementation**

```python
# add to kala/agent/part_runner.py
from dataclasses import dataclass
from typing import Literal

from kala.parts.catalog import PartsCatalog
from kala.procedures.gate import ClarifyNeeded, PartSpec
from kala.procedures.schema import list_procedure_ids


@dataclass
class BoundPart:
    spec: PartSpec
    mode: Literal["procedure", "catalog", "primitive", "clarify"]
    procedure_id: str | None = None
    catalog_part_id: str | None = None
    clarify: ClarifyNeeded | None = None


def bind_part(spec: PartSpec, catalog: PartsCatalog) -> BoundPart:
    known = set(list_procedure_ids())
    pid = spec.procedure_id
    if pid:
        if pid in known:
            return BoundPart(spec=spec, mode="procedure", procedure_id=pid)
        return BoundPart(
            spec=spec,
            mode="clarify",
            clarify=ClarifyNeeded(
                reason=f"Unknown procedure_id {pid!r} for part {spec.local_name!r}.",
                questions=[
                    f"Pick a known procedure for {spec.local_name}: {', '.join(sorted(known))}",
                    "Or clear procedure_id and use catalog/primitive.",
                ],
            ),
        )
    # null procedure — catalog resolve by local_name / brief tokens
    resolved = catalog.resolve(spec.local_name) or catalog.resolve(spec.brief)
    if resolved:
        return BoundPart(spec=spec, mode="catalog", catalog_part_id=resolved)
    # last resort: primitive envelope under machine_assembly (no invented procedure)
    return BoundPart(spec=spec, mode="primitive", procedure_id="machine_assembly")
```

Use the real `PartsCatalog.resolve` / lookup API already in `kala/parts/catalog.py` (read the method name before coding — if it is `lookup` / `resolve_id`, call that exact symbol; do not invent).

- [ ] **Step 4: Run tests to verify they pass**

Run: `.venv/bin/pytest tests/agent/test_part_runner.py -k bind -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add kala/agent/part_runner.py tests/agent/test_part_runner.py
git commit -m "feat(agent): bind PartSpec to procedure/catalog/clarify (no invented ids)"
```

---

### Task 3: Shared-backend per-part loop + frozen `part:<local_name>` aliases

**Files:**
- Modify: `kala/agent/part_runner.py`
- Modify: `kala/session/state.py` (add `part_body_map`)
- Modify: `kala/agent/loop.py` (extract or reuse single-procedure run-on-existing-state helper if needed)
- Test: `tests/agent/test_part_runner.py`, `tests/agent/test_alias_persistence.py` (extend)

**Interfaces:**
- Consumes: `BoundPart`, existing `id_aliases` remap in `Agent.run` loop
- Produces:
  - `PartPlanRunner.run` iterates parts, runs modeling, sets `state.id_aliases[f"part:{local_name}"] = live_body_id` and `state.part_body_map[local_name] = live_body_id`
  - After fuse/cut inside a part, update frozen alias to the remapped live id (same chain resolution as `test_alias_chain_resolution`)
  - `def freeze_part_alias(state: SessionState, local_name: str, body_id: str) -> None`

- [ ] **Step 1: Write the failing tests**

```python
from kala.agent.part_runner import PartPlanRunner, freeze_part_alias
from kala.llm.stub import StubPlanner
from kala.procedures.gate import PartPlan, PartSpec
from kala.procedures.schema import load_default_procedure
from kala.session.state import SessionState


def test_freeze_part_alias_writes_both_maps():
    state = SessionState(
        goal="t",
        backend_name="mock",
        standard_parts=False,
        procedure=load_default_procedure("machine_assembly"),
    )
    freeze_part_alias(state, "housing", "Box_1")
    assert state.id_aliases["part:housing"] == "Box_1"
    assert state.part_body_map["housing"] == "Box_1"
    data = state.to_dict()
    assert data["part_body_map"]["housing"] == "Box_1"
    assert data["id_aliases"]["part:housing"] == "Box_1"


def test_freeze_updates_when_body_remapped():
    state = SessionState(
        goal="t",
        backend_name="mock",
        standard_parts=False,
        procedure=load_default_procedure("simple_bracket"),
    )
    freeze_part_alias(state, "bracket", "Box_1")
    # simulate fuse remap
    state.id_aliases["Box_1"] = "Fuse_1"
    # runner must re-freeze to live id after part completes
    live = "Fuse_1"
    freeze_part_alias(state, "bracket", live)
    assert state.id_aliases["part:bracket"] == "Fuse_1"
    assert state.part_body_map["bracket"] == "Fuse_1"


def test_runner_sets_frozen_ids_for_procedure_parts(monkeypatch):
    monkeypatch.setenv("KALA_EXECUTE_PART_PLAN", "1")
    plan = PartPlan(
        parts=[
            PartSpec("housing", "housing", "housing_cover", True),
        ],
        notes="one part",
    )
    runner = PartPlanRunner(backend_name="mock", planner=StubPlanner(), max_turns=12)
    out = runner.run("multi-part housing assembly", plan)
    # After real loop: either done with frozen id, or error — never silent empty
    assert out.state.part_plan is not None
    if out.state.status == "done":
        assert "housing" in out.state.part_body_map
        assert out.state.id_aliases.get("part:housing") == out.state.part_body_map["housing"]
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `.venv/bin/pytest tests/agent/test_part_runner.py::test_freeze_part_alias_writes_both_maps -v`
Expected: FAIL (`part_body_map` / `freeze_part_alias` missing).

- [ ] **Step 3: Write minimal implementation**

Add to `SessionState`:

```python
    # local_name → live body_id for BOM parts (frozen labels; remapped after boolean)
    part_body_map: dict[str, str] = field(default_factory=dict)
```

Include `"part_body_map": self.part_body_map` in `to_dict()`.

```python
def freeze_part_alias(state: SessionState, local_name: str, body_id: str) -> None:
    key = f"part:{local_name}"
    state.id_aliases[key] = body_id
    state.part_body_map[local_name] = body_id


def _resolve_alias(state: SessionState, bid: str) -> str:
    seen: set[str] = set()
    cur = bid
    while cur in state.id_aliases and cur not in seen:
        seen.add(cur)
        cur = state.id_aliases[cur]
    return cur
```

Implement `PartPlanRunner.run` roughly:

1. `state, registry = agent._build(goal)` with `procedure_id="machine_assembly"`.
2. `state.part_plan = plan.to_dict()`; `state.status = "running"`.
3. For each `spec` in `plan.parts`:
   - `bound = bind_part(spec, catalog)`
   - If `bound.mode == "clarify"`: set `state.status="needs_clarify"`, `state.clarify=bound.clarify.to_dict()`, return.
   - If `procedure`: temporarily set `state.procedure = load_default_procedure(bound.procedure_id)`, `state.step_index = 0`, run the **existing** planner turn loop for up to `max_turns` **on this shared state/registry** (extract `_run_turns(state, registry, contexts)` from `Agent` if needed — prefer extract over copy-paste).
   - If `catalog`: `registry.call("insert_part", part_id=bound.catalog_part_id, ...)` (use exact insert_part args from `kala/cad/registry.py`).
   - If `primitive`: allow only envelope creates for this part’s brief (one box/cylinder); no invented tools.
   - Pick primary body for the part (last successful `body_id` in history since part start, resolved through aliases).
   - If `spec.keep_separate is False`: fuse all bodies created for this part into one (use `boolean_fuse` pairwise), then freeze.
   - If `keep_separate is True`: do **not** fuse with other parts’ bodies; freeze primary body only.
   - `freeze_part_alias(state, spec.local_name, _resolve_alias(state, primary))`
   - Append to `part_results`.
4. After all parts: assembly export (Task 4) or leave `status=running` until Task 4 lands.

Critical: **do not call `Agent.run(part_goal)` per part** if that re-enters `assess_goal` and rebuilds a fresh backend — parts must share one document. Extract `_run_turns` or pass `skip_gate=True` / run-on-state helper.

- [ ] **Step 4: Run tests to verify they pass**

Run: `.venv/bin/pytest tests/agent/test_part_runner.py tests/agent/test_alias_persistence.py -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add kala/agent/part_runner.py kala/agent/loop.py kala/session/state.py tests/agent/test_part_runner.py
git commit -m "feat(agent): per-part shared-backend run + freeze part:<name> aliases"
```

---

### Task 4: Assembly keep_separate export (export ALL / compound)

**Files:**
- Modify: `kala/agent/part_runner.py`
- Modify: `kala/agent/loop.py` (`_auto_export` awareness of `part_body_map` optional)
- Test: `tests/agent/test_part_runner.py`

**Interfaces:**
- Consumes: `state.part_body_map`, `registry.call("export", ...)`, `list_bodies`
- Produces: `state.last_export` path; `status="done"` when export ok; export must include every frozen part body when all `keep_separate=True`

- [ ] **Step 1: Write the failing tests**

```python
def test_assembly_export_keeps_separate_bodies(monkeypatch):
    monkeypatch.setenv("KALA_EXECUTE_PART_PLAN", "1")
    plan = PartPlan(
        parts=[
            PartSpec("housing", "housing envelope", "housing_cover", True),
            PartSpec("shaft", "shaft", "stepped_shaft", True),
        ],
        notes="keep separate",
    )
    runner = PartPlanRunner(backend_name="mock", planner=StubPlanner(), max_turns=16)
    out = runner.run("multi-part housing and shaft assembly", plan)
    assert out.state.status == "done"
    assert out.state.last_export
    # two frozen parts, distinct live ids
    assert set(out.state.part_body_map) >= {"housing", "shaft"}
    assert out.state.part_body_map["housing"] != out.state.part_body_map["shaft"]


def test_merged_part_fuses_before_freeze(monkeypatch):
    monkeypatch.setenv("KALA_EXECUTE_PART_PLAN", "1")
    plan = PartPlan(
        parts=[
            PartSpec("bracket", "L bracket fused solid", "simple_bracket", False),
        ],
        notes="single merged part",
    )
    runner = PartPlanRunner(backend_name="mock", planner=StubPlanner(), max_turns=16)
    out = runner.run("multi-part bracket assembly", plan)
    # keep_separate False → one frozen id for the part
    assert "bracket" in out.state.part_body_map
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `.venv/bin/pytest tests/agent/test_part_runner.py::test_assembly_export_keeps_separate_bodies -v`
Expected: FAIL (`status != done` or missing export).

- [ ] **Step 3: Write minimal implementation**

```python
def _export_assembly(state: SessionState, registry) -> None:
    """Export all frozen part bodies. Prefer multi-body/compound if backend supports it."""
    if not registry.has("export"):
        state.status = "error"
        state.error = "export tool missing"
        return
    bodies = [state.part_body_map[n] for n in state.part_body_map]
    if not bodies:
        state.status = "error"
        state.error = "no frozen part bodies to export"
        return
    # Mock/FreeCAD: export primary then rely on backend compound behavior.
    # If export only takes one body_id, export the first and attach siblings via
    # backend-specific compound API if present; else export each to
    # outputs/parts/<local_name>.step and set last_export to a manifest path.
    primary = bodies[0]
    path = f"outputs/assembly_{abs(hash(state.goal)) % 10_000_000}.step"
    result = registry.call("export", body_id=primary, path=path, fmt="step")
    # If backend supports body_ids=list, prefer that — check registry schema first.
    state.history.append(
        # ToolEvent(...) same shape as Agent loop
    )
    if result.ok:
        state.last_export = str(result.data.get("path") or path)
        state.status = "done"
    else:
        state.status = "error"
        state.error = result.message
```

Read `export` tool schema in `kala/cad/registry.py` / FreeCAD backend **before** coding and match the real signature (likely `body_id`, `path`, `fmt`). If only single-body export exists, P1 acceptable fallback:

1. Write each part to `outputs/parts/<local_name>.step`
2. Write `outputs/assembly_manifest.json` listing paths + frozen ids
3. Set `state.last_export` to the manifest path

Document that full STEP compound is P1.1 if backend lacks multi-body export.

- [ ] **Step 4: Run tests to verify they pass**

Run: `.venv/bin/pytest tests/agent/test_part_runner.py -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add kala/agent/part_runner.py tests/agent/test_part_runner.py
git commit -m "feat(agent): assembly export honors keep_separate + frozen part bodies"
```

---

### Task 5: Wire CLI + desktop to execute path

**Files:**
- Modify: `kala/cli.py:151-177`
- Modify: `kala/ui/desktop.py:909-936`
- Test: `tests/agent/test_part_runner.py` (CLI smoke via subprocess optional) + manual desktop checklist in commit message

**Interfaces:**
- Consumes: `execute_part_plan_enabled()`, `Agent.run` execute path
- Produces: CLI exit 0 on `done`; desktop rail `running` → `done` with part list; still exit 3 / rail `part_plan` when flag off

- [ ] **Step 1: Write the failing test (CLI behavior via Agent already covered; add flag integration)**

```python
def test_agent_execute_path_not_part_plan_status(monkeypatch):
    monkeypatch.setenv("KALA_EXECUTE_PART_PLAN", "1")
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=16)
    result = agent.run("multi-part housing and cover assembly")
    assert result.state.status != "part_plan"
    assert result.state.part_plan is not None  # BOM retained on state
```

- [ ] **Step 2: Run test to verify fail/pass against current wire-up**

Run: `.venv/bin/pytest tests/agent/test_part_runner.py::test_agent_execute_path_not_part_plan_status -v`

- [ ] **Step 3: CLI + desktop minimal changes**

CLI: remove the hard early `SystemExit(3)` for `PartPlan` when `execute_part_plan_enabled()`; fall through to `Agent(...).run(args.goal)` so the loop+runner handle it. Keep exit 3 for `ClarifyNeeded`.

Desktop `_send`: when `PartPlan` and flag on, show BOM sketch lines **then** start `RunWorker` (do not `return` early). When flag off, keep current early return UI.

```python
# desktop sketch
        if isinstance(gate, PartPlan):
            # ... existing part_bits / lines ...
            self._add(self._agent_msg("\n".join(lines)))
            from kala.agent.part_runner import execute_part_plan_enabled
            if not execute_part_plan_enabled():
                self._rail_set("part_plan", export="—", tools="—", sync="—")
                return
            # else fall through to worker start (same as single-part path)
```

- [ ] **Step 4: Run regression**

Run: `.venv/bin/pytest tests/agent/test_decompose_gate.py tests/agent/test_part_runner.py tests/procedures/test_gate.py -v`
Expected: PASS; default flag-off still `part_plan`.

- [ ] **Step 5: Commit**

```bash
git add kala/cli.py kala/ui/desktop.py tests/agent/test_part_runner.py
git commit -m "feat(ui,cli): execute PartPlan when KALA_EXECUTE_PART_PLAN=1"
```

---

### Task 6: Eval smoke + docs note + non-goals lock

**Files:**
- Create: `tests/agent/test_part_runner_eval_smoke.py` (mock-only)
- Modify: `org/drafts/per-part-runner-cto-plan.md` only if implementers discover API skew (this file); **no** `kala/` drive-by

**Interfaces:**
- Consumes: `assess_goal("planetary gearbox…")` → PartPlan → runner with flag on
- Produces: smoke test that does not require FreeCAD/API key (`StubPlanner` + mock)

- [ ] **Step 1: Write failing smoke test**

```python
def test_gearbox_brief_execute_smoke(monkeypatch):
    monkeypatch.setenv("KALA_EXECUTE_PART_PLAN", "1")
    from kala.procedures.gate import assess_goal, PartPlan
    g = assess_goal("planetary gearbox with sun and planets")
    assert isinstance(g, PartPlan)
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=20, standard_parts=True)
    result = agent.run("planetary gearbox with sun and planets")
    assert result.state.status in {"done", "needs_clarify", "error"}
    assert result.state.status != "part_plan"
    if result.state.status == "done":
        assert result.state.part_body_map
        assert result.state.last_export
```

- [ ] **Step 2–4: Implement any glue gaps, run full agent+procedures suite, commit**

```bash
.venv/bin/pytest tests/agent/ tests/procedures/ -v
git add tests/agent/test_part_runner_eval_smoke.py
git commit -m "test(agent): gearbox PartPlan execute smoke on mock backend"
```

---

## Non-goals (P2 — do not implement in this plan)

- Mating / constraint solver (align faces, gears mesh kinematics).
- Per-part FEA / CalculiX as part of BOM runner.
- Authoring new procedure JSON files without Founder approval.
- Turning `keep_separate=False` into whole-assembly single brick (forbidden by CTO assembly rule).
- Auto-enabling `KALA_EXECUTE_PART_PLAN` by default before mock+FreeCAD eval bars pass.

## Patch order (CTO style)

1. Task 1 — stub + opt-in (safe default)
2. Task 2 — bind without invented ids
3. Task 3 — shared backend + frozen aliases
4. Task 4 — keep_separate export
5. Task 5 — CLI/desktop wire
6. Task 6 — smoke

## Re-run

```bash
KALA_EXECUTE_PART_PLAN=0 .venv/bin/pytest tests/agent/test_decompose_gate.py tests/procedures/test_gate.py -v
KALA_EXECUTE_PART_PLAN=1 .venv/bin/pytest tests/agent/test_part_runner.py tests/agent/test_part_runner_eval_smoke.py -v
# FreeCAD (optional, after mock green):
KALA_EXECUTE_PART_PLAN=1 .venv/bin/kala run -b freecad "multi-part housing and cover assembly" --json
```

## Self-review

1. **Spec coverage:** PartSpec consume ✓, keep_separate ✓, frozen ids via id_aliases ✓, no invented procedure/part ids ✓, CLI/desktop ✓, TDD ✓, P2 non-goals ✓.
2. **Placeholders:** No TBD/TODO; code blocks are concrete; catalog method name must be verified against `PartsCatalog` at implement time (called out explicitly).
3. **Type consistency:** `PartPlanRunner.run(goal, plan) -> PartRunResult`; `freeze_part_alias(state, local_name, body_id)`; alias key `part:<local_name>`; env `KALA_EXECUTE_PART_PLAN`.

---

*Draft for Founder/Eng Lead. Planning artifact only — product code lands in implement PRs on a feature branch, not this docs PR.*
