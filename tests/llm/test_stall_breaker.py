"""Unit tests for search_parts and list_bodies stall-breaker policies."""

from __future__ import annotations

from unittest.mock import Mock

import pytest

from kala.llm.base import PlannerTurn, ToolCall
from kala.llm.openai_compat import OpenAICompatPlanner
from kala.llm.providers import ProviderConfig
from kala.llm.stub import StubPlanner
from kala.ml.base import DynamicContext
from kala.procedures.schema import Procedure, ProcedureStep
from kala.session.state import SessionState, ToolEvent


def make_test_procedure() -> Procedure:
    """Create a minimal test procedure with standard_parts step."""
    return Procedure(
        id="test_proc",
        name="Test Procedure",
        description="Test",
        steps=[
            ProcedureStep(
                id="envelope",
                goal="Create envelope",
                allowed_tools=["create_box", "create_cylinder"],
                optional_parts=False,
                exit_criteria="Envelope created",
            ),
            ProcedureStep(
                id="features",
                goal="Add features",
                allowed_tools=["boolean_fuse", "boolean_cut", "fillet", "list_bodies"],
                optional_parts=False,
                exit_criteria="Features added",
            ),
            ProcedureStep(
                id="standard_parts",
                goal="Add standard parts",
                allowed_tools=["search_parts", "insert_part"],
                optional_parts=True,
                exit_criteria="Parts added",
            ),
            ProcedureStep(
                id="export",
                goal="Export model",
                allowed_tools=["export"],
                optional_parts=False,
                exit_criteria="Model exported",
            ),
        ],
    )


def make_test_state(
    goal: str = "Test goal",
    standard_parts: bool = True,
    step_index: int = 0,
    history: list[ToolEvent] | None = None,
) -> SessionState:
    """Create a test session state."""
    return SessionState(
        goal=goal,
        backend_name="freecad",
        standard_parts=standard_parts,
        procedure=make_test_procedure(),
        step_index=step_index,
        history=history or [],
    )


def make_test_config() -> ProviderConfig:
    """Create a test provider config."""
    return ProviderConfig(
        id="test-id",
        name="Test Provider",
        provider="test",
        model="test",
        api_key="test",
        base_url="http://test",
        enabled=True,
    )


