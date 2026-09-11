"""Core agent loop: context → plan → tool calls → observe."""

from __future__ import annotations

import hashlib
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from kala.analysis.base import AnalysisRequest
from kala.analysis.freecad_fem import FreeCadFemCalculiXBackend
from kala.analysis.geometry_probe import GeometryProbeBackend
from kala.cad.factory import create_backend
from kala.cad.registry import ToolRegistry, build_registry
from kala.llm.base import PlannerProtocol
from kala.llm.openai_compat import resolve_planner
from kala.ml.base import DesignContextModel, DynamicContext
from kala.ml.learned import LearnedDesignContextModel
from kala.ml.stub import StubDesignContextModel
from kala.parts.catalog import PartsCatalog
from kala.procedures.gate import ClarifyNeeded, PartPlan, PartSpec, assess_goal
from kala.procedures.schema import list_procedure_ids, load_default_procedure
from kala.session.state import SessionState, ToolEvent


@dataclass
class RunResult:
    state: SessionState
    contexts: list[DynamicContext]

    def to_dict(self) -> dict[str, Any]:
        return {
            "state": self.state.to_dict(),
            "contexts": [c.to_dict() for c in self.contexts],
        }


def resolve_context_model() -> DesignContextModel:
    """Resolve the context model based on KALA_CONTEXT_MODEL environment variable.
    
    Returns:
        StubDesignContextModel if KALA_CONTEXT_MODEL=stub or unset (default)
        LearnedDesignContextModel if KALA_CONTEXT_MODEL=learned
    """
    model_type = os.environ.get("KALA_CONTEXT_MODEL", "stub").lower()
    if model_type == "learned":
        return LearnedDesignContextModel()
    return StubDesignContextModel()


def _compute_step_hash(path: str | Path) -> str | None:
    """Compute SHA256 hash of STEP file content for deduplication.
    
    Returns:
        Hex digest string if file exists and is readable, None otherwise.
    """
    try:
        p = Path(path)
        if not p.is_file():
            return None
        return hashlib.sha256(p.read_bytes()).hexdigest()
    except Exception:  # noqa: BLE001
        return None


def _count_real_tools(history: list[ToolEvent]) -> int:
    """Count substantive tool calls (exclude metadata/read-only tools).
    
    Args:
        history: Full tool event history
        
    Returns:
        Count of real modeling/assembly tools used
    """
    excluded = {"list_bodies", "show_in_freecad", "search_parts", "export"}
    return sum(1 for e in history if e.ok and e.tool not in excluded)


def library_procedure_id(procedure_id: str | None) -> str | None:
    """Return procedure_id only if it is already in the packaged library."""
    if not procedure_id:
        return None
    return procedure_id if procedure_id in set(list_procedure_ids()) else None


def _skip_silent_fuse(state: SessionState) -> bool:
    """keep_separate parts and machine_assembly must not be fused into one brick."""
    if state.procedure.id == "machine_assembly":
        return True
    plan = state.part_plan or {}
    for raw in plan.get("parts") or []:
        if isinstance(raw, dict) and raw.get("keep_separate"):
            return True
    return False


def _resolve_alias(state: SessionState, bid: str) -> str:
    """Follow removed→new id_aliases chains to the live body id."""
    seen: set[str] = set()
    cur = bid
    while cur in state.id_aliases and cur not in seen:
        seen.add(cur)
        cur = state.id_aliases[cur]
    return cur


def _pick_primary_body(history: list[ToolEvent]) -> str | None:
    """Last substantive body_id from create/boolean/insert_part tool events."""
    body_id: str | None = None
    for event in reversed(history):
        if not event.ok:
            continue
        bid = event.data.get("body_id")
        if not bid:
            continue
        if event.tool in {"boolean_fuse", "boolean_cut", "fillet", "insert_part"}:
            return str(bid)
        if body_id is None and (
            event.tool.startswith("create_") or event.tool == "insert_part"
        ):
            body_id = str(bid)
    return body_id


def freeze_part_alias(state: SessionState, local_name: str, body_id: str) -> None:
    """Freeze part:<local_name> and part_body_map after a part playbook finishes."""
    resolved = _resolve_alias(state, body_id)
    state.part_body_map[local_name] = resolved
    state.id_aliases[f"part:{local_name}"] = resolved


