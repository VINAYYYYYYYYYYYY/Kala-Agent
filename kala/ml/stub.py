"""Heuristic design-context stub until a real ML model is trained."""

from __future__ import annotations

from kala.ml.base import DynamicContext
from kala.session.state import SessionState


class StubDesignContextModel:
    def enrich(self, state: SessionState) -> DynamicContext:
        step = state.current_step
        if step is None:
            return DynamicContext(focus="Procedure complete — review exports.")

        focus = f"[{step.id}] {step.goal}"
        constraints = [
            "Prefer parametric solids over freehand sketches for this prototype.",
            f"Stay within allowed tools: {', '.join(step.allowed_tools) or 'any'}.",
        ]
        notes = [
            "Keep wall thickness manufacturable.",
            "Prefer through-holes on standard pitch when fasteners are enabled.",
        ]
        warnings: list[str] = []
        candidates: list[dict] = []
        snippets = [
            f"Procedure: {state.procedure.name}",
            f"Exit when: {step.exit_criteria}",
        ]

        if step.optional_parts and not state.standard_parts:
            warnings.append("Standard-parts step active but toggle is OFF — skip insert_part.")
            snippets.append("Continue without catalog parts.")
        elif step.optional_parts and state.standard_parts:
            candidates = [
                {"part_id": "hex_m6x20", "reason": "Common bracket fastener"},
                {"part_id": "hex_m8x25", "reason": "Heavier mount option"},
            ]
            snippets.append("Prefer catalog fasteners over custom bolt geometry.")

        goal_l = state.goal.lower()
        if "thin" in goal_l or "sheet" in goal_l:
            warnings.append("Sheet-metal style goals may need thinner envelopes.")
        if step.id == "export" and not any(e.tool == "export" and e.ok for e in state.history):
            warnings.append("No successful export yet.")

        return DynamicContext(
            focus=focus,
            constraints=constraints,
            manufacturing_notes=notes,
            recommended_tools=list(step.allowed_tools),
            candidate_parts=candidates,
            warnings=warnings,
            snippets=snippets,
        )
