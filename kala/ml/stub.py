"""Stub design context model — default fallback with no-op enrichment."""

from typing import Any

from kala.ml.base import DesignContextModel, DynamicContext


class StubDesignContextModel:
    """No-op stub returning default DynamicContext values.
    
    Used as default fallback when KALA_CONTEXT_MODEL=stub or when
    learned model artifacts are missing/invalid.
    """
    
    def enrich(self, state: dict[str, Any]) -> DynamicContext:
        """Return default DynamicContext without state analysis.
        
        Args:
            state: Agent state (ignored in stub).
            
        Returns:
            DynamicContext with default field values.
        """
        return DynamicContext()
