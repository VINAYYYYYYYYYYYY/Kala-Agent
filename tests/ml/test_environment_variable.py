"""Test environment variable for context model selection."""

from __future__ import annotations

import os

import pytest

from kala.agent.loop import resolve_context_model
from kala.ml.learned import LearnedDesignContextModel
from kala.ml.stub import StubDesignContextModel


class TestContextModelSelection:
    """Test KALA_CONTEXT_MODEL environment variable."""
    
    def test_default_is_stub(self, monkeypatch):
        """Default should be stub when env var is not set."""
        monkeypatch.delenv("KALA_CONTEXT_MODEL", raising=False)
        
        model = resolve_context_model()
        
        assert isinstance(model, StubDesignContextModel)
        assert not isinstance(model, LearnedDesignContextModel)
    
    def test_explicit_stub(self, monkeypatch):
        """Should return stub when KALA_CONTEXT_MODEL=stub."""
        monkeypatch.setenv("KALA_CONTEXT_MODEL", "stub")
        
        model = resolve_context_model()
        
        assert isinstance(model, StubDesignContextModel)
    
    def test_explicit_stub_uppercase(self, monkeypatch):
        """Should handle uppercase STUB."""
        monkeypatch.setenv("KALA_CONTEXT_MODEL", "STUB")
        
        model = resolve_context_model()
        
        assert isinstance(model, StubDesignContextModel)
    
    def test_learned_model(self, monkeypatch):
        """Should return learned when KALA_CONTEXT_MODEL=learned."""
        monkeypatch.setenv("KALA_CONTEXT_MODEL", "learned")
        
        model = resolve_context_model()
        
        assert isinstance(model, LearnedDesignContextModel)
    
    def test_learned_uppercase(self, monkeypatch):
        """Should handle uppercase LEARNED."""
        monkeypatch.setenv("KALA_CONTEXT_MODEL", "LEARNED")
        
        model = resolve_context_model()
        
        assert isinstance(model, LearnedDesignContextModel)
    
    def test_invalid_value_defaults_to_stub(self, monkeypatch):
        """Invalid values should default to stub."""
        monkeypatch.setenv("KALA_CONTEXT_MODEL", "invalid")
        
        model = resolve_context_model()
        
        assert isinstance(model, StubDesignContextModel)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