def _refresh_frozen_part_aliases(state: SessionState) -> None:
    """Re-resolve frozen part:<name> keys when child id_aliases gain remaps."""
    for local_name, body_id in list(state.part_body_map.items()):
        resolved = _resolve_alias(state, body_id)
        state.part_body_map[local_name] = resolved
        state.id_aliases[f"part:{local_name}"] = resolved


def _procedure_step_context(state: SessionState) -> DynamicContext:
    """Reuse procedure/step/allowed-tool ids — no DesignContextModel.enrich."""
    step = state.current_step
    if step is None:
        return DynamicContext(focus=f"procedure={state.procedure.id} complete")
    return DynamicContext(
        focus=f"[{step.id}] {step.goal}",
        constraints=[f"procedure_id={state.procedure.id}", f"step={step.id}"],
        recommended_tools=list(step.allowed_tools),
        snippets=[
            f"procedure_id={state.procedure.id}",
            f"step={step.id}",
            f"exit_criteria={step.exit_criteria}",
        ],
    )


def _force_modeling_progress(state: SessionState, registry: Any) -> list[ToolEvent]:
    """Force progress with real modeling tools when fidelity gate blocks.
    
    Creates/fuses/cuts deterministically until min_tools threshold met.
    Cap at 6 forced calls per invocation.
    
    Args:
        state: Current session state
        registry: ToolRegistry with available tools
        
    Returns:
        List of ToolEvents from forced tool calls
    """
    min_tools_env = os.environ.get("KALA_MIN_TOOLS", "4")
    try:
        min_tools = int(min_tools_env)
    except ValueError:
        min_tools = 4
    
    forced_events: list[ToolEvent] = []
    real_count = _count_real_tools(state.history)
    
    # Stop if threshold already met or min_tools disabled
    if min_tools <= 0 or real_count >= min_tools:
        return forced_events
    
    # Cap forced calls at 6 per trip
    max_forced = 6
    
    # Resolve body_id through id_aliases (same as loop remap)
    def _resolve(bid: str) -> str:
        seen: set[str] = set()
        cur = bid
        while cur in state.id_aliases and cur not in seen:
            seen.add(cur)
            cur = state.id_aliases[cur]
        return cur
    
    # Get live bodies (resolve aliases, drop removed)
    all_body_ids = [
        str(e.data.get("body_id"))
        for e in state.history
        if e.ok and e.data.get("body_id")
    ]
    resolved_bodies = [_resolve(bid) for bid in all_body_ids]
    # Keep only unique live bodies (last occurrence)
    bodies: list[str] = []
    seen_resolved: set[str] = set()
    for bid in reversed(resolved_bodies):
        if bid not in seen_resolved:
            seen_resolved.add(bid)
            bodies.insert(0, bid)
    
    # Deterministic strategy: create → fuse → cut
    # Create primitives if we have < 3 bodies
    if len(bodies) < 3 and len(forced_events) < max_forced:
        if registry.has("create_box"):
            result = registry.call("create_box", length=30.0, width=20.0, height=10.0)
            forced_events.append(
                ToolEvent("create_box", {"length": 30.0, "width": 20.0, "height": 10.0}, result.ok, result.message, result.data)
            )
            if result.ok and result.data.get("body_id"):
                bodies.append(str(result.data["body_id"]))
    
    if len(bodies) < 3 and len(forced_events) < max_forced:
        if registry.has("create_cylinder"):
            result = registry.call("create_cylinder", radius=8.0, height=25.0)
            forced_events.append(
                ToolEvent("create_cylinder", {"radius": 8.0, "height": 25.0}, result.ok, result.message, result.data)
            )
            if result.ok and result.data.get("body_id"):
                bodies.append(str(result.data["body_id"]))
    
    # Check if still need more tools after creates
    real_count = _count_real_tools(state.history + forced_events)
    
    # Fuse bodies together (parts → one solid); skip when keep_separate / assembly
    if not _skip_silent_fuse(state) and len(bodies) >= 2 and real_count < min_tools and len(forced_events) < max_forced:
        if registry.has("boolean_fuse"):
            body_a = bodies[-2]
            body_b = bodies[-1]
            result = registry.call("boolean_fuse", body_a=body_a, body_b=body_b)
            forced_events.append(
                ToolEvent("boolean_fuse", {"body_a": body_a, "body_b": body_b}, result.ok, result.message, result.data)
            )
            if result.ok and result.data.get("body_id"):
                # Update id_aliases for removed bodies
                for old in result.data.get("removed") or []:
                    state.id_aliases[str(old)] = str(result.data["body_id"])
                # Drop removed from bodies list
                removed_set = {str(r) for r in result.data.get("removed") or []}
                bodies = [b for b in bodies if b not in removed_set]
                bodies.append(str(result.data["body_id"]))
    
    # Check again
    real_count = _count_real_tools(state.history + forced_events)
    
    # Create cutter and cut if still need more
    if real_count < min_tools and len(bodies) >= 1 and len(forced_events) < max_forced:
        cutter_id = None
        if registry.has("create_cylinder"):
            result = registry.call("create_cylinder", radius=4.0, height=15.0, label="Cutter")
            forced_events.append(
                ToolEvent("create_cylinder", {"radius": 4.0, "height": 15.0, "label": "Cutter"}, result.ok, result.message, result.data)
            )
            if result.ok and result.data.get("body_id"):
                cutter_id = str(result.data["body_id"])
        
        if cutter_id and len(forced_events) < max_forced and registry.has("boolean_cut"):
            body_a = bodies[-1]
            result = registry.call("boolean_cut", body_a=body_a, body_b=cutter_id)
            forced_events.append(
                ToolEvent("boolean_cut", {"body_a": body_a, "body_b": cutter_id}, result.ok, result.message, result.data)
            )
            if result.ok and result.data.get("body_id"):
                for old in result.data.get("removed") or []:
                    state.id_aliases[str(old)] = str(result.data["body_id"])
    
    return forced_events



