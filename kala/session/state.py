"""Agent session state."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from kala.procedures.schema import Procedure


@dataclass
class ToolEvent:
    tool: str
    args: dict[str, Any]
    ok: bool
    message: str
    data: dict[str, Any] = field(default_factory=dict)


@dataclass
class SessionState:
    goal: str
    backend_name: str
    standard_parts: bool
    procedure: Procedure
    step_index: int = 0
    history: list[ToolEvent] = field(default_factory=list)
    last_export: str | None = None
    live_document: str | None = None
    gui_note: str | None = None
    status: str = "idle"
    error: str | None = None
    # Accumulated LLM usage across planner calls this run
    prompt_tokens: int = 0
    completion_tokens: int = 0
    total_tokens: int = 0
    llm_calls: int = 0
    analysis_by_body: dict[str, Any] = field(default_factory=dict)
    # Persistent body_id aliases across planner turns
    id_aliases: dict[str, str] = field(default_factory=dict)
    # Decompose-or-clarify gate (set when status is needs_clarify / part_plan)
    clarify: dict[str, Any] | None = None
    part_plan: dict[str, Any] | None = None
    # Sequential per-part playbook results (session metadata, not CAD Protocol)
    part_runs: list[dict[str, Any]] = field(default_factory=list)
    # local_name → resolved live body_id after each part playbook finishes
    part_body_map: dict[str, str] = field(default_factory=dict)

    def add_usage(self, usage: dict[str, Any] | None) -> None:
        if not usage:
            return
        pt = int(usage.get("prompt_tokens") or usage.get("input_tokens") or 0)
        ct = int(usage.get("completion_tokens") or usage.get("output_tokens") or 0)
        tt = int(usage.get("total_tokens") or (pt + ct))
        self.prompt_tokens += pt
        self.completion_tokens += ct
        self.total_tokens += tt
        self.llm_calls += 1

    @property
    def current_step(self):
        return self.procedure.step(self.step_index)

    def advance_step(self) -> None:
        if self.step_index < len(self.procedure.steps) - 1:
            self.step_index += 1

    def to_dict(self) -> dict[str, Any]:
        step = self.current_step
        return {
            "goal": self.goal,
            "backend_name": self.backend_name,
            "standard_parts": self.standard_parts,
            "status": self.status,
            "error": self.error,
            "last_export": self.last_export,
            "live_document": self.live_document,
            "gui_note": self.gui_note,
            "usage": {
                "prompt_tokens": self.prompt_tokens,
                "completion_tokens": self.completion_tokens,
                "total_tokens": self.total_tokens,
                "llm_calls": self.llm_calls,
            },
            "procedure": {
                "id": self.procedure.id,
                "name": self.procedure.name,
                "step_index": self.step_index,
                "step": None
                if step is None
                else {
                    "id": step.id,
                    "goal": step.goal,
                    "allowed_tools": step.allowed_tools,
                    "exit_criteria": step.exit_criteria,
                },
            },
            "history": [
                {
                    "tool": e.tool,
                    "args": e.args,
                    "ok": e.ok,
                    "message": e.message,
                    "data": e.data,
                }
                for e in self.history
            ],
            "analysis_by_body": self.analysis_by_body,
            "id_aliases": self.id_aliases,
            "clarify": self.clarify,
            "part_plan": self.part_plan,
            "part_runs": self.part_runs,
            "part_body_map": self.part_body_map,
        }
