"""Rule-based planner — picks shape from the goal text (not always a box)."""

from __future__ import annotations

import re
from typing import Any

from kala.llm.base import PlannerTurn, ToolCall
from kala.ml.base import DynamicContext
from kala.session.state import SessionState


def _nums(goal: str) -> list[float]:
    return [float(n) for n in re.findall(r"(\d+(?:\.\d+)?)", goal)]


def _dims_box(goal: str) -> tuple[float, float, float]:
    nums = _nums(goal)
    if len(nums) >= 3:
        return nums[0], nums[1], nums[2]
    if len(nums) == 2:
        return nums[0], nums[1], max(nums) * 0.15
    if len(nums) == 1:
        return nums[0], nums[0], nums[0]
    return 80.0, 50.0, 8.0


def _cylinder_dims(goal: str) -> tuple[float, float]:
    nums = _nums(goal)
    g = goal.lower()
    if len(nums) >= 2:
        a, b = nums[0], nums[1]
        if "diameter" in g or "dia" in g:
            return a / 2.0, b
        if "radius" in g:
            return a, b
        # Assume diameter × height when two numbers
        return a / 2.0, b
    if len(nums) == 1:
        v = nums[0]
        if "diameter" in g:
            return v / 2.0, max(v * 2.0, 20.0)
        return v, max(v * 4.0, 20.0)
    return 10.0, 40.0


def _sphere_radius(goal: str) -> float:
    nums = _nums(goal)
    if not nums:
        return 15.0
    g = goal.lower()
    if "diameter" in g:
        return nums[0] / 2.0
    return nums[0]


def _pick_envelope(goal: str, allowed: set[str]) -> ToolCall | None:
    g = goal.lower()
    if any(k in g for k in ("sphere", "ball", "globe")) and "create_sphere" in allowed:
        return ToolCall("create_sphere", {"radius": _sphere_radius(goal)})
    if any(
        k in g
        for k in (
            "cylinder",
            "shaft",
            "pin",
            "rod",
            "pipe",
            "tube",
            "dowel",
            "axle",
        )
    ) and "create_cylinder" in allowed:
        r, h = _cylinder_dims(goal)
        return ToolCall("create_cylinder", {"radius": r, "height": h})
    if any(k in g for k in ("cone", "taper")) and "create_cone" in allowed:
        nums = _nums(goal)
        r1 = (nums[0] / 2.0) if nums else 12.0
        r2 = (nums[1] / 2.0) if len(nums) > 1 else 0.0
        h = nums[2] if len(nums) > 2 else (nums[1] if len(nums) > 1 else 30.0)
        return ToolCall(
            "create_cone", {"radius1": r1, "radius2": r2, "height": h}
        )
    if "create_box" in allowed:
        l, w, h = _dims_box(goal)
        return ToolCall("create_box", {"length": l, "width": w, "height": h})
    if "create_cylinder" in allowed:
        r, h = _cylinder_dims(goal)
        return ToolCall("create_cylinder", {"radius": r, "height": h})
    return None


