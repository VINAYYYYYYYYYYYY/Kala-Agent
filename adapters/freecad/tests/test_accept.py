"""A script is built only when the engineering duties are present."""

from __future__ import annotations

from kala_agent.contracts import load_construction
from kala_agent_freecad.cad.accept import accept_design


def _jack(
    *,
    diameter: float = 16,
    pitch: float = 4,
    cup_x: float = 0,
) -> dict[str, object]:
    base = max(90.0, diameter * 3)
    return {
        "assembly": [
            {"component": "base", "op": "place", "x": 0, "y": 0, "z": 0},
            {"component": "screw", "op": "place", "x": 0, "y": 0, "z": 12},
            {"component": "cup", "op": "place", "x": cup_x, "y": 0, "z": 90},
            {"component": "tommy", "op": "place", "x": 0, "y": 0, "z": 70},
        ],
        "components": [
            {
                "name": "base",
                "steps": [
                    {"height": base * 0.8, "op": "sketch", "profile": "rectangle", "width": base},
                    {"length": 12, "op": "extrude"},
                ],
            },
            {
                "name": "screw",
                "steps": [
                    {"diameter": diameter, "op": "sketch", "profile": "circle"},
                    {"length": 90, "op": "extrude"},
                    {"diameter": diameter, "length": 40, "op": "helix", "pitch": pitch},
                ],
            },
            {
                "name": "cup",
                "steps": [
                    {"diameter": diameter + 20, "op": "sketch", "profile": "circle"},
                    {"length": 20, "op": "extrude"},
                ],
            },
            {
                "name": "tommy",
                "steps": [
                    {
                        "height": 10,
                        "op": "sketch",
                        "profile": "rectangle",
                        "width": diameter + 64,
                    },
                    {"length": 10, "op": "extrude"},
                ],
            },
        ],
        "summary": "A screw jack.",
    }


def test_a_complete_screw_jack_is_accepted() -> None:
    duties = accept_design("a screw jack", load_construction(_jack()))

    assert duties == []


def test_a_cylinder_is_not_a_screw_jack() -> None:
    script = load_construction(
        {
            "assembly": [{"component": "body", "op": "place", "x": 0, "y": 0, "z": 0}],
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

    duties = accept_design("a screw jack", script)

    assert any("plate wider than the screw" in duty for duty in duties)
    assert any("shaft with a helix" in duty for duty in duties)
    assert any("cup" in duty for duty in duties)
    assert any("tommy bar" in duty for duty in duties)


def test_a_scattered_screw_jack_is_refused() -> None:
    duties = accept_design("a screw jack", load_construction(_jack(cup_x=120)))

    assert duties == ["The base, the screw, and the cup must share one axis."]


def test_a_hole_that_is_its_own_body_is_refused() -> None:
    script = load_construction(
        {
            "components": [
                {
                    "name": "plate",
                    "steps": [
                        {"height": 40, "op": "sketch", "profile": "rectangle", "width": 80},
                        {"length": 8, "op": "extrude"},
                    ],
                },
                {
                    "name": "hole",
                    "steps": [
                        {"diameter": 10, "op": "sketch", "profile": "circle"},
                        {"length": 8, "op": "extrude"},
                    ],
                },
            ],
            "summary": "A plate and a separate hole.",
        }
    )

    duties = accept_design("an 80 x 40 x 8 mm plate with a 10 mm hole in the center", script)

    assert duties == ["A hole must be a pocket in an existing part, not a separate body."]


def test_a_plain_plate_is_left_alone() -> None:
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
            "summary": "An 80 by 40 by 8 mm plate.",
        }
    )

    assert accept_design("an 80 x 40 by 8 mm plate", script) == []


def test_a_named_screw_diameter_overrides_the_default_range() -> None:
    oversized = _jack(diameter=100, pitch=16)

    refused = accept_design("a screw jack", load_construction(oversized))
    accepted = accept_design("a screw jack with a 100 mm screw", load_construction(oversized))

    assert any("between 12 and 80 mm" in duty for duty in refused)
    assert accepted == []
