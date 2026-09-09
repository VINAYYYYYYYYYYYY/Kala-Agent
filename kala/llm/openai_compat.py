"""OpenAI-compatible planner (OpenRouter / OpenAI / Groq / Ollama)."""

from __future__ import annotations

import json
import urllib.error
import urllib.request
from typing import Any

from kala.llm.base import PlannerTurn, ToolCall
from kala.llm.providers import ProviderConfig
from kala.llm.stub import StubPlanner
from kala.ml.base import DynamicContext
from kala.session.state import SessionState

_CREATE_TOOLS = frozenset(
    {"create_box", "create_cylinder", "create_sphere", "create_cone"}
)
_CUT_HINTS = ("hole", "bore", "cutout", "clearance", "through-hole", " thru ")
_FUSE_HINTS = (
    "fuse",
    "flange",
    "bracket",
    "join",
    "weld",
    "attach",
    "mounting",
    "tomb",
    "dome",
    "minaret",
    "building",
    "monument",
    "palace",
    "temple",
    "tower",
)


def _schemas_to_openai_tools(tool_schemas: list[dict[str, Any]]) -> list[dict[str, Any]]:
    tools = []
    for spec in tool_schemas:
        tools.append(
            {
                "type": "function",
                "function": {
                    "name": spec["name"],
                    "description": spec.get("description") or spec["name"],
                    "parameters": spec.get("parameters")
                    or {"type": "object", "properties": {}},
                },
            }
        )
    return tools


def _history_brief(state: SessionState) -> str:
    lines = []
    for e in state.history[-16:]:
        flag = "ok" if e.ok else "FAIL"
        bid = e.data.get("body_id")
        extra = f" body_id={bid}" if bid else ""
        msg = ""
        if not e.ok and e.message:
            msg = f" err={e.message.splitlines()[0][:80]}"
        lines.append(f"- [{flag}] {e.tool} {e.args}{extra}{msg}")
    return "\n".join(lines) if lines else "(no tools called yet)"


def _known_body_ids(state: SessionState) -> list[str]:
    """Active body ids only — drop any later marked removed by fuse/cut/fillet."""
    removed: set[str] = set()
    ids: list[str] = []
    for e in state.history:
        if not e.ok:
            continue
        for r in e.data.get("removed") or []:
            removed.add(str(r))
        bid = e.data.get("body_id")
        if bid:
            ids.append(str(bid))
    # Keep order, drop removed / duplicates (latest occurrence wins)
    out: list[str] = []
    seen: set[str] = set()
    for bid in reversed(ids):
        if bid in removed or bid in seen:
            continue
        seen.add(bid)
        out.append(bid)
    out.reverse()
    return out


def _create_count(state: SessionState) -> int:
    return sum(1 for e in state.history if e.ok and e.tool in _CREATE_TOOLS)


def _goal_needs_cut(goal: str) -> bool:
    g = f" {goal.lower()} "
    if any(k in g for k in _CUT_HINTS):
        return True
    # "M6 holes", "bolt holes", "4 holes"
    if "hole" in g or "holes" in g:
        return True
    return False


def _goal_needs_fuse(goal: str, state: SessionState) -> bool:
    g = goal.lower()
    # Multi-part assemblies should stay separate solids
    if _goal_is_assembly(g):
        return False
    if any(k in g for k in _FUSE_HINTS):
        return True
    # Multiple solids created → must fuse into one finished model
    return _create_count(state) >= 3


def _goal_is_assembly(goal_l: str) -> bool:
    return any(
        k in goal_l
        for k in (
            "assembly",
            "gearbox",
            "planetary",
            "actuator",
            "separate parts",
            "don't fuse",
            "do not fuse",
            "keep separate",
            "bom",
            "standard parts",
            "nema",
        )
    )


def _did_ok(state: SessionState, tool: str) -> bool:
    return any(e.tool == tool and e.ok for e in state.history)


def _count_consecutive_ok(state: SessionState, tool: str, from_end: int = 0) -> int:
    """Count consecutive ok calls of tool from history end (or from_end position back)."""
    count = 0
    start = len(state.history) - 1 - from_end
    for i in range(start, -1, -1):
        e = state.history[i]
        if e.tool == tool and e.ok:
            count += 1
        else:
            break
    return count


