"""FreeCAD is the first compiler of the shared construction script."""

from __future__ import annotations

import pytest

from kala_agent.contracts.construction import (
    EditStep,
    RevisionAction,
    load_analysis,
    load_construction,
    load_reading,
)
from kala_agent_freecad.cad.compile import (
    apply_edits,
    compile_construction,
    compile_revision,
    fillets_from_edits,
    rewrite_component,
)
from kala_agent_freecad.cad.procedure import parse_calls
from kala_agent_freecad.errors import FreeCADInputError


def _pins() -> dict[str, object]:
    return {
        "assembly": [{"components": ["pin_a", "pin_b"], "op": "fuse"}],
        "components": [
            {
                "name": "pin_a",
                "steps": [
                    {"diameter": 10, "op": "sketch", "profile": "circle"},
                    {"length": 20, "op": "extrude"},
                ],
            },
            {
                "name": "pin_b",
                "steps": [
                    {"diameter": 8, "op": "sketch", "profile": "circle"},
                    {"length": 12, "op": "extrude"},
                ],
            },
        ],
        "summary": "Two pins.",
    }


def _jack() -> dict[str, object]:
    return {
        "assembly": [
            {"component": "base", "op": "place", "x": 0, "y": 0, "z": 0},
            {"component": "screw", "op": "place", "x": 0, "y": 0, "z": 12},
            {"components": ["base", "screw"], "op": "fuse"},
        ],
        "components": [
            {
                "name": "base",
                "steps": [
                    {"height": 40, "op": "sketch", "profile": "rectangle", "width": 80},
                    {"length": 12, "op": "extrude"},
                ],
            },
            {
                "name": "screw",
                "steps": [
                    {"diameter": 16, "op": "sketch", "profile": "circle"},
                    {"length": 40, "op": "extrude"},
                    {"diameter": 16, "length": 24, "op": "helix", "pitch": 4},
                ],
            },
        ],
        "summary": "A screw jack.",
    }


def test_two_circle_extrudes_and_a_fuse_are_legal_calls() -> None:
    calls = parse_calls(compile_construction(load_construction(_pins())))
    tools = [call.tool for call in calls]

    assert [tool for tool in tools if tool in {"partdesign.body", "part.fuse"}] == [
        "partdesign.body",
        "partdesign.body",
        "part.fuse",
    ]
    assert calls[-1].arguments["components"] == ["pin_a", "pin_b"]


def test_a_rectangle_extrude_compiles_to_a_pad() -> None:
    calls = parse_calls(
        compile_construction(
            load_construction(
                {
                    "components": [
                        {
                            "name": "plate",
                            "steps": [
                                {
                                    "height": 40,
                                    "op": "sketch",
                                    "profile": "rectangle",
                                    "width": 80,
                                },
                                {"length": 12, "op": "extrude"},
                            ],
                        }
                    ],
                    "summary": "A plate.",
                }
            )
        )
    )
    tools = [call.tool for call in calls]
    pad = next(call for call in calls if call.tool == "partdesign.pad")

    assert "sketch.rectangle" in tools
    assert "sketch.horizontal" in tools
    assert "partdesign.pad" in tools
    assert pad.arguments["length"] == 12
    assert "reversed" not in pad.arguments


def test_a_helix_compiles_to_a_pad_and_a_helix() -> None:
    calls = parse_calls(
        compile_construction(
            load_construction(
                {
                    "components": [
                        {
                            "name": "screw",
                            "steps": [
                                {"diameter": 16, "op": "sketch", "profile": "circle"},
                                {"length": 40, "op": "extrude"},
                                {"diameter": 16, "length": 24, "op": "helix", "pitch": 4},
                            ],
                        }
                    ],
                    "summary": "A screw.",
                }
            )
        )
    )
    tools = [call.tool for call in calls]
    helix = next(call for call in calls if call.tool == "partdesign.helix")
    polyline = next(call for call in calls if call.tool == "sketch.polyline")
    diameter = 16.0
    pitch = 4.0
    length = 24.0
    radius = diameter / 2.0
    depth = max(1.0, diameter * 0.08)
    tooth = max(0.5, min(pitch * 0.45, length * 0.25))

    assert "partdesign.pad" in tools
    assert "partdesign.helix" in tools
    assert tools.index("partdesign.pad") < tools.index("partdesign.helix")
    assert tools.count("sketch.circle") == 1
    assert [call.arguments["plane"] for call in calls if call.tool == "sketch.new"] == [
        "XY",
        "XZ",
    ]
    assert polyline.arguments["closed"] is True
    assert polyline.arguments["points"] == [
        [radius, 0.5],
        [radius + depth, 0.5],
        [radius + depth, 0.5 + tooth],
        [radius, 0.5 + tooth],
    ]
    assert helix.arguments == {"height": length, "pitch": pitch}