class TestSearchPartsStallBreaker:
    """Tests for search_parts stall-breaker policy."""

    def test_history_with_one_ok_search_then_llm_proposes_search_forces_insert(self) -> None:
        """history [search ok] + LLM proposes search → insert_part with fixture catalog id."""
        # Setup: history with 1 ok search_parts
        history = [
            ToolEvent(
                tool="search_parts",
                args={"query": "bearing"},
                ok=True,
                message="",
                data={"parts": [{"part_id": "bearing_6709", "name": "Bearing 6709"}]},
            )
        ]
        state = make_test_state(step_index=2, history=history, standard_parts=True)

        # Mock planner
        config = make_test_config()
        fallback = StubPlanner()
        planner = OpenAICompatPlanner(config, fallback=fallback)

        # Mock _propose_llm to return a turn with search_parts
        llm_turn = PlannerTurn(
            thought="Searching for more parts",
            calls=[ToolCall("search_parts", {"query": "gear"})],
            advance_step=False,
            done=False,
        )
        planner._propose_llm = Mock(return_value=llm_turn)  # type: ignore

        # Act
        turn = planner.propose(state, DynamicContext(focus="test"), [])

        # Assert: should force insert_part with bearing_6709
        assert len(turn.calls) == 1
        assert turn.calls[0].name == "insert_part"
        assert turn.calls[0].arguments["part_id"] == "bearing_6709"
        assert turn.calls[0].arguments["x"] == 0
        assert turn.calls[0].arguments["y"] == 0
        assert turn.calls[0].arguments["z"] == 0
        assert "bearing_6709" in turn.thought
        assert turn.done is False

    def test_empty_search_no_invented_part_id(self) -> None:
        """empty search → stub fallback, never invent part_id."""
        # Setup: history with empty ok search_parts
        history = [
            ToolEvent(
                tool="search_parts",
                args={"query": "nonexistent"},
                ok=True,
                message="",
                data={"parts": []},  # Empty results
            )
        ]
        state = make_test_state(step_index=2, history=history, standard_parts=True)

        config = make_test_config()
        fallback = StubPlanner()
        planner = OpenAICompatPlanner(config, fallback=fallback)

        # Mock _propose_llm to return a turn with search_parts
        llm_turn = PlannerTurn(
            thought="Searching again",
            calls=[ToolCall("search_parts", {"query": "another"})],
            advance_step=False,
            done=False,
        )
        planner._propose_llm = Mock(return_value=llm_turn)  # type: ignore

        # Act
        turn = planner.propose(state, DynamicContext(focus="test"), [])

        # Assert: should NOT have insert_part, should fallback to stub
        assert all(c.name != "insert_part" for c in turn.calls)
        # Should not invent part_id
        for call in turn.calls:
            if call.name == "insert_part":
                assert False, "Should not insert_part with empty search"

    def test_standard_parts_false_no_forced_insert(self) -> None:
        """standard_parts=False → do not force insert; stub/skip instead."""
        # Setup: history with ok search_parts but standard_parts=False
        history = [
            ToolEvent(
                tool="search_parts",
                args={"query": "bearing"},
                ok=True,
                message="",
                data={"parts": [{"part_id": "bearing_6709", "name": "Bearing 6709"}]},
            )
        ]
        state = make_test_state(step_index=2, history=history, standard_parts=False)

        config = make_test_config()
        fallback = StubPlanner()
        planner = OpenAICompatPlanner(config, fallback=fallback)

        # Mock _propose_llm to return a turn with search_parts
        llm_turn = PlannerTurn(
            thought="Searching for parts",
            calls=[ToolCall("search_parts", {"query": "gear"})],
            advance_step=False,
            done=False,
        )
        planner._propose_llm = Mock(return_value=llm_turn)  # type: ignore

        # Act
        turn = planner.propose(state, DynamicContext(focus="test"), [])

        # Assert: should NOT force insert_part
        assert all(c.name != "insert_part" for c in turn.calls)

    def test_search_then_insert_then_search_allowed(self) -> None:
        """After ok search + ok insert, another search is allowed."""
        # Setup: history with search → insert cycle
        history = [
            ToolEvent(
                tool="search_parts",
                args={"query": "bearing"},
                ok=True,
                message="",
                data={"parts": [{"part_id": "bearing_6709", "name": "Bearing 6709"}]},
            ),
            ToolEvent(
                tool="insert_part",
                args={"part_id": "bearing_6709", "x": 0, "y": 0, "z": 0},
                ok=True,
                message="",
                data={"body_id": "Part001"},
            ),
        ]
        state = make_test_state(step_index=2, history=history, standard_parts=True)

        config = make_test_config()
        fallback = StubPlanner()
        planner = OpenAICompatPlanner(config, fallback=fallback)

        # Mock _propose_llm to return a turn with search_parts
        llm_turn = PlannerTurn(
            thought="Searching for more parts",
            calls=[ToolCall("search_parts", {"query": "gear"})],
            advance_step=False,
            done=False,
        )
        planner._propose_llm = Mock(return_value=llm_turn)  # type: ignore

        # Act
        turn = planner.propose(state, DynamicContext(focus="test"), [])

        # Assert: search should be allowed since we had insert after last search
        assert turn.calls[0].name == "search_parts"


