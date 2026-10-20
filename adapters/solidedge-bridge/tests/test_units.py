"""Unit conversions used by the bridge, with finite-input validation."""

from __future__ import annotations

import math
from typing import Any, cast

import pytest

from kala_solidedge_bridge.units import deg_to_rad, m2_to_mm2, m3_to_mm3, m_to_mm, mm_to_m


def test_length_conversions() -> None:
    assert mm_to_m(1_000) == 1.0
    assert mm_to_m(1) == 0.001
    assert m_to_mm(1.5) == 1_500.0
    assert m_to_mm(0) == 0.0


def test_angle_and_area_volume_conversions() -> None:
    assert deg_to_rad(180) == pytest.approx(math.pi)
    assert deg_to_rad(90) == pytest.approx(math.pi / 2)
    assert m2_to_mm2(1) == 1_000_000.0
    assert m3_to_mm3(1) == 1_000_000_000.0
    assert m3_to_mm3(0.000000001) == pytest.approx(1.0)


def test_negative_values_are_valid_conversions() -> None:
    assert mm_to_m(-1_000) == -1.0
    assert deg_to_rad(-90) == pytest.approx(-math.pi / 2)


@pytest.mark.parametrize("value", [True, False, "1000", float("nan"), float("inf"), float("-inf")])
def test_all_conversions_reject_bools_strings_and_non_finite(value: object) -> None:
    for conversion in (mm_to_m, m_to_mm, deg_to_rad, m3_to_mm3, m2_to_mm2):
        with pytest.raises(ValueError):
            conversion(cast(Any, value))
