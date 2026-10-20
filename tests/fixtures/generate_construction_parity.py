#!/usr/bin/env python3
"""Freeze the construction-semantics corpus into a core-readable fixture.

Run once from the repository root (the command is recorded in the JSON)::

    PYTHONPATH=adapters/solidedge/src:src .venv/bin/python \
        tests/fixtures/generate_construction_parity.py

The generator is the only place that imports the Solid Edge oracle. It records
the oracle source digest so the core parity test can detect drift without ever
importing the adapter itself. Every measured number printed in
``docs/construction-semantics.md`` is encoded as ``documented`` and checked
against the oracle before the fixture is written.
"""

from __future__ import annotations

import hashlib
import json
import sys
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any, Protocol

REPO_ROOT = Path(__file__).resolve().parents[2]
CORE_SRC = REPO_ROOT / "src"
CLIENT_SRC = REPO_ROOT / "adapters" / "solidedge" / "src"
BRIDGE_SRC = REPO_ROOT / "adapters" / "solidedge-bridge" / "src"
for _path in (CORE_SRC, CLIENT_SRC, BRIDGE_SRC):
    if str(_path) not in sys.path:
        sys.path.insert(0, str(_path))

from kala_agent_solidedge.oracle import (  # type: ignore[import-not-found]  # noqa: E402
    OracleExpectation,
    expectation,
)

from kala_agent.contracts.construction import (  # noqa: E402
    ConstructionScript,
    load_construction,
)

FIXTURE_PATH = Path(__file__).with_name("construction_parity.json")
ORACLE_SOURCE = CLIENT_SRC / "kala_agent_solidedge" / "oracle.py"
GENERATOR_COMMAND = (
    "PYTHONPATH=adapters/solidedge/src:src .venv/bin/python "
    "tests/fixtures/generate_construction_parity.py"
)
VOLUME_RELATIVE_TOLERANCE = 1e-6
BOUNDS_ABSOLUTE_TOLERANCE_MM = 0.001
_DECIMALS = 6


class _RangeBox(Protocol):
    """The subset of ``kala_solidedge_bridge.protocol.RangeBox`` this script reads."""

    min_mm: tuple[float, float, float]
    max_mm: tuple[float, float, float]


