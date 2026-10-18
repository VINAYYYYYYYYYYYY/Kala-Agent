"""The reasoning model writes parts; the decision model only chooses among them."""

from __future__ import annotations

import json
from collections.abc import Mapping
from pathlib import Path

import pytest

from kala_agent.contracts import AnalysisReading, load_construction, load_reading
from kala_agent_freecad.cad.compose import continue_design, design_request, realize, revise_design
from kala_agent_freecad.cad.models import ModelChoice, WrittenIntent
from kala_agent_freecad.errors import FreeCADDependencyError, FreeCADInputError


def _choose(state: str, options: Mapping[str, str]) -> str:
    del state
    return next(key for key in options if key != "stop")


def _plate() -> dict[str, object]:
    return {
        "analyze": False,
        "parts": (
            {
                "features": (
                    {
                        "height": 20,
                        "kind": "pad",
                        "length": 10,
                        "plane": "XY",
                        "profile": "rectangle",
                        "width": 30,
                    },
                ),
                "name": "plate",
                "x": 0,
                "y": 0,
                "z": 0,
            },
        ),
        "summary": "A plate.",
    }


def test_a_plate_intent_becomes_a_pad() -> None:
    plan = realize(_plate(), _choose)
    tools = [call.tool for call in plan.calls]
    assert "sketch.rectangle" in tools
    assert "partdesign.pad" in tools
    assert tools.count("partdesign.body") == 1


def test_the_decision_model_picks_the_next_part() -> None:
    seen: list[str] = []

    def choose(state: str, options: Mapping[str, str]) -> str:
        seen.append(state)
        return next(key for key in options if key != "stop")

    raw = {
        "analyze": True,
        "parts": (
            {
                "features": (
                    {
                        "diameter": 40,
                        "kind": "pad",
                        "length": 12,
                        "profile": "circle",
                    },
                    {"diameter": 8, "kind": "pocket", "profile": "circle", "x": 12, "y": 0},
                ),
                "name": "base",
            },
            {
                "features": (
                    {"diameter": 16, "kind": "pad", "length": 40, "profile": "circle"},
                    {"diameter": 16, "kind": "helix", "length": 24, "pitch": 4},
                ),
                "name": "screw",
                "z": 12,
            },
        ),
        "summary": "A screw jack.",
    }
    plan = realize(raw, choose)
    tools = [call.tool for call in plan.calls]
    assert tools.count("partdesign.body") == 2
    assert "partdesign.placement" in tools
    assert "partdesign.helix" in tools
    assert "partdesign.pocket" in tools
    assert "fem.analysis" not in tools
    assert any(line.startswith("Analysis: off") for line in plan.log)
    assert len(seen) == 2
    assert any(line.startswith("Reasoning ") and "screw" in line for line in plan.log)
    assert any("helix on XZ" in line for line in plan.log)
    assert any(line.startswith("Decision ") and "base" in line for line in plan.log)
    assert any(line.startswith("  state:") and "Remaining: base" in line for line in plan.log)
    assert any(line.startswith("  options:") and "base:" in line for line in plan.log)


def test_analysis_runs_only_when_the_user_asks() -> None:
    def choose(state: str, options: Mapping[str, str]) -> str:
        del state
        assert "analyze" not in options
        return next(key for key in options if key != "stop")

    plan = realize(_plate(), choose, analyze=True)
    assert "fem.analysis" in [call.tool for call in plan.calls]
    assert any(line.startswith("Analysis: on") for line in plan.log)


def test_a_stop_before_the_design_is_finished_is_retried() -> None:
    answers = iter(("stop", "plate"))

    def choose(state: str, options: Mapping[str, str]) -> str:
        del state, options
        return next(answers)

    plan = realize(_plate(), choose)
    assert any(call.tool == "partdesign.pad" for call in plan.calls)


def test_an_illegal_choice_is_refused() -> None:
    def choose(state: str, options: Mapping[str, str]) -> str:
        del state, options
        return "invented"

    with pytest.raises(FreeCADInputError, match="legal next part"):
        realize(_plate(), choose)


