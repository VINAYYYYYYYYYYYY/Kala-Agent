"""Core agent loop: context → plan → tool calls → observe."""

from __future__ import annotations

import os
from dataclasses import dataclass
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
from kala.procedures.schema import load_default_procedure
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

    def _build(self, goal: str) -> tuple[SessionState, ToolRegistry]:
        backend = create_backend(self.backend_name)
        self._backend = backend
        registry = build_registry(
            backend,
            standard_parts=self.standard_parts,
            catalog=self.catalog,
        )
        state = SessionState(
            goal=goal,
            backend_name=backend.name,
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

    def run(self, goal: str) -> RunResult:
        contexts: list[DynamicContext] = []
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

        try:
            for _ in range(self.max_turns):
                context = self.context_model.enrich(state)
                contexts.append(context)
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
                id_aliases: dict[str, str] = {}

                def _resolve(bid: str) -> str:
                    seen: set[str] = set()
                    cur = bid
                    while cur in id_aliases and cur not in seen:
                        seen.add(cur)
                        cur = id_aliases[cur]
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
                                id_aliases[str(old)] = str(new_id)
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
                    state.advance_step()
                if turn.done:
                    # Never mark done on a blocked/failed export or unfinished model
                    if any(c.name == "export" for c in turn.calls):
                        if exported_this_turn or state.last_export:
                            state.status = "done"
                            break
                        # keep running so the agent can fuse/fix/export
                        continue
                    if state.last_export:
                        state.status = "done"
                        break
                    # Ignore premature done without an export
                    continue
            else:
                state.status = "max_turns"
                self._auto_export(state, registry)
                # Exported model counts as finished even if the planner never said done
                if state.last_export:
                    state.status = "done"
        except Exception as exc:  # noqa: BLE001
            state.status = "error"
            state.error = str(exc)

        self._finalize_gui(state)
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
        return RunResult(state=state, contexts=contexts)
