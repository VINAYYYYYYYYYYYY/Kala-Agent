"""Learned design context model with rule-based enrichment."""

import json
import logging
from pathlib import Path
from typing import Any

from kala.ml.base import DesignContextModel, DynamicContext
from kala.ml.features import extract_features
from kala.ml.stub import StubDesignContextModel

logger = logging.getLogger(__name__)


class LearnedDesignContextModel:
    """Rule-based context model loading hand-authored rules from artifacts.
    
    Falls back to StubDesignContextModel when artifact is missing or invalid.
    Implements CTX-0 enrichment via extract_features → rule evaluation → DynamicContext.
    """
    
    def __init__(self, artifact_path: Path | None = None) -> None:
        """Initialize learned model with artifact loading and fallback.
        
        Args:
            artifact_path: Optional override for artifact location.
                          Defaults to kala/ml/artifacts/context_v0.json.
        """
        self._fallback = StubDesignContextModel()
        self._rules: dict[str, Any] | None = None
        self._defaults: dict[str, Any] = {}
        
        if artifact_path is None:
            artifact_path = Path(__file__).parent / "artifacts" / "context_v0.json"
        
        self._load_artifact(artifact_path)
    
    def _load_artifact(self, path: Path) -> None:
        """Load and validate artifact JSON.
        
        Args:
            path: Path to context_v0.json artifact.
        """
        try:
            if not path.exists():
                logger.warning(f"Artifact not found: {path}, falling back to stub")
                return
            
            with open(path) as f:
                artifact = json.load(f)
            
            self._rules = artifact.get("rules")
            self._defaults = artifact.get("defaults", {})
            
            if not self._rules:
                logger.warning(f"Invalid artifact (missing rules): {path}, falling back to stub")
                self._rules = None
                
        except (json.JSONDecodeError, OSError) as e:
            logger.warning(f"Failed to load artifact {path}: {e}, falling back to stub")
            self._rules = None
    
    def enrich(self, state: dict[str, Any]) -> DynamicContext:
        """Enrich agent state using learned rules or fallback to stub.
        
        Args:
            state: Agent state dictionary.
            
        Returns:
            DynamicContext populated via rule evaluation or stub defaults.
        """
        if self._rules is None:
            return self._fallback.enrich(state)
        
        features = extract_features(state)
        
        focus = self._evaluate_focus(features)
        constraints_active = self._evaluate_constraints(features)
        export_ready = self._evaluate_export(features)
        search_density = self._evaluate_search_density(features)
        
        return DynamicContext(
            focus=focus,
            constraints_active=constraints_active,
            export_ready=export_ready,
            search_density=search_density,
        )
    
    def _evaluate_focus(self, features: dict[str, Any]) -> str:
        """Evaluate focus mode from features using rules.
        
        Args:
            features: Extracted feature dictionary.
            
        Returns:
            Focus mode string (exploration, refinement, validation).
        """
        focus_rules = self._rules.get("focus", {})
        
        for mode in ["validation", "refinement", "exploration"]:
            if mode in focus_rules:
                conditions = focus_rules[mode].get("conditions", [])
                if self._check_conditions(features, conditions):
                    return mode
        
        return self._defaults.get("focus", "exploration")
    
    def _evaluate_constraints(self, features: dict[str, Any]) -> bool:
        """Evaluate constraints_active flag from features.
        
        Args:
            features: Extracted feature dictionary.
            
        Returns:
            True if constraints are detected as active.
        """
        conditions = self._rules.get("constraints_active", {}).get("conditions", [])
        return self._check_conditions(features, conditions)
    
    def _evaluate_export(self, features: dict[str, Any]) -> bool:
        """Evaluate export_ready flag from features.
        
        Args:
            features: Extracted feature dictionary.
            
        Returns:
            True if export is ready.
        """
        conditions = self._rules.get("export_ready", {}).get("conditions", [])
        return self._check_conditions(features, conditions)
    
    def _evaluate_search_density(self, features: dict[str, Any]) -> float:
        """Evaluate search_density metric from features.
        
        Args:
            features: Extracted feature dictionary.
            
        Returns:
            Density value [0.0, 1.0].
        """
        density_rules = self._rules.get("search_density", {})
        
        for level in ["high", "medium", "low"]:
            if level in density_rules:
                level_spec = density_rules[level]
                conditions = level_spec.get("conditions", [])
                if self._check_conditions(features, conditions):
                    return level_spec.get("value", 0.0)
        
        return self._defaults.get("search_density", 0.0)
    
    def _check_conditions(self, features: dict[str, Any], conditions: list[dict[str, Any]]) -> bool:
        """Check if all conditions match features (AND logic).
        
        Args:
            features: Extracted feature dictionary.
            conditions: List of condition specs with feature, op, value.
            
        Returns:
            True if all conditions pass.
        """
        if not conditions:
            return False
        
        for condition in conditions:
            feature_name = condition.get("feature")
            op = condition.get("op")
            expected = condition.get("value")
            
            if feature_name not in features:
                return False
            
            actual = features[feature_name]
            
            if not self._check_op(actual, op, expected, condition):
                return False
        
        return True
    
    def _check_op(
        self, 
        actual: Any, 
        op: str, 
        expected: Any, 
        condition: dict[str, Any]
    ) -> bool:
        """Check single condition operator.
        
        Args:
            actual: Feature value.
            op: Operator string (eq, lt, gte, contains, has_any, range).
            expected: Expected value for comparison.
            condition: Full condition spec (may contain min/max for range).
            
        Returns:
            True if condition passes.
        """
        if op == "eq":
            return actual == expected
        elif op == "lt":
            return actual < expected
        elif op == "lte":
            return actual <= expected
        elif op == "gt":
            return actual > expected
        elif op == "gte":
            return actual >= expected
        elif op == "contains":
            return expected in actual if isinstance(actual, (list, str)) else False
        elif op == "has_any":
            if not isinstance(actual, list) or not isinstance(expected, list):
                return False
            return any(item in actual for item in expected)
        elif op == "range":
            min_val = condition.get("min", float("-inf"))
            max_val = condition.get("max", float("inf"))
            return min_val <= actual <= max_val
        
        return False
