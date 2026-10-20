"""The oracle predicts only the measured corpus and says so otherwise."""

from __future__ import annotations

import math
from collections.abc import Mapping, Sequence

import pytest

from kala_agent.contracts.construction import ConstructionScript, load_construction
from kala_agent_solidedge import oracle
from kala_solidedge_bridge.protocol import (
    BuildResult,
    JobState,
    PartResult,
    RangeBox,
)


def script(
    steps: Sequence[Mapping[str, object]],
    *,
    name: str = "plate",
    assembly: Sequence[Mapping[str, object]] | None = None,
    extra: Sequence[Mapping[str, object]] | None = None,
) -> ConstructionScript:
    components: list[Mapping[str, object]] = [{"name": name, "steps": steps}]
    if extra is not None:
        components.extend(extra)
    return load_construction(
        {"summary": "oracle case", "components": components, "assembly": assembly or []}
    )


def pad(
    *,
    plane: str = "XY",
    width: float = 20.0,
    height: float = 20.0,
    length: float = 10.0,
    x: float = 0.0,
    y: float = 0.0,
    reversed_: bool = False,
) -> list[Mapping[str, object]]:
    return [
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
    ]


def plate_with_hole(*, cut_length: float | None = None) -> list[Mapping[str, object]]:
    steps = pad()
    steps.extend(
        [
            {"op": "sketch", "profile": "circle", "diameter": 2, "x": 5, "y": 5},
            {"op": "pocket"},
        ]
    )
    if cut_length is not None:
        steps[-1] = {"op": "pocket", "length": cut_length}
    return steps


def result(
    name: str,
    volume: float,
    minimum: tuple[float, float, float],
    maximum: tuple[float, float, float],
    *,
    valid: bool = True,
) -> BuildResult:
    return BuildResult(
        build_id="build-1",
        state=JobState.SUCCEEDED,
        parts=(
            PartResult(
                name=name,
                valid=valid,
                volume_mm3=volume,
                surface_area_mm2=1.0,
                range_box=RangeBox(min_mm=minimum, max_mm=maximum),
            ),
        ),
    )


def test_plate_with_through_hole_matches_the_measured_value() -> None:
    expectation = oracle.expectation(script(plate_with_hole()))

    assert expectation.parts[0].checked
    assert expectation.parts[0].volume_mm3 == pytest.approx(3968.584073, abs=1e-6)
    assert expectation.parts[0].min_mm == (0.0, 0.0, 0.0)
    assert expectation.parts[0].max_mm == (20.0, 20.0, 10.0)


def test_finite_hole_matches_the_measured_value() -> None:
    expectation = oracle.expectation(script(plate_with_hole(cut_length=4.0)))
    assert expectation.parts[0].volume_mm3 == pytest.approx(3987.433629, abs=1e-6)


@pytest.mark.parametrize(
    ("plane", "minimum", "maximum"),
    [
        ("XY", (2.0, 3.0, 0.0), (6.0, 8.0, 6.0)),
        ("XZ", (2.0, -6.0, 3.0), (6.0, 0.0, 8.0)),
        ("YZ", (0.0, 2.0, 3.0), (6.0, 6.0, 8.0)),
    ],
)
def test_plane_conventions_match_the_documented_measurements(
    plane: str,
    minimum: tuple[float, float, float],
    maximum: tuple[float, float, float],
) -> None:
    expectation = oracle.expectation(
        script(pad(plane=plane, width=4, height=5, length=6, x=2, y=3))
    )

    assert expectation.parts[0].volume_mm3 == pytest.approx(120.0)
    assert expectation.parts[0].min_mm == pytest.approx(minimum)
    assert expectation.parts[0].max_mm == pytest.approx(maximum)


@pytest.mark.parametrize("plane", ["XY", "XZ", "YZ"])
def test_reversal_flips_the_normal_extent_and_keeps_the_volume(plane: str) -> None:
    forward = oracle.expectation(script(pad(plane=plane, width=4, height=5, length=6, x=2, y=3)))
    reverse = oracle.expectation(
        script(pad(plane=plane, width=4, height=5, length=6, x=2, y=3, reversed_=True))
    )

    assert forward.parts[0].volume_mm3 == reverse.parts[0].volume_mm3
    assert forward.parts[0].min_mm != reverse.parts[0].min_mm
    assert forward.parts[0].max_mm != reverse.parts[0].max_mm


