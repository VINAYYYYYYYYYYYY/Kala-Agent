"""Planner (LLM) contracts — live providers deferred."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Protocol, runtime_checkable

from kala.ml.base import DynamicContext
from kala.session.state import SessionState


@dataclass
class ToolCall:
    name: str
    arguments: dict[str, Any] = field(default_factory=dict)


@dataclass
class PlannerTurn:
    thought: str
    calls: list[ToolCall] = field(default_factory=list)
    advance_step: bool = False
    done: bool = False


@runtime_checkable
class PlannerProtocol(Protocol):
    def propose(
        self,
        state: SessionState,
        context: DynamicContext,
        tool_schemas: list[dict[str, Any]],
    ) -> PlannerTurn: ...
