"""Per-part runner consumes PartPlan.parts sequentially."""

from __future__ import annotations

import json
from pathlib import Path

from kala.agent.loop import (
    Agent,
    _procedure_step_context,
    _refresh_frozen_part_aliases,
    _resolve_alias,
    _strip_boolean_fuse_from_allowlist,
    freeze_part_alias,
    _skip_silent_fuse,
    library_procedure_id,
)
from kala.cad.factory import create_backend
from kala.cad.mock import MockBackend
from kala.cad.protocol import ToolResult
from kala.cad.registry import build_registry
from kala.llm.base import PlannerTurn, ToolCall
from kala.llm.stub import StubPlanner
from kala.ml.base import DynamicContext
from kala.ml.stub import StubDesignContextModel
from kala.procedures import ClarifyNeeded, PartPlan, PartSpec, assess_goal
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


def test_gear_only_bom_catalog_binds_without_procedure_id():
    """assess_goal unchanged: gear-only BOM has no library id but catalog alias may bind."""
    goal = "planetary gearbox with sun and planets"
    gate = assess_goal(goal)
    assert isinstance(gate, PartPlan)
    assert all(p.procedure_id is None or p.procedure_id in _KNOWN for p in gate.parts)
    assert not any(p.procedure_id in _KNOWN for p in gate.parts)

    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=8)
    result = agent.run(goal)
    st = result.state
    assert st.status == "done"
    assert st.part_plan is not None
    gear_runs = [r for r in st.part_runs if r.get("local_name") == "gear"]
    assert gear_runs
    assert gear_runs[0].get("status") == "catalog"
    assert gear_runs[0].get("catalog_part_id") == "gear_planet"
    assert gear_runs[0].get("procedure_id") is None


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

    catalog_runs = [r for r in st.part_runs if r.get("status") == "catalog"]
    assert any(r.get("local_name") == "gear" for r in catalog_runs)
    assert any(r.get("catalog_part_id") == "gear_planet" for r in catalog_runs)
    assert any(e.tool == "part_catalog" for e in st.history)
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


def test_catalog_bind_null_procedure_id_motor():
    """Null procedure_id + catalog alias hit inserts standard part instead of skip."""
    goal = "NEMA17 planetary gearbox with shaft and housing"
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=32)
    result = agent.run(goal)
    st = result.state
    assert st.status == "done"

    motor_runs = [r for r in st.part_runs if r.get("local_name") == "motor"]
    assert motor_runs
    motor = motor_runs[0]
    assert motor.get("procedure_id") is None
    assert motor.get("status") == "catalog"
    assert motor.get("catalog_part_id") == "nema17_body"

    assert any(e.tool == "part_catalog" for e in st.history)
    assert any(e.ok and e.tool == "insert_part" for e in st.history)
    assert "motor" in st.part_body_map
    assert st.id_aliases.get("part:motor") == st.part_body_map["motor"]


def test_catalog_bind_bearing_608_from_brief_token():
    """Brief token 608 resolves to bearing_608 and runs via catalog, not skip."""
    plan = PartPlan(
        parts=[
            PartSpec(
                local_name="bearing",
                brief="608 ball bearing skate",
                procedure_id=None,
                keep_separate=True,
            ),
            PartSpec(
                local_name="shaft",
                brief="shaft",
                procedure_id="stepped_shaft",
                keep_separate=True,
            ),
        ]
    )
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=32)
    result = agent._run_part_plan("bearing assembly with shaft", plan, [])
    st = result.state

    bearing_runs = [r for r in st.part_runs if r.get("local_name") == "bearing"]
    assert bearing_runs
    bearing = bearing_runs[0]
    assert bearing.get("procedure_id") is None
    assert bearing.get("status") == "catalog"
    assert bearing.get("catalog_part_id") == "bearing_608"
    assert "bearing" in st.part_body_map


def test_ambiguous_bearing_still_skips_without_catalog_hit():
    goal = "planetary gearbox with shaft and housing and bearing"
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=32)
    result = agent.run(goal)
    bearing_runs = [r for r in result.state.part_runs if r.get("local_name") == "bearing"]
    assert bearing_runs
    assert bearing_runs[0].get("status") == "skipped"
    assert bearing_runs[0].get("catalog_part_id") is None