def test_resize_changes_the_recompiled_pad_length() -> None:
    script = load_construction(_pins())
    edited = apply_edits(
        script,
        (
            EditStep.model_validate(
                {"component": "pin_a", "diameter": 14, "length": 30, "op": "resize"}
            ),
        ),
    )
    calls = parse_calls(compile_construction(edited))
    pads = [call for call in calls if call.tool == "partdesign.pad"]
    circles = [call for call in calls if call.tool == "sketch.circle"]

    assert pads[0].arguments["length"] == 30
    assert pads[1].arguments["length"] == 12
    assert circles[0].arguments["radius"] == 7
    assert circles[1].arguments["radius"] == 4


def test_fillets_from_edits_add_a_fillet_call() -> None:
    script = load_construction(_pins())
    edits = (EditStep.model_validate({"component": "pin_a", "op": "fillet", "radius": 1.5}),)
    revised = apply_edits(script, edits)

    assert "partdesign.fillet" not in [
        call.tool for call in parse_calls(compile_construction(revised))
    ]
    assert fillets_from_edits(edits) == {"pin_a": 1.5}

    calls = parse_calls(compile_construction(script, fillets=fillets_from_edits(edits)))
    fillet = next(call for call in calls if call.tool == "partdesign.fillet")
    tools = [call.tool for call in calls]
    bodies = [index for index, tool in enumerate(tools) if tool == "partdesign.body"]

    assert dict(fillet.arguments) == {"name": "pin_a", "radius": 1.5}
    assert bodies[0] < tools.index("partdesign.fillet") < bodies[1]


def test_analysis_lowers_pressure_to_force() -> None:
    script = load_construction(_pins())
    analysis = load_analysis(
        {
            "steps": [
                {"name": "StructuralSteel", "op": "material"},
                {"face": 1, "op": "fix"},
                {"face": 4, "op": "pressure", "pressure_mpa": 2.5},
                {"op": "mesh"},
                {"op": "solve"},
            ]
        }
    )
    calls = parse_calls(compile_construction(script, analysis=analysis))
    tools = [call.tool for call in calls]
    force = next(call for call in calls if call.tool == "fem.force")

    assert "fem.analysis" in tools
    assert "fem.solve" in tools
    assert "fem.result" in tools
    assert "fem.pressure" not in tools
    assert tools.index("part.fuse") < tools.index("fem.analysis")
    assert tools.index("fem.solve") < tools.index("fem.result")
    assert tools[-1] == "fem.result"
    assert force.arguments == {"face": 4, "force_n": 2.5}


def test_a_place_step_is_emitted_on_that_body() -> None:
    script = load_construction(
        {
            "assembly": [
                {"components": ["pin_a", "pin_b"], "op": "fuse"},
            ],
            "components": _pins()["components"],
            "summary": "Two pins.",
        }
    )
    moved = apply_edits(
        script,
        (EditStep.model_validate({"component": "pin_b", "op": "move", "x": 3, "y": 4, "z": 5}),),
    )
    calls = parse_calls(compile_construction(moved))
    tools = [call.tool for call in calls]
    bodies = [index for index, tool in enumerate(tools) if tool == "partdesign.body"]
    placement = next(call for call in calls if call.tool == "partdesign.placement")

    assert tools[bodies[1] + 1] == "partdesign.placement"
    assert dict(placement.arguments) == {"name": "pin_b", "x": 3.0, "y": 4.0, "z": 5.0}
    places = [step for step in moved.assembly if step.op == "place"]
    assert len(places) == 1
    assert places[0].z == 5


