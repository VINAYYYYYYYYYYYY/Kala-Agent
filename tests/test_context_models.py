"""Unit tests for design context models — Protocol compliance and fallback."""

import json
import tempfile
from pathlib import Path

from kala.ml.base import DesignContextModel, DynamicContext
from kala.ml.stub import StubDesignContextModel
from kala.ml.learned import LearnedDesignContextModel
from kala.ml.features import extract_features


class TestProtocolCompliance:
    """Test that models satisfy DesignContextModel protocol."""
    
    def test_stub_implements_protocol(self) -> None:
        """StubDesignContextModel implements enrich() returning DynamicContext."""
        model = StubDesignContextModel()
        assert hasattr(model, "enrich")
        
        state = {"history": [], "step_id": 0}
        result = model.enrich(state)
        
        assert isinstance(result, DynamicContext)
        assert hasattr(result, "focus")
        assert hasattr(result, "constraints_active")
        assert hasattr(result, "export_ready")
        assert hasattr(result, "search_density")
    
    def test_learned_implements_protocol(self) -> None:
        """LearnedDesignContextModel implements enrich() returning DynamicContext."""
        model = LearnedDesignContextModel()
        assert hasattr(model, "enrich")
        
        state = {"history": [], "step_id": 0}
        result = model.enrich(state)
        
        assert isinstance(result, DynamicContext)
        assert hasattr(result, "focus")
        assert hasattr(result, "constraints_active")
        assert hasattr(result, "export_ready")
        assert hasattr(result, "search_density")


class TestEmptyHistoryEnrich:
    """Test enrich() with empty/minimal state (no crash, valid output)."""
    
    def test_stub_empty_state(self) -> None:
        """StubDesignContextModel handles empty state without crash."""
        model = StubDesignContextModel()
        
        empty_state = {}
        result = model.enrich(empty_state)
        
        assert isinstance(result, DynamicContext)
        assert result.focus == "exploration"
        assert result.constraints_active is False
        assert result.export_ready is False
        assert result.search_density == 0.0
    
    def test_stub_minimal_state(self) -> None:
        """StubDesignContextModel returns defaults for minimal state."""
        model = StubDesignContextModel()
        
        minimal_state = {"history": [], "step_id": 0}
        result = model.enrich(minimal_state)
        
        assert isinstance(result, DynamicContext)
        assert result.focus == "exploration"
    
    def test_learned_empty_state_with_artifact(self) -> None:
        """LearnedDesignContextModel handles empty state with valid artifact."""
        model = LearnedDesignContextModel()
        
        empty_state = {}
        result = model.enrich(empty_state)
        
        assert isinstance(result, DynamicContext)
    
    def test_learned_minimal_state(self) -> None:
        """LearnedDesignContextModel handles minimal state."""
        model = LearnedDesignContextModel()
        
        minimal_state = {"history": [], "step_id": 0, "tools_used": []}
        result = model.enrich(minimal_state)
        
        assert isinstance(result, DynamicContext)
        assert result.focus in ["exploration", "refinement", "validation"]
        assert isinstance(result.constraints_active, bool)
        assert isinstance(result.export_ready, bool)
        assert 0.0 <= result.search_density <= 1.0


class TestFallbackBehavior:
    """Test fallback to StubDesignContextModel when artifact is missing/invalid."""
    
    def test_missing_artifact_fallback(self) -> None:
        """LearnedDesignContextModel falls back when artifact is missing."""
        nonexistent_path = Path("/tmp/nonexistent_artifact_xyz123.json")
        model = LearnedDesignContextModel(artifact_path=nonexistent_path)
        
        state = {"history": [], "step_id": 0}
        result = model.enrich(state)
        
        assert isinstance(result, DynamicContext)
        assert result.focus == "exploration"
        assert result.constraints_active is False
    
    def test_invalid_json_fallback(self) -> None:
        """LearnedDesignContextModel falls back when artifact JSON is invalid."""
        with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as f:
            f.write("{invalid json content")
            invalid_path = Path(f.name)
        
        try:
            model = LearnedDesignContextModel(artifact_path=invalid_path)
            
            state = {"history": [], "step_id": 0}
            result = model.enrich(state)
            
            assert isinstance(result, DynamicContext)
            assert result.focus == "exploration"
        finally:
            invalid_path.unlink()
    
    def test_empty_rules_fallback(self) -> None:
        """LearnedDesignContextModel falls back when artifact has no rules."""
        with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as f:
            json.dump({"version": "0.1.0", "defaults": {}}, f)
            empty_rules_path = Path(f.name)
        
        try:
            model = LearnedDesignContextModel(artifact_path=empty_rules_path)
            
            state = {"history": [], "step_id": 0}
            result = model.enrich(state)
            
            assert isinstance(result, DynamicContext)
            assert result.focus == "exploration"
        finally:
            empty_rules_path.unlink()