def main() -> None:
    cases = _cases()
    _verify_documented(cases)
    fixture = {
        "schema_version": 1,
        "generator": {
            "command": GENERATOR_COMMAND,
            "oracle_module": "kala_agent_solidedge.oracle",
            "oracle_source": str(ORACLE_SOURCE.relative_to(REPO_ROOT)),
            "oracle_sha256": hashlib.sha256(ORACLE_SOURCE.read_bytes()).hexdigest(),
            "comparison": {
                "volume_relative_tolerance": VOLUME_RELATIVE_TOLERANCE,
                "bounds_absolute_tolerance_mm": BOUNDS_ABSOLUTE_TOLERANCE_MM,
            },
        },
        "cases": cases,
    }
    FIXTURE_PATH.write_text(
        json.dumps(fixture, indent=2, ensure_ascii=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    print(f"wrote {FIXTURE_PATH} with {len(cases)} cases")


# --------------------------------------------------------------------- cases


def _cases() -> list[dict[str, Any]]:
    cases: list[dict[str, Any]] = []
    cases.extend(_rectangle_pad_cases())
    cases.extend(_circle_pad_cases())
    cases.extend(_revolve_cases())
    cases.extend(_pocket_cases())
    cases.extend(_assembly_cases())
    return cases


def _rectangle_pad_cases() -> list[dict[str, Any]]:
    documented_bounds = {
        "XY": ((2.0, 3.0, 0.0), (6.0, 8.0, 6.0)),
        "XZ": ((2.0, -6.0, 3.0), (6.0, 0.0, 8.0)),
        "YZ": ((0.0, 2.0, 3.0), (6.0, 6.0, 8.0)),
    }
    cases: list[dict[str, Any]] = []
    for plane in ("XY", "XZ", "YZ"):
        for offset in (False, True):
            for reversed_ in (False, True):
                x = 2.0 if offset else 0.0
                y = 3.0 if offset else 0.0
                script = _script(
                    [
                        _rectangle_component(
                            "plate",
                            plane=plane,
                            width=4.0,
                            height=5.0,
                            length=6.0,
                            x=x,
                            y=y,
                            reversed_=reversed_,
                        )
                    ]
                )
                documented = None
                if offset and not reversed_:
                    minimum, maximum = documented_bounds[plane]
                    documented = {
                        "source": (
                            "docs/construction-semantics.md#a-worked-example-offset-rectangle-pad-6"
                        ),
                        "volume_mm3": 120.0,
                        "min_mm": list(minimum),
                        "max_mm": list(maximum),
                    }
                suffix = "offset" if offset else "origin"
                direction = "reversed" if reversed_ else "forward"
                cases.append(
                    _case(
                        f"rectangle_pad_{plane.lower()}_{suffix}_{direction}",
                        (
                            f"4 x 5 mm rectangle on {plane} at ({x:g}, {y:g}), "
                            f"extruded 6 mm {'reversed' if reversed_ else 'forward'}"
                        ),
                        "rectangle_pad",
                        script,
                        documented=documented,
                    )
                )
    return cases


def _circle_pad_cases() -> list[dict[str, Any]]:
    cases: list[dict[str, Any]] = []
    for plane in ("XY", "XZ", "YZ"):
        for offset in (False, True):
            x = 1.0 if offset else 0.0
            y = 1.0 if offset else 0.0
            script = _script(
                [
                    _circle_component(
                        "pin",
                        plane=plane,
                        diameter=4.0,
                        length=5.0,
                        x=x,
                        y=y,
                    )
                ]
            )
            suffix = "offset" if offset else "origin"
            cases.append(
                _case(
                    f"circle_pad_{plane.lower()}_{suffix}",
                    (
                        f"circle of diameter 4 mm on {plane} at ({x:g}, {y:g}), "
                        "extruded 5 mm forward"
                    ),
                    "circle_pad",
                    script,
                )
            )
    return cases


def _revolve_cases() -> list[dict[str, Any]]:
    circle_xy = _script(
        [
            {
                "name": "pin",
                "steps": [
                    {"op": "sketch", "profile": "circle", "plane": "XY", "diameter": 6.0},
                    {"op": "revolve"},
                ],
            }
        ]
    )
    circle_yz = _script(
        [
            {
                "name": "pin",
                "steps": [
                    {"op": "sketch", "profile": "circle", "plane": "YZ", "diameter": 6.0},
                    {"op": "revolve", "length": 8.0},
                ],
            }
        ]
    )
    rectangle_xy = _script(
        [
            {
                "name": "bush",
                "steps": [
                    {
                        "op": "sketch",
                        "profile": "rectangle",
                        "plane": "XY",
                        "width": 4.0,
                        "height": 5.0,
                        "x": 2.0,
                        "y": 0.0,
                    },
                    {"op": "revolve", "angle": 360.0},
                ],
            }
        ]
    )
    rectangle_yz = _script(
        [
            {
                "name": "bush",
                "steps": [
                    {
                        "op": "sketch",
                        "profile": "rectangle",
                        "plane": "YZ",
                        "width": 5.0,
                        "height": 4.0,
                        "x": 0.0,
                        "y": 2.0,
                    },
                    {"op": "revolve", "angle": 360.0},
                ],
            }
        ]
    )
    return [
        _case(
            "circle_revolve_xy_quirk",
            "circle revolve on XY is lowered to an extrusion of the diameter",
            "circle_revolve_quirk",
            circle_xy,
        ),
        _case(
            "circle_revolve_yz_quirk_with_length",
            "circle revolve on YZ with a length is lowered to that extrusion",
            "circle_revolve_quirk",
            circle_yz,
        ),
        _case(
            "rectangle_revolve_xy",
            "4 x 5 mm rectangle at x = 2 on XY revolved 360 degrees about Y",
            "rectangle_revolve",
            rectangle_xy,
        ),
        _case(
            "rectangle_revolve_yz",
            "5 x 4 mm rectangle at z = 2 on YZ revolved 360 degrees about Y",
            "rectangle_revolve",
            rectangle_yz,
        ),
    ]


def _pocket_cases() -> list[dict[str, Any]]:
    cases: list[dict[str, Any]] = []
    base = [
        {
            "name": "plate",
            "steps": [
                {
                    "op": "sketch",
                    "profile": "rectangle",
                    "plane": "XY",
                    "width": 20.0,
                    "height": 20.0,
                },
                {"op": "extrude", "length": 10.0},
            ],
        }
    ]
    cases.append(
        _case(
            "rectangle_pad_20_20_10_xy",
            "20 x 20 mm rectangle on XY extruded 10 mm, the pocket reference pad",
            "rectangle_pad",
            _script(base),
            documented={
                "source": ("docs/construction-semantics.md#cuts-and-pockets"),
                "volume_mm3": 4000.0,
                "min_mm": [0.0, 0.0, 0.0],
                "max_mm": [20.0, 20.0, 10.0],
            },
        )
    )

    for length, case_id, volume in (
        (4.0, "pocket_circle_xy_finite", 3987.433629),
        (None, "pocket_circle_xy_through_all", 3968.584073),
    ):
        steps: list[dict[str, object]] = [
            {
                "op": "sketch",
                "profile": "rectangle",
                "plane": "XY",
                "width": 20.0,
                "height": 20.0,
            },
            {"op": "extrude", "length": 10.0},
            {
                "op": "sketch",
                "profile": "circle",
                "plane": "XY",
                "diameter": 2.0,
                "x": 5.0,
                "y": 5.0,
            },
            {"op": "pocket", "length": length} if length is not None else {"op": "pocket"},
        ]
        cases.append(
            _case(
                case_id,
                (
                    "diameter 2 mm circle at (5, 5) on XY cut "
                    + ("4 mm deep" if length is not None else "through all")
                ),
                "pocket",
                _script([{"name": "plate", "steps": steps}]),
                documented={
                    "source": ("docs/construction-semantics.md#cuts-and-pockets"),
                    "volume_mm3": volume,
                    "min_mm": [0.0, 0.0, 0.0],
                    "max_mm": [20.0, 20.0, 10.0],
                },
            )
        )
    return cases


def _assembly_cases() -> list[dict[str, Any]]:
    two = _script(
        [
            _rectangle_component("base", width=10.0, height=10.0, length=5.0),
            _rectangle_component("riser", width=10.0, height=10.0, length=5.0),
        ],
        assembly=[
            {"op": "place", "component": "base", "x": 0.0, "y": 0.0, "z": 0.0},
            {"op": "place", "component": "riser", "x": 20.0, "y": 0.0, "z": 0.0},
        ],
    )
    three = _script(
        [
            _rectangle_component("a", width=10.0, height=10.0, length=2.0),
            _rectangle_component("b", width=10.0, height=10.0, length=2.0),
            _rectangle_component("c", width=10.0, height=10.0, length=2.0),
        ],
        assembly=[
            {"op": "place", "component": "a", "x": 0.0, "y": 0.0, "z": 0.0},
            {"op": "place", "component": "b", "x": 15.0, "y": 0.0, "z": 0.0},
            {"op": "place", "component": "c", "x": 0.0, "y": 20.0, "z": 0.0},
        ],
    )
    translated = _script(
        [_rectangle_component("base", width=10.0, height=10.0, length=5.0)],
        assembly=[{"op": "place", "component": "base", "x": 5.0, "y": 6.0, "z": 7.0}],
    )
    return [
        _case(
            "assembly_two_parts_placed_once",
            "two 10 x 10 x 5 pads placed at (0, 0, 0) and (20, 0, 0)",
            "assembly",
            two,
        ),
        _case(
            "assembly_three_parts_placed_once",
            "three 10 x 10 x 2 pads placed at three translations",
            "assembly",
            three,
        ),
        _case(
            "assembly_single_translated_part",
            "one 10 x 10 x 5 pad placed at (5, 6, 7)",
            "assembly",
            translated,
        ),
    ]


# --------------------------------------------------------------------- helpers


def _script(
    components: Sequence[Mapping[str, object]],
    assembly: Sequence[Mapping[str, object]] = (),
) -> ConstructionScript:
    return load_construction(
        {
            "summary": "construction-semantics parity corpus",
            "components": list(components),
            "assembly": list(assembly),
        }
    )


def _rectangle_component(
    name: str,
    *,
    plane: str = "XY",
    width: float,
    height: float,
    length: float,
    x: float = 0.0,
    y: float = 0.0,
    reversed_: bool = False,
) -> dict[str, object]:
    return {
        "name": name,
        "steps": [
            {
                "op": "sketch",
                "profile": "rectangle",
                "plane": plane,
                "width": width,
                "height": height,
                "x": x,
                "y": y,
            },
            {"op": "extrude", "length": length, "reversed": reversed_},
        ],
    }


def _circle_component(
    name: str,
    *,
    plane: str = "XY",
    diameter: float,
    length: float,
    x: float = 0.0,
    y: float = 0.0,
) -> dict[str, object]:
    return {
        "name": name,
        "steps": [
            {
                "op": "sketch",
                "profile": "circle",
                "plane": plane,
                "diameter": diameter,
                "x": x,
                "y": y,
            },
            {"op": "extrude", "length": length},
        ],
    }


def _case(
    case_id: str,
    description: str,
    corpus: str,
    script: ConstructionScript,
    *,
    documented: Mapping[str, object] | None = None,
) -> dict[str, Any]:
    expected = expectation(script)
    return {
        "id": case_id,
        "description": description,
        "corpus": corpus,
        "script": json.loads(script.model_dump_json()),
        "oracle": _oracle_json(expected),
        "documented": dict(documented) if documented is not None else None,
    }


def _oracle_json(expected: OracleExpectation) -> dict[str, Any]:
    return {
        "parts": [
            {
                "name": part.name,
                "checked": part.checked,
                "reason": part.reason,
                "volume_mm3": _number(part.volume_mm3),
                "min_mm": _point(part.min_mm),
                "max_mm": _point(part.max_mm),
            }
            for part in expected.parts
        ],
        "assembly_expected": expected.assembly_expected,
        "assembly_bounds": _range_json(expected.assembly_bounds),
        "reason": expected.reason,
    }


def _number(value: float | None) -> float | None:
    if value is None:
        return None
    return round(value, _DECIMALS)


def _point(value: tuple[float, float, float] | None) -> list[float] | None:
    if value is None:
        return None
    return [round(component, _DECIMALS) for component in value]


def _range_json(value: _RangeBox | None) -> dict[str, list[float]] | None:
    if value is None:
        return None
    minimum = value.min_mm
    maximum = value.max_mm
    return {
        "min_mm": [round(component, _DECIMALS) for component in minimum],
        "max_mm": [round(component, _DECIMALS) for component in maximum],
    }


def _verify_documented(cases: Sequence[Mapping[str, Any]]) -> None:
    """The fixture may not disagree with a number printed in the semantics doc."""
    for case in cases:
        documented = case["documented"]
        if documented is None:
            continue
        oracle = case["oracle"]
        part = oracle["parts"][0]
        if not part["checked"]:
            raise AssertionError(f"{case['id']}: a documented case must be oracle-checked")
        expected_volume = float(part["volume_mm3"])
        documented_volume = float(documented["volume_mm3"])
        if abs(expected_volume - documented_volume) > 1e-6:
            raise AssertionError(
                f"{case['id']}: oracle volume {expected_volume} does not match "
                f"documented {documented_volume}"
            )
        for key in ("min_mm", "max_mm"):
            oracle_point = part[key]
            documented_point = documented[key]
            for axis in range(3):
                if abs(float(oracle_point[axis]) - float(documented_point[axis])) > 1e-6:
                    raise AssertionError(
                        f"{case['id']}: oracle {key} does not match documented {key}"
                    )


if __name__ == "__main__":
    main()
