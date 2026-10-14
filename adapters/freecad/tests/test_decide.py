"""The decision model emits toolbar calls, not a Python script."""

from __future__ import annotations

import pytest

from kala_agent_freecad.cad.decide import decide
from kala_agent_freecad.cad.design import _HANDLERS
from kala_agent_freecad.errors import FreeCADInputError


def _tools(prompt: str) -> list[str]:
    return [call.tool for call in decide(prompt).calls]


def test_a_plate_is_a_constrained_sketch_and_a_pad() -> None:
    plan = decide("80 x 40 x 8 mm plate")
    tools = [call.tool for call in plan.calls]
    assert tools[:3] == ["partdesign.body", "sketch.new", "sketch.rectangle"]
    assert "sketch.distance_x" in tools
    assert "sketch.distance_y" in tools
    assert "sketch.leave" in tools
    assert "partdesign.pad" in tools
    assert "Part.makeBox" not in plan.summary
    assert plan.calls[-1].tool == "fem.result"
    assert all(call.tool in _HANDLERS for call in plan.calls)


def test_a_box_uses_the_same_feature_procedure() -> None:
    plan = decide("a 30 x 20 x 10 mm box")
    rectangle = plan.calls[2]
    assert rectangle.tool == "sketch.rectangle"
    assert rectangle.arguments["width"] == 30
    assert rectangle.arguments["height"] == 20
    pad = next(call for call in plan.calls if call.tool == "partdesign.pad")
    assert pad.arguments["length"] == 10
    assert "partdesign.pocket" not in _tools("a 30 x 20 x 10 mm box")


def test_a_hole_is_a_later_sketch_and_a_pocket() -> None:
    tools = _tools("120 by 40 by 8 mm plate with an 8 mm hole")
    assert tools.index("partdesign.pad") < tools.index("sketch.circle")
    assert tools.index("sketch.circle") < tools.index("partdesign.pocket")
    assert "fem.solve" in tools


def test_a_bracket_is_a_closed_polyline_then_a_pad() -> None:
    tools = _tools("80 x 40 x 8 x 40 mm bracket")
    assert "sketch.polyline" in tools
    assert "partdesign.pad" in tools
    assert all(tool in _HANDLERS for tool in tools)


def test_a_cylinder_is_a_constrained_circle_then_a_pad() -> None:
    tools = _tools("a 10 mm radius cylinder 40 mm tall")
    assert "sketch.circle" in tools
    assert "sketch.diameter" in tools
    assert "partdesign.pad" in tools
    assert "partdesign.revolution" not in tools


def test_a_turned_part_uses_revolution() -> None:
    tools = _tools("a turned pin 10 mm radius and 40 mm tall")
    assert "partdesign.revolution" in tools
    assert "partdesign.pad" not in tools


def test_an_empty_request_is_refused() -> None:
    with pytest.raises(FreeCADInputError, match="must not be empty"):
        decide("   ")
    with pytest.raises(FreeCADInputError, match="must be a string"):
        decide(None)  # type: ignore[arg-type]
