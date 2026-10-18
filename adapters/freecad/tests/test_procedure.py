"""The feature procedure, checked without opening FreeCAD."""

from __future__ import annotations

import pytest

from kala_agent_freecad.cad.catalog import CATALOG
from kala_agent_freecad.cad.procedure import CLASSIFIED_TOOLS, DesignCall, parse_calls
from kala_agent_freecad.errors import FreeCADInputError


def test_every_catalog_tool_belongs_to_a_stage() -> None:
    assert CLASSIFIED_TOOLS == CATALOG


def test_a_line_before_a_sketch_is_refused() -> None:
    with pytest.raises(FreeCADInputError, match="not available during the start stage"):
        parse_calls(({"arguments": {}, "tool": "sketch.line"},))


def test_a_pad_before_a_profile_is_refused() -> None:
    with pytest.raises(FreeCADInputError, match="not available during the body stage"):
        parse_calls(
            (
                {"arguments": {}, "tool": "partdesign.body"},
                {"arguments": {"length": 8}, "tool": "partdesign.pad"},
            )
        )


def test_a_fillet_before_a_solid_is_refused() -> None:
    with pytest.raises(FreeCADInputError, match="not available during the start stage"):
        parse_calls(({"arguments": {"radius": 1}, "tool": "partdesign.fillet"},))


def test_a_pocket_needs_a_base_solid() -> None:
    with pytest.raises(FreeCADInputError, match="not available"):
        parse_calls(
            (
                {"arguments": {}, "tool": "partdesign.body"},
                {"arguments": {"plane": "XY"}, "tool": "sketch.new"},
                {"arguments": {}, "tool": "sketch.leave"},
                {"arguments": {}, "tool": "partdesign.pocket"},
            )
        )


def test_an_open_sketch_does_not_count_as_a_solid() -> None:
    with pytest.raises(FreeCADInputError, match="left open"):
        parse_calls(
            (
                {"arguments": {}, "tool": "partdesign.body"},
                {"arguments": {"plane": "XY"}, "tool": "sketch.new"},
                {"arguments": {}, "tool": "sketch.line"},
            )
        )


def test_a_second_body_is_legal_after_a_solid() -> None:
    calls = parse_calls(
        (
            {"arguments": {}, "tool": "partdesign.body"},
            {"arguments": {"plane": "XY"}, "tool": "sketch.new"},
            {"arguments": {}, "tool": "sketch.rectangle"},
            {"arguments": {}, "tool": "sketch.leave"},
            {"arguments": {"length": 8}, "tool": "partdesign.pad"},
            {"arguments": {"x": 0, "y": 0, "z": 12}, "tool": "partdesign.placement"},
            {"arguments": {}, "tool": "partdesign.body"},
            {"arguments": {"plane": "XY"}, "tool": "sketch.new"},
            {"arguments": {}, "tool": "sketch.circle"},
            {"arguments": {}, "tool": "sketch.leave"},
            {"arguments": {"length": 20}, "tool": "partdesign.pad"},
        )
    )
    assert [call.tool for call in calls].count("partdesign.body") == 2


def test_a_model_script_longer_than_160_calls_is_accepted() -> None:
    cycle = (
        {"arguments": {}, "tool": "partdesign.body"},
        {"arguments": {"plane": "XY"}, "tool": "sketch.new"},
        {"arguments": {}, "tool": "sketch.rectangle"},
        {"arguments": {}, "tool": "sketch.leave"},
        {"arguments": {"length": 8}, "tool": "partdesign.pad"},
        {"arguments": {"x": 0, "y": 0, "z": 0}, "tool": "partdesign.placement"},
    )
    calls = parse_calls(cycle * 30)
    assert len(calls) == 180


def test_a_body_sketch_and_pad_reach_a_solid() -> None:
    calls = parse_calls(
        (
            {"arguments": {}, "tool": "partdesign.body"},
            {"arguments": {"plane": "XY"}, "tool": "sketch.new"},
            {"arguments": {}, "tool": "sketch.rectangle"},
            {"arguments": {}, "tool": "sketch.leave"},
            {"arguments": {"length": 8}, "tool": "partdesign.pad"},
            {"arguments": {}, "tool": "fem.solve"},
        )
    )
    assert calls[-1] == DesignCall(tool="fem.solve", arguments={})


def test_calls_must_be_a_list_of_known_tools() -> None:
    with pytest.raises(FreeCADInputError, match="nonempty list"):
        parse_calls(())
    with pytest.raises(FreeCADInputError, match="unknown CAD tool"):
        parse_calls(({"arguments": {}, "tool": "sketch.nope"},))
    with pytest.raises(FreeCADInputError, match="must be an object"):
        parse_calls(("sketch.line",))
