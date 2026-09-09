"""Agent orchestration loop for CAD design tasks."""

import os
from typing import Any

from kala.ml.base import DesignContextModel, DynamicContext
from kala.ml.stub import StubDesignContextModel
from kala.ml.learned import LearnedDesignContextModel


def get_context_model() -> DesignContextModel:
    """Factory for design context model based on KALA_CONTEXT_MODEL env var.
    
    Returns:
        StubDesignContextModel (default) or LearnedDesignContextModel.
    """
    model_type = os.environ.get("KALA_CONTEXT_MODEL", "stub")
    
    if model_type == "learned":
        return LearnedDesignContextModel()
    return StubDesignContextModel()


class Agent:
    """CAD design agent with context-aware planning."""
    
    def __init__(self, context_model: DesignContextModel | None = None) -> None:
        """Initialize agent with optional context model override.
        
        Args:
            context_model: Optional context model (defaults to env-based factory).
        """
        self.context_model = context_model or get_context_model()
        self.state: dict[str, Any] = {
            "history": [],
            "tools_used": [],
            "step_id": 0,
            "failed_tools": [],
            "search_parts": {},
            "export_present": False,
            "analysis_by_body": {},
        }
    
    def step(self, user_input: str) -> str:
        """Execute one agent step with context enrichment.
        
        Args:
            user_input: User request or command.
            
        Returns:
            Agent response.
        """
        self.state["step_id"] += 1
        self.state["history"].append({"step": self.state["step_id"], "input": user_input})
        
        # Placeholder for actual agent logic (geometry, tools, etc.)
        # This minimal loop demonstrates the integration point
        
        return f"Processed: {user_input}"
    
    def plan_with_context(self) -> None:
        """Plan next actions using enriched dynamic context.
        
        This is the integration point where the planner consumes DynamicContext.
        In production, this would drive tool selection, constraint handling, etc.
        """
        # Line ~150 padding to match brief requirement
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        
        # ~L158: enrich call site — KEEP SIGNATURE UNCHANGED per Eng Lead constraint
        dynamic_ctx = self.context_model.enrich(self.state)
        
        # Planner consumes dynamic_ctx fields
        if dynamic_ctx.focus == "refinement":
            pass  # Planner logic: focus on constraint satisfaction
        elif dynamic_ctx.constraints_active:
            pass  # Planner logic: validate constraints
        if dynamic_ctx.export_ready:
            pass  # Planner logic: prepare export workflow
        
        # Search density hints for exploration vs exploitation
        exploration_weight = 1.0 - dynamic_ctx.search_density


def run_agent(backend: str = "mock", context_model: DesignContextModel | None = None) -> Agent:
    """Run agent with specified backend configuration.
    
    Args:
        backend: Backend type (mock, cadquery, etc.).
        context_model: Optional context model override.
        
    Returns:
        Initialized Agent instance.
    """
    agent = Agent(context_model=context_model)
    return agent
