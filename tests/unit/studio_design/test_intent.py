"""DesignIntent validation, schema export, and model-reply parsing tests."""

from __future__ import annotations

import json

import pytest
from pydantic import ValidationError

from kala_agent.contracts.construction import MAX_COMPONENTS, MAX_TOTAL_STEPS
from kala_agent.studio.design.intent import (
    DESIGN_INTENT_INSTRUCTION,
    DesignIntent,
    DesignIntentParseError,
    HousingCall,
    LbracketCall,
    ShaftSegment,
    SteppedShaftCall,
    _validation_text,
    design_intent_json_schema,
    design_intent_schema_text,
    design_schema_excerpt,
    parse_design_intent_text,
    strip_design_json_block,
)

BOX_SCRIPT = {
    "summary": "20 x 30 x 40 box",
    "components": [
        {
            "name": "box",
            "steps": [
                {
                    "op": "sketch",
                    "profile": "rectangle",
                    "plane": "XY",
                    "width": 20,
                    "height": 30,
                    "x": -10,
                    "y": -15,
                },
                {"op": "extrude", "length": 40},
            ],
        }
    ],
}


def _lbracket_payload() -> dict[str, object]:
    return {
        "template": {
            "template": "l_bracket",
            "width": 60,
            "depth": 45,
            "height": 40,
            "wall": 8,
            "bore_d": 10,
            "hole_d": 5.5,
            "hole_count": 4,
        },
        "material": "aluminium",
        "tolerances": ["ISO 2768-m"],
        "label": "Mounting bracket",
    }


def test_template_intent_round_trips() -> None:
    intent = DesignIntent.model_validate(_lbracket_payload())
    assert isinstance(intent.template, LbracketCall)
    assert intent.template.width == 60
    assert intent.material == "aluminium"
    dumped = intent.model_dump(mode="json", exclude_none=True)
    assert dumped["template"]["template"] == "l_bracket"


def test_script_intent_round_trips() -> None:
    intent = DesignIntent.model_validate({"script": BOX_SCRIPT, "material": "steel"})
    assert intent.script is not None
    assert intent.template is None
    assert intent.script.summary == "20 x 30 x 40 box"


def test_the_flange_repro_is_refused_before_the_cut() -> None:
    # The live 2026-10-20 flange: an XY revolve, then each Ø8 bolt hole
    # sketched on XZ with a pocket that states plane XY. FreeCAD cut nothing
    # and still reported a verified B-Rep.
    script = {
        "summary": "flange",
        "components": [
            {
                "name": "half_a",
                "steps": [
                    {
                        "op": "sketch",
                        "plane": "XY",
                        "profile": "rectangle",
                        "width": 35,
                        "height": 12,
                        "x": 15,
                    },
                    {"op": "revolve", "plane": "XY", "angle": 180},
                    {
                        "op": "sketch",
                        "plane": "XZ",
                        "profile": "circle",
                        "diameter": 8,
                        "x": 40,
                    },
                    {"op": "pocket", "plane": "XY"},
                ],
            }
        ],
    }
    with pytest.raises(ValidationError) as caught:
        DesignIntent.model_validate({"script": script})
    assert (
        _validation_text(caught.value)
        == "script: component 'half_a': pocket at step index 3 on XZ over a XY solid"
    )


def test_a_pocket_sketch_on_another_plane_than_the_solid_is_refused() -> None:
    script = {
        "summary": "cross plane",
        "components": [
            {
                "name": "plate",
                "steps": [
                    {
                        "op": "sketch",
                        "plane": "XY",
                        "profile": "rectangle",
                        "width": 20,
                        "height": 20,
                    },
                    {"op": "extrude", "length": 10},
                    {
                        "op": "sketch",
                        "plane": "XZ",
                        "profile": "circle",
                        "diameter": 2,
                        "x": 5,
                        "y": 5,
                    },
                    {"op": "pocket", "plane": "XZ"},
                ],
            }
        ],
    }
    with pytest.raises(ValidationError) as caught:
        DesignIntent.model_validate({"script": script})
    assert (
        _validation_text(caught.value)
        == "script: component 'plate': pocket at step index 3 on XZ over a XY solid"
    )


