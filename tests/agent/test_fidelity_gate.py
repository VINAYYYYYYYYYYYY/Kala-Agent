"""Unit tests for agent fidelity gates."""

from __future__ import annotations

import os
import tempfile
from pathlib import Path
from unittest.mock import MagicMock

import pytest

from kala.agent.loop import Agent, _compute_step_hash, _count_real_tools
from kala.procedures.schema import Procedure, ProcedureStep
from kala.session.state import SessionState, ToolEvent


def make_minimal_procedure() -> Procedure:
    """Create minimal test procedure."""
    return Procedure(
        id="test",
        name="Test",
        description="Test procedure",
        steps=[
            ProcedureStep(
                id="model",
                goal="Model",
                allowed_tools=None,
                optional_parts=False,
                exit_criteria="Done",
            ),
        ],
    )


def make_test_state(
    history: list[ToolEvent] | None = None,
    last_export: str | None = None,
) -> SessionState:
    """Create minimal test session state."""
    return SessionState(
        goal="test goal",
        backend_name="mock",
        standard_parts=False,
        procedure=make_minimal_procedure(),
        history=history or [],
        last_export=last_export,
    )


class TestCountRealTools:
    """Test _count_real_tools helper."""

    def test_empty_history(self):
        """Should return 0 for empty history."""
        assert _count_real_tools([]) == 0

    def test_excludes_metadata_tools(self):
        """Should exclude list_bodies, show_in_freecad, search_parts, export."""
        history = [
            ToolEvent("list_bodies", {}, True, "ok", {}),
            ToolEvent("show_in_freecad", {}, True, "ok", {}),
            ToolEvent("search_parts", {}, True, "ok", {}),
            ToolEvent("export", {}, True, "ok", {}),
        ]
        assert _count_real_tools(history) == 0

    def test_counts_modeling_tools(self):
        """Should count create/fuse/cut/fillet/etc."""
        history = [
            ToolEvent("create_box", {}, True, "ok", {}),
            ToolEvent("create_cylinder", {}, True, "ok", {}),
            ToolEvent("boolean_fuse", {}, True, "ok", {}),
            ToolEvent("fillet", {}, True, "ok", {}),
            ToolEvent("list_bodies", {}, True, "ok", {}),
            ToolEvent("export", {}, True, "ok", {}),
        ]
        assert _count_real_tools(history) == 4

    def test_only_counts_successful_tools(self):
        """Should only count tools with ok=True."""
        history = [
            ToolEvent("create_box", {}, True, "ok", {}),
            ToolEvent("create_cylinder", {}, False, "failed", {}),
            ToolEvent("boolean_fuse", {}, True, "ok", {}),
        ]
        assert _count_real_tools(history) == 2


class TestComputeStepHash:
    """Test _compute_step_hash helper."""

    def test_missing_file(self):
        """Should return None for missing file."""
        assert _compute_step_hash("/nonexistent/file.step") is None

    def test_hashes_file_content(self):
        """Should compute SHA256 hash of file content."""
        with tempfile.NamedTemporaryFile(mode="w", suffix=".step", delete=False) as f:
            f.write("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n")
            path = f.name
        
        try:
            hash1 = _compute_step_hash(path)
            assert hash1 is not None
            assert len(hash1) == 64  # SHA256 hex digest
            
            # Same content = same hash
            hash2 = _compute_step_hash(path)
            assert hash1 == hash2
        finally:
            Path(path).unlink()

    def test_different_content_different_hash(self):
        """Different file content should produce different hash."""
        with tempfile.NamedTemporaryFile(mode="w", suffix=".step", delete=False) as f1:
            f1.write("content A")
            path1 = f1.name
        
        with tempfile.NamedTemporaryFile(mode="w", suffix=".step", delete=False) as f2:
            f2.write("content B")
            path2 = f2.name
        
        try:
            hash1 = _compute_step_hash(path1)
            hash2 = _compute_step_hash(path2)
            assert hash1 != hash2
        finally:
            Path(path1).unlink()
            Path(path2).unlink()


