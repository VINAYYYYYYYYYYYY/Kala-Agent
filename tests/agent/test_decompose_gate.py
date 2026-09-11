"""Agent refuses silent done on product-like briefs."""

from __future__ import annotations

from kala.agent.loop import Agent
from kala.llm.stub import StubPlanner


def test_laptop_does_not_complete_as_done():
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=4)
    result = agent.run("16 inch laptop")
    assert result.state.status == "needs_clarify"
    assert result.state.status != "done"
    assert result.state.clarify is not None
    assert not result.state.history  # never entered modeling loop


def test_l_bracket_still_runs():
    agent = Agent(backend_name="mock", planner=StubPlanner(), max_turns=8)
    result = agent.run("L-bracket base 60x40x4 and vertical wall, fuse, export")
    assert result.state.status != "needs_clarify"
    assert result.state.status != "part_plan"