def test_a_pocket_plane_field_is_ignored_like_the_compiler_ignores_it() -> None:
    # The shipped L-bracket template's canonical wire form states plane XY
    # on its XZ upright pocket; the compiler cuts on the pending sketch
    # plane, so the pocket's own field is not part of the rule.
    script = {
        "summary": "ignored field",
        "components": [
            {
                "name": "plate",
                "steps": [
                    {
                        "op": "sketch",
                        "plane": "XZ",
                        "profile": "rectangle",
                        "width": 20,
                        "height": 20,
                    },
                    {"op": "extrude", "length": 10},
                    {
                        "op": "sketch",
                        "plane": "XZ",
                        "profile": "circle",
                        "diameter": 2,
                        "x": 5,
                        "y": 5,
                    },
                    {"op": "pocket", "plane": "XY"},
                ],
            }
        ],
    }
    intent = DesignIntent.model_validate({"script": script})
    assert intent.script is not None


def test_intent_requires_exactly_one_source() -> None:
    with pytest.raises(ValidationError):
        DesignIntent()
    with pytest.raises(ValidationError):
        DesignIntent.model_validate(
            {"template": _lbracket_payload()["template"], "script": BOX_SCRIPT}
        )


def test_intent_refuses_unknown_fields() -> None:
    payload = _lbracket_payload()
    payload["extra"] = True
    with pytest.raises(ValidationError):
        DesignIntent.model_validate(payload)


def test_template_call_refuses_unknown_template() -> None:
    with pytest.raises(ValidationError):
        DesignIntent.model_validate(
            {
                "template": {
                    "template": "flange",
                    "width": 10,
                    "depth": 10,
                    "height": 10,
                    "wall": 2,
                }
            }
        )


def test_tolerances_and_material_are_bounded() -> None:
    with pytest.raises(ValidationError):
        DesignIntent.model_validate(
            {"template": _lbracket_payload()["template"], "material": "x" * 65}
        )
    with pytest.raises(ValidationError):
        DesignIntent.model_validate(
            {
                "template": _lbracket_payload()["template"],
                "tolerances": ["x" * 65],
            }
        )
    with pytest.raises(ValidationError):
        DesignIntent.model_validate(
            {
                "template": _lbracket_payload()["template"],
                "tolerances": ["ok"] * 17,
            }
        )


def test_stepped_shaft_segments_are_bounded() -> None:
    with pytest.raises(ValidationError):
        DesignIntent.model_validate({"template": {"template": "stepped_shaft", "segments": []}})
    with pytest.raises(ValidationError):
        DesignIntent.model_validate(
            {
                "template": {
                    "template": "stepped_shaft",
                    "segments": [{"diameter": 10, "length": 10}] * 9,
                }
            }
        )


def test_adr_style_field_aliases_are_accepted() -> None:
    intent = DesignIntent.model_validate(
        {
            "template": {
                "template": "l_bracket",
                "W": 60,
                "D": 45,
                "H": 40,
                "T": 8,
                "B": 10,
                "h": 5.5,
                "holes": 4,
                "e": 8.25,
            }
        }
    )
    assert isinstance(intent.template, LbracketCall)
    assert intent.template.width == 60
    assert intent.template.wall == 8
    assert intent.template.bore_d == 10
    assert intent.template.hole_d == 5.5
    assert intent.template.hole_count == 4
    assert intent.template.edge == 8.25


def test_housing_call_parses() -> None:
    intent = DesignIntent.model_validate(
        {
            "template": {
                "template": "housing_envelope",
                "width": 80,
                "depth": 60,
                "height": 40,
                "wall": 5,
            }
        }
    )
    assert isinstance(intent.template, HousingCall)


def test_stepped_shaft_call_parses() -> None:
    intent = DesignIntent.model_validate(
        {
            "template": {
                "template": "stepped_shaft",
                "segments": [
                    {"diameter": 25, "length": 120},
                    {"diameter": 15, "length": 30},
                ],
            }
        }
    )
    assert isinstance(intent.template, SteppedShaftCall)
    assert intent.template.segments == (
        ShaftSegment(diameter=25, length=120),
        ShaftSegment(diameter=15, length=30),
    )


def test_json_schema_export_is_a_valid_schema() -> None:
    schema = design_intent_json_schema()
    assert schema["type"] == "object"
    properties = schema.get("properties")
    assert isinstance(properties, dict)
    assert "template" in properties
    assert "script" in properties
    text = design_intent_schema_text()
    reparsed = json.loads(text)
    assert reparsed["title"] == "DesignIntent"
    assert "l_bracket" in text
    assert "JSON Schema" in DESIGN_INTENT_INSTRUCTION


def test_parse_fenced_json_with_prose() -> None:
    text = (
        "Here is the bracket.\n\n"
        f"```json\n{json.dumps(_lbracket_payload())}\n```\n"
        "The kernel will measure it."
    )
    intent = parse_design_intent_text(text)
    assert isinstance(intent.template, LbracketCall)