def test_an_unknown_component_edit_is_refused() -> None:
    script = load_construction(_pins())
    missing = EditStep.model_validate({"component": "cup", "op": "move", "z": 4})
    replacement = load_construction(
        {
            "components": [
                {
                    "name": "cup",
                    "steps": [
                        {"diameter": 10, "op": "sketch", "profile": "circle"},
                        {"length": 10, "op": "extrude"},
                    ],
                }
            ],
            "summary": "A cup.",
        }
    ).components[0]

    with pytest.raises(FreeCADInputError, match="unknown component cup"):
        apply_edits(script, (missing,))
    with pytest.raises(FreeCADInputError, match="unknown component cup"):
        rewrite_component(script, replacement)


def test_rewrite_component_replaces_only_the_named_steps() -> None:
    replacement = load_construction(
        {
            "components": [
                {
                    "name": "pin_a",
                    "steps": [
                        {"height": 10, "op": "sketch", "profile": "rectangle", "width": 10},
                        {"length": 5, "op": "extrude"},
                    ],
                }
            ],
            "summary": "A block.",
        }
    ).components[0]
    revised = rewrite_component(load_construction(_pins()), replacement)
    calls = parse_calls(compile_construction(revised))

    assert calls[0].arguments["name"] == "pin_a"
    assert any(call.tool == "sketch.rectangle" for call in calls)
    assert any(call.tool == "sketch.circle" for call in calls)
    assert [call.arguments["name"] for call in calls if call.tool == "partdesign.body"] == [
        "pin_a",
        "pin_b",
    ]


def test_profiles_and_solids_follow_the_compose_rules() -> None:
    offset = parse_calls(
        compile_construction(
            load_construction(
                {
                    "components": [
                        {
                            "name": "plate",
                            "steps": [
                                {
                                    "height": 8,
                                    "op": "sketch",
                                    "profile": "rectangle",
                                    "width": 20,
                                    "x": 10,
                                    "y": 5,
                                },
                                {"length": 4, "op": "extrude", "reversed": True},
                            ],
                        }
                    ],
                    "summary": "An offset plate.",
                }
            )
        )
    )
    offset_tools = [call.tool for call in offset]
    pad = next(call for call in offset if call.tool == "partdesign.pad")
    assert "sketch.polyline" in offset_tools
    assert "sketch.rectangle" not in offset_tools
    assert pad.arguments["reversed"] is True

    turned = parse_calls(
        compile_construction(
            load_construction(
                {
                    "components": [
                        {
                            "name": "hub",
                            "steps": [
                                {"height": 20, "op": "sketch", "profile": "rectangle", "width": 10},
                                {"angle": 180, "op": "revolve"},
                            ],
                        }
                    ],
                    "summary": "A hub.",
                }
            )
        )
    )
    assert any(
        call.tool == "partdesign.revolution" and call.arguments["angle"] == 180 for call in turned
    )
    assert "partdesign.pad" not in [call.tool for call in turned]

    spun = parse_calls(
        compile_construction(
            load_construction(
                {
                    "components": [
                        {
                            "name": "pin",
                            "steps": [
                                {"diameter": 10, "op": "sketch", "profile": "circle"},
                                {"length": 15, "op": "revolve"},
                            ],
                        }
                    ],
                    "summary": "A pin.",
                }
            )
        )
    )
    assert any(call.tool == "partdesign.pad" and call.arguments["length"] == 15 for call in spun)
    assert "partdesign.revolution" not in [call.tool for call in spun]

    pocket = parse_calls(
        compile_construction(
            load_construction(
                {
                    "components": [
                        {
                            "name": "plate",
                            "steps": [
                                {"height": 40, "op": "sketch", "profile": "rectangle", "width": 80},
                                {"length": 12, "op": "extrude"},
                                {"diameter": 8, "op": "sketch", "profile": "circle"},
                                {"op": "pocket"},
                            ],
                        }
                    ],
                    "summary": "A plate with a hole.",
                }
            )
        )
    )
    holes = [call for call in pocket if call.tool == "partdesign.pocket"]
    assert holes[0].arguments == {"step_index": 3}