def test_a_helix_without_a_shaft_is_asked_again(monkeypatch: pytest.MonkeyPatch) -> None:
    helix_only = json.dumps(
        {
            "components": (
                {
                    "name": "screw",
                    "steps": ({"diameter": 16, "length": 24, "op": "helix", "pitch": 4},),
                },
            ),
            "summary": "A screw.",
        }
    )
    shaft = json.dumps(
        {
            "components": (
                {
                    "name": "screw",
                    "steps": (
                        {"diameter": 16, "op": "sketch", "profile": "circle"},
                        {"length": 40, "op": "extrude"},
                        {"diameter": 16, "length": 24, "op": "helix", "pitch": 4},
                    ),
                },
            ),
            "summary": "A screw.",
        }
    )
    replies = iter(
        (
            WrittenIntent(text=helix_only, model="openrouter/free"),
            WrittenIntent(text=shaft, model="openrouter/free"),
        )
    )
    monkeypatch.setattr(
        "kala_agent_freecad.cad.compose.write_intent", lambda _prompt: next(replies)
    )

    plan = design_request("a screw")

    tools = [call.tool for call in plan.calls]
    assert "partdesign.pad" in tools
    assert "partdesign.helix" in tools
    assert "fem.analysis" not in tools


def test_pins_on_the_wrong_plane_still_land_on_the_plate() -> None:
    raw = {
        "parts": (
            {
                "features": (
                    {
                        "height": 35,
                        "kind": "pad",
                        "length": 3,
                        "plane": "XY",
                        "profile": "rectangle",
                        "width": 35,
                        "x": 0,
                        "y": 0,
                    },
                    {
                        "diameter": 4,
                        "kind": "pad",
                        "length": 12,
                        "plane": "YZ",
                        "profile": "circle",
                        "x": -8.75,
                        "y": -15.155,
                    },
                    {
                        "diameter": 6,
                        "kind": "pad",
                        "length": 15,
                        "plane": "YZ",
                        "profile": "circle",
                        "reversed": True,
                        "x": 0,
                        "y": 0,
                    },
                ),
                "name": "carrier",
            },
        ),
        "summary": "A carrier.",
    }
    calls = realize(raw, _choose).calls
    planes = [call.arguments.get("plane") for call in calls if call.tool == "sketch.new"]
    assert planes == ["XY", "XY", "XY"]
    plate = next(call for call in calls if call.tool == "sketch.polyline")
    points = plate.arguments["points"]
    assert isinstance(points, list) and points
    corner = points[0]
    assert isinstance(corner, list)
    corner_x, corner_y = corner[0], corner[1]
    assert isinstance(corner_x, (int, float)) and corner_x < -8.75
    assert isinstance(corner_y, (int, float)) and corner_y < -15.155
    pads = [call for call in calls if call.tool == "partdesign.pad"]
    assert pads[-1].arguments["reversed"] is True


def test_a_revolved_circle_is_built_as_a_pad() -> None:
    raw = {
        "parts": (
            {
                "features": (
                    {
                        "diameter": 4,
                        "kind": "revolution",
                        "length": 12,
                        "profile": "circle",
                        "x": 17.5,
                        "y": 0,
                    },
                ),
                "name": "pin",
            },
        ),
        "summary": "A pin.",
    }
    tools = [call.tool for call in realize(raw, _choose).calls]
    assert "partdesign.pad" in tools
    assert "partdesign.revolution" not in tools


def test_six_placed_parts_are_accepted() -> None:
    parts = tuple(
        {
            "features": ({"diameter": 10, "kind": "pad", "length": 5, "profile": "circle"},),
            "name": f"gear-{index}",
            "x": index * 20,
        }
        for index in range(6)
    )
    plan = realize({"parts": parts, "summary": "Six gears."}, _choose)
    assert [call.tool for call in plan.calls].count("partdesign.body") == 6


def test_nine_parts_are_refused() -> None:
    parts = tuple(
        {
            "features": ({"diameter": 10, "kind": "pad", "length": 5, "profile": "circle"},),
            "name": f"gear-{index}",
        }
        for index in range(9)
    )
    with pytest.raises(FreeCADInputError, match="too many parts"):
        realize({"parts": parts, "summary": "Too many."}, _choose)