def test_parse_bare_json_object() -> None:
    intent = parse_design_intent_text(json.dumps(_lbracket_payload()))
    assert isinstance(intent.template, LbracketCall)


def test_parse_refuses_a_reply_without_json() -> None:
    with pytest.raises(DesignIntentParseError, match="fenced"):
        parse_design_intent_text("I need the width and depth first.")


def test_parse_refuses_malformed_json() -> None:
    with pytest.raises(DesignIntentParseError, match="not valid JSON"):
        parse_design_intent_text("```json\n{not json}\n```")


def test_parse_refuses_an_invalid_intent_and_names_the_field() -> None:
    payload = _lbracket_payload()
    template = payload["template"]
    assert isinstance(template, dict)
    payload["template"] = {**template, "width": -3}
    with pytest.raises(DesignIntentParseError, match="width"):
        parse_design_intent_text(f"```json\n{json.dumps(payload)}\n```")


def test_strip_design_json_block_keeps_prose() -> None:
    text = f"A 60 mm plate with a 10 mm bore.\n\n```json\n{json.dumps(_lbracket_payload())}\n```"
    assert strip_design_json_block(text) == "A 60 mm plate with a 10 mm bore."
    assert strip_design_json_block("```json\n{}\n```") == "Design intent received."


def _instruction_examples() -> list[str]:
    examples: list[str] = []
    rest = DESIGN_INTENT_INSTRUCTION
    while "```json\n" in rest:
        body = rest.split("```json\n", 1)[1]
        example, rest = body.split("\n```", 1)
        examples.append(example)
    return examples


def test_every_instruction_example_validates_and_the_prompt_stays_compact() -> None:
    examples = _instruction_examples()
    assert len(examples) == 3
    for example in examples:
        assert isinstance(DesignIntent.model_validate(json.loads(example)).script, object)
    assert len(DESIGN_INTENT_INSTRUCTION) < 6000
    assert "When no template matches, author a ConstructionScript" in DESIGN_INTENT_INSTRUCTION
    assert "If a value needed to build the part is missing" in DESIGN_INTENT_INSTRUCTION
    assert "Never invent a dimension" in DESIGN_INTENT_INSTRUCTION
    assert "solid.revolve" not in DESIGN_INTENT_INSTRUCTION
    assert "a rectangle's x/y is its lower-left corner" in DESIGN_INTENT_INSTRUCTION
    assert "a circle's x/y is its centre" in DESIGN_INTENT_INSTRUCTION
    assert (
        "Every pocket must cut into material that its own component already has"
        in DESIGN_INTENT_INSTRUCTION
    )


def test_the_instruction_states_the_whole_design_budget() -> None:
    assert (
        f"A design may use up to {MAX_TOTAL_STEPS} steps in total across up to "
        f"{MAX_COMPONENTS} components"
    ) in DESIGN_INTENT_INSTRUCTION
    assert "and one component may use all of them" in DESIGN_INTENT_INSTRUCTION
    assert "Each solid body is one component" in DESIGN_INTENT_INSTRUCTION
    assert (
        "do not split a body into components just to save steps"
        in DESIGN_INTENT_INSTRUCTION
    )
    assert "A component has at most" not in DESIGN_INTENT_INSTRUCTION


def test_parse_bare_json_wrapped_in_prose() -> None:
    text = "Here is the bracket. " + json.dumps(_lbracket_payload()) + " Measured later."
    intent = parse_design_intent_text(text)
    assert isinstance(intent.template, LbracketCall)


def test_parse_refuses_conflicting_blocks_but_accepts_identical_ones() -> None:
    payload = json.dumps(_lbracket_payload())
    parse_design_intent_text(f"```json\n{payload}\n```\n\n```json\n{payload}\n```")
    with pytest.raises(DesignIntentParseError, match="conflicting"):
        parse_design_intent_text(
            f"```json\n{payload}\n```\n\n```json\n{json.dumps({'script': BOX_SCRIPT})}\n```"
        )


def test_schema_excerpt_names_the_failing_field() -> None:
    excerpt = design_schema_excerpt("template.l_bracket.width: Input should be greater than 0")
    assert excerpt is not None
    assert '"Width"' in excerpt
    assert '"exclusiveMinimum"' in excerpt
    nested = design_schema_excerpt("script.components.0.steps.0.op: Input should be a valid string")
    assert nested is not None
    assert '"Op"' in nested
    root = design_schema_excerpt("a design intent must carry exactly one of template or script")
    assert root is not None
    assert '"template"' in root
    assert '"script"' in root