class TestFeatureExtraction:
    """Test extract_features() function."""
    
    def test_extract_from_empty_state(self) -> None:
        """extract_features handles empty state without crash."""
        features = extract_features({})
        
        assert isinstance(features, dict)
        assert "body_count" in features
        assert "tool_count" in features
        assert "fail_count" in features
        assert "step_id" in features
    
    def test_extract_body_count(self) -> None:
        """extract_features counts body_created entries."""
        state = {
            "history": [
                {"type": "body_created", "id": 1},
                {"type": "other", "id": 2},
                {"type": "body_created", "id": 3},
            ]
        }
        
        features = extract_features(state)
        assert features["body_count"] == 2
    
    def test_extract_tool_count(self) -> None:
        """extract_features counts tools_used entries."""
        state = {
            "tools_used": [
                {"name": "box"},
                {"name": "sphere"},
                {"name": "export"},
            ]
        }
        
        features = extract_features(state)
        assert features["tool_count"] == 3
    
    def test_extract_last_n_tools(self) -> None:
        """extract_features captures last 5 tool names."""
        state = {
            "tools_used": [
                {"name": f"tool_{i}"} for i in range(10)
            ]
        }
        
        features = extract_features(state)
        assert len(features["last_n_tools"]) == 5
        assert features["last_n_tools"] == ["tool_5", "tool_6", "tool_7", "tool_8", "tool_9"]


class TestRuleEvaluation:
    """Test learned model rule evaluation logic."""
    
    def test_focus_early_exploration(self) -> None:
        """Early steps with no bodies trigger exploration focus."""
        model = LearnedDesignContextModel()
        
        state = {
            "history": [],
            "step_id": 1,
            "tools_used": [],
            "failed_tools": [],
            "search_parts": {},
            "export_present": False,
        }
        
        result = model.enrich(state)
        assert result.focus == "exploration"
    
    def test_constraints_detection(self) -> None:
        """Constraints active when constraint_add tool used."""
        model = LearnedDesignContextModel()
        
        state = {
            "history": [],
            "step_id": 5,
            "tools_used": [{"name": "constraint_add"}],
            "failed_tools": [{"name": "constraint_add"}],
            "search_parts": {},
            "export_present": False,
        }
        
        result = model.enrich(state)
        assert result.constraints_active is True
    
    def test_export_ready_detection(self) -> None:
        """Export ready when export present and bodies exist."""
        model = LearnedDesignContextModel()
        
        state = {
            "history": [{"type": "body_created", "id": 1}],
            "step_id": 10,
            "tools_used": [],
            "failed_tools": [],
            "search_parts": {},
            "export_present": True,
        }
        
        result = model.enrich(state)
        assert result.export_ready is True
    
    def test_search_density_levels(self) -> None:
        """Search density scales with search_parts hits."""
        model = LearnedDesignContextModel()
        
        low_state = {
            "history": [],
            "step_id": 5,
            "tools_used": [],
            "failed_tools": [],
            "search_parts": {"query1": ["hit1"]},
            "export_present": False,
        }
        result_low = model.enrich(low_state)
        assert result_low.search_density == 0.2
        
        high_state = {
            "history": [],
            "step_id": 5,
            "tools_used": [],
            "failed_tools": [],
            "search_parts": {f"query{i}": [f"hit{j}" for j in range(5)] for i in range(3)},
            "export_present": False,
        }
        result_high = model.enrich(high_state)
        assert result_high.search_density == 0.8


def test_integration_agent_loop() -> None:
    """Integration test: Agent.plan_with_context() uses enrich() without crash."""
    from kala.agent.loop import Agent
    
    agent = Agent()
    agent.plan_with_context()


def test_integration_cli_env_var() -> None:
    """Integration test: get_context_model() respects KALA_CONTEXT_MODEL."""
    import os
    from kala.agent.loop import get_context_model
    
    os.environ["KALA_CONTEXT_MODEL"] = "stub"
    model_stub = get_context_model()
    assert isinstance(model_stub, StubDesignContextModel)
    
    os.environ["KALA_CONTEXT_MODEL"] = "learned"
    model_learned = get_context_model()
    assert isinstance(model_learned, LearnedDesignContextModel)
    
    del os.environ["KALA_CONTEXT_MODEL"]