class _FuseBracketPlanner:
    """Deterministic simple_bracket run: two boxes → fuse → export."""

    def propose(self, state: SessionState, context: DynamicContext, tool_schemas: list) -> PlannerTurn:
        step = state.current_step
        if step is None:
            return PlannerTurn(thought="done", done=True)
        allowed = {s["name"] for s in tool_schemas}
        if step.id == "envelope":
            if not any(e.ok and e.tool == "create_box" for e in state.history):
                return PlannerTurn(
                    thought="envelope box",
                    calls=[ToolCall("create_box", {"length": 60.0, "width": 40.0, "height": 4.0})],
                    advance_step=True,
                )
            return PlannerTurn(thought="envelope done", advance_step=True)
        if step.id == "features":
            creates = [e for e in state.history if e.ok and e.tool == "create_box"]
            if len(creates) < 2 and "create_box" in allowed:
                return PlannerTurn(
                    thought="wall box",
                    calls=[ToolCall("create_box", {"length": 60.0, "width": 4.0, "height": 50.0})],
                )
            if (
                len(creates) >= 2
                and "boolean_fuse" in allowed
                and not any(e.ok and e.tool == "boolean_fuse" for e in state.history)
            ):
                a = str(creates[0].data["body_id"])
                b = str(creates[1].data["body_id"])
                return PlannerTurn(
                    thought="fuse bracket",
                    calls=[ToolCall("boolean_fuse", {"body_a": a, "body_b": b})],
                    advance_step=True,
                )
            return PlannerTurn(thought="features done", advance_step=True)
        if step.id == "standard_parts":
            return PlannerTurn(thought="skip standards", advance_step=True)
        if step.id == "export":
            if any(e.ok and e.tool == "export" for e in state.history):
                return PlannerTurn(thought="exported", done=True)
            fuse = next(
                (e for e in reversed(state.history) if e.ok and e.tool == "boolean_fuse"),
                None,
            )
            bid = fuse.data.get("body_id") if fuse else None
            if bid and "export" in allowed:
                return PlannerTurn(
                    thought="export fused bracket",
                    calls=[
                        ToolCall(
                            "export",
                            {"body_id": str(bid), "path": "outputs/bracket_fused.step", "fmt": "step"},
                        )
                    ],
                    advance_step=True,
                    done=True,
                )
            return PlannerTurn(thought="nothing to export", done=True)
        return PlannerTurn(thought=f"skip {step.id}", advance_step=True)


def test_keep_separate_false_bracket_alias_after_fuse():
    """keep_separate=False + simple_bracket: part:bracket tracks live fused id."""
    plan = PartPlan(
        parts=[
            PartSpec(
                local_name="bracket",
                brief="L-bracket fuse two boxes",
                procedure_id="simple_bracket",
                keep_separate=False,
            ),
        ]
    )
    agent = Agent(
        backend_name="mock",
        planner=_FuseBracketPlanner(),
        max_turns=32,
    )
    result = agent._run_part_plan("fused bracket part", plan, [])
    st = result.state
    assert st.status == "done"

    fuse_events = [e for e in st.history if e.ok and e.tool == "boolean_fuse"]
    assert fuse_events, "simple_bracket part must fuse two solids"
    fused_id = str(fuse_events[-1].data["body_id"])
    removed = {str(r) for r in fuse_events[-1].data.get("removed") or []}

    assert "bracket" in st.part_body_map
    assert st.part_body_map["bracket"] == fused_id
    assert st.part_body_map["bracket"] not in removed
    assert st.id_aliases.get("part:bracket") == fused_id
    assert _resolve_alias(st, "part:bracket") == fused_id


class _NoAllExportBackend(MockBackend):
    """Mock backend that rejects assembly ALL export but supports per-body STEP."""

    def export(self, body_id: str, path: str, fmt: str = "step") -> ToolResult:
        if str(body_id).upper() in {"*", "ALL", "__ALL__", "ASSEMBLY"}:
            return ToolResult(ok=False, message="[mock] ALL export not supported")
        return super().export(body_id, path, fmt)


