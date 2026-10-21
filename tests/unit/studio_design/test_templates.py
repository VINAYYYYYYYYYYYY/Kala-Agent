"""Template construction, measurement, and feasibility tests."""

from __future__ import annotations

import math

import pytest

from kala_agent.studio.design.compile_ocp import compile_script
from kala_agent.studio.design.measure import Bounds, build_checks, measure_shape
from kala_agent.studio.design.service import canonical_script_json
from kala_agent.studio.design.templates import (
    TemplateError,
    TemplateResult,
    housing_envelope,
    l_bracket,
    stepped_shaft,
)

ADR_VOLUME_MM3 = 35_571.416047


def _lbracket() -> TemplateResult:
    return l_bracket(
        width=60,
        depth=45,
        height=40,
        wall=8,
        bore_d=10,
        hole_d=5.5,
        hole_count=4,
    )


def test_l_bracket_matches_the_adr_oracle() -> None:
    result = _lbracket()
    measurement = measure_shape(compile_script(result.script).assembly_shape)
    assert measurement.volume_mm3 == pytest.approx(ADR_VOLUME_MM3, rel=1e-6)
    assert measurement.bounds == Bounds(
        min_mm=(-30.0, 0.0, 0.0),
        max_mm=(30.0, 45.0, 40.0),
    )
    assert measurement.valid is True
    assert measurement.solid_count == 1


def test_l_bracket_checks_include_the_declared_envelope() -> None:
    result = _lbracket()
    measurement = measure_shape(compile_script(result.script).assembly_shape)
    report = build_checks(measurement, expected_envelope=result.envelope)
    assert report.verified is True
    assert [check.id for check in report.checks] == [
        "brep_valid",
        "single_solid",
        "envelope_x",
        "envelope_y",
        "envelope_z",
    ]
    assert all(check.passed for check in report.checks)


def test_l_bracket_declares_label_tolerances_and_envelope() -> None:
    result = _lbracket()
    assert result.label == "L-bracket 60 x 45 x 40"
    assert result.envelope.min_mm == (-30.0, 0.0, 0.0)
    assert result.envelope.max_mm == (30.0, 45.0, 40.0)
    assert any("bore diameter 10" in item for item in result.tolerances)
    assert any("4 x hole diameter 5.5" in item for item in result.tolerances)
    assert "ISO 2768-m (declared)" in result.tolerances


def test_l_bracket_without_bore_or_holes_is_a_plain_angle() -> None:
    result = l_bracket(width=60, depth=45, height=40, wall=8)
    measurement = measure_shape(compile_script(result.script).assembly_shape)
    expected = 60 * 45 * 8 + 60 * 8 * (40 - 8)
    assert measurement.volume_mm3 == pytest.approx(expected, rel=1e-9)
    assert measurement.solid_count == 1


def test_l_bracket_two_holes_are_centred_in_depth() -> None:
    result = l_bracket(
        width=60,
        depth=45,
        height=40,
        wall=8,
        hole_d=5.5,
        hole_count=2,
    )
    measurement = measure_shape(compile_script(result.script).assembly_shape)
    expected = 60 * 45 * 8 + 60 * 8 * 32 - 2 * math.pi * (5.5 / 2) ** 2 * 8
    assert measurement.volume_mm3 == pytest.approx(expected, rel=1e-9)
    assert measurement.solid_count == 1


@pytest.mark.parametrize(
    "values",
    [
        {"width": 12, "depth": 45, "height": 40, "wall": 8, "hole_d": 5.5, "hole_count": 4},
        {"width": 60, "depth": 45, "height": 20, "wall": 8, "bore_d": 12},
        {"width": 60, "depth": 45, "height": 40, "wall": 8, "hole_d": 8, "hole_count": 4},
        {"width": 60, "depth": 45, "height": 40, "wall": 8, "hole_d": 5.5, "hole_count": 3},
        {"width": 60, "depth": 45, "height": 8, "wall": 8},
        {"width": 60, "depth": 8, "height": 40, "wall": 8},
    ],
)
def test_l_bracket_refuses_infeasible_values(values: dict[str, float]) -> None:
    with pytest.raises(TemplateError):
        l_bracket(**values)  # type: ignore[arg-type]


def test_l_bracket_refuses_hole_edge_closer_than_the_radius() -> None:
    with pytest.raises(TemplateError, match="edge distance"):
        l_bracket(
            width=60,
            depth=45,
            height=40,
            wall=8,
            hole_d=5.5,
            hole_count=4,
            edge=2.0,
        )


def test_stepped_shaft_is_one_fused_solid_with_the_right_volume() -> None:
    result = stepped_shaft([(25, 120), (15, 30)])
    measurement = measure_shape(compile_script(result.script).assembly_shape)
    expected = math.pi * (25 / 2) ** 2 * 120 + math.pi * (15 / 2) ** 2 * 30
    assert measurement.volume_mm3 == pytest.approx(expected, rel=1e-9)
    assert measurement.bounds.min_mm == (-12.5, -12.5, 0.0)
    assert measurement.bounds.max_mm == (12.5, 12.5, 150.0)
    assert measurement.solid_count == 1
    assert measurement.valid is True
    report = build_checks(measurement, expected_envelope=result.envelope)
    assert report.verified is True


def test_stepped_shaft_refuses_no_segments() -> None:
    with pytest.raises(TemplateError):
        stepped_shaft([])


def test_stepped_shaft_refuses_more_than_the_component_budget() -> None:
    with pytest.raises(TemplateError):
        stepped_shaft([(10, 10)] * 65)


def test_housing_matches_the_shell_volume_and_envelope() -> None:
    result = housing_envelope(width=80, depth=60, height=40, wall=5)
    measurement = measure_shape(compile_script(result.script).assembly_shape)
    expected = 80 * 60 * 40 - (80 - 10) * (60 - 10) * (40 - 5)
    assert measurement.volume_mm3 == pytest.approx(expected, rel=1e-9)
    assert measurement.bounds == result.envelope
    assert measurement.solid_count == 1
    assert measurement.valid is True
    report = build_checks(measurement, expected_envelope=result.envelope)
    assert report.verified is True


@pytest.mark.parametrize(
    "values",
    [
        {"width": 10, "depth": 60, "height": 40, "wall": 5},
        {"width": 80, "depth": 10, "height": 40, "wall": 5},
        {"width": 80, "depth": 60, "height": 5, "wall": 5},
    ],
)
def test_housing_refuses_a_closed_or_empty_interior(values: dict[str, float]) -> None:
    with pytest.raises(TemplateError):
        housing_envelope(**values)


def test_templates_are_deterministic() -> None:
    first = _lbracket()
    second = _lbracket()
    assert first.label == second.label
    assert canonical_script_json(first.script) == canonical_script_json(second.script)
    assert first.tolerances == second.tolerances