class TestListBodiesStallBreaker:
    """Tests for list_bodies stall-breaker policy."""

    def test_two_consecutive_ok_lists_only_stub_progress(self) -> None:
        """2× list_bodies only → stub progress, done=False."""
        # Setup: history with 2 ok list_bodies + known bodies
        history = [
            ToolEvent(
                tool="create_box",
                args={"length": 10, "width": 10, "height": 10},
                ok=True,
                message="",
                data={"body_id": "Box001"},
            ),
            ToolEvent(
                tool="list_bodies",
                args={},
                ok=True,
                message="",
                data={"bodies": [{"body_id": "Box001"}]},
            ),
            ToolEvent(
                tool="list_bodies",
                args={},
                ok=True,
                message="",
                data={"bodies": [{"body_id": "Box001"}]},
            ),
        ]
        state = make_test_state(step_index=1, history=history, goal="test bracket with hole")

        config = make_test_config()
        fallback = StubPlanner()
        planner = OpenAICompatPlanner(config, fallback=fallback)

        # Mock _propose_llm to return a turn with list_bodies only
        llm_turn = PlannerTurn(
            thought="Listing bodies again",
            calls=[ToolCall("list_bodies", {})],
            advance_step=False,
            done=False,
        )
        planner._propose_llm = Mock(return_value=llm_turn)  # type: ignore

        # Act
        turn = planner.propose(state, DynamicContext(focus="test"), [])

        # Assert: should stub fallback with done=False
        assert turn.done is False
        # Should not have list_bodies in calls (sanitized out)
        assert all(c.name != "list_bodies" for c in turn.calls)

    def test_one_list_allowed_after_fuse_cut_remap(self) -> None:
        """One list allowed after fuse/cut remap."""
        # Setup: history with fuse (which remaps body_ids)
        history = [
            ToolEvent(
                tool="create_box",
                args={"length": 10, "width": 10, "height": 10},
                ok=True,
                message="",
                data={"body_id": "Box001"},
            ),
            ToolEvent(
                tool="create_box",
                args={"length": 5, "width": 5, "height": 5},
                ok=True,
                message="",
                data={"body_id": "Box002"},
            ),
            ToolEvent(
                tool="boolean_fuse",
                args={"body_a": "Box001", "body_b": "Box002"},
                ok=True,
                message="",
                data={"body_id": "Fuse_001", "removed": ["Box001", "Box002"]},
            ),
        ]
        state = make_test_state(step_index=1, history=history)

        config = make_test_config()
        fallback = StubPlanner()
        planner = OpenAICompatPlanner(config, fallback=fallback)

        # Mock _propose_llm to return a turn with list_bodies
        llm_turn = PlannerTurn(
            thought="Listing to see new body",
            calls=[ToolCall("list_bodies", {})],
            advance_step=False,
            done=False,
        )
        planner._propose_llm = Mock(return_value=llm_turn)  # type: ignore

        # Act
        turn = planner.propose(state, DynamicContext(focus="test"), [])

        # Assert: first list after fuse should be allowed
        assert any(c.name == "list_bodies" for c in turn.calls)

    def test_list_allowed_when_zero_bodies_known(self) -> None:
        """List allowed when zero bodies known."""
        # Setup: empty history, no bodies yet
        state = make_test_state(step_index=0, history=[])

        config = make_test_config()
        fallback = StubPlanner()
        planner = OpenAICompatPlanner(config, fallback=fallback)

        # Mock _propose_llm to return a turn with list_bodies
        llm_turn = PlannerTurn(
            thought="Checking initial state",
            calls=[ToolCall("list_bodies", {})],
            advance_step=False,
            done=False,
        )
        planner._propose_llm = Mock(return_value=llm_turn)  # type: ignore

        # Act
        turn = planner.propose(state, DynamicContext(focus="test"), [])

        # Assert: list should be allowed when no bodies exist yet
        assert any(c.name == "list_bodies" for c in turn.calls)

    def test_list_bodies_with_other_calls_not_blocked(self) -> None:
        """list_bodies mixed with other calls should only remove list if already had 1."""
        # Setup: history with 1 ok list_bodies + known bodies
        history = [
            ToolEvent(
                tool="create_box",
                args={"length": 10, "width": 10, "height": 10},
                ok=True,
                message="",
                data={"body_id": "Box001"},
            ),
            ToolEvent(
                tool="list_bodies",
                args={},
                ok=True,
                message="",
                data={"bodies": [{"body_id": "Box001"}]},
            ),
        ]
        state = make_test_state(step_index=1, history=history)

        config = make_test_config()
        fallback = StubPlanner()
        planner = OpenAICompatPlanner(config, fallback=fallback)

        # Mock _propose_llm to return a turn with list_bodies + fillet
        llm_turn = PlannerTurn(
            thought="Listing and filleting",
            calls=[ToolCall("list_bodies", {}), ToolCall("fillet", {"body_id": "Box001", "radius": 2})],
            advance_step=False,
            done=False,
        )
        planner._propose_llm = Mock(return_value=llm_turn)  # type: ignore

        # Act
        turn = planner.propose(state, DynamicContext(focus="test"), [])

        # Assert: list_bodies should be removed, but fillet should remain
        assert all(c.name != "list_bodies" for c in turn.calls)
        assert any(c.name == "fillet" for c in turn.calls)

    def test_one_list_with_needs_cut_still_calls_llm_soft_note(self) -> None:
        """consecutive_list==1 + needs_cut: soft note path — still call LLM, do not hard-force stub."""
        history = [
            ToolEvent(
                tool="create_box",
                args={"length": 10, "width": 10, "height": 10},
                ok=True,
                message="",
                data={"body_id": "Box001"},
            ),
            ToolEvent(
                tool="list_bodies",
                args={},
                ok=True,
                message="",
                data={"bodies": [{"body_id": "Box001"}]},
            ),
        ]
        # Goal with hole → needs_cut True
        state = make_test_state(step_index=1, history=history, goal="bracket with hole")

        config = make_test_config()
        fallback = StubPlanner()
        planner = OpenAICompatPlanner(config, fallback=fallback)

        llm_turn = PlannerTurn(
            thought="Cutting hole",
            calls=[ToolCall("boolean_cut", {"body_a": "Box001", "body_b": "Cyl001"})],
            advance_step=False,
            done=False,
        )
        mock_llm = Mock(return_value=llm_turn)
        planner._propose_llm = mock_llm  # type: ignore

        turn = planner.propose(state, DynamicContext(focus="test"), [])

        mock_llm.assert_called_once()
        assert turn.calls[0].name == "boolean_cut"
        assert "LLM stuck on list_bodies" not in turn.thought

    def test_two_lists_with_needs_cut_hard_forces_stub(self) -> None:
        """consecutive_list>=2 + needs_cut: hard force stub before LLM."""
        history = [
            ToolEvent(
                tool="create_box",
                args={"length": 10, "width": 10, "height": 10},
                ok=True,
                message="",
                data={"body_id": "Box001"},
            ),
            ToolEvent(
                tool="list_bodies",
                args={},
                ok=True,
                message="",
                data={"bodies": [{"body_id": "Box001"}]},
            ),
            ToolEvent(
                tool="list_bodies",
                args={},
                ok=True,
                message="",
                data={"bodies": [{"body_id": "Box001"}]},
            ),
        ]
        state = make_test_state(step_index=1, history=history, goal="bracket with hole")

        config = make_test_config()
        fallback = StubPlanner()
        planner = OpenAICompatPlanner(config, fallback=fallback)

        mock_llm = Mock(
            return_value=PlannerTurn(
                thought="should not be used",
                calls=[ToolCall("list_bodies", {})],
                advance_step=False,
                done=False,
            )
        )
        planner._propose_llm = mock_llm  # type: ignore

        turn = planner.propose(state, DynamicContext(focus="test"), [])

        mock_llm.assert_not_called()
        assert "LLM stuck on list_bodies" in turn.thought
        assert turn.done is False

    def test_soft_note_injected_into_user_prompt(self) -> None:
        """At consecutive_list==1 with bodies known, user prompt includes Eng soft note."""
        import json
        import urllib.request
        from unittest.mock import patch, MagicMock

        history = [
            ToolEvent(
                tool="create_box",
                args={"length": 10, "width": 10, "height": 10},
                ok=True,
                message="",
                data={"body_id": "Box001"},
            ),
            ToolEvent(
                tool="list_bodies",
                args={},
                ok=True,
                message="",
                data={"bodies": [{"body_id": "Box001"}]},
            ),
        ]
        state = make_test_state(step_index=1, history=history, goal="simple plate")

        config = make_test_config()
        planner = OpenAICompatPlanner(config, fallback=StubPlanner())

        captured: dict = {}

        class FakeResp:
            def __enter__(self):
                return self
            def __exit__(self, *a):
                return False
            def read(self):
                return json.dumps({
                    "choices": [{
                        "message": {
                            "content": "ok",
                            "tool_calls": [{
                                "function": {
                                    "name": "fillet",
                                    "arguments": json.dumps({"body_id": "Box001", "radius": 1}),
                                }
                            }],
                        }
                    }],
                    "usage": {},
                }).encode()

        def fake_urlopen(req, timeout=90):
            body = json.loads(req.data.decode())
            captured["user"] = body["messages"][1]["content"]
            return FakeResp()

        with patch.object(urllib.request, "urlopen", side_effect=fake_urlopen):
            turn = planner._propose_llm(state, DynamicContext(focus="test"), [])

        assert "SOFT NOTE" in captured["user"]
        assert "Do NOT call list_bodies again" in captured["user"]
        assert turn.calls[0].name == "fillet"
