"""Feature extraction from session state for learned models."""

from __future__ import annotations

from typing import Any

from kala.session.state import SessionState


def extract_features(state: SessionState) -> dict[str, Any]:
    """Extract features from session state for ML model input.
    
    Returns a dictionary of features that can be used by learned models
    to make context-aware predictions.
    """
    step = state.current_step
    
    # Basic session features
    features: dict[str, Any] = {
        "goal": state.goal,
        "goal_length": len(state.goal),
        "backend_name": state.backend_name,
        "standard_parts": state.standard_parts,
        "status": state.status,
        "has_error": state.error is not None,
        "history_length": len(state.history),
        "step_index": state.step_index,
        "total_steps": len(state.procedure.steps),
        "procedure_id": state.procedure.id,
        "procedure_name": state.procedure.name,
    }
    
    # Current step features
    if step is not None:
        features.update({
            "current_step_id": step.id,
            "current_step_goal": step.goal,
            "allowed_tools_count": len(step.allowed_tools),
            "allowed_tools": list(step.allowed_tools),
            "optional_parts": step.optional_parts,
            "exit_criteria": step.exit_criteria,
        })
    else:
        features.update({
            "current_step_id": None,
            "current_step_goal": None,
            "allowed_tools_count": 0,
            "allowed_tools": [],
            "optional_parts": False,
            "exit_criteria": None,
        })
    
    # History analysis features
    success_count = sum(1 for e in state.history if e.ok)
    failure_count = sum(1 for e in state.history if not e.ok)
    
    features.update({
        "success_count": success_count,
        "failure_count": failure_count,
        "success_rate": success_count / len(state.history) if state.history else 0.0,
        "has_export": state.last_export is not None,
        "export_path": state.last_export or "",
    })
    
    # Tool usage patterns
    tool_usage: dict[str, int] = {}
    recent_tools: list[str] = []
    for event in state.history:
        tool_usage[event.tool] = tool_usage.get(event.tool, 0) + 1
        if len(recent_tools) < 5:
            recent_tools.append(event.tool)
    
    features.update({
        "unique_tools_used": len(tool_usage),
        "most_used_tool": max(tool_usage.items(), key=lambda x: x[1])[0] if tool_usage else None,
        "recent_tools": recent_tools,
        "tool_usage": tool_usage,
    })
    
    # Goal keyword analysis
    goal_lower = state.goal.lower()
    features.update({
        "goal_keywords": {
            "thin": "thin" in goal_lower,
            "sheet": "sheet" in goal_lower,
            "bracket": "bracket" in goal_lower,
            "fastener": "fastener" in goal_lower,
            "mount": "mount" in goal_lower,
            "plate": "plate" in goal_lower,
            "box": "box" in goal_lower,
            "enclosure": "enclosure" in goal_lower,
        }
    })
    
    return features
