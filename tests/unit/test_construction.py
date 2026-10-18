"""The construction script is refused before any CAD software sees it."""

from __future__ import annotations

from collections.abc import Callable

import pytest

from kala_agent.contracts import (
    ConstructionError,
    RevisionAction,
    load_analysis,
    load_construction,
    load_reading,
)


def _rectangle(width: float, height: float) -> dict[str, object]:
    return {"height": height, "op": "sketch", "profile": "rectangle", "width": width}


def _extrude(length: float) -> dict[str, object]:
    return {"length": length, "op": "extrude"}


def _component(name: str, steps: list[dict[str, object]]) -> dict[str, object]:
    return {"name": name, "steps": steps}


def _plate_body(
    name: str, *, width: float = 80, height: float = 40, length: float = 8
) -> dict[str, object]:
    return _component(name, [_rectangle(width, height), _extrude(length)])


def _script(
    components: list[dict[str, object]],
    assembly: list[dict[str, object]] | None = None,
    summary: str = "A plate.",
) -> dict[str, object]:
    raw: dict[str, object] = {"components": components, "summary": summary}
    if assembly is not None:
        raw["assembly"] = assembly
    return raw


def test_a_nested_profile_object_is_the_models_sketch() -> None:
    script = load_construction(
        {
            "assembly": [{"component": "Screw", "op": "place", "x": 0, "y": 0, "z": 5}],
            "components": [
                {
                    "name": "Screw",
                    "steps": [
                        {
                            "op": "sketch",
                            "plane": "XY",
                            "profile": {"circle": {"diameter": 20}, "x": 0, "y": 0},
                        },
                        {"length": 100, "op": "extrude"},
                        {"diameter": 20, "length": 100, "op": "helix", "pitch": 5},
                    ],
                }
            ],
            "summary": "A screw jack.",
        }
    )
    sketch = script.components[0].steps[0]
    assert sketch.profile == "circle"
    assert sketch.diameter == 20
    assert script.components[0].steps[2].op == "helix"


def test_a_long_summary_is_kept() -> None:
    summary = "Wall-mounted belt-driven spindle. " * 20
    script = load_construction(_script([_plate_body("plate")], summary=summary))
    assert script.summary == summary.strip()


def test_a_loose_sketch_profile_becomes_a_rectangle_or_a_circle() -> None:
    script = load_construction(
        _script(
            [
                _component(
                    "shaft",
                    [
                        {"diameter": 16, "op": "sketch", "profile": "circular"},
                        _extrude(40),
                        {"height": 40, "op": "sketch", "profile": "flange", "width": 40},
                        _extrude(8),
                    ],
                )
            ]
        )
    )
    profiles = [step.profile for step in script.components[0].steps if step.op == "sketch"]
    assert profiles == ["circle", "rectangle"]


def test_a_plate_place_and_fuse_load() -> None:
    script = load_construction(
        _script(
            [_plate_body("plate"), _plate_body("rib", width=10, length=20)],
            [
                {"component": "plate", "op": "place", "x": 0, "y": 0, "z": 0},
                {"components": ["plate", "rib"], "op": "fuse"},
            ],
            summary="A plate and a rib.",
        )
    )

    assert script.summary == "A plate and a rib."
    assert script.components[0].name == "plate"
    assert [step.op for step in script.components[0].steps] == ["sketch", "extrude"]
    assert script.components[0].steps[0].profile == "rectangle"
    assert script.components[0].steps[1].length == 8
    assert script.assembly[0].op == "place"
    assert script.assembly[0].component == "plate"
    assert script.assembly[1].op == "fuse"
    assert script.assembly[1].components == ("plate", "rib")


@pytest.mark.parametrize(
    ("load", "raw"),
    [
        (
            load_construction,
            _script([_component("plate", [{"op": "python", "profile": "rectangle"}])]),
        ),
        (
            load_construction,
            _script(
                [_plate_body("plate")],
                [{"component": "plate", "op": "weld"}],
            ),
        ),
        (load_analysis, {"steps": [{"op": "python"}]}),
        (
            load_reading,
            {"tool_edit": [{"component": "plate", "op": "twist"}], "verdict": "No."},
        ),
    ],
)
def test_an_unknown_operation_is_refused(load: Callable[[object], object], raw: object) -> None:
    with pytest.raises(ConstructionError, match="unknown operation") as caught:
        load(raw)

    message = str(caught.value)
    assert message.startswith("unknown operation")
    assert "\n" not in message
    assert "pydantic" not in message.lower()


def test_a_component_that_starts_with_pocket_is_refused() -> None:
    with pytest.raises(ConstructionError, match="solid"):
        load_construction(_script([_component("plate", [{"op": "pocket"}])]))


def test_a_helix_without_a_prior_solid_is_refused() -> None:
    with pytest.raises(ConstructionError, match="solid"):
        load_construction(
            _script(
                [
                    _component(
                        "screw",
                        [
                            {"diameter": 16, "op": "sketch", "profile": "circle"},
                            {"diameter": 16, "length": 24, "op": "helix", "pitch": 4},
                        ],
                    )
                ],
                summary="A screw.",
            )
        )


def test_an_assembly_that_names_a_missing_component_is_refused() -> None:
    with pytest.raises(ConstructionError, match="missing component cup"):
        load_construction(
            _script(
                [_plate_body("plate")],
                [{"component": "cup", "op": "place"}],
            )
        )