def _last_ok_search_parts(state: SessionState) -> list[dict] | None:
    """Return parts list from the most recent ok search_parts, or None."""
    for e in reversed(state.history):
        if e.ok and e.tool == "search_parts":
            return list((e.data or {}).get("parts") or [])
    return None


def _has_insert_after_last_search(state: SessionState) -> bool:
    """True if there's an ok insert_part after the most recent ok search_parts."""
    for e in reversed(state.history):
        if e.ok and e.tool == "insert_part":
            # Found insert before search → there's insert after last search
            return True
        if e.ok and e.tool == "search_parts":
            # Found search before insert → no insert after last search
            return False
    return False


class OpenAICompatPlanner:
    """Chat Completions + tools against an OpenAI-compatible endpoint."""

    def __init__(self, config: ProviderConfig, *, fallback: StubPlanner | None = None) -> None:
        self.config = config
        self.fallback = fallback or StubPlanner()

    def propose(
        self,
        state: SessionState,
        context: DynamicContext,
        tool_schemas: list[dict[str, Any]],
    ) -> PlannerTurn:
        step = state.current_step
        needs_cut = _goal_needs_cut(state.goal)
        needs_fuse = _goal_needs_fuse(state.goal, state)
        bodies = _known_body_ids(state)
        bodies_known = len(bodies) > 0

        # NEW POLICY: list_bodies stall-breaker
        # Max 1 ok list once bodies known; ≥2 consecutive ok lists → stub fallback
        consecutive_list = _count_consecutive_ok(state, "list_bodies")
        stalled_list = False
        if bodies_known and consecutive_list >= 2:
            stalled_list = True
        elif consecutive_list >= 2 and step and step.id == "features":
            # Also catch list-only turn while bodies known in features step
            stalled_list = True

        if stalled_list and (
            (needs_cut and not _did_ok(state, "boolean_cut"))
            or (needs_fuse and not _did_ok(state, "boolean_fuse"))
        ):
            turn = self.fallback.propose(state, context, tool_schemas)
            turn.thought = f"LLM stuck on list_bodies; stub: {turn.thought}"
            return turn

        # NEW POLICY: search_parts stall-breaker
        # Max 1 ok search per step without intervening ok insert_part using a hit from that search
        # If LLM proposes another search after ≥1 ok search without insert → force insert_part
        # This check will be done post-LLM in _sanitize_turn, but we still need pre-check for spam
        if not _has_insert_after_last_search(state):
            last_parts = _last_ok_search_parts(state)
            if last_parts:
                # There's a search without insert — we're at risk of stall
                # Let LLM propose, then sanitize in _sanitize_turn
                pass

        try:
            turn = self._propose_llm(state, context, tool_schemas)
            # Post-LLM sanitization
            return self._sanitize_turn(turn, state, context)
        except Exception as exc:  # noqa: BLE001
            turn = self.fallback.propose(state, context, tool_schemas)
            turn.thought = f"LLM planner failed ({exc}); stub: {turn.thought}"
            return turn

    def _sanitize_turn(
        self,
        turn: PlannerTurn,
        state: SessionState,
        context: DynamicContext,
    ) -> PlannerTurn:
        """Post-LLM sanitization: enforce stall-breaker policies on proposed calls."""
        bodies = _known_body_ids(state)
        bodies_known = len(bodies) > 0

        if not turn.calls:
            # Empty turn, check for list-only stall
            consecutive_list = _count_consecutive_ok(state, "list_bodies")
            if consecutive_list >= 2 and bodies_known:
                # List-only turn while bodies known → stub fallback
                stub_turn = self.fallback.propose(state, context, [])
                stub_turn.thought = f"List-only turn after {consecutive_list} lists; stub: {stub_turn.thought}"
                stub_turn.done = False
                return stub_turn
            return turn

        names = {c.name for c in turn.calls}

        # POLICY: search_parts rewrite
        # If LLM proposes search_parts AND there's already ≥1 ok search without insert → force insert_part
        if "search_parts" in names:
            if not _has_insert_after_last_search(state):
                last_parts = _last_ok_search_parts(state)
                if last_parts and state.standard_parts:
                    # Force insert with parts[0].part_id from most recent ok search at x=y=z=0
                    pid = str(last_parts[0].get("part_id") or "")
                    if pid:
                        turn.calls = [ToolCall("insert_part", {"part_id": pid, "x": 0, "y": 0, "z": 0})]
                        turn.thought = f"Search stall-breaker: inserting {pid} from prior search"
                        turn.advance_step = False
                        turn.done = False
                        return turn
                # Empty search or standard_parts=False → don't force insert
                # Remove search_parts to avoid repeat empty search
                if last_parts == [] or not state.standard_parts:
                    turn.calls = [c for c in turn.calls if c.name != "search_parts"]
                    if not turn.calls:
                        # Stub fallback
                        stub_turn = self.fallback.propose(state, context, [])
                        stub_turn.thought = f"Empty search fallback; stub: {stub_turn.thought}"
                        stub_turn.done = False
                        return stub_turn

        # POLICY: list_bodies rewrite
        # If LLM proposes list_bodies AND ≥2 consecutive ok lists OR list-only while bodies known → stub
        if "list_bodies" in names:
            consecutive_list = _count_consecutive_ok(state, "list_bodies")
            # Check if this is list-only turn
            is_list_only = names == {"list_bodies"}
            
            if consecutive_list >= 1 and bodies_known:
                # Already had 1 ok list with bodies known → this would be 2nd+ → stub fallback
                turn.calls = [c for c in turn.calls if c.name != "list_bodies"]
                if not turn.calls or is_list_only:
                    stub_turn = self.fallback.propose(state, context, [])
                    stub_turn.thought = f"List stall-breaker after {consecutive_list} lists; stub: {stub_turn.thought}"
                    stub_turn.done = False
                    return stub_turn

        return turn

    def _propose_llm(
        self,
        state: SessionState,
        context: DynamicContext,
        tool_schemas: list[dict[str, Any]],
    ) -> PlannerTurn:
        step = state.current_step
        step_txt = "complete"
        if step:
            step_txt = (
                f"{step.id}: {step.goal}. Allowed tools: {', '.join(step.allowed_tools)}"
            )

        bodies = _known_body_ids(state)
        bodies_txt = ", ".join(bodies[-20:]) if bodies else "(none yet — use returned body_id values)"
        needs_cut = _goal_needs_cut(state.goal)
        needs_fuse = _goal_needs_fuse(state.goal, state)
        n_create = _create_count(state)
        did_cut = _did_ok(state, "boolean_cut")
        did_fuse = _did_ok(state, "boolean_fuse")
        did_export = _did_ok(state, "export")

        checklist = [
            f"creates={n_create}",
            f"fuse={'done' if did_fuse else ('REQUIRED' if needs_fuse else 'optional')}",
            f"cut={'done' if did_cut else ('REQUIRED' if needs_cut else 'optional')}",
            f"export={'done' if did_export else 'REQUIRED before finish'}",
        ]

        system = (
            "You are Kala, a CAD design agent. Build geometry ONLY via the provided "
            "tools (FreeCAD). Never invent FreeCAD scripts or body_id names.\n"
            "COMPLETE MODEL workflow (follow in order):\n"
            "  1) create solids for each major piece (box/cylinder/sphere/cone)\n"
            "  2) translate/rotate pieces into final positions (mm)\n"
            "  3) boolean_fuse repeatedly until the design is ONE finished solid "
            "(mandatory when the goal is multi-part / architectural / flange+plate)\n"
            "  4) for holes/bores: create cutters → translate → boolean_cut into the "
            "fused body (use the Fuse_* / Cut_* body_id from the last result)\n"
            "  5) optional fillet on the finished body\n"
            "  6) export that finished body_id as STEP, then stop\n"
            "L-BRACKET / ANGLE BRACKET (critical — do NOT stack two flat plates):\n"
            "  create_box uses length=X, width=Y, height=Z from the origin.\n"
            "  Base = horizontal: length=W, width=D, height=T (thin in Z). Leave at origin.\n"
            "  Wall = VERTICAL: length=W, width=T (thin in Y), height=H (tall in Z). "
            "Leave at origin too — do NOT translate the wall in Z (it must share z=0..T "
            "with the base so the outer corner is a true L).\n"
            "  Fuse base+wall → L profile in the YZ view.\n"
            "  Base holes: cylinder along Z, translate to (x,y,-1), boolean_cut.\n"
            "  Wall holes through thickness: create_cylinder → rotate axis=x angle_deg=90 "
            "→ translate to (x, -1, z) → boolean_cut (cylinder axis must be Y).\n"
            "  Wrong: two boxes both with small height stacked in Z (slab).\n"
            "  Wrong: translating the wall by z=+T (lifts wall off the L corner).\n"
            "COMPLEX MONUMENTS (Taj Mahal etc.):\n"
            "  Platform (wide flat box) + central tomb box + dome (sphere overlapping tomb top) "
            "+ 4 minaret cylinders at the FOUR CORNERS "
            "(not mid-edges) + optional finial on dome.\n"
            "  Pieces MUST overlap in volume (sink minarets into the platform, overlap "
            "dome with tomb) — touching faces alone leaves multiple solids after fuse.\n"
            "  Fuse in a CHAIN using the latest Fuse_* each time. Keep ≤10 primitives.\n"
            "MULTI-PART MACHINES / GEARBOXES / ASSEMBLIES:\n"
            "  Do NOT fuse everything into one brick. Keep motor, gears, bearings, bolts "
            "as separate solids. search_parts THEN insert_part using the EXACT part_id "
            "string from search results (e.g. nema17_body, bearing_mr128, bearing_6709, "
            "hex_m3x8, gear_sun_nema17, gear_planet, gear_ring_internal). "
            "Never invent ids like NEMA17 or MR128. "
            "After at most ONE search per category, insert and translate — do not "
            "spam search_parts. Position planets at 90° around the sun. "
            "Export with body_id='ALL' when the layout is ready.\n"
            "RULES:\n"
            "- body_id MUST be copied exactly from tool results (never invent names)\n"
            "- After fuse/cut, donors are deleted — only use the new Fuse_*/Cut_* id\n"
            "- Several boolean_fuse calls in one batch must chain: each body_a = previous result\n"
            "- For boolean_cut: body_a = stock (latest Fuse_*/Cut_*), body_b = cutter cylinder "
            "that still exists; never reuse a deleted cutter id\n"
            "- Loose unfused solids are NOT a finished model — fuse them\n"
            "- Prefer a coarse valid solid + export over endless micro-feature retries "
            "(fillet/cut failures → skip that feature and continue)\n"
            "- create_cylinder needs radius + height (mm); do not pass diameter as radius by mistake "
            "or omit axes\n"
            "- Do NOT spam list_bodies; at most once when ids are unclear\n"
            "- Prefer several tool calls in one turn when possible\n"
            "- Never translate the same body twice to place copies; create a new solid per copy\n"
            "- Dimensions in the user goal are millimeters"
        )
        user = (
            f"Goal: {state.goal}\n"
            f"Backend: {state.backend_name}\n"
            f"Procedure step: {step_txt}\n"
            f"Progress: {', '.join(checklist)}\n"
            f"Known body_ids (use these exact strings): {bodies_txt}\n"
            f"Design context: {json.dumps(context.to_dict())}\n"
            f"Recent tool results:\n{_history_brief(state)}\n"
            "Call the next tool(s) now. If multiple solids exist and fuse is REQUIRED, "
            "boolean_fuse next. If holes are required and not cut, boolean_cut next. "
            "If the model is one finished solid, export it."
        )

        tools = _schemas_to_openai_tools(tool_schemas)
        if step and step.allowed_tools:
            allowed = set(step.allowed_tools)
            tools = [t for t in tools if t["function"]["name"] in allowed]

        base = self.config.base_url.rstrip("/")
        url = f"{base}/chat/completions"
        body = {
            "model": self.config.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "tools": tools,
            "tool_choice": "auto",
            "temperature": 0.1,
        }
        headers = {
            "Content-Type": "application/json",
            "User-Agent": "kala-agent/0.1",
        }
        if self.config.api_key.strip():
            headers["Authorization"] = f"Bearer {self.config.api_key.strip()}"
        if self.config.provider == "openrouter":
            headers["HTTP-Referer"] = "https://github.com/VINAYYYYYYYYYYYY/Kala-Agent"
            headers["X-Title"] = "Kala-Agent"

        req = urllib.request.Request(
            url,
            data=json.dumps(body).encode("utf-8"),
            headers=headers,
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=90) as resp:
                raw = json.loads(resp.read().decode("utf-8"))
        except urllib.error.HTTPError as exc:
            detail = exc.read().decode("utf-8", errors="replace")[:400]
            raise RuntimeError(f"HTTP {exc.code}: {detail}") from exc

        state.add_usage(raw.get("usage") if isinstance(raw, dict) else None)

        choice = (raw.get("choices") or [{}])[0]
        message = choice.get("message") or {}
        tool_calls = message.get("tool_calls") or []
        content = (message.get("content") or "").strip()

        calls: list[ToolCall] = []
        for tc in tool_calls:
            fn = tc.get("function") or {}
            name = fn.get("name")
            if not name:
                continue
            try:
                args = json.loads(fn.get("arguments") or "{}")
            except json.JSONDecodeError:
                args = {}
            if not isinstance(args, dict):
                args = {}
            calls.append(ToolCall(name=str(name), arguments=args))

        advance = False
        names = {c.name for c in calls}
        step_id = step.id if step else ""
        proposing_cut = "boolean_cut" in names
        proposing_fuse = "boolean_fuse" in names
        proposing_export = "export" in names
        cut_ok = did_cut or proposing_cut
        fuse_ok = did_fuse or proposing_fuse

        if calls:
            if step_id == "envelope":
                advance = bool(names & _CREATE_TOOLS)
            elif step_id == "features":
                # Leave features only once geometry is finished enough to export/fillet
                advance = proposing_export or "fillet" in names
                if needs_fuse and not fuse_ok:
                    advance = False
                if needs_cut and not cut_ok:
                    advance = False
            elif step_id == "standard_parts":
                advance = "insert_part" in names or proposing_export
            elif step_id == "export":
                advance = proposing_export
            else:
                advance = proposing_export

        done = False
        if proposing_export:
            # Loop will only honor done after export succeeds
            done = True
            advance = True
        elif not calls:
            if step_id == "features":
                ready = True
                if needs_fuse and not did_fuse:
                    ready = False
                if needs_cut and not did_cut:
                    ready = False
                if ready and (did_fuse or did_cut or n_create >= 1):
                    advance = True
            elif step_id == "standard_parts":
                advance = True
            elif step_id == "export":
                done = did_export
            elif step_id == "envelope":
                advance = True
            else:
                done = did_export

        # Hard locks: never finish incomplete models
        if step_id == "features":
            if needs_fuse and not fuse_ok:
                advance = False
                done = False
            if needs_cut and not cut_ok:
                advance = False
                done = False
        if proposing_export and needs_fuse and not fuse_ok and n_create >= 3:
            # Still allow export attempt, but prefer fuse first — drop export if fuse missing
            calls = [c for c in calls if c.name != "export"]
            if not calls:
                # Force a fuse between the two latest bodies when possible
                if len(bodies) >= 2:
                    calls = [
                        ToolCall(
                            "boolean_fuse",
                            {"body_a": bodies[-2], "body_b": bodies[-1]},
                        )
                    ]
                    names = {"boolean_fuse"}
            done = False
            advance = False

        if names == {"list_bodies"}:
            done = False
            if step_id == "features" and (
                (needs_cut and not did_cut) or (needs_fuse and not did_fuse)
            ):
                advance = False

        return PlannerTurn(
            thought=content or f"LLM ({self.config.provider}/{self.config.model})",
            calls=calls,
            advance_step=advance,
            done=done,
        )


def resolve_planner() -> StubPlanner | OpenAICompatPlanner:
    """Use active API provider when configured with a real key; else stub."""
    from kala.llm.providers import looks_like_api_key, load_store

    store = load_store()
    active = store.active()
    stub = StubPlanner()
    if active is None or not active.enabled:
        return stub
    if not active.model.strip() or not active.base_url.strip():
        return stub
    if active.provider == "ollama":
        return OpenAICompatPlanner(active, fallback=stub)
    if not looks_like_api_key(active.api_key, active.provider):
        return stub
    return OpenAICompatPlanner(active, fallback=stub)
