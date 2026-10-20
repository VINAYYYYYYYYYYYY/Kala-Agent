"""Expected solid counts are derived from the script, not assumed.

A single part keeps the ``single_solid`` check. A place-only assembly is a set
of occurrences (``docs/construction-semantics.md``, "Assembly occurrences"), so
it gets a ``solid_count`` check whose expected value is the number of placed
components. A join step declares its own result: ``fuse`` is one joined solid,
``compound`` keeps one solid per compounded member.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, cast

from kala_agent.contracts.construction import ConstructionScript
from kala_agent.studio.design.measure import SOLID_COUNT_TOLERANCE
from kala_agent.studio.design.providers.base import MeasuredProperties
from kala_agent.studio.design.service import (
    build_provider_checks,
    expected_solid_count,
)
from kala_agent.studio.design.templates import l_bracket

PARITY_FIXTURE = Path(__file__).parents[2] / "fixtures" / "construction_parity.json"


def _case_script(case_id: str) -> ConstructionScript:
    data = cast("dict[str, Any]", json.loads(PARITY_FIXTURE.read_text(encoding="utf-8")))
    cases = cast("list[dict[str, Any]]", data["cases"])
    case = next(case for case in cases if case["id"] == case_id)
    return ConstructionScript.model_validate(case["script"])


def _box_component(name: str) -> dict[str, object]:
    return {
        "name": name,
        "steps": [
            {
                "op": "sketch",
                "profile": "rectangle",
                "plane": "XY",
                "width": 10.0,
                "height": 10.0,
            },
            {"op": "extrude", "length": 5.0},
        ],
    }


def _two_part_script(assembly: list[dict[str, object]]) -> ConstructionScript:
    return ConstructionScript.model_validate(
        {
            "summary": "two 10 x 10 x 5 boxes",
            "components": [_box_component("a"), _box_component("b")],
            "assembly": assembly,
        }
    )


def test_a_single_part_declares_one_solid() -> None:
    script = l_bracket(
        width=60,
        depth=45,
        height=40,
        wall=8,
        bore_d=10,
        hole_d=5.5,
        hole_count=4,
    ).script
    assert expected_solid_count(script) == 1
    assert expected_solid_count(_case_script("assembly_single_translated_part")) == 1


def test_a_place_only_assembly_declares_one_solid_per_component() -> None:
    assert expected_solid_count(_case_script("assembly_two_parts_placed_once")) == 2
    assert expected_solid_count(_case_script("assembly_three_parts_placed_once")) == 3


def test_a_fuse_declares_one_joined_solid() -> None:
    assert expected_solid_count(_two_part_script([{"op": "fuse", "components": ["a", "b"]}])) == 1


def test_a_compound_declares_one_solid_per_member() -> None:
    script = _two_part_script([{"op": "compound", "components": ["a", "b"]}])
    assert expected_solid_count(script) == 2


def test_a_component_named_by_a_join_is_not_counted_twice() -> None:
    script = _two_part_script(
        [
            {"op": "place", "component": "a", "x": 0.0, "y": 0.0, "z": 0.0},
            {"op": "fuse", "components": ["a", "b"]},
        ]
    )
    assert expected_solid_count(script) == 1


def test_the_single_solid_row_is_kept_for_a_one_solid_result() -> None:
    checks = build_provider_checks(
        measured=MeasuredProperties(),
        valid=True,
        solid_count=1,
        expected_envelope=None,
    )
    assert [check.id for check in checks] == ["brep_valid", "single_solid"]
    single = checks[1]
    assert single.label == "Single solid"
    assert single.expected == 1.0
    assert single.measured == 1.0
    assert single.status == "passed"


def test_an_assembly_gets_a_solid_count_row() -> None:
    checks = build_provider_checks(
        measured=MeasuredProperties(),
        valid=True,
        solid_count=2,
        expected_solids=2,
        expected_envelope=None,
    )
    assert [check.id for check in checks] == ["brep_valid", "solid_count"]
    row = checks[1]
    assert row.label == "Solid count"
    assert row.expected == 2.0
    assert row.measured == 2.0
    assert row.tolerance == SOLID_COUNT_TOLERANCE
    assert row.unit == "solid"
    assert row.passed is True
    assert row.status == "passed"


def test_a_wrong_assembly_solid_count_fails() -> None:
    checks = build_provider_checks(
        measured=MeasuredProperties(),
        valid=True,
        solid_count=3,
        expected_solids=2,
        expected_envelope=None,
    )
    row = checks[1]
    assert row.passed is False
    assert row.status == "failed"
    assert (row.expected, row.measured) == (2.0, 3.0)


def test_a_missing_assembly_solid_count_is_not_reported() -> None:
    checks = build_provider_checks(
        measured=MeasuredProperties(),
        valid=True,
        solid_count=None,
        expected_solids=2,
        expected_envelope=None,
    )
    row = checks[1]
    assert row.measured is None
    assert row.passed is False
    assert row.status == "not_reported"


def test_a_multi_solid_result_still_fails_a_single_part_script() -> None:
    checks = build_provider_checks(
        measured=MeasuredProperties(),
        valid=True,
        solid_count=2,
        expected_solids=1,
        expected_envelope=None,
    )
    row = checks[1]
    assert row.id == "single_solid"
    assert row.status == "failed"
