"""CLI entry point for Kala-Agent."""

from __future__ import annotations

import argparse
import json
import sys

from kala.agent.loop import Agent


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="kala", description="CAD design agent")
    sub = parser.add_subparsers(dest="command")

    run = sub.add_parser("run", help="Run a design session")
    run.add_argument("goal", help="Design goal / instruction")
    run.add_argument(
        "--backend",
        default="freecad",
        choices=["freecad", "mock", "ocp"],
        help="CAD backend (default: freecad)",
    )
    run.add_argument(
        "--standard-parts",
        choices=["on", "off"],
        default="off",
        help="Toggle standard parts catalog tools",
    )
    run.add_argument(
        "--procedure",
        default="simple_bracket",
        help="Procedure playbook id",
    )
    run.add_argument("--json", action="store_true", help="Print machine-readable result")

    ui = sub.add_parser("ui", help="Launch the dark desktop UI")
    ui.add_argument(
        "--backend",
        default="freecad",
        choices=["freecad", "mock", "ocp"],
    )

    mcp_cmd = sub.add_parser(
        "mcp-freecad",
        help="Run the FreeCAD MCP server (stdio) for tool-calling CAD ops",
    )

    logs = sub.add_parser("logs", help="Show Kala agent + FreeCAD logs")
    logs.add_argument(
        "--tail",
        type=int,
        default=3,
        help="How many recent agent runs to summarize (default 3)",
    )
    logs.add_argument(
        "--freecad-only",
        action="store_true",
        help="Only print FreeCAD logs",
    )
    logs.add_argument(
        "--path",
        action="store_true",
        help="Only print log file paths",
    )
    return parser


def main(argv: list[str] | None = None) -> None:
    # AppImage / desktop shortcut may pass no subcommand — open UI.
    if argv is None and len(sys.argv) == 1:
        from kala.ui.desktop import run_app

        raise SystemExit(run_app(default_backend="freecad"))

    parser = build_parser()
    args = parser.parse_args(argv)

    if args.command is None:
        parser.print_help()
        print('\nTry: kala ui --backend mock')
        print('     kala run "80x50x8 bracket" --backend mock')
        return

    if args.command == "ui":
        from kala.ui.desktop import run_app

        raise SystemExit(run_app(default_backend=args.backend))

    if args.command == "mcp-freecad":
        from kala.mcp_freecad.server import main as mcp_main

        mcp_main()
        return

    if args.command == "logs":
        from kala.session.runlog import (
            agent_latest_path,
            agent_log_path,
            freecad_gui_log_path,
            freecad_native_log_path,
            freecad_native_log_candidates,
            read_tail,
            summarize_logs,
        )

        if args.path:
            print(f"agent:    {agent_log_path()}")
            print(f"latest:   {agent_latest_path()}")
            print(f"fc_gui:   {freecad_gui_log_path()}")
            native = freecad_native_log_path()
            print(f"fc_native:{native or freecad_native_log_candidates()[0]}")
            return
        if args.freecad_only:
            print(f"=== {freecad_gui_log_path()} ===")
            print(read_tail(freecad_gui_log_path()))
            native = freecad_native_log_path()
            print(f"\n=== {native or freecad_native_log_candidates()[0]} ===")
            print(read_tail(native) if native else "(missing)")
            return
        print(summarize_logs(agent_lines=max(1, args.tail)))
        return

    if args.command == "run":
        agent = Agent(
            backend_name=args.backend,
            standard_parts=(args.standard_parts == "on"),
            procedure_id=args.procedure,
        )
        result = agent.run(args.goal)
        if args.json:
            print(json.dumps(result.to_dict(), indent=2))
            return

        state = result.state
        print(f"status: {state.status}")
        print(f"backend: {state.backend_name}")
        print(f"standard_parts: {state.standard_parts}")
        step = state.current_step
        if step:
            print(f"procedure step: [{step.id}] {step.goal}")
        if state.last_export:
            print(f"export: {state.last_export}")
        if state.total_tokens or state.llm_calls:
            print(
                f"tokens: prompt={state.prompt_tokens} "
                f"completion={state.completion_tokens} "
                f"total={state.total_tokens} "
                f"(llm_calls={state.llm_calls})"
            )
        if state.live_document:
            print(f"live FreeCAD doc: {state.live_document}")
        if state.gui_note:
            print(f"gui: {state.gui_note}")
        if state.error:
            print(f"error: {state.error}", file=sys.stderr)
        print("--- tool log ---")
        for event in state.history:
            flag = "ok" if event.ok else "FAIL"
            print(f"[{flag}] {event.tool} {event.args}")
            print(f"  {event.message}")
            cad = event.data.get("cad_software")
            api = event.data.get("cad_api")
            if cad or api:
                print(f"  CAD software: {cad or '—'}")
                if api:
                    print(f"  CAD tool: {api}")
        return


if __name__ == "__main__":
    main()
