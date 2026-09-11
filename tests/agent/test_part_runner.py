"""Per-part runner consumes PartPlan.parts sequentially."""

from __future__ import annotations

from kala.agent.loop import (
    Agent,
    _refresh_frozen_part_aliases,
    _resolve_alias,
    freeze_part_alias,
    _skip_silent_fuse,
    library_procedure_id,
)
from kala.llm.stub import StubPlanner
from kala.ml.base import DynamicContext
from kala.ml.stub import StubDesignContextModel
from kala.procedures import ClarifyNeeded, PartPlan, assess_goal
from kala.procedures.schema import list_procedure_ids, load_default_procedure
from kala.session.state import SessionState


_KNOWN = set(list_procedure_ids())
_GEARBOX_BINDABLE = "planetary gearbox with shaft and housing"


class _CountingContextModel:
    def __init__(self) -> None:
        self.calls = 0
        self._inner = StubDesignContextModel()

    def enrich(self, state: SessionState) -> DynamicContext:
        self.calls += 1
        return self._inner.enrich(state)


def test_library_procedure_id_never_invents():
    assert library_procedure_id(None) is None
    assert library_procedure_id("invented_gear_mesh_v1") is None
    assert library_procedure_id("stepped_shaft") == "stepped_shaft"
    assert library_procedure_id("stepped_shaft") in _KNOWN


def test_laptop_still_clarify_no_modeling():
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=8)
    result = agent.run("16 inch laptop")
    assert result.state.status == "needs_clarify"
    assert result.state.clarify is not None
    assert not result.state.history
    assert not result.state.part_runs
    assert isinstance(assess_goal("16 inch laptop"), ClarifyNeeded)


def test_bare_gearbox_without_library_parts_clarifies():
    """assess_goal unchanged: gear-only BOM has no library id → clarify, no invent."""
    goal = "planetary gearbox with sun and planets"
    gate = assess_goal(goal)
    assert isinstance(gate, PartPlan)
    assert all(p.procedure_id is None or p.procedure_id in _KNOWN for p in gate.parts)
    assert not any(p.procedure_id in _KNOWN for p in gate.parts)

    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=8)
    result = agent.run(goal)
    assert result.state.status == "needs_clarify"
    assert result.state.part_plan is not None
    assert not any(e.ok and e.tool.startswith("create_") for e in result.state.history)


def test_gearbox_executes_bindable_parts():
    gate = assess_goal(_GEARBOX_BINDABLE)
    assert isinstance(gate, PartPlan)
    bindable = [p for p in gate.parts if p.procedure_id in _KNOWN]
    assert bindable

    ctx = _CountingContextModel()
    agent = Agent(
        backend_name="mock",
        planner=StubPlanner(),
        context_model=ctx,
        max_turns=32,
    )
    result = agent.run(_GEARBOX_BINDABLE)
    st = result.state
    assert st.status == "done"
    assert st.part_plan is not None
    assert st.history, "playbook must run for bindable parts"
    assert ctx.calls == 0, "part sub-runs must not call DesignContextModel.enrich"

    executed = [
        r
        for r in st.part_runs
        if r.get("procedure_id") in _KNOWN and r.get("status") in {"done", "max_turns"}
    ]
    assert executed, "≥1 part with existing procedure_id must run"
    assert {r["procedure_id"] for r in executed} <= _KNOWN

    skipped = [r for r in st.part_runs if r.get("status") == "skipped"]
    assert any(r.get("local_name") == "gear" for r in skipped)
    assert any(e.tool == "part_skip" for e in st.history)
    assert any(e.tool == "part_bind" for e in st.history)
    assert any(e.ok and e.tool.startswith("create_") for e in st.history)


def test_keep_separate_does_not_fuse_all_parts():
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=32)
    result = agent.run(_GEARBOX_BINDABLE)
    st = result.state
    assert any(p.get("keep_separate") for p in (st.part_plan or {}).get("parts") or [])

    bodies = getattr(agent._backend, "_bodies", {})
    executed_parts = [
        r["local_name"]
        for r in st.part_runs
        if r.get("procedure_id") and r.get("status") in {"done", "max_turns"}
    ]
    if len(executed_parts) >= 2:
        assert len(bodies) >= 2, "keep_separate must leave distinct solids, not one fused brick"

    all_exports = [
        e
        for e in st.history
        if e.ok and e.tool == "export" and str(e.args.get("body_id", "")).upper() == "ALL"
    ]
    assert all_exports, "keep_separate assembly must export body_id=ALL"


def test_all_null_procedure_ids_clarify():
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=8)
    result = agent.run("multi-part assembly of unnamed widgets")
    assert result.state.status == "needs_clarify"
    assert result.state.part_plan is not None
    assert not any(e.tool.startswith("create_") for e in result.state.history)
    assert all(r.get("status") == "skipped" for r in result.state.part_runs)


def test_l_bracket_still_enriches_single_playbook():
    ctx = _CountingContextModel()
    agent = Agent(
        backend_name="mock",
        planner=StubPlanner(),
        context_model=ctx,
        max_turns=16,
    )
    result = agent.run("L-bracket base 60x40x4 and vertical wall, fuse, export")
    assert result.state.status != "needs_clarify"
    assert result.state.part_plan is None
    assert result.state.part_runs == []
    assert ctx.calls > 0


def test_freeze_part_alias_after_bindable_part():
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=32)
    result = agent.run(_GEARBOX_BINDABLE)
    st = result.state
    for r in st.part_runs:
        if r.get("status") in {"done", "max_turns"} and r.get("procedure_id"):
            name = r["local_name"]
            assert name in st.part_body_map
            assert st.id_aliases.get(f"part:{name}") == st.part_body_map[name]
            assert _resolve_alias(st, f"part:{name}") == st.part_body_map[name]
    data = st.to_dict()
    assert "part_body_map" in data
    assert data["part_body_map"] == st.part_body_map


def test_frozen_part_alias_follows_child_id_alias_remaps():
    proc = load_default_procedure("simple_bracket")
    state = SessionState(
        goal="t",
        backend_name="mock",
        standard_parts=False,
        procedure=proc,
    )
    freeze_part_alias(state, "bracket", "Box_1")
    state.id_aliases["Box_1"] = "Fuse_2"
    _refresh_frozen_part_aliases(state)
    assert state.part_body_map["bracket"] == "Fuse_2"
    assert state.id_aliases["part:bracket"] == "Fuse_2"
    assert _resolve_alias(state, "part:bracket") == "Fuse_2"


def test_skip_silent_fuse_only_when_keep_separate():
    proc = load_default_procedure("simple_bracket")
    keep = SessionState(
        goal="t",
        backend_name="mock",
        standard_parts=False,
        procedure=proc,
        part_plan={"parts": [{"keep_separate": True}]},
    )
    merge = SessionState(
        goal="t",
        backend_name="mock",
        standard_parts=False,
        procedure=proc,
        part_plan={"parts": [{"keep_separate": False}]},
    )
    assert _skip_silent_fuse(keep) is True
    assert _skip_silent_fuse(merge) is False
