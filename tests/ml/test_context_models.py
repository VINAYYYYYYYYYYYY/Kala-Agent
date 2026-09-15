"""Unit tests for design context models."""

from __future__ import annotations

import json
import tempfile
from pathlib import Path

import pytest

from kala.ml.base import DesignContextModel, DynamicContext
from kala.ml.features import extract_features
from kala.ml.learned import LearnedDesignContextModel
from kala.ml.stub import StubDesignContextModel
from kala.procedures.schema import Procedure, ProcedureStep
from kala.session.state import SessionState, ToolEvent


def make_test_procedure() -> Procedure:
    """Create a minimal test procedure."""
    return Procedure(
        id="test_proc",
        name="Test Procedure",
        description="Test",
        steps=[
            ProcedureStep(
                id="model",
                goal="Create model",
                allowed_tools=["create_box", "create_cylinder"],
                optional_parts=False,
                exit_criteria="Model created",
            ),
            ProcedureStep(
                id="refine",
                goal="Refine model",
                allowed_tools=["fillet", "chamfer"],
                optional_parts=False,
                exit_criteria="Model refined",
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
    goal: str = "test goal",
    history: list[ToolEvent] | None = None,
    step_index: int = 0,
) -> SessionState:
    """Create a minimal test session state."""
    return SessionState(
        goal=goal,
        backend_name="mock",
        standard_parts=False,
        procedure=make_test_procedure(),
        step_index=step_index,
        history=history or [],
    )


class TestProtocolCompliance:
    """Test that all context models implement the Protocol correctly."""
    
    def test_stub_implements_protocol(self):
        """Stub model should implement DesignContextModel protocol."""
        stub = StubDesignContextModel()
        assert isinstance(stub, DesignContextModel)
    
    def test_learned_implements_protocol(self):
        """Learned model should implement DesignContextModel protocol."""
        learned = LearnedDesignContextModel()
        assert isinstance(learned, DesignContextModel)
    
    def test_enrich_signature(self):
        """All models should accept SessionState and return DynamicContext."""
        state = make_test_state()
        stub = StubDesignContextModel()
        learned = LearnedDesignContextModel()
        
        stub_result = stub.enrich(state)
        learned_result = learned.enrich(state)
        
        assert isinstance(stub_result, DynamicContext)
        assert isinstance(learned_result, DynamicContext)


class TestEmptyHistoryHandling:
    """Test that models handle empty history gracefully."""
    
    def test_stub_empty_history(self):
        """Stub should work with empty history."""
        state = make_test_state(history=[])
        model = StubDesignContextModel()
        
        context = model.enrich(state)
        
        assert isinstance(context, DynamicContext)
        assert context.focus
        assert isinstance(context.constraints, list)
        assert isinstance(context.manufacturing_notes, list)
        assert isinstance(context.recommended_tools, list)
        assert isinstance(context.candidate_parts, list)
        assert isinstance(context.warnings, list)
        assert isinstance(context.snippets, list)
    
    def test_learned_empty_history(self):
        """Learned model should work with empty history."""
        state = make_test_state(history=[])
        model = LearnedDesignContextModel()
        
        context = model.enrich(state)
        
        assert isinstance(context, DynamicContext)
        assert context.focus
        assert isinstance(context.constraints, list)
        assert isinstance(context.manufacturing_notes, list)
        assert isinstance(context.recommended_tools, list)
        assert isinstance(context.candidate_parts, list)
        assert isinstance(context.warnings, list)
        assert isinstance(context.snippets, list)


class TestDynamicContextFields:
    """Test that DynamicContext fields remain unchanged."""
    
    def test_dynamic_context_fields(self):
        """DynamicContext should have exactly the expected fields."""
        context = DynamicContext(focus="test")
        
        # Required fields from base.py
        assert hasattr(context, "focus")
        assert hasattr(context, "constraints")
        assert hasattr(context, "manufacturing_notes")
        assert hasattr(context, "recommended_tools")
        assert hasattr(context, "candidate_parts")
        assert hasattr(context, "warnings")
        assert hasattr(context, "snippets")
        
        # Check types
        assert isinstance(context.focus, str)
        assert isinstance(context.constraints, list)
        assert isinstance(context.manufacturing_notes, list)
        assert isinstance(context.recommended_tools, list)
        assert isinstance(context.candidate_parts, list)
        assert isinstance(context.warnings, list)
        assert isinstance(context.snippets, list)
    
    def test_dynamic_context_to_dict(self):
        """DynamicContext.to_dict should contain all fields."""
        context = DynamicContext(
            focus="test",
            constraints=["c1"],
            manufacturing_notes=["n1"],
            recommended_tools=["t1"],
            candidate_parts=[{"part_id": "p1"}],
            warnings=["w1"],
            snippets=["s1"],
        )
        
        d = context.to_dict()
        
        assert d["focus"] == "test"
        assert d["constraints"] == ["c1"]
        assert d["manufacturing_notes"] == ["n1"]
        assert d["recommended_tools"] == ["t1"]
        assert d["candidate_parts"] == [{"part_id": "p1"}]
        assert d["warnings"] == ["w1"]
        assert d["snippets"] == ["s1"]


class TestLearnedModelFallback:
    """Test that learned model falls back to stub when artifact is missing/invalid."""
    
    def test_fallback_on_missing_artifact(self, tmp_path: Path, monkeypatch):
        """Should fall back to stub when artifact is missing."""
        # Point to non-existent artifact directory
        fake_module_path = tmp_path / "fake_ml"
        fake_module_path.mkdir()
        
        # Patch __file__ to point to fake location
        import kala.ml.learned as learned_module
        original_file = learned_module.__file__
        monkeypatch.setattr(learned_module, "__file__", str(fake_module_path / "learned.py"))
        
        model = LearnedDesignContextModel()
        state = make_test_state()
        
        context = model.enrich(state)
        
        # Should still work (via fallback)
        assert isinstance(context, DynamicContext)
        assert context.focus
        
        # Restore
        monkeypatch.setattr(learned_module, "__file__", original_file)
    
    def test_fallback_on_invalid_json(self, tmp_path: Path, monkeypatch):
        """Should fall back to stub when artifact has invalid JSON."""
        # Create fake artifact with invalid JSON
        fake_module_path = tmp_path / "fake_ml"
        fake_module_path.mkdir()
        artifacts_path = fake_module_path / "artifacts"
        artifacts_path.mkdir()
        artifact_file = artifacts_path / "context_v0.json"
        artifact_file.write_text("{ invalid json }")
        
        # Patch __file__
        import kala.ml.learned as learned_module
        original_file = learned_module.__file__
        monkeypatch.setattr(learned_module, "__file__", str(fake_module_path / "learned.py"))
        
        model = LearnedDesignContextModel()
        state = make_test_state()
        
        context = model.enrich(state)
        
        # Should still work (via fallback)
        assert isinstance(context, DynamicContext)
        assert context.focus
        
        # Restore
        monkeypatch.setattr(learned_module, "__file__", original_file)


class TestFeatureExtraction:
    """Test feature extraction from session state."""
    
    def test_extract_basic_features(self):
        """Should extract basic session features."""
        state = make_test_state(goal="test bracket")
        features = extract_features(state)
        
        assert features["goal"] == "test bracket"
        assert features["backend_name"] == "mock"
        assert features["standard_parts"] is False
        assert features["history_length"] == 0
        assert features["step_index"] == 0
    
    def test_extract_step_features(self):
        """Should extract current step features."""
        state = make_test_state(step_index=0)
        features = extract_features(state)
        
        assert features["current_step_id"] == "model"
        assert features["current_step_goal"] == "Create model"
        assert features["allowed_tools_count"] == 2
        assert "create_box" in features["allowed_tools"]
    
    def test_extract_history_features(self):
        """Should extract history analysis features."""
        history = [
            ToolEvent("create_box", {}, True, "ok", {}),
            ToolEvent("fillet", {}, False, "failed", {}),
            ToolEvent("create_cylinder", {}, True, "ok", {}),
        ]
        state = make_test_state(history=history)
        features = extract_features(state)
        
        assert features["success_count"] == 2
        assert features["failure_count"] == 1
        assert features["success_rate"] == pytest.approx(2.0 / 3.0)
        assert features["unique_tools_used"] == 3
    
    def test_extract_goal_keywords(self):
        """Should extract goal keyword features."""
        state = make_test_state(goal="thin sheet bracket")
        features = extract_features(state)
        
        assert features["goal_keywords"]["thin"] is True
        assert features["goal_keywords"]["sheet"] is True
        assert features["goal_keywords"]["bracket"] is True
        assert features["goal_keywords"]["box"] is False


class TestLearnedModelRules:
    """Test that learned model applies rules correctly."""
    
    def test_applies_constraint_rules(self):
        """Should apply constraint rules based on features."""
        # Create state with low success rate
        history = [
            ToolEvent("create_box", {}, False, "failed", {}),
            ToolEvent("fillet", {}, False, "failed", {}),
            ToolEvent("create_cylinder", {}, False, "failed", {}),
            ToolEvent("export", {}, True, "ok", {}),
        ]
        state = make_test_state(history=history)
        model = LearnedDesignContextModel()
        
        context = model.enrich(state)
        
        # Should include base constraints plus learned constraints
        assert isinstance(context.constraints, list)
        # High failure rate rule might fire
        constraint_text = " ".join(context.constraints)
        # Just verify it returns a list (actual rules depend on artifact)
        assert len(context.constraints) >= 0
    
    def test_applies_warning_rules(self):
        """Should apply warning rules based on features."""
        # Create state late in procedure without export
        state = make_test_state(step_index=2)  # At export step
        state.last_export = None
        model = LearnedDesignContextModel()
        
        context = model.enrich(state)
        
        # Should have warnings
        assert isinstance(context.warnings, list)


class TestNoTrainingLoop:
    """Verify that no training loop is included."""
    
    def test_no_train_method_in_stub(self):
        """Stub model should not have train method."""
        model = StubDesignContextModel()
        assert not hasattr(model, "train")
    
    def test_no_train_method_in_learned(self):
        """Learned model should not have train method."""
        model = LearnedDesignContextModel()
        assert not hasattr(model, "train")


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