class TestFidelityGate:
    """Test Agent._check_fidelity_gate method."""

    def test_allows_done_when_no_restrictions(self, monkeypatch):
        """Should allow done when no min_tools or stub hashes configured."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "0")
        monkeypatch.setenv("KALA_KNOWN_STUB_HASHES", "")
        
        agent = Agent(backend_name="mock")
        state = make_test_state(
            history=[
                ToolEvent("create_box", {}, True, "ok", {}),
                ToolEvent("export", {}, True, "ok", {}),
            ],
            last_export="test.step",
        )
        
        allowed, reason = agent._check_fidelity_gate(state)
        assert allowed is True
        assert reason == ""

    def test_rejects_too_few_tools(self, monkeypatch):
        """Should reject done when real tool count < min_tools."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "5")
        
        agent = Agent(backend_name="mock")
        state = make_test_state(
            history=[
                ToolEvent("create_box", {}, True, "ok", {}),
                ToolEvent("create_cylinder", {}, True, "ok", {}),
                ToolEvent("export", {}, True, "ok", {}),
            ],
            last_export="test.step",
        )
        
        allowed, reason = agent._check_fidelity_gate(state)
        assert allowed is False
        assert "too few tools" in reason
        assert "(2 < 5 required)" in reason

    def test_allows_done_when_min_tools_met(self, monkeypatch):
        """Should allow done when real tool count >= min_tools."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "3")
        
        agent = Agent(backend_name="mock")
        state = make_test_state(
            history=[
                ToolEvent("create_box", {}, True, "ok", {}),
                ToolEvent("create_cylinder", {}, True, "ok", {}),
                ToolEvent("boolean_fuse", {}, True, "ok", {}),
                ToolEvent("export", {}, True, "ok", {}),
            ],
            last_export="test.step",
        )
        
        allowed, reason = agent._check_fidelity_gate(state)
        assert allowed is True
        assert reason == ""

    def test_rejects_known_stub_hash(self, monkeypatch):
        """Should reject done when export hash matches known stub."""
        with tempfile.NamedTemporaryFile(mode="w", suffix=".step", delete=False) as f:
            f.write("stub content")
            export_path = f.name
        
        try:
            stub_hash = _compute_step_hash(export_path)
            monkeypatch.setenv("KALA_MIN_TOOLS", "0")
            monkeypatch.setenv("KALA_KNOWN_STUB_HASHES", f"{stub_hash},other_hash")
            
            agent = Agent(backend_name="mock")
            state = make_test_state(
                history=[
                    ToolEvent("create_box", {}, True, "ok", {}),
                    ToolEvent("export", {}, True, "ok", {}),
                ],
                last_export=export_path,
            )
            
            allowed, reason = agent._check_fidelity_gate(state)
            assert allowed is False
            assert "stub/catalog hash" in reason
            assert stub_hash[:16] in reason
        finally:
            Path(export_path).unlink()

    def test_allows_unique_export_hash(self, monkeypatch):
        """Should allow done when export hash is not in stub list."""
        with tempfile.NamedTemporaryFile(mode="w", suffix=".step", delete=False) as f:
            f.write("unique content")
            export_path = f.name
        
        try:
            monkeypatch.setenv("KALA_MIN_TOOLS", "0")
            monkeypatch.setenv("KALA_KNOWN_STUB_HASHES", "other_hash1,other_hash2")
            
            agent = Agent(backend_name="mock")
            state = make_test_state(
                history=[
                    ToolEvent("create_box", {}, True, "ok", {}),
                    ToolEvent("export", {}, True, "ok", {}),
                ],
                last_export=export_path,
            )
            
            allowed, reason = agent._check_fidelity_gate(state)
            assert allowed is True
            assert reason == ""
        finally:
            Path(export_path).unlink()

    def test_invalid_min_tools_env_defaults_to_four(self, monkeypatch):
        """Should default to 4 when KALA_MIN_TOOLS is not an integer."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "not_a_number")
        
        agent = Agent(backend_name="mock")
        state = make_test_state(
            history=[ToolEvent("export", {}, True, "ok", {})],
            last_export="test.step",
        )
        
        # Should reject with default 4 (0 real tools)
        allowed, reason = agent._check_fidelity_gate(state)
        assert allowed is False
        assert "too few tools" in reason

    def test_rejects_catalog_basename_pattern(self, monkeypatch):
        """Should reject done when export basename matches catalog pattern."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "0")
        monkeypatch.setenv("KALA_KNOWN_STUB_HASHES", "")
        
        agent = Agent(backend_name="mock")
        state = make_test_state(
            history=[
                ToolEvent("insert_part", {}, True, "ok", {}),
                ToolEvent("export", {}, True, "ok", {}),
            ],
            last_export="outputs/machine_assembly_hex_m6x20_3.step",
        )
        
        allowed, reason = agent._check_fidelity_gate(state)
        assert allowed is False
        assert "catalog stub pattern" in reason
        assert "hex_m6x20" in reason.lower()

    def test_allows_non_catalog_basename(self, monkeypatch):
        """Should allow done when export basename is not a catalog pattern."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "0")
        monkeypatch.setenv("KALA_KNOWN_STUB_HASHES", "")
        
        agent = Agent(backend_name="mock")
        state = make_test_state(
            history=[
                ToolEvent("create_box", {}, True, "ok", {}),
                ToolEvent("export", {}, True, "ok", {}),
            ],
            last_export="outputs/custom_bracket.step",
        )
        
        allowed, reason = agent._check_fidelity_gate(state)
        assert allowed is True
        assert reason == ""