def test_keep_separate_falls_back_to_per_part_manifest_when_all_export_fails(
    monkeypatch, tmp_path
):
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(
        "kala.agent.loop.create_backend",
        lambda _name: _NoAllExportBackend(),
    )

    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=32)
    result = agent.run(_GEARBOX_BINDABLE)
    st = result.state
    assert st.status == "done"
    assert st.part_body_map

    all_attempts = [
        e
        for e in st.history
        if e.tool == "export" and str(e.args.get("body_id", "")).upper() == "ALL"
    ]
    assert all_attempts, "keep_separate must attempt body_id=ALL first"
    assert not any(e.ok for e in all_attempts)

    for local_name in st.part_body_map:
        part_exports = [
            e
            for e in st.history
            if e.ok
            and e.tool == "export"
            and e.args.get("path") == f"outputs/parts/{local_name}.step"
        ]
        assert part_exports, f"missing per-part export for {local_name}"
        assert Path(f"outputs/parts/{local_name}.step").is_file()

    assert st.last_export == "outputs/assembly_manifest.json"
    manifest = json.loads(Path("outputs/assembly_manifest.json").read_text(encoding="utf-8"))
    assert manifest.get("kind") == "assembly_manifest"
    manifest_names = {p["local_name"] for p in manifest.get("parts") or []}
    assert manifest_names == set(st.part_body_map)
    for entry in manifest.get("parts") or []:
        assert entry["step"] == f"outputs/parts/{entry['local_name']}.step"
        assert entry["body_id"] == st.part_body_map[entry["local_name"]]

    bodies = getattr(agent._backend, "_bodies", {})
    if len(st.part_body_map) >= 2:
        assert len(bodies) >= 2, "fallback must not fuse-all into one brick"


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


def test_keep_separate_assembly_strips_boolean_fuse_from_allowlist():
    """Cross-part assembly turns must not offer boolean_fuse to the planner."""
    proc = load_default_procedure("machine_assembly")
    state = SessionState(
        goal="gearbox assembly",
        backend_name="mock",
        standard_parts=False,
        procedure=proc,
        part_plan={
            "kind": "part_plan",
            "parts": [
                {"local_name": "housing", "keep_separate": True},
                {"local_name": "shaft", "keep_separate": True},
            ],
        },
    )
    features = proc.step(1)
    assert features is not None
    allowed = _strip_boolean_fuse_from_allowlist(state, list(features.allowed_tools))
    assert "boolean_fuse" not in allowed
    assert "boolean_cut" in allowed
    assert "export" in allowed

    state.step_index = 1
    ctx = _procedure_step_context(state)
    assert "boolean_fuse" not in ctx.recommended_tools
    assert "boolean_cut" in ctx.recommended_tools


def test_intra_part_playbook_keeps_boolean_fuse_even_when_keep_separate():
    """Single PartSpec child runs may fuse that part's own solids."""
    proc = load_default_procedure("simple_bracket")
    state = SessionState(
        goal="bracket",
        backend_name="mock",
        standard_parts=False,
        procedure=proc,
        part_plan={
            "kind": "part_plan",
            "parts": [
                {
                    "local_name": "bracket",
                    "brief": "L-bracket",
                    "procedure_id": "simple_bracket",
                    "keep_separate": True,
                },
            ],
        },
    )
    features = proc.step(1)
    assert features is not None
    allowed = _strip_boolean_fuse_from_allowlist(state, list(features.allowed_tools))
    assert "boolean_fuse" in allowed

    state.step_index = 1
    ctx = _procedure_step_context(state)
    assert "boolean_fuse" in ctx.recommended_tools


def test_keep_separate_false_single_part_keeps_boolean_fuse():
    proc = load_default_procedure("simple_bracket")
    state = SessionState(
        goal="fused bracket",
        backend_name="mock",
        standard_parts=False,
        procedure=proc,
        part_plan={
            "kind": "part_plan",
            "parts": [
                {
                    "local_name": "bracket",
                    "brief": "fuse two boxes",
                    "procedure_id": "simple_bracket",
                    "keep_separate": False,
                },
            ],
        },
    )
    features = proc.step(1)
    assert features is not None
    allowed = _strip_boolean_fuse_from_allowlist(state, list(features.allowed_tools))
    assert "boolean_fuse" in allowed

    state.step_index = 1
    ctx = _procedure_step_context(state)
    assert "boolean_fuse" in ctx.recommended_tools