def test_two_components_compile_to_two_bodies_and_one_fuse() -> None:
    calls = parse_calls(compile_construction(load_construction(_jack())))
    tools = [call.tool for call in calls]

    assert tools.count("partdesign.body") == 2
    assert "partdesign.pad" in tools
    assert "partdesign.helix" in tools
    assert tools[-1] == "part.fuse"
    assert calls[-1].arguments["components"] == ["base", "screw"]
    placed = [call for call in calls if call.tool == "partdesign.placement"]
    assert placed[-1].arguments["z"] == 12


def test_an_analysis_script_compiles_after_the_solid() -> None:
    script = load_construction(_jack())
    analysis = load_analysis(
        {
            "steps": [
                {"op": "material", "name": "StructuralSteel"},
                {"face": 1, "op": "fix"},
                {"face": 2, "force_n": 100, "op": "force"},
                {"op": "mesh"},
                {"op": "solve"},
            ]
        }
    )

    tools = [call.tool for call in parse_calls(compile_construction(script, analysis=analysis))]

    assert tools.index("part.fuse") < tools.index("fem.analysis")
    assert "fem.solve" in tools
    assert "fem.result" in tools
    assert "fem.pressure" not in tools
    assert tools[-1] == "fem.result"


def test_revision_choices_select_the_compiler_path() -> None:
    script = load_construction(_jack())
    reading = load_reading(
        {
            "tool_edit": [{"component": "base", "length": 16, "op": "resize"}],
            "verdict": "The base is thin.",
        }
    )

    assert compile_revision(script, reading.tool_edit, None, RevisionAction.ACCEPT) is None
    assert compile_revision(script, (), None, RevisionAction.TOOL_EDIT) is None
    assert compile_revision(script, (), None, RevisionAction.SCRIPT_REWRITE) is None

    resized = compile_revision(script, reading.tool_edit, None, RevisionAction.TOOL_EDIT)
    assert resized is not None
    pads = [call for call in parse_calls(resized[1]) if call.tool == "partdesign.pad"]
    assert pads[0].arguments["length"] == 16

    rewrite = load_construction(
        {
            "components": [
                {
                    "name": "base",
                    "steps": [
                        {"diameter": 30, "op": "sketch", "profile": "circle"},
                        {"length": 10, "op": "extrude"},
                    ],
                }
            ],
            "summary": "A round base.",
        }
    ).components[0]
    rewritten = compile_revision(script, (), rewrite, RevisionAction.SCRIPT_REWRITE)
    assert rewritten is not None
    parsed = parse_calls(rewritten[1])
    assert parsed[0].arguments["name"] == "base"
    assert any(call.tool == "sketch.circle" for call in parsed)


def test_a_fillet_edit_is_a_tool_call_on_the_named_component() -> None:
    script = load_construction(_jack())
    reading = load_reading(
        {
            "tool_edit": [{"component": "base", "op": "fillet", "radius": 1.5}],
            "verdict": "Break the base edges.",
        }
    )

    edited = compile_revision(script, reading.tool_edit, None, RevisionAction.TOOL_EDIT)
    assert edited is not None
    parsed = parse_calls(edited[1])
    fillet = next(call for call in parsed if call.tool == "partdesign.fillet")

    assert dict(fillet.arguments) == {"name": "base", "radius": 1.5}


def test_revision_refuses_an_unknown_component() -> None:
    script = load_construction(_jack())
    reading = load_reading(
        {
            "tool_edit": [{"component": "cup", "op": "move", "z": 4}],
            "verdict": "Lift the cup.",
        }
    )

    with pytest.raises(FreeCADInputError, match="unknown component cup"):
        compile_revision(script, reading.tool_edit, None, RevisionAction.TOOL_EDIT)