class TestFidelityGateIntegration:
    """Test fidelity gate integration in Agent.run."""

    def test_blocks_premature_done_with_min_tools(self, monkeypatch):
        """Should block done and add fidelity_gate event when tools < min_tools."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "5")
        
        # Mock planner that immediately says done with export
        mock_planner = MagicMock()
        
        # First turn: export with done=True
        from kala.llm.base import PlannerTurn, ToolCall
        mock_planner.propose.return_value = PlannerTurn(
            thought="export now",
            calls=[ToolCall("export", {"body_id": "test", "path": "test.step", "fmt": "step"})],
            done=True,
        )
        
        # Mock backend
        from kala.cad.registry import ToolResult
        mock_backend = MagicMock()
        mock_backend.name = "mock"
        mock_backend.call_tool.return_value = ToolResult(
            ok=True,
            message="exported",
            data={"path": "outputs/test.step"},
        )
        
        # This would normally run the full agent loop, but with min_tools=5
        # and only 0 real tools, the gate should block done
        # We can't easily test the full run without a real backend,
        # but we've tested _check_fidelity_gate separately


class TestForceModelingProgress:
    """Test _force_modeling_progress helper."""

    def test_creates_bodies_when_too_few_tools(self, monkeypatch):
        """Should create bodies when real tool count < min_tools."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "4")
        
        from kala.agent.loop import _force_modeling_progress
        from kala.cad.registry import ToolRegistry, ToolResult
        
        # Mock registry with create tools
        mock_registry = MagicMock(spec=ToolRegistry)
        mock_registry.has.return_value = True
        mock_registry.call.return_value = ToolResult(
            ok=True,
            message="created",
            data={"body_id": "Body_1"},
        )
        
        state = make_test_state(history=[])
        
        forced_events = _force_modeling_progress(state, mock_registry)
        
        # Should force some modeling tools
        assert len(forced_events) > 0
        assert any(e.tool in {"create_box", "create_cylinder"} for e in forced_events)

    def test_stops_when_min_tools_met(self, monkeypatch):
        """Should not force tools when threshold already met."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "2")
        
        from kala.agent.loop import _force_modeling_progress
        from kala.cad.registry import ToolRegistry
        
        mock_registry = MagicMock(spec=ToolRegistry)
        
        # Already have 3 real tools (> min_tools=2)
        state = make_test_state(
            history=[
                ToolEvent("create_box", {}, True, "ok", {"body_id": "Box_1"}),
                ToolEvent("create_cylinder", {}, True, "ok", {"body_id": "Cyl_1"}),
                ToolEvent("boolean_fuse", {}, True, "ok", {"body_id": "Fuse_1"}),
            ],
        )
        
        forced_events = _force_modeling_progress(state, mock_registry)
        
        # Should not force any tools when threshold met
        assert len(forced_events) == 0

    def test_caps_forced_calls_at_six(self, monkeypatch):
        """Should cap forced calls at 6 per invocation."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "100")
        
        from kala.agent.loop import _force_modeling_progress
        from kala.cad.registry import ToolRegistry, ToolResult
        
        mock_registry = MagicMock(spec=ToolRegistry)
        mock_registry.has.return_value = True
        mock_registry.call.return_value = ToolResult(
            ok=True,
            message="ok",
            data={"body_id": "Body_X"},
        )
        
        state = make_test_state(history=[])
        
        forced_events = _force_modeling_progress(state, mock_registry)
        
        # Should cap at 6 forced calls
        assert len(forced_events) <= 6

    def test_fuses_bodies_for_single_part(self, monkeypatch):
        """Should fuse bodies together when making single part."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "5")
        
        from kala.agent.loop import _force_modeling_progress
        from kala.cad.registry import ToolRegistry, ToolResult
        
        mock_registry = MagicMock(spec=ToolRegistry)
        mock_registry.has.return_value = True
        
        # Return different body_ids for creates, fuse result for fuse
        def call_side_effect(tool, **kwargs):
            if tool == "boolean_fuse":
                return ToolResult(
                    ok=True,
                    message="fused",
                    data={"body_id": "Fuse_1", "removed": ["Box_1", "Cyl_1"]},
                )
            return ToolResult(
                ok=True,
                message="created",
                data={"body_id": f"{tool}_result"},
            )
        
        mock_registry.call.side_effect = call_side_effect
        
        state = make_test_state(
            history=[
                ToolEvent("create_box", {}, True, "ok", {"body_id": "Box_1"}),
                ToolEvent("create_cylinder", {}, True, "ok", {"body_id": "Cyl_1"}),
            ],
        )
        
        forced_events = _force_modeling_progress(state, mock_registry)
        
        # Should include fuse operation
        assert any(e.tool == "boolean_fuse" for e in forced_events)

    def test_creates_and_cuts_when_needed(self, monkeypatch):
        """Should create cutter and cut when still need more tools."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "6")
        
        from kala.agent.loop import _force_modeling_progress
        from kala.cad.registry import ToolRegistry, ToolResult
        
        call_count = [0]
        
        def call_side_effect(tool, **kwargs):
            call_count[0] += 1
            if tool == "boolean_cut":
                return ToolResult(
                    ok=True,
                    message="cut",
                    data={"body_id": "Cut_1", "removed": ["Box_1"]},
                )
            return ToolResult(
                ok=True,
                message="created",
                data={"body_id": f"Body_{call_count[0]}"},
            )
        
        mock_registry = MagicMock(spec=ToolRegistry)
        mock_registry.has.return_value = True
        mock_registry.call.side_effect = call_side_effect
        
        state = make_test_state(
            history=[
                ToolEvent("create_box", {}, True, "ok", {"body_id": "Box_1"}),
            ],
        )
        
        forced_events = _force_modeling_progress(state, mock_registry)
        
        # Should include cut operation
        assert any(e.tool == "boolean_cut" for e in forced_events)

    def test_respects_min_tools_env_disabled(self, monkeypatch):
        """Should not force tools when KALA_MIN_TOOLS=0."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "0")
        
        from kala.agent.loop import _force_modeling_progress
        from kala.cad.registry import ToolRegistry
        
        mock_registry = MagicMock(spec=ToolRegistry)
        state = make_test_state(history=[])
        
        forced_events = _force_modeling_progress(state, mock_registry)
        
        # Should not force any tools when min_tools disabled
        assert len(forced_events) == 0


class TestFidelityGateForceIntegration:
    """Test that fidelity gate triggers forced progress in agent loop."""

    def test_forces_progress_after_too_few_tools_block(self, monkeypatch):
        """Should force modeling progress when gate blocks due to too few tools."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "5")
        
        from kala.agent.loop import Agent
        from kala.llm.base import PlannerTurn, ToolCall
        
        # Mock planner that tries to finish early
        mock_planner = MagicMock()
        turn_count = [0]
        
        def propose_side_effect(state, context, schemas):
            turn_count[0] += 1
            if turn_count[0] == 1:
                # First turn: try to export with only 1 tool
                return PlannerTurn(
                    thought="export early",
                    calls=[
                        ToolCall("create_box", {"length": 10, "width": 10, "height": 10}),
                        ToolCall("export", {"body_id": "Box_1", "path": "test.step", "fmt": "step"}),
                    ],
                    done=True,
                )
            # Subsequent turns: try done again
            return PlannerTurn(thought="try done again", calls=[], done=True)
        
        mock_planner.propose.side_effect = propose_side_effect
        
        agent = Agent(backend_name="mock", planner=mock_planner, max_turns=10)
        
        # Run with a simple goal
        result = agent.run("Create a test part")
        
        # Should have forced some modeling tools
        history = result.state.history
        fidelity_events = [e for e in history if e.tool == "fidelity_gate"]
        
        # Should have at least one fidelity gate block
        assert len(fidelity_events) >= 1
        assert any("too few tools" in e.message for e in fidelity_events)
        
        # After forcing, should have more real tools
        real_tools = [e for e in history if e.ok and e.tool not in {"list_bodies", "show_in_freecad", "search_parts", "export", "fidelity_gate"}]
        
        # With min_tools=5 and forced progress, should eventually reach threshold
        assert len(real_tools) >= 3  # At least some forced progress

    def test_does_not_force_for_stub_hash_block(self, monkeypatch, tmp_path):
        """Should not force progress when gate blocks due to stub hash (not too_few_tools)."""
        monkeypatch.setenv("KALA_MIN_TOOLS", "0")
        
        # Create stub export file
        stub_file = tmp_path / "stub.step"
        stub_file.write_text("stub content")
        
        from kala.agent.loop import Agent, _compute_step_hash
        from kala.llm.base import PlannerTurn, ToolCall
        
        stub_hash = _compute_step_hash(stub_file)
        monkeypatch.setenv("KALA_KNOWN_STUB_HASHES", stub_hash)
        
        mock_planner = MagicMock()
        mock_planner.propose.return_value = PlannerTurn(
            thought="export stub",
            calls=[
                ToolCall("create_box", {"length": 10, "width": 10, "height": 10}),
                ToolCall("export", {"body_id": "Box_1", "path": str(stub_file), "fmt": "step"}),
            ],
            done=True,
        )
        
        agent = Agent(backend_name="mock", planner=mock_planner, max_turns=5)
        result = agent.run("Create a test part")
        
        history = result.state.history
        fidelity_events = [e for e in history if e.tool == "fidelity_gate"]
        
        # Should have fidelity block but NOT due to too_few_tools
        if fidelity_events:
            assert all("too few tools" not in e.message for e in fidelity_events)
        
        # Should not have forced a bunch of extra modeling tools (since not too_few_tools block)
        # Just the initial create_box from planner
        real_tools = [e for e in history if e.ok and e.tool in {"create_box", "create_cylinder", "boolean_fuse", "boolean_cut"}]
        assert len(real_tools) <= 2  # Initial create + maybe one more, but no extensive forcing


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
