"""Feature extraction from agent state for context model inference."""

from typing import Any


def extract_features(state: dict[str, Any]) -> dict[str, Any]:
    """Extract feature dictionary from agent state for context enrichment.
    
    Extracts counts, patterns, and signals used by LearnedDesignContextModel
    to populate DynamicContext fields.
    
    Args:
        state: Agent state dictionary with history, tools, step_id, etc.
        
    Returns:
        Feature dictionary with extracted signals:
        - body_count: Number of geometry bodies created
        - tool_count: Total tools invoked
        - fail_count: Failed tool invocations
        - step_id: Current step number
        - search_hits: Search_parts result count
        - last_n_tools: Recent tool names (up to 5)
        - export_present: Boolean export file detection
        - analysis_keys: Optional analysis_by_body metric keys
    """
    features: dict[str, Any] = {}
    
    # Basic counts
    history = state.get("history", [])
    features["body_count"] = len([
        entry for entry in history 
        if entry.get("type") == "body_created"
    ])
    
    tools_used = state.get("tools_used", [])
    features["tool_count"] = len(tools_used)
    
    failed_tools = state.get("failed_tools", [])
    features["fail_count"] = len(failed_tools)
    
    # Step progression
    features["step_id"] = state.get("step_id", 0)
    
    # Search_parts hit detection
    search_parts = state.get("search_parts", {})
    features["search_hits"] = sum(len(v) for v in search_parts.values()) if isinstance(search_parts, dict) else 0
    
    # Recent tool sequence (last 5)
    features["last_n_tools"] = [
        tool.get("name") for tool in tools_used[-5:]
        if isinstance(tool, dict) and "name" in tool
    ]
    
    # Export detection
    features["export_present"] = state.get("export_present", False)
    
    # Optional analysis metrics
    analysis_by_body = state.get("analysis_by_body", {})
    if analysis_by_body:
        features["analysis_keys"] = list(analysis_by_body.keys())
    
    return features