def test_a_pocket_depth_is_kept_and_a_bare_pocket_goes_through() -> None:
    raw = {
        "parts": (
            {
                "features": (
                    {
                        "height": 100,
                        "kind": "pad",
                        "length": 20,
                        "profile": "rectangle",
                        "width": 100,
                    },
                    {
                        "diameter": 50,
                        "kind": "pocket",
                        "length": 10,
                        "profile": "circle",
                        "x": 50,
                        "y": 50,
                    },
                    {"diameter": 6.5, "kind": "pocket", "profile": "circle", "x": 10, "y": 10},
                ),
                "name": "baseplate",
            },
        ),
        "summary": "A baseplate.",
    }
    plan = realize(raw, _choose)
    pockets = [call for call in plan.calls if call.tool == "partdesign.pocket"]
    assert pockets[0].arguments["length"] == 10
    assert "length" not in pockets[1].arguments


def test_a_part_cannot_start_with_a_pocket() -> None:
    raw = _plate()
    part = raw["parts"]
    assert isinstance(part, tuple)
    feature = dict(part[0]["features"][0])
    feature["kind"] = "pocket"
    broken = {
        "parts": ({"features": (feature,), "name": "plate"},),
        "summary": "A pocket.",
    }
    with pytest.raises(FreeCADInputError, match="solid feature"):
        realize(broken, _choose)


def test_revise_design_follows_the_decision(monkeypatch: pytest.MonkeyPatch) -> None:
    script = load_construction(
        {
            "components": [
                {
                    "name": "plate",
                    "steps": [
                        {"height": 40, "op": "sketch", "profile": "rectangle", "width": 80},
                        {"length": 8, "op": "extrude"},
                    ],
                }
            ],
            "summary": "A plate.",
        }
    )
    readings: dict[str, AnalysisReading] = {
        "accept": load_reading({"verdict": "The plate meets the request."}),
        "script_rewrite": load_reading(
            {
                "rewrite": {
                    "name": "plate",
                    "steps": [
                        {"diameter": 30, "op": "sketch", "profile": "circle"},
                        {"length": 12, "op": "extrude"},
                    ],
                },
                "verdict": "Replace the plate with a disk.",
            }
        ),
        "tool_edit": load_reading(
            {
                "tool_edit": [{"component": "plate", "length": 20, "op": "resize"}],
                "verdict": "The plate is too thin.",
            }
        ),
    }
    chosen = {"name": "accept"}
    logged: list[str] = []

    def read_analysis(
        prompt: str,
        got: object,
        measurements: Mapping[str, object],
    ) -> AnalysisReading:
        del prompt, got, measurements
        return readings[chosen["name"]]

    def choose_revision(state: str) -> ModelChoice:
        assert readings[chosen["name"]].verdict in state
        assert "tool_edit=" in state
        assert "rewrite=" in state
        return ModelChoice(choice=chosen["name"], note=f"Decision test: {chosen['name']}")

    monkeypatch.setattr("kala_agent_freecad.cad.compose.read_analysis", read_analysis)
    monkeypatch.setattr("kala_agent_freecad.cad.compose.choose_revision", choose_revision)

    accepted = revise_design(
        "a plate",
        script,
        {"analysis_status": "completed"},
        logged.append,
    )
    assert accepted is None
    assert any("accept" in line for line in logged)

    chosen["name"] = "tool_edit"
    edited = revise_design("a plate", script, {"peak_stress_mpa": 12})
    assert edited is not None
    pads = [call for call in edited.calls if call.tool == "partdesign.pad"]
    assert pads[0].arguments["length"] == 20

    chosen["name"] = "script_rewrite"
    rewritten = revise_design("a plate", script, {"analysis_detail": "solved"})
    assert rewritten is not None
    tools = [call.tool for call in rewritten.calls]
    assert "sketch.circle" in tools
    assert "sketch.rectangle" not in tools


def _accepted_jack() -> dict[str, object]:
    return {
        "assembly": [
            {"component": "base", "op": "place", "x": 0, "y": 0, "z": 0},
            {"component": "screw", "op": "place", "x": 0, "y": 0, "z": 12},
            {"component": "cup", "op": "place", "x": 0, "y": 0, "z": 90},
            {"component": "tommy", "op": "place", "x": 0, "y": 0, "z": 70},
        ],
        "components": [
            {
                "name": "base",
                "steps": [
                    {"height": 72, "op": "sketch", "profile": "rectangle", "width": 90},
                    {"length": 12, "op": "extrude"},
                ],
            },
            {
                "name": "screw",
                "steps": [
                    {"diameter": 16, "op": "sketch", "profile": "circle"},
                    {"length": 90, "op": "extrude"},
                    {"diameter": 16, "length": 40, "op": "helix", "pitch": 4},
                ],
            },
            {
                "name": "cup",
                "steps": [
                    {"diameter": 36, "op": "sketch", "profile": "circle"},
                    {"length": 20, "op": "extrude"},
                ],
            },
            {
                "name": "tommy",
                "steps": [
                    {"height": 10, "op": "sketch", "profile": "rectangle", "width": 80},
                    {"length": 10, "op": "extrude"},
                ],
            },
        ],
        "summary": "A screw jack.",
    }


