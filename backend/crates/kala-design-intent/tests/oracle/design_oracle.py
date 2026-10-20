#!/usr/bin/env python3
"""Live Python oracle for the native `kala-design-intent` crate.

Test-only. The production crate never invokes Python. The oracle reads one JSON
document from stdin and prints one JSON document with the live Python results for
the same template, prompt, reply and intent cases the Rust tests evaluate.

Run it through `tests/python_differential.rs`.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path


def _add_repo_source() -> None:
    repo = Path(__file__).resolve().parents[5]
    source = repo / "src"
    if source.is_dir() and str(source) not in sys.path:
        sys.path.insert(0, str(source))


def canonical(value: object) -> str:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=True,
        allow_nan=False,
    )


def main() -> int:
    _add_repo_source()
    from pydantic import ValidationError

    from kala_agent.studio.design import intent as design_intent
    from kala_agent.studio.design import templates
    from kala_agent.studio.providers import local

    case = json.load(sys.stdin)

    output: dict[str, object] = {}

    if case.get("schema"):
        output["schema_text"] = design_intent.design_intent_schema_text()
        output["schema_canonical"] = canonical(design_intent.design_intent_json_schema())
        output["instruction"] = design_intent.DESIGN_INTENT_INSTRUCTION
        output["constants"] = {
            "max_tolerances": design_intent.MAX_TOLERANCES,
            "max_material_length": design_intent.MAX_MATERIAL_LENGTH,
            "max_label_length": design_intent.MAX_LABEL_LENGTH,
            "max_tolerance_length": design_intent.MAX_TOLERANCE_LENGTH,
            "max_segments": design_intent.MAX_SEGMENTS,
            "default_width": design_intent.DEFAULT_BRACKET_WIDTH_MM,
            "default_depth": design_intent.DEFAULT_BRACKET_DEPTH_MM,
            "template_max_segments": templates.MAX_SEGMENTS,
            "default_hole_edge": templates.DEFAULT_HOLE_EDGE_MM,
            "max_response_length": local.MAX_RESPONSE_LENGTH,
        }

    template_results = []
    for item in case.get("templates", []):
        kind = item["kind"]
        try:
            if kind == "l_bracket":
                result = templates.l_bracket(
                    width=item["width"],
                    depth=item["depth"],
                    height=item["height"],
                    wall=item["wall"],
                    bore_d=item["bore_d"],
                    hole_d=item["hole_d"],
                    hole_count=item["hole_count"],
                    edge=item["edge"],
                )
            elif kind == "stepped_shaft":
                result = templates.stepped_shaft(
                    [(segment[0], segment[1]) for segment in item["segments"]]
                )
            elif kind == "housing_envelope":
                result = templates.housing_envelope(
                    width=item["width"],
                    depth=item["depth"],
                    height=item["height"],
                    wall=item["wall"],
                )
            else:
                raise AssertionError(f"unknown template kind {kind}")
            script = result.script.model_dump(mode="json", exclude_none=True)
            template_results.append(
                {
                    "ok": True,
                    "canonical": canonical(script),
                    "label": result.label,
                    "tolerances": list(result.tolerances),
                    "envelope_min": list(result.envelope.min_mm),
                    "envelope_max": list(result.envelope.max_mm),
                }
            )
        except templates.TemplateError as error:
            template_results.append({"ok": False, "error": str(error)})
        except ValidationError as error:
            template_results.append(
                {"ok": False, "error": f"ValidationError: {error}"}
            )
        except Exception as error:  # noqa: BLE001 - the class name is the oracle
            template_results.append(
                {"ok": False, "error": f"{type(error).__name__}: {error}"}
            )
    output["templates"] = template_results

    prompt_results = []
    for prompt in case.get("prompts", []):
        try:
            parsed = local.parse_design_prompt(prompt)
            if parsed is None:
                prompt_results.append({"none": True})
            else:
                prompt_results.append(
                    {
                        "none": False,
                        "family": parsed.family,
                        "intent": (
                            None
                            if parsed.intent is None
                            else canonical(
                                parsed.intent.model_dump(mode="json", exclude_none=True)
                            )
                        ),
                        "missing": list(parsed.missing),
                        "notes": list(parsed.notes),
                        "reply": local._design_prompt_reply(parsed),
                    }
                )
        except ValidationError as error:
            prompt_results.append({"error": design_intent._validation_text(error)})
        except Exception as error:  # noqa: BLE001
            prompt_results.append(
                {"error": f"{type(error).__name__}: {error}"}
            )
    output["prompts"] = prompt_results

    reply_results = []
    for text in case.get("replies", []):
        entry: dict[str, object] = {
            "stripped": design_intent.strip_design_json_block(text)
        }
        try:
            intent = design_intent.parse_design_intent_text(text)
            entry["ok"] = True
            entry["canonical"] = canonical(
                intent.model_dump(mode="json", exclude_none=True)
            )
        except design_intent.DesignIntentParseError as error:
            entry["ok"] = False
            entry["error"] = str(error)
        except Exception as error:  # noqa: BLE001
            entry["ok"] = False
            entry["error"] = f"{type(error).__name__}: {error}"
        reply_results.append(entry)
    output["replies"] = reply_results

    intent_results = []
    for text in case.get("intents", []):
        try:
            payload = json.loads(text)
        except json.JSONDecodeError as error:
            intent_results.append(
                {"kind": "json", "error": error.msg}
            )
            continue
        try:
            intent = design_intent.DesignIntent.model_validate(payload)
            intent_results.append(
                {
                    "kind": "ok",
                    "canonical": canonical(
                        intent.model_dump(mode="json", exclude_none=True)
                    ),
                }
            )
        except ValidationError as error:
            intent_results.append(
                {
                    "kind": "validation",
                    "error": design_intent._validation_text(error),
                }
            )
        except Exception as error:  # noqa: BLE001
            intent_results.append(
                {"kind": "other", "error": f"{type(error).__name__}: {error}"}
            )
    output["intents"] = intent_results

    json.dump(output, sys.stdout, sort_keys=True, ensure_ascii=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