def test_circle_pad_volume_and_bounds_are_closed_form() -> None:
    expectation = oracle.expectation(
        script(
            [
                {"op": "sketch", "profile": "circle", "diameter": 4, "x": 1, "y": 1},
                {"op": "extrude", "length": 5},
            ]
        )
    )

    part = expectation.parts[0]
    assert part.checked
    assert part.volume_mm3 == pytest.approx(math.pi * 4 * 5)
    assert part.min_mm == pytest.approx((-1.0, -1.0, 0.0))
    assert part.max_mm == pytest.approx((3.0, 3.0, 5.0))


def test_a_circle_revolve_is_predicted_as_the_lowered_pad() -> None:
    expectation = oracle.expectation(
        script(
            [
                {"op": "sketch", "profile": "circle", "diameter": 6},
                {"op": "revolve"},
            ]
        )
    )

    assert expectation.parts[0].checked
    assert expectation.parts[0].volume_mm3 == pytest.approx(math.pi * 9 * 6)


def test_an_out_of_corpus_script_is_explicitly_unchecked() -> None:
    expectation = oracle.expectation(
        script(
            [
                {"op": "sketch", "profile": "rectangle", "width": 10, "height": 10},
                {"op": "extrude", "length": 2},
                {"op": "sketch", "profile": "circle", "diameter": 4, "x": 5, "y": 5},
                {"op": "extrude", "length": 6},
            ]
        )
    )

    assert expectation.parts[0].checked is False
    assert "outside the closed-form corpus" in expectation.parts[0].reason

    report = oracle.compare(
        script(
            [
                {"op": "sketch", "profile": "rectangle", "width": 10, "height": 10},
                {"op": "extrude", "length": 2},
                {"op": "sketch", "profile": "circle", "diameter": 4, "x": 5, "y": 5},
                {"op": "extrude", "length": 6},
            ]
        ),
        result("plate", 1.0, (0.0, 0.0, 0.0), (10.0, 10.0, 8.0)),
    )
    assert report.checked is False
    assert report.passed is False
    assert report.failures == ()


def test_a_cross_plane_hole_is_unchecked() -> None:
    steps = pad()
    steps.extend(
        [
            {"op": "sketch", "profile": "circle", "plane": "XZ", "diameter": 2, "x": 5, "y": 5},
            {"op": "pocket"},
        ]
    )

    expectation = oracle.expectation(script(steps))
    assert expectation.parts[0].checked is False
    assert "same-plane" in expectation.parts[0].reason


def test_a_hole_outside_the_pad_is_unchecked() -> None:
    steps = pad()
    steps.extend(
        [
            {"op": "sketch", "profile": "circle", "diameter": 2, "x": 19.5, "y": 5},
            {"op": "pocket"},
        ]
    )

    expectation = oracle.expectation(script(steps))
    assert expectation.parts[0].checked is False
    assert "not contained" in expectation.parts[0].reason


def test_a_cut_that_removes_the_whole_solid_is_unchecked() -> None:
    steps: list[Mapping[str, object]] = [
        {"op": "sketch", "profile": "circle", "diameter": 4},
        {"op": "extrude", "length": 2},
        {"op": "sketch", "profile": "circle", "diameter": 4},
        {"op": "pocket"},
    ]

    expectation = oracle.expectation(script(steps))
    assert expectation.parts[0].checked is False


def test_a_cut_on_a_reversed_pad_is_unchecked_not_predicted_as_noop() -> None:
    steps = pad(reversed_=True)
    steps.extend(
        [
            {"op": "sketch", "profile": "circle", "diameter": 2, "x": 5, "y": 5},
            {"op": "pocket"},
        ]
    )

    expectation = oracle.expectation(script(steps))
    assert expectation.parts[0].checked is False
    assert "reversed" in expectation.parts[0].reason


def test_compare_accepts_the_measured_result() -> None:
    parsed = script(plate_with_hole())
    report = oracle.compare(
        parsed,
        result("plate", 3968.584073, (0.0, 0.0, 0.0), (20.0, 20.0, 10.0)),
    )

    assert report.checked
    assert report.passed