class _FuseAllowlistSpy:
    """Planner that records schemas and tries boolean_fuse once."""

    def __init__(self) -> None:
        self.schema_names: list[set[str]] = []

    def propose(self, state: SessionState, context: DynamicContext, tool_schemas: list) -> PlannerTurn:
        self.schema_names.append({s["name"] for s in tool_schemas})
        if any(e.tool == "boolean_fuse" for e in state.history):
            return PlannerTurn(thought="fuse already attempted", done=True)
        return PlannerTurn(
            thought="try fuse",
            calls=[ToolCall("boolean_fuse", {"body_a": "Box_1", "body_b": "Box_2"})],
        )


def test_execute_playbook_blocks_boolean_fuse_on_keep_separate_assembly():
    """Schemas + playbook gate must drop boolean_fuse on cross-part keep_separate."""
    proc = load_default_procedure("machine_assembly")
    state = SessionState(
        goal="gearbox assembly",
        backend_name="mock",
        standard_parts=False,
        procedure=proc,
        step_index=1,
        part_plan={
            "kind": "part_plan",
            "parts": [
                {"local_name": "housing", "keep_separate": True},
                {"local_name": "shaft", "keep_separate": True},
            ],
        },
    )
    spy = _FuseAllowlistSpy()
    backend = create_backend("mock")
    registry = build_registry(backend)
    agent = Agent(backend_name="mock", planner=spy, max_turns=3)
    agent._backend = backend
    agent._execute_playbook(state, registry, [], enrich=False)

    assert spy.schema_names
    assert all("boolean_fuse" not in names for names in spy.schema_names)
    fuse_events = [e for e in state.history if e.tool == "boolean_fuse"]
    assert fuse_events
    assert all(not e.ok for e in fuse_events)
    assert all("blocked" in e.message for e in fuse_events)
    assert not any(e.ok and e.tool == "boolean_fuse" for e in state.history)


def test_execute_playbook_keeps_boolean_fuse_on_intra_part_keep_separate():
    """Child simple_bracket playbook still sees boolean_fuse in planner schemas."""
    proc = load_default_procedure("simple_bracket")
    state = SessionState(
        goal="bracket",
        backend_name="mock",
        standard_parts=False,
        procedure=proc,
        step_index=1,
        part_plan={
            "kind": "part_plan",
            "parts": [
                {
                    "local_name": "bracket",
                    "brief": "L-bracket",
                    "procedure_id": "simple_bracket",
                    "keep_separate": True,
                },
            ],
        },
    )
    spy = _FuseAllowlistSpy()
    backend = create_backend("mock")
    registry = build_registry(backend)
    agent = Agent(backend_name="mock", planner=spy, max_turns=2)
    agent._backend = backend
    agent._execute_playbook(state, registry, [], enrich=False)

    assert spy.schema_names
    assert any("boolean_fuse" in names for names in spy.schema_names)


def test_execute_playbook_keeps_boolean_fuse_when_keep_separate_false():
    """Single-part keep_separate=False runs still expose boolean_fuse."""
    proc = load_default_procedure("simple_bracket")
    state = SessionState(
        goal="fused bracket",
        backend_name="mock",
        standard_parts=False,
        procedure=proc,
        step_index=1,
        part_plan={
            "kind": "part_plan",
            "parts": [
                {
                    "local_name": "bracket",
                    "brief": "fuse two boxes",
                    "procedure_id": "simple_bracket",
                    "keep_separate": False,
                },
            ],
        },
    )
    spy = _FuseAllowlistSpy()
    backend = create_backend("mock")
    registry = build_registry(backend)
    agent = Agent(backend_name="mock", planner=spy, max_turns=2)
    agent._backend = backend
    agent._execute_playbook(state, registry, [], enrich=False)

    assert spy.schema_names
    assert any("boolean_fuse" in names for names in spy.schema_names)
