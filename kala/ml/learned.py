"""Learned design context model with artifact-based rules."""

from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any

from kala.ml.base import DynamicContext
from kala.ml.features import extract_features
from kala.ml.stub import StubDesignContextModel
from kala.session.state import SessionState


class LearnedDesignContextModel:
    """Design context model that loads learned rules from artifacts.
    
    Falls back to StubDesignContextModel if artifact is missing or invalid.
    """
    
    def __init__(self) -> None:
        self._rules: dict[str, Any] | None = None
        self._fallback = StubDesignContextModel()
        self._load_artifact()
    
    def _load_artifact(self) -> None:
        """Load the learned rules artifact from disk."""
        artifact_path = Path(__file__).parent / "artifacts" / "context_v0.json"
        
        if not artifact_path.exists():
            return
        
        try:
            with artifact_path.open("r") as f:
                self._rules = json.load(f)
        except (json.JSONDecodeError, OSError):
            self._rules = None
    
    def enrich(self, state: SessionState) -> DynamicContext:
        """Enrich session state with dynamic context using learned rules.
        
        Falls back to stub model if rules are not loaded or invalid.
        """
        # Fallback if no rules loaded
        if self._rules is None:
            return self._fallback.enrich(state)
        
        # Extract features for rule matching
        features = extract_features(state)
        
        # Start with stub as base
        base_context = self._fallback.enrich(state)
        
        # Apply learned rules to augment context
        try:
            context = self._apply_rules(features, base_context)
            return context
        except Exception:  # noqa: BLE001
            # On any error, fall back to stub
            return base_context
    
    def _apply_rules(
        self,
        features: dict[str, Any],
        base_context: DynamicContext,
    ) -> DynamicContext:
        """Apply learned rules to augment the base context."""
        if self._rules is None:
            return base_context
        
        # Start with base context fields
        focus = base_context.focus
        constraints = list(base_context.constraints)
        manufacturing_notes = list(base_context.manufacturing_notes)
        recommended_tools = list(base_context.recommended_tools)
        candidate_parts = list(base_context.candidate_parts)
        warnings = list(base_context.warnings)
        snippets = list(base_context.snippets)
        
        # Apply constraint rules
        constraint_rules = self._rules.get("constraints", {})
        for rule_id, rule in constraint_rules.items():
            if self._matches_conditions(features, rule.get("conditions", [])):
                constraints.extend(rule.get("add", []))
        
        # Apply manufacturing note rules
        manufacturing_rules = self._rules.get("manufacturing_notes", {})
        for rule_id, rule in manufacturing_rules.items():
            if self._matches_conditions(features, rule.get("conditions", [])):
                manufacturing_notes.extend(rule.get("add", []))
        
        # Apply warning rules
        warning_rules = self._rules.get("warnings", {})
        for rule_id, rule in warning_rules.items():
            if self._matches_conditions(features, rule.get("conditions", [])):
                warnings.extend(rule.get("add", []))
        
        # Apply tool recommendation rules
        tool_rules = self._rules.get("recommended_tools", {})
        for rule_id, rule in tool_rules.items():
            if self._matches_conditions(features, rule.get("conditions", [])):
                tools_to_add = rule.get("add", [])
                for tool in tools_to_add:
                    if tool not in recommended_tools:
                        recommended_tools.append(tool)
        
        # Apply candidate parts rules
        parts_rules = self._rules.get("candidate_parts", {})
        for rule_id, rule in parts_rules.items():
            if self._matches_conditions(features, rule.get("conditions", [])):
                candidate_parts.extend(rule.get("add", []))
        
        # Apply focus override rules (optional)
        focus_rules = self._rules.get("focus", {})
        for rule_id, rule in focus_rules.items():
            if self._matches_conditions(features, rule.get("conditions", [])):
                focus_override = rule.get("override")
                if focus_override:
                    focus = focus_override
                    break
        
        # Apply snippet rules
        snippet_rules = self._rules.get("snippets", {})
        for rule_id, rule in snippet_rules.items():
            if self._matches_conditions(features, rule.get("conditions", [])):
                snippets.extend(rule.get("add", []))
        
        return DynamicContext(
            focus=focus,
            constraints=constraints,
            manufacturing_notes=manufacturing_notes,
            recommended_tools=recommended_tools,
            candidate_parts=candidate_parts,
            warnings=warnings,
            snippets=snippets,
        )
    
    def _matches_conditions(
        self,
        features: dict[str, Any],
        conditions: list[dict[str, Any]],
    ) -> bool:
        """Check if all conditions match the current features."""
        if not conditions:
            return True
        
        for condition in conditions:
            feature_path = condition.get("feature", "")
            operator = condition.get("op", "eq")
            value = condition.get("value")
            
            # Navigate nested feature path
            current = features
            for key in feature_path.split("."):
                if not isinstance(current, dict) or key not in current:
                    return False
                current = current[key]
            
            # Apply operator
            if operator == "eq":
                if current != value:
                    return False
            elif operator == "ne":
                if current == value:
                    return False
            elif operator == "gt":
                if not (isinstance(current, (int, float)) and current > value):
                    return False
            elif operator == "lt":
                if not (isinstance(current, (int, float)) and current < value):
                    return False
            elif operator == "gte":
                if not (isinstance(current, (int, float)) and current >= value):
                    return False
            elif operator == "lte":
                if not (isinstance(current, (int, float)) and current <= value):
                    return False
            elif operator == "contains":
                if not (isinstance(current, str) and value in current):
                    return False
            elif operator == "in":
                if current not in value:
                    return False
            else:
                return False
        
        return True