def test_compare_rejects_a_corner_error_that_keeps_the_extents() -> None:
    parsed = script(plate_with_hole())
    report = oracle.compare(
        parsed,
        result("plate", 3968.584073, (0.0, 0.0, -10.0), (20.0, 20.0, 0.0)),
    )

    assert report.checked
    assert not report.passed
    assert any("min z" in failure for failure in report.failures)


def test_compare_rejects_a_volume_outside_the_tolerance() -> None:
    parsed = script(plate_with_hole())
    report = oracle.compare(
        parsed,
        result("plate", 3990.0, (0.0, 0.0, 0.0), (20.0, 20.0, 10.0)),
    )

    assert not report.passed
    assert any("volume" in failure for failure in report.failures)


def test_compare_rejects_a_zero_volume_or_invalid_solid() -> None:
    parsed = script(plate_with_hole())
    zero = oracle.compare(parsed, result("plate", 0.0, (0.0, 0.0, 0.0), (20.0, 20.0, 10.0)))
    invalid = oracle.compare(
        parsed, result("plate", 3968.58, (0.0, 0.0, 0.0), (20.0, 20.0, 10.0), valid=False)
    )

    assert not zero.passed
    assert any("zero" in failure for failure in zero.failures)
    assert not invalid.passed
    assert any("invalid" in failure for failure in invalid.failures)


def test_a_missing_part_is_a_failure() -> None:
    parsed = script(plate_with_hole())
    empty = BuildResult(build_id="build-1", state=JobState.SUCCEEDED)
    report = oracle.compare(parsed, empty)

    assert not report.passed
    assert any("no measured part" in failure for failure in report.failures)


def test_assembly_bounds_are_the_union_of_translated_parts() -> None:
    first = pad()
    second = pad(width=4, height=4, length=4)
    parsed = script(
        first,
        name="base",
        extra=[{"name": "post", "steps": second}],
        assembly=[
            {"op": "place", "component": "base", "x": 0, "y": 0, "z": 0},
            {"op": "place", "component": "post", "x": 12, "y": 0, "z": 0},
        ],
    )

    expectation = oracle.expectation(parsed)
    assert expectation.assembly_expected
    assert expectation.assembly_bounds is not None
    assert expectation.assembly_bounds.min_mm == pytest.approx((0.0, 0.0, 0.0))
    assert expectation.assembly_bounds.max_mm == pytest.approx((20.0, 20.0, 10.0))


def test_assembly_comparison_never_claims_a_union_volume() -> None:
    first = pad()
    second = pad(width=4, height=4, length=4)
    parsed = script(
        first,
        name="base",
        extra=[{"name": "post", "steps": second}],
        assembly=[
            {"op": "place", "component": "base", "x": 0, "y": 0, "z": 0},
            {"op": "place", "component": "post", "x": 12, "y": 0, "z": 0},
        ],
    )
    overlapping = BuildResult(
        build_id="build-1",
        state=JobState.SUCCEEDED,
        parts=(
            PartResult(
                name="base",
                valid=True,
                volume_mm3=4000.0,
                surface_area_mm2=1.0,
                range_box=RangeBox(min_mm=(0.0, 0.0, 0.0), max_mm=(20.0, 20.0, 10.0)),
            ),
            PartResult(
                name="post",
                valid=True,
                volume_mm3=64.0,
                surface_area_mm2=1.0,
                range_box=RangeBox(min_mm=(0.0, 0.0, 0.0), max_mm=(4.0, 4.0, 4.0)),
            ),
        ),
        assembly_bounds=RangeBox(min_mm=(0.0, 0.0, 0.0), max_mm=(20.0, 20.0, 10.0)),
    )

    report = oracle.compare(parsed, overlapping)
    assert report.passed
    assert not any("union" in note for note in report.notes)
    assert not any("volume" in failure for failure in report.failures)


def test_assembly_bounds_for_recomputes_from_local_boxes() -> None:
    parts = (
        PartResult(
            name="a",
            valid=True,
            volume_mm3=1.0,
            surface_area_mm2=1.0,
            range_box=RangeBox(min_mm=(0.0, 0.0, 0.0), max_mm=(2.0, 2.0, 2.0)),
        ),
    )

    bounds = oracle.assembly_bounds_for(parts, {"a": (5.0, 0.0, -1.0)})
    assert bounds.min_mm == (5.0, 0.0, -1.0)
    assert bounds.max_mm == (7.0, 2.0, 1.0)