def test_an_analysis_script_missing_solve_is_refused() -> None:
    script = load_analysis(
        {
            "steps": [
                {"name": "StructuralSteel", "op": "material"},
                {"face": 1, "op": "fix"},
                {"face": 2, "force_n": 100, "op": "force"},
                {"op": "mesh"},
                {"op": "solve"},
            ]
        }
    )

    assert [step.op for step in script.steps] == ["material", "fix", "force", "mesh", "solve"]
    with pytest.raises(ConstructionError, match="solve"):
        load_analysis({"steps": [{"op": "material"}, {"op": "fix"}, {"op": "mesh"}]})


def test_an_edit_resize_without_length_or_diameter_is_refused() -> None:
    reading = load_reading(
        {
            "tool_edit": [{"component": "plate", "length": 12, "op": "resize"}],
            "verdict": "Make the plate thicker.",
        }
    )

    assert reading.verdict == "Make the plate thicker."
    assert reading.tool_edit[0].op == "resize"
    assert reading.tool_edit[0].length == 12
    assert reading.rewrite is None
    with pytest.raises(ConstructionError, match="length or a diameter"):
        load_reading(
            {
                "tool_edit": [{"component": "plate", "op": "resize"}],
                "verdict": "Thicker.",
            }
        )


def test_revision_action_has_accept_tool_edit_and_script_rewrite() -> None:
    assert issubclass(ConstructionError, ValueError)
    assert RevisionAction("accept") is RevisionAction.ACCEPT
    assert RevisionAction("tool_edit") is RevisionAction.TOOL_EDIT
    assert RevisionAction("script_rewrite") is RevisionAction.SCRIPT_REWRITE
    assert {item.value for item in RevisionAction} == {"accept", "tool_edit", "script_rewrite"}


def test_a_helix_follows_a_shaft_without_another_sketch() -> None:
    script = load_construction(
        _script(
            [
                _component(
                    "screw",
                    [
                        {"diameter": 16, "op": "sketch", "profile": "circle"},
                        {"length": 40, "op": "extrude"},
                        {"diameter": 16, "length": 24, "op": "helix", "pitch": 4},
                    ],
                )
            ],
            summary="A screw.",
        )
    )

    assert [step.op for step in script.components[0].steps] == ["sketch", "extrude", "helix"]
    with pytest.raises(ConstructionError, match="consumed before a helix"):
        load_construction(
            _script(
                [
                    _component(
                        "screw",
                        [
                            {"diameter": 16, "op": "sketch", "profile": "circle"},
                            {"length": 40, "op": "extrude"},
                            {"diameter": 18, "op": "sketch", "profile": "circle"},
                            {"diameter": 16, "length": 24, "op": "helix", "pitch": 4},
                        ],
                    )
                ],
                summary="A screw.",
            )
        )


def test_revolve_angle_defaults_to_a_full_turn() -> None:
    script = load_construction(
        _script(
            [
                _component(
                    "disk",
                    [_rectangle(10, 4), {"op": "revolve"}],
                )
            ],
            summary="A turned disk.",
        )
    )

    assert script.components[0].steps[1].angle == 360
    with pytest.raises(ConstructionError, match="360"):
        load_construction(
            _script(
                [_component("disk", [_rectangle(10, 4), {"angle": 400, "op": "revolve"}])],
                summary="A turned disk.",
            )
        )


def test_analysis_rejects_a_later_step_before_an_earlier_rank() -> None:
    script = load_analysis(
        {
            "steps": [
                {"force_n": 10, "op": "force"},
                {"op": "pressure", "pressure_mpa": 2},
                {"op": "solve"},
            ]
        }
    )

    assert [step.op for step in script.steps] == ["force", "pressure", "solve"]
    with pytest.raises(ConstructionError, match="out of order"):
        load_analysis({"steps": [{"op": "solve"}, {"op": "mesh"}]})


def test_reversed_is_only_meaningful_on_extrude() -> None:
    script = load_construction(
        _script(
            [
                _component(
                    "plate", [_rectangle(80, 40), {"length": 8, "op": "extrude", "reversed": True}]
                )
            ]
        )
    )

    assert script.components[0].steps[1].reversed is True
    with pytest.raises(ConstructionError, match="reversed"):
        load_construction(_script([_component("plate", [{"op": "pocket", "reversed": True}])]))


def test_sizes_and_names_stay_inside_the_protocol() -> None:
    script = load_construction(
        _script(
            [_component("A" + ("b" * 39), [_rectangle(0.001, 5000), _extrude(8)])],
            [{"component": "A" + ("b" * 39), "op": "place", "x": -5000, "y": 5000, "z": 0}],
        )
    )

    assert script.components[0].steps[0].width == 0.001
    assert script.assembly[0].x == -5000
    with pytest.raises(ConstructionError, match="component name"):
        load_construction(_script([_plate_body("1plate")]))
    with pytest.raises(ConstructionError, match="width"):
        load_construction(_script([_component("plate", [_rectangle(0.0009, 10), _extrude(8)])]))
    with pytest.raises(ConstructionError, match="pressure_mpa"):
        load_analysis({"steps": [{"op": "pressure", "pressure_mpa": 5000.1}, {"op": "solve"}]})