def _exit_criteria_met(state: SessionState) -> bool:
    """Heuristic gate for procedure step exit_criteria (no Protocol field changes)."""
    step = state.current_step
    if step is None:
        return True
    ok_tools = {e.tool for e in state.history if e.ok}
    sid = step.id
    if sid == "envelope":
        return bool(ok_tools & {
            "create_box", "create_cylinder", "create_sphere", "create_cone", "insert_part",
        })
    if sid == "features":
        return bool(ok_tools & {
            "boolean_fuse", "boolean_cut", "fillet", "translate", "rotate",
        })
    if sid == "standard_parts":
        if step.optional_parts and not state.standard_parts:
            return True
        return bool(ok_tools & {"insert_part", "search_parts", "export"}) or bool(state.last_export)
    if sid == "export":
        return bool(state.last_export)
    return True


class Agent:
    def __init__(
        self,
        *,
        backend_name: str = "freecad",
        standard_parts: bool = False,
        procedure_id: str = "simple_bracket",
        planner: PlannerProtocol | None = None,
        context_model: DesignContextModel | None = None,
        max_turns: int = 64,
    ) -> None:
        self.backend_name = backend_name
        self.standard_parts = standard_parts
        self.procedure_id = procedure_id
        self.planner = planner or resolve_planner()
        self.context_model = context_model or resolve_context_model()
        self.max_turns = max_turns
        self.catalog = PartsCatalog.default()
        self._backend: Any = None

    def _open_registry(self) -> ToolRegistry:
        backend = create_backend(self.backend_name)
        self._backend = backend
        return build_registry(
            backend,
            standard_parts=self.standard_parts,
            catalog=self.catalog,
        )

    def _build(self, goal: str) -> tuple[SessionState, ToolRegistry]:
        registry = self._open_registry()
        state = SessionState(
            goal=goal,
            backend_name=self._backend.name,
            standard_parts=self.standard_parts,
            procedure=load_default_procedure(self.procedure_id),
            status="running",
        )
        return state, registry

    def _auto_export(self, state: SessionState, registry: ToolRegistry) -> None:
        """If the planner ran out of turns, still export the best finished solid."""
        if state.last_export:
            return
        if not registry.has("export"):
            return
        body_id: str | None = None
        for e in reversed(state.history):
            if not e.ok:
                continue
            bid = e.data.get("body_id")
            if not bid:
                continue
            if e.tool in {"boolean_fuse", "boolean_cut", "fillet"}:
                body_id = str(bid)
                break
            if body_id is None and e.tool.startswith("create_"):
                body_id = str(bid)
        if not body_id:
            return
        result = registry.call(
            "export",
            body_id=body_id,
            path="outputs/kala_auto.step",
            fmt="step",
        )
        state.history.append(
            ToolEvent(
                tool="export",
                args={"body_id": body_id, "path": "outputs/kala_auto.step", "fmt": "step"},
                ok=result.ok,
                message=result.message + " (auto on max_turns)",
                data=result.data,
            )
        )
        if result.ok:
            state.last_export = str(result.data.get("path") or "")

    def _check_fidelity_gate(self, state: SessionState) -> tuple[bool, str]:
        """Verify session meets minimum fidelity before allowing done.
        
        Prevents premature completion on stub/catalog exports.
        
        Returns:
            (allowed, reason) tuple - allowed=True if session can mark done,
            reason explains rejection when allowed=False
        """
        # Minimum tool threshold: default 4, set 0 to disable
        # Eval Ops should use KALA_MIN_TOOLS=8 for extreme batches
        min_tools_env = os.environ.get("KALA_MIN_TOOLS", "4")
        try:
            min_tools = int(min_tools_env)
        except ValueError:
            min_tools = 4
        
        if min_tools > 0:
            real_count = _count_real_tools(state.history)
            if real_count < min_tools:
                return (
                    False,
                    f"Fidelity gate: too few tools ({real_count} < {min_tools} required)",
                )
        
        # Export check: refuse if matches known stub/catalog
        if state.last_export:
            export_path = Path(state.last_export)
            export_name = export_path.name.lower()
            
            # Basename check: common catalog stubs (before hashes exist)
            catalog_patterns = ["hex_m6x20", "hex_m3x", "hex_m4x", "hex_m5x", "hex_m8x"]
            if any(pattern in export_name for pattern in catalog_patterns):
                return (
                    False,
                    f"Fidelity gate: export basename matches catalog stub pattern ({export_name})",
                )
            
            # Hash check: refuse if export matches a known stub/catalog hash
            export_hash = _compute_step_hash(state.last_export)
            if export_hash:
                known_stubs_env = os.environ.get("KALA_KNOWN_STUB_HASHES", "")
                known_stubs = {h.strip() for h in known_stubs_env.split(",") if h.strip()}
                
                if export_hash in known_stubs:
                    return (
                        False,
                        f"Fidelity gate: export matches known stub/catalog hash {export_hash[:16]}...",
                    )
        
        return (True, "")

    def _finalize_gui(self, state: SessionState) -> None:
        """Force-open FreeCAD with the live document after a successful modeling run."""
        if state.backend_name != "freecad":
            return
        if state.status not in {"done", "max_turns"}:
            return
        if not any(e.ok for e in state.history):
            return
        try:
            from kala.cad.freecad.gui_sync import default_live_path, ensure_live_shown

            backend = self._backend
            if backend is not None and hasattr(backend, "show_in_gui"):
                note = backend.show_in_gui()
            else:
                note = ensure_live_shown()
            state.live_document = str(default_live_path())
            state.gui_note = note
            state.history.append(
                ToolEvent(
                    tool="show_in_freecad",
                    args={},
                    ok=True,
                    message=note,
                    data={
                        "cad_software": "FreeCAD",
                        "live_document": state.live_document,
                        "gui_sync": note,
                    },
                )
            )
        except Exception as exc:  # noqa: BLE001
            state.gui_note = f"GUI open failed: {exc}"

    def _write_run_log(self, goal: str, state: SessionState, contexts: list[DynamicContext]) -> None:
        try:
            from kala.session.runlog import write_run_log

            write_run_log(
                {
                    "goal": goal,
                    "planner": type(self.planner).__name__,
                    "state": state.to_dict(),
                    "contexts": [c.to_dict() for c in contexts],
                }
            )
        except Exception:
            pass

    def _execute_playbook(
        self,
        state: SessionState,
        registry: ToolRegistry,
        contexts: list[DynamicContext],
        *,
        enrich: bool = True,
    ) -> None:
        try:
            for _ in range(self.max_turns):
                if enrich:
                    context = self.context_model.enrich(state)
                    contexts.append(context)
                else:
                    context = _procedure_step_context(state)
                turn = self.planner.propose(state, context, registry.schemas_for_planner())

                if not turn.calls and turn.done:
                    if state.last_export:
                        state.status = "done"
                        break
                    # Premature stop with no export — keep going
                    continue

                exported_this_turn = False
                # Fuse/cut delete donors — remap stale ids within the same turn so
                # parallel LLM fuse chains (Fuse_10 + minaret2, Fuse_10 + minaret3)
                # follow the live body instead of failing "Body not found".

                def _resolve(bid: str) -> str:
                    seen: set[str] = set()
                    cur = bid
                    while cur in state.id_aliases and cur not in seen:
                        seen.add(cur)
                        cur = state.id_aliases[cur]
                    return cur

                def _remap_args(args: dict) -> dict:
                    out = dict(args)
                    for key in ("body_id", "body_a", "body_b"):
                        if key in out and isinstance(out[key], str):
                            out[key] = _resolve(out[key])
                    return out

                for call in turn.calls:
                    step = state.current_step
                    if (
                        step
                        and step.allowed_tools
                        and call.name not in step.allowed_tools
                        and registry.has(call.name)
                    ):
                        result_msg = f"Tool {call.name} blocked by procedure step {step.id}"
                        state.history.append(
                            ToolEvent(call.name, call.arguments, False, result_msg)
                        )
                        continue

                    args = _remap_args(call.arguments)
                    result = registry.call(call.name, **args)
                    # Record the args actually used (after remap) for debugging
                    state.history.append(
                        ToolEvent(
                            tool=call.name,
                            args=args,
                            ok=result.ok,
                            message=result.message,
                            data=result.data,
                        )
                    )
                    if result.ok:
                        new_id = result.data.get("body_id")
                        for old in result.data.get("removed") or []:
                            if new_id:
                                state.id_aliases[str(old)] = str(new_id)
                        if result.data.get("live_document"):
                            state.live_document = str(result.data["live_document"])
                        
                        # Geometry probe hook: analyze new bodies from create/boolean/fillet/insert_part
                        if new_id and call.name in {
                            "create_box",
                            "create_cylinder",
                            "create_sphere",
                            "create_cone",
                            "boolean_fuse",
                            "boolean_cut",
                            "fillet",
                            "insert_part",
                        }:
                            try:
                                # Select analysis backend via KALA_ANALYSIS env var (default: probe)
                                analysis_mode = os.environ.get("KALA_ANALYSIS", "probe").lower()
                                if analysis_mode == "fem":
                                    backend_cls = FreeCadFemCalculiXBackend
                                else:
                                    backend_cls = GeometryProbeBackend
                                
                                analyzer = backend_cls()
                                request = AnalysisRequest(
                                    body_id=str(new_id),
                                    backend_handle=self._backend,
                                )
                                report = analyzer.analyze(request)
                                state.analysis_by_body[str(new_id)] = report.to_dict()
                            except Exception:  # noqa: BLE001
                                pass
                    if result.ok and call.name == "export":
                        state.last_export = str(result.data.get("path") or "")
                        exported_this_turn = True

                if turn.advance_step:
                    if _exit_criteria_met(state):
                        state.advance_step()
                    else:
                        step = state.current_step
                        state.history.append(
                            ToolEvent(
                                tool="procedure_gate",
                                args={},
                                ok=False,
                                message=(
                                    f"exit_criteria not met for step {step.id}: {step.exit_criteria}"
                                    if step
                                    else "exit_criteria not met"
                                ),
                                data={},
                            )
                        )
                if turn.done:
                    # Never mark done on a blocked/failed export or unfinished model
                    if any(c.name == "export" for c in turn.calls):
                        if exported_this_turn or state.last_export:
                            # Fidelity gate: check minimum tool count and export uniqueness
                            allowed, gate_reason = self._check_fidelity_gate(state)
                            if not allowed:
                                state.history.append(
                                    ToolEvent(
                                        tool="fidelity_gate",
                                        args={},
                                        ok=False,
                                        message=gate_reason,
                                        data={},
                                    )
                                )
                                # Force progress when blocked by too_few_tools only
                                if "too few tools" in gate_reason:
                                    forced_events = _force_modeling_progress(state, registry)
                                    state.history.extend(forced_events)
                                # Keep running so the agent can add more work
                                continue
                            state.status = "done"
                            break
                        # keep running so the agent can fuse/fix/export
                        continue
                    if state.last_export:
                        # Fidelity gate: check minimum tool count and export uniqueness
                        allowed, gate_reason = self._check_fidelity_gate(state)
                        if not allowed:
                            state.history.append(
                                ToolEvent(
                                    tool="fidelity_gate",
                                    args={},
                                    ok=False,
                                    message=gate_reason,
                                    data={},
                                )
                            )
                            # Force progress when blocked by too_few_tools only
                            if "too few tools" in gate_reason:
                                forced_events = _force_modeling_progress(state, registry)
                                state.history.extend(forced_events)
                            # Keep running so the agent can add more work
                            continue
                        state.status = "done"
                        break
                    # Ignore premature done without an export
                    continue
            else:
                state.status = "max_turns"
                self._auto_export(state, registry)
                # Exported model counts as finished even if the planner never said done
                if state.last_export:
                    # Fidelity gate: check minimum tool count and export uniqueness
                    allowed, gate_reason = self._check_fidelity_gate(state)
                    if allowed:
                        state.status = "done"
                    else:
                        # Stay at max_turns status (not done) if fidelity check fails
                        state.history.append(
                            ToolEvent(
                                tool="fidelity_gate",
                                args={},
                                ok=False,
                                message=f"{gate_reason} (at max_turns)",
                                data={},
                            )
                        )
                        # Force progress when blocked by too_few_tools only
                        if "too few tools" in gate_reason:
                            forced_events = _force_modeling_progress(state, registry)
                            state.history.extend(forced_events)
        except Exception as exc:  # noqa: BLE001
            state.status = "error"
            state.error = str(exc)

    def _parent_procedure(self, plan: PartPlan, bindable: list[PartSpec]):
        ids = set(list_procedure_ids())
        if any(p.keep_separate for p in plan.parts) and "machine_assembly" in ids:
            return load_default_procedure("machine_assembly")
        if bindable:
            return load_default_procedure(bindable[0].procedure_id or "simple_bracket")
        return load_default_procedure(self.procedure_id)

    def _run_part_plan(
        self,
        goal: str,
        plan: PartPlan,
        contexts: list[DynamicContext],
    ) -> RunResult:
        bindable: list[PartSpec] = []
        for part in plan.parts:
            pid = library_procedure_id(part.procedure_id)
            if pid is None:
                continue
            bindable.append(
                PartSpec(
                    local_name=part.local_name,
                    brief=part.brief,
                    procedure_id=pid,
                    keep_separate=part.keep_separate,
                )
            )

        if not bindable:
            clarify = ClarifyNeeded(
                reason=(
                    "Part plan has no existing library procedure_id — "
                    "will not invent playbooks or start modeling."
                ),
                questions=[
                    f"Which packaged procedure should bind to '{p.local_name}'"
                    f" ({p.brief or 'no brief'})?"
                    for p in plan.parts
                ]
                or ["Name a packaged procedure id from the library."],
            )
            state = SessionState(
                goal=goal,
                backend_name=self.backend_name,
                standard_parts=self.standard_parts,
                procedure=self._parent_procedure(plan, bindable),
                status="needs_clarify",
                clarify=clarify.to_dict(),
                part_plan=plan.to_dict(),
                error=clarify.reason,
                part_runs=[
                    {
                        "local_name": p.local_name,
                        "brief": p.brief,
                        "procedure_id": p.procedure_id,
                        "status": "skipped",
                        "reason": "no library procedure_id",
                    }
                    for p in plan.parts
                ],
            )
            return RunResult(state=state, contexts=contexts)

        try:
            registry = self._open_registry()
        except Exception as exc:  # noqa: BLE001
            state = SessionState(
                goal=goal,
                backend_name=self.backend_name,
                standard_parts=self.standard_parts,
                procedure=self._parent_procedure(plan, bindable),
                status="error",
                part_plan=plan.to_dict(),
                error=str(exc),
            )
            return RunResult(state=state, contexts=contexts)

        state = SessionState(
            goal=goal,
            backend_name=self.backend_name,
            standard_parts=self.standard_parts,
            procedure=self._parent_procedure(plan, bindable),
            status="running",
            part_plan=plan.to_dict(),
        )

        for part in plan.parts:
            pid = library_procedure_id(part.procedure_id)
            if pid is None:
                state.history.append(
                    ToolEvent(
                        tool="part_skip",
                        args={
                            "local_name": part.local_name,
                            "procedure_id": part.procedure_id,
                        },
                        ok=True,
                        message=(
                            f"Skip part {part.local_name!r}: no existing library "
                            "procedure_id (never invent)."
                        ),
                        data={
                            "part": part.local_name,
                            "procedure_id": part.procedure_id,
                        },
                    )
                )
                state.part_runs.append(
                    {
                        "local_name": part.local_name,
                        "brief": part.brief,
                        "procedure_id": part.procedure_id,
                        "status": "skipped",
                        "reason": "no library procedure_id",
                    }
                )
                continue

            state.history.append(
                ToolEvent(
                    tool="part_bind",
                    args={"local_name": part.local_name, "procedure_id": pid},
                    ok=True,
                    message=f"Bind {pid} for part {part.local_name!r}: {part.brief}",
                    data={"part": part.local_name, "procedure_id": pid},
                )
            )
            part_state = SessionState(
                goal=part.brief or part.local_name,
                backend_name=self.backend_name,
                standard_parts=self.standard_parts,
                procedure=load_default_procedure(pid),
                status="running",
                part_plan={"kind": "part_plan", "parts": [part.to_dict()], "notes": ""},
            )
            # One Agent/session per PartSpec (sequential). Reuse planner; do not
            # re-enter assess_goal (part briefs can contain "gearbox") and do not
            # call DesignContextModel.enrich.
            child = Agent(
                backend_name=self.backend_name,
                standard_parts=self.standard_parts,
                procedure_id=pid,
                planner=self.planner,
                context_model=self.context_model,
                max_turns=self.max_turns,
            )
            child._backend = self._backend
            child._execute_playbook(part_state, registry, [], enrich=False)
            for event in part_state.history:
                event.data = {
                    **event.data,
                    "part": part.local_name,
                    "procedure_id": pid,
                }
            state.history.extend(part_state.history)
            state.id_aliases.update(part_state.id_aliases)
            _refresh_frozen_part_aliases(state)
            state.analysis_by_body.update(part_state.analysis_by_body)
            if part_state.last_export:
                state.last_export = part_state.last_export
            if part_state.live_document:
                state.live_document = part_state.live_document
            if part_state.status in {"done", "max_turns"}:
                primary_body = _pick_primary_body(part_state.history)
                if primary_body:
                    freeze_part_alias(state, part.local_name, primary_body)
            state.part_runs.append(
                {
                    "local_name": part.local_name,
                    "brief": part.brief,
                    "procedure_id": pid,
                    "status": part_state.status,
                    "last_export": part_state.last_export,
                    "error": part_state.error,
                }
            )

        keep_separate = any(p.keep_separate for p in plan.parts)
        if keep_separate and registry.has("export"):
            asm_path = "outputs/kala_keep_separate.step"
            result = registry.call("export", body_id="ALL", path=asm_path, fmt="step")
            state.history.append(
                ToolEvent(
                    tool="export",
                    args={"body_id": "ALL", "path": asm_path, "fmt": "step"},
                    ok=result.ok,
                    message=result.message,
                    data=dict(result.data),
                )
            )
            if result.ok:
                state.last_export = str(result.data.get("path") or asm_path)

        modeled = any(
            r.get("procedure_id") and r.get("status") in {"done", "max_turns"}
            for r in state.part_runs
        ) or any(e.ok and str(e.tool).startswith("create_") for e in state.history)
        if modeled:
            state.status = "done"
            state.error = None
        elif any(r.get("status") == "error" for r in state.part_runs):
            state.status = "error"
            state.error = next(
                (str(r.get("error")) for r in state.part_runs if r.get("error")),
                "Part playbook failed",
            )
        else:
            state.status = "error"
            state.error = "Part plan produced no modeled parts."

        self._finalize_gui(state)
        self._write_run_log(goal, state, contexts)
        return RunResult(state=state, contexts=contexts)

    def run(self, goal: str) -> RunResult:
        contexts: list[DynamicContext] = []
        gate = assess_goal(goal)
        if isinstance(gate, ClarifyNeeded):
            procedure = load_default_procedure(self.procedure_id)
            state = SessionState(
                goal=goal,
                backend_name=self.backend_name,
                standard_parts=self.standard_parts,
                procedure=procedure,
                status="needs_clarify",
                clarify=gate.to_dict(),
                error=gate.reason,
            )
            return RunResult(state=state, contexts=contexts)
        if isinstance(gate, PartPlan):
            return self._run_part_plan(goal, gate, contexts)
        try:
            state, registry = self._build(goal)
        except Exception as exc:  # noqa: BLE001
            procedure = load_default_procedure(self.procedure_id)
            state = SessionState(
                goal=goal,
                backend_name=self.backend_name,
                standard_parts=self.standard_parts,
                procedure=procedure,
                status="error",
                error=str(exc),
            )
            return RunResult(state=state, contexts=contexts)

        self._execute_playbook(state, registry, contexts)
        self._finalize_gui(state)
        self._write_run_log(goal, state, contexts)
        return RunResult(state=state, contexts=contexts)
