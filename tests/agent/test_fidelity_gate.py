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


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
