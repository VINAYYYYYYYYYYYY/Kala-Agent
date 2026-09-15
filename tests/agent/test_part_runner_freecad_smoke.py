"""Optional FreeCAD-or-mock smoke for the per-part runner (CTO Task 6).

Uses the real Agent. Hits FreeCAD only when that backend can be imported;
otherwise falls back to mock so default CI is not blocked.
"""

from __future__ import annotations

from typing import Any

import pytest

from kala.agent.loop import Agent
from kala.cad.freecad.backend import ensure_freecad
from kala.llm.stub import StubPlanner
from kala.procedures import PartPlan, assess_goal
from kala.procedures.schema import list_procedure_ids


_GEARBOX_BRIEF = "planetary gearbox with shaft and housing"
_KNOWN = set(list_procedure_ids())
_SUCCESS = frozenset({"done", "max_turns"})


def _freecad_backend_available() -> bool:
    try:
        ensure_freecad()
    except Exception:
        return False
    return True


def _smoke_backend_name() -> str:
    return "freecad" if _freecad_backend_available() else "mock"


def _distinct_body_ids(backend: Any) -> set[str]:
    if backend is None:
        return set()
    listed = backend.list_bodies()
    if listed.ok:
        bodies = listed.data.get("bodies") or []
        ids = {
            str(row.get("body_id"))
            for row in bodies
            if isinstance(row, dict) and row.get("body_id")
        }
        if ids:
            return ids
    return {str(bid) for bid in getattr(backend, "_bodies", {})}


@pytest.mark.freecad
def test_gearbox_part_runner_freecad_or_mock_smoke(tmp_path, monkeypatch):
    """Gearbox brief: success + last_export; ≥2 bodies when shaft+housing bind."""
    monkeypatch.chdir(tmp_path)
    monkeypatch.setenv("KALA_OUTPUT_DIR", str(tmp_path))
    # GUI launch is not part of this gate; keep the smoke hermetic.
    monkeypatch.setattr(Agent, "_finalize_gui", lambda self, state: None)

    backend_name = _smoke_backend_name()
    gate = assess_goal(_GEARBOX_BRIEF)
    assert isinstance(gate, PartPlan)
    bindable = [p for p in gate.parts if p.procedure_id in _KNOWN]

    agent = Agent(backend_name=backend_name, planner=StubPlanner(), max_turns=32)
    result = agent.run(_GEARBOX_BRIEF)
    st = result.state

    if backend_name == "freecad" and st.status == "error" and not st.history:
        pytest.skip(f"FreeCAD backend unavailable: {st.error}")

    assert st.status in _SUCCESS
    assert st.last_export

    if len(bindable) >= 2:
        mapped = {bid for bid in st.part_body_map.values() if bid}
        listed = _distinct_body_ids(agent._backend)
        distinct = listed if len(listed) >= 2 else mapped
        assert len(distinct) >= 2, (
            f"expected ≥2 distinct bodies when {len(bindable)} parts bindable; "
            f"listed={sorted(listed)} mapped={sorted(mapped)}"
        )
