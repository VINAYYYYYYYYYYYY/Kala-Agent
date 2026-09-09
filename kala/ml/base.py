"""Design ML context contracts — LLM-independent dynamic context."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Protocol, runtime_checkable

from kala.session.state import SessionState


@dataclass
class DynamicContext:
    """Structured assistive context injected into any planner/LLM turn."""

    focus: str
    constraints: list[str] = field(default_factory=list)
    manufacturing_notes: list[str] = field(default_factory=list)
    recommended_tools: list[str] = field(default_factory=list)
    candidate_parts: list[dict[str, Any]] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    snippets: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return {
            "focus": self.focus,
            "constraints": self.constraints,
            "manufacturing_notes": self.manufacturing_notes,
            "recommended_tools": self.recommended_tools,
            "candidate_parts": self.candidate_parts,
            "warnings": self.warnings,
            "snippets": self.snippets,
        }


@runtime_checkable
class DesignContextModel(Protocol):
    def enrich(self, state: SessionState) -> DynamicContext: ...
