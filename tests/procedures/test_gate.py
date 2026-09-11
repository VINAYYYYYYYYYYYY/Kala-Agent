"""decompose-or-clarify assess_goal gate."""

from __future__ import annotations

from kala.procedures import ClarifyNeeded, PartPlan, assess_goal


def test_l_bracket_passes():
    assert assess_goal("L-bracket base 60x40") is None


def test_plate_passes():
    assert assess_goal("plate with four holes") is None


def test_laptop_clarify():
    g = assess_goal("16 inch laptop")
    assert isinstance(g, ClarifyNeeded)
    assert g.questions


def test_car_clarify():
    assert isinstance(assess_goal("design a car body"), ClarifyNeeded)


def test_gearbox_part_plan():
    g = assess_goal("planetary gearbox with sun and planets")
    assert isinstance(g, PartPlan)
    assert g.parts