class StubPlanner:
    def propose(
        self,
        state: SessionState,
        context: DynamicContext,
        tool_schemas: list[dict[str, Any]],
    ) -> PlannerTurn:
        step = state.current_step
        if step is None:
            return PlannerTurn(thought="Procedure complete.", done=True)

        available = {s["name"] for s in tool_schemas}
        allowed = set(step.allowed_tools) & available if step.allowed_tools else available
        done_create = {
            e.tool
            for e in state.history
            if e.ok
            and e.tool
            in {"create_box", "create_cylinder", "create_sphere", "create_cone"}
        }

        if step.id == "envelope":
            if not done_create:
                call = _pick_envelope(state.goal, allowed)
                if call:
                    return PlannerTurn(
                        thought=f"Envelope from goal → {call.name} {call.arguments}",
                        calls=[call],
                        advance_step=True,
                    )
            return PlannerTurn(thought="Envelope already present.", advance_step=True)

        if step.id == "features":
            bodies = [
                str(e.data.get("body_id"))
                for e in state.history
                if e.ok and e.data.get("body_id")
            ]
            body_id = bodies[-1] if bodies else None
            created = next(iter(done_create), "")
            g = state.goal.lower()
            creates = [
                e
                for e in state.history
                if e.ok
                and e.tool
                in {"create_box", "create_cylinder", "create_sphere", "create_cone"}
            ]
            # Multi-solid goals: fuse the last two unfinished pieces
            if (
                len(creates) >= 2
                and "boolean_fuse" in allowed
                and "boolean_fuse" not in {e.tool for e in state.history if e.ok}
                and len(bodies) >= 2
            ):
                return PlannerTurn(
                    thought="Fuse solids into one finished body.",
                    calls=[
                        ToolCall(
                            "boolean_fuse",
                            {"body_a": bodies[-2], "body_b": bodies[-1]},
                        )
                    ],
                )
            # Holes for brackets/plates: cut a cylinder if requested
            if (
                body_id
                and any(k in g for k in ("hole", "bore", "cutout", "clearance"))
                and "create_cylinder" in allowed
                and "boolean_cut" in allowed
                and "boolean_cut" not in {e.tool for e in state.history if e.ok}
            ):
                # create cutter then cut
                if "create_cylinder" not in {e.tool for e in state.history if e.ok}:
                    r, h = _cylinder_dims(state.goal)
                    r = min(r, 5.0)
                    return PlannerTurn(
                        thought="Create hole cutter cylinder.",
                        calls=[
                            ToolCall(
                                "create_cylinder",
                                {"radius": r, "height": h * 2 or 20.0, "label": "Hole"},
                            )
                        ],
                    )
                cutter = None
                for e in reversed(state.history):
                    if e.ok and e.tool == "create_cylinder" and e.data.get("body_id"):
                        cutter = e.data["body_id"]
                        break
                if cutter and body_id:
                    return PlannerTurn(
                        thought="Cut hole from envelope.",
                        calls=[
                            ToolCall(
                                "boolean_cut",
                                {"body_a": str(body_id), "body_b": str(cutter)},
                            )
                        ],
                        advance_step=True,
                    )
            if (
                body_id
                and created == "create_box"
                and "fillet" not in {e.tool for e in state.history if e.ok}
                and "fillet" in allowed
            ):
                return PlannerTurn(
                    thought="Light fillet on box envelope.",
                    calls=[ToolCall("fillet", {"body_id": body_id, "radius": 1.0})],
                    advance_step=True,
                )
            return PlannerTurn(thought="Feature step complete.", advance_step=True)

        if step.id == "standard_parts":
            if not state.standard_parts:
                return PlannerTurn(
                    thought="Standard parts disabled — skipping catalog.",
                    advance_step=True,
                )
            if "insert_part" in allowed and "insert_part" not in {
                e.tool for e in state.history if e.ok
            }:
                part_id = "hex_m6x20"
                if context.candidate_parts:
                    part_id = str(context.candidate_parts[0]["part_id"])
                return PlannerTurn(
                    thought=f"Insert standard part {part_id}.",
                    calls=[
                        ToolCall(
                            "insert_part",
                            {"part_id": part_id, "x": 0, "y": 0, "z": 10},
                        )
                    ],
                    advance_step=True,
                )
            return PlannerTurn(thought="Parts step complete.", advance_step=True)

        if step.id == "export":
            if "export" in {e.tool for e in state.history if e.ok}:
                return PlannerTurn(thought="Export finished.", done=True)
            body_id = None
            for event in reversed(state.history):
                if event.ok and event.data.get("body_id"):
                    body_id = event.data["body_id"]
                    break
            if body_id is None and "list_bodies" in allowed:
                return PlannerTurn(
                    thought="List bodies before export.",
                    calls=[ToolCall("list_bodies", {})],
                )
            if body_id and "export" in allowed:
                path = f"outputs/{state.procedure.id}_{body_id}.step"
                return PlannerTurn(
                    thought=f"Export {body_id}.",
                    calls=[
                        ToolCall(
                            "export",
                            {"body_id": body_id, "path": path, "fmt": "step"},
                        )
                    ],
                    advance_step=True,
                    done=True,
                )
            return PlannerTurn(thought="Nothing to export.", done=True)

        return PlannerTurn(thought=f"No stub policy for step {step.id}.", advance_step=True)
