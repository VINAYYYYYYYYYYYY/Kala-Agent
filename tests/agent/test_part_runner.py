"""Per-part runner consumes PartPlan.parts sequentially."""

from __future__ import annotations

from kala.agent.loop import Agent, library_procedure_id
from kala.llm.stub import StubPlanner
from kala.procedures import ClarifyNeeded, PartPlan, assess_goal
from kala.procedures.schema import list_procedure_ids


_KNOWN = set(list_procedure_ids())


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
    assert assess_goal("16 inch laptop") is not None
    assert isinstance(assess_goal("16 inch laptop"), ClarifyNeeded)


def test_gearbox_executes_bindable_parts():
    goal = "planetary gearbox with sun and planets"
    gate = assess_goal(goal)
    assert isinstance(gate, PartPlan)
    bindable = [p for p in gate.parts if p.procedure_id in _KNOWN]
    assert bindable

    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=32)
    result = agent.run(goal)
    st = result.state
    assert st.status == "done"
    assert st.part_plan is not None
    assert st.part_plan.get("kind") == "part_plan"
    assert st.history, "playbook must run for bindable parts"

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
    result = agent.run("planetary gearbox with sun and planets")
    st = result.state
    assert any(p.get("keep_separate") for p in (st.part_plan or {}).get("parts") or [])

    fuse_by_part: dict[str, int] = {}
    for e in st.history:
        if e.ok and e.tool == "boolean_fuse":
            fuse_by_part[str(e.data.get("part"))] = fuse_by_part.get(str(e.data.get("part")), 0) + 1

    bind_events = [e for e in st.history if e.tool == "part_bind"]
    assert len(bind_events) >= 1

    bodies = getattr(agent._backend, "_bodies", {})
    executed_parts = [
        r["local_name"]
        for r in st.part_runs
        if r.get("procedure_id") and r.get("status") in {"done", "max_turns"}
    ]
    if len(executed_parts) >= 2:
        assert len(bodies) >= 2, "keep_separate must leave distinct solids, not one fused brick"


def test_all_null_procedure_ids_clarify():
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=8)
    result = agent.run("multi-part assembly of unnamed widgets")
    assert result.state.status == "needs_clarify"
    assert result.state.part_plan is not None
    assert not any(e.tool.startswith("create_") for e in result.state.history)
    assert all(r.get("status") == "skipped" for r in result.state.part_runs)


def test_l_bracket_unchanged_single_playbook():
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=16)
    result = agent.run("L-bracket base 60x40x4 and vertical wall, fuse, export")
    assert result.state.status != "needs_clarify"
    assert result.state.part_plan is None
    assert result.state.part_runs == []