def test_a_refused_script_is_sent_back_to_the_model(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    seen: list[str] = []
    jack = json.dumps(_accepted_jack())

    def write(prompt: str) -> WrittenIntent:
        seen.append(prompt)
        if len(seen) == 1:
            return WrittenIntent(
                text='{"summary":"no","components":[{"name":"plate","steps":[{"op":"sketch","profile":"gear"}]}]}',
                model="cohere/north-mini-code:free",
            )
        return WrittenIntent(text=jack, model="cohere/north-mini-code:free")

    monkeypatch.setattr("kala_agent_freecad.cad.compose.write_intent", write)

    logged: list[str] = []
    plan = design_request("a screw jack", on_log=logged.append)

    assert len(seen) == 2
    assert "Model input:" in logged
    assert "Model output:" in logged
    assert any("profile" in line for line in logged)
    assert "a screw jack" in seen[1]
    assert "refused" in seen[1]
    assert plan.summary == "A screw jack."
    assert "partdesign.helix" in [call.tool for call in plan.calls]


def test_a_missing_duty_is_sent_back_before_the_build(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[str] = []
    cylinder = json.dumps(
        {
            "components": [
                {
                    "name": "body",
                    "steps": [
                        {"diameter": 16, "op": "sketch", "profile": "circle"},
                        {"length": 40, "op": "extrude"},
                    ],
                }
            ],
            "summary": "A cylinder.",
        }
    )

    def write(prompt: str) -> WrittenIntent:
        seen.append(prompt)
        if len(seen) == 1:
            return WrittenIntent(text=cylinder, model="cohere/north-mini-code:free")
        return WrittenIntent(text=json.dumps(_accepted_jack()), model="cohere/north-mini-code:free")

    monkeypatch.setattr("kala_agent_freecad.cad.compose.write_intent", write)
    logged: list[str] = []
    plan = design_request("a screw jack", on_log=logged.append)

    assert "tommy bar" in seen[1]
    assert "plate wider than the screw" in seen[1]
    assert any("tommy bar" in line for line in logged)
    assert plan.summary == "A screw jack."
    assert [call.tool for call in plan.calls].count("partdesign.body") == 4


def test_a_second_refused_design_is_not_built(monkeypatch: pytest.MonkeyPatch) -> None:
    cylinder = json.dumps(
        {
            "components": [
                {
                    "name": "body",
                    "steps": [
                        {"diameter": 16, "op": "sketch", "profile": "circle"},
                        {"length": 40, "op": "extrude"},
                    ],
                }
            ],
            "summary": "A cylinder.",
        }
    )
    monkeypatch.setattr(
        "kala_agent_freecad.cad.compose.write_intent",
        lambda _prompt: WrittenIntent(text=cylinder, model="cohere/north-mini-code:free"),
    )

    with pytest.raises(FreeCADInputError, match="tommy bar"):
        design_request("a screw jack")


def test_a_missing_key_does_not_invent_a_plate(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.delenv("OPENROUTER_API_KEY", raising=False)
    monkeypatch.delenv("OPENROUTER_DECIDE_API_KEY", raising=False)
    monkeypatch.delenv("KALA_OPENCODE_MODEL", raising=False)
    monkeypatch.setattr("kala_agent_freecad.cad.models.Path.home", lambda: tmp_path)
    with pytest.raises(FreeCADDependencyError, match="reasoning key"):
        design_request("a screw jack")


def _built_plate() -> dict[str, object]:
    return {
        "assembly": [{"component": "plate", "op": "place", "x": 0, "y": 0, "z": 0}],
        "components": [
            {
                "name": "plate",
                "steps": [
                    {"height": 40, "op": "sketch", "profile": "rectangle", "width": 80},
                    {"length": 8, "op": "extrude"},
                ],
            }
        ],
        "summary": "An 80 by 40 by 8 mm plate.",
    }


def _plate_with_hole() -> dict[str, object]:
    return {
        "assembly": [{"component": "plate", "op": "place", "x": 0, "y": 0, "z": 0}],
        "components": [
            {
                "name": "plate",
                "steps": [
                    {"height": 40, "op": "sketch", "profile": "rectangle", "width": 80},
                    {"length": 8, "op": "extrude"},
                    {"diameter": 10, "op": "sketch", "profile": "circle"},
                    {"op": "pocket"},
                ],
            }
        ],
        "summary": "The plate with a 10 mm hole through the center.",
    }


def _different_part() -> dict[str, object]:
    return {
        "assembly": [{"component": "block", "op": "place", "x": 0, "y": 0, "z": 0}],
        "components": [
            {
                "name": "block",
                "steps": [
                    {"height": 20, "op": "sketch", "profile": "rectangle", "width": 20},
                    {"length": 8, "op": "extrude"},
                    {"diameter": 10, "op": "sketch", "profile": "circle"},
                    {"op": "pocket"},
                ],
            }
        ],
        "summary": "A different part.",
    }


def test_a_change_keeps_the_plate_and_adds_the_hole(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[str] = []

    def write(prompt: str) -> WrittenIntent:
        seen.append(prompt)
        return WrittenIntent(
            text=json.dumps(_plate_with_hole()), model="cohere/north-mini-code:free"
        )

    monkeypatch.setattr("kala_agent_freecad.cad.compose.write_intent", write)
    logged: list[str] = []
    plan = continue_design("a 10 mm hole in the center", _built_plate(), on_log=logged.append)

    assert seen[0].startswith("Change this same design.")
    assert "plate" in seen[0]
    assert "Changing the current design." in logged
    assert plan.kind == "revised"
    assert "partdesign.pocket" in [call.tool for call in plan.calls]
    assert "sketch.rectangle" in [call.tool for call in plan.calls]


def test_a_replacement_is_sent_back_and_then_refused(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[str] = []

    def write(prompt: str) -> WrittenIntent:
        seen.append(prompt)
        if len(seen) == 1:
            return WrittenIntent(
                text=json.dumps(_different_part()), model="cohere/north-mini-code:free"
            )
        return WrittenIntent(
            text=json.dumps(_plate_with_hole()), model="cohere/north-mini-code:free"
        )

    monkeypatch.setattr("kala_agent_freecad.cad.compose.write_intent", write)
    plan = continue_design("a 10 mm hole in the center", _built_plate())

    assert "replaced the design" in seen[1]
    assert plan.summary == "The plate with a 10 mm hole through the center."

    def replace(prompt: str) -> WrittenIntent:
        del prompt
        return WrittenIntent(
            text=json.dumps(_different_part()), model="cohere/north-mini-code:free"
        )

    monkeypatch.setattr("kala_agent_freecad.cad.compose.write_intent", replace)
    with pytest.raises(FreeCADInputError, match="replaced the current design"):
        continue_design("a 10 mm hole in the center", _built_plate())


def test_a_new_part_must_add_a_component(monkeypatch: pytest.MonkeyPatch) -> None:
    pin = _plate_with_hole()
    components = pin["components"]
    assert isinstance(components, list)
    components.append(
        {
            "name": "pin",
            "steps": [
                {"diameter": 6, "op": "sketch", "profile": "circle"},
                {"length": 12, "op": "extrude"},
            ],
        }
    )
    assembly = pin["assembly"]
    assert isinstance(assembly, list)
    assembly.append({"component": "pin", "op": "place", "x": 30, "y": 0, "z": 8})
    seen: list[str] = []

    def write(prompt: str) -> WrittenIntent:
        seen.append(prompt)
        if len(seen) == 1:
            return WrittenIntent(
                text=json.dumps(_plate_with_hole()), model="cohere/north-mini-code:free"
            )
        return WrittenIntent(text=json.dumps(pin), model="cohere/north-mini-code:free")

    monkeypatch.setattr("kala_agent_freecad.cad.compose.write_intent", write)
    plan = continue_design("a 6 mm pin", _built_plate(), add_part=True)

    assert "Add one new component" in seen[0]
    assert "did not add one new component" in seen[1]
    assert plan.calls
    assert [call.tool for call in plan.calls].count("partdesign.body") == 2
