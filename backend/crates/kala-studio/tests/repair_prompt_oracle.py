#!/usr/bin/env python3
"""Live Python oracle for the native validation-repair prompt.

Test-only. The production runtime never invokes Python. The oracle reads one
JSON document from stdin and prints one JSON document with the schema text and
the repair prompts the live Python reference builds for the same prompt, error
and previous reply the Rust test evaluates.

Run it through `tests/repair_prompt_parity.rs`.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path


def _add_repo_source() -> None:
    repo = Path(__file__).resolve().parents[4]
    source = repo / "src"
    if source.is_dir() and str(source) not in sys.path:
        sys.path.insert(0, str(source))


def main() -> int:
    _add_repo_source()
    from kala_agent.studio.design.intent import design_intent_schema_text
    from kala_agent.studio.providers.dispatch import (
        _build_repair_prompt,
        _display_text,
        _repair_prompt,
    )

    schema_text = design_intent_schema_text()

    payload = json.load(sys.stdin)
    cases = payload.get("cases", [])
    prompts = [_repair_prompt(case["prompt"], case["error"], case["previous"]) for case in cases]
    build_prompts = [
        _build_repair_prompt(case["build_prompt"], case["build_feedback"], schema_text)
        for case in cases
    ]
    output = {
        "schema_text": schema_text,
        "prompts": prompts,
        "build_prompts": build_prompts,
    }
    display_reply = payload.get("display_reply")
    if isinstance(display_reply, str):
        from kala_agent.studio.design.intent import parse_design_intent_text

        output["display_text"] = _display_text(display_reply)
        output["display_steps"] = len(parse_design_intent_text(display_reply).script.components[0].steps)
    sys.stdout.write(json.dumps(output, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
