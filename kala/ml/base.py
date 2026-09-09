"""Base protocol and data structures for design context models."""

from dataclasses import dataclass
from typing import Any, Protocol


@dataclass
class DynamicContext:
    """Context enrichment output from design context models.
    
    Fields populated by enrich() to guide the planner.
    """
    
    focus: str = "exploration"
    constraints_active: bool = False
    export_ready: bool = False
    search_density: float = 0.0


class DesignContextModel(Protocol):
    """Protocol for design context enrichment models.
    
    Implementations must provide enrich() to map agent state → DynamicContext.
    """
    
    def enrich(self, state: dict[str, Any]) -> DynamicContext:
        """Enrich agent state with dynamic context for planner guidance.
        
        Args:
            state: Agent state dictionary containing history, tools, etc.
            
        Returns:
            DynamicContext with enriched fields for planner consumption.
        """
        ...
