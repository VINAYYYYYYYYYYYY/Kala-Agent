"""decompose-or-clarify assess_goal gate."""

from __future__ import annotations

from kala.procedures import ClarifyNeeded, PartPlan, PartSpec, assess_goal
from kala.procedures.schema import list_procedure_ids


_KNOWN = set(list_procedure_ids())
_PART_FIELDS = {"local_name", "brief", "procedure_id", "keep_separate"}


def _assert_part_spec(p: PartSpec) -> None:
    assert isinstance(p, PartSpec)
    d = p.to_dict()
    assert set(d.keys()) == _PART_FIELDS
    assert isinstance(d["local_name"], str) and d["local_name"]
    assert isinstance(d["brief"], str)
    assert d["procedure_id"] is None or d["procedure_id"] in _KNOWN
    assert isinstance(d["keep_separate"], bool)
    assert d["keep_separate"] is True  # P0 default


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
    assert all(isinstance(p, PartSpec) for p in g.parts)
    for p in g.parts:
        _assert_part_spec(p)
    # gear keyword → local_name gear; no packaged gear playbook → null
    names = {p.local_name for p in g.parts}
    assert "gear" in names
    gear = next(p for p in g.parts if p.local_name == "gear")
    assert gear.procedure_id is None
    assert gear.keep_separate is True
    bindable = [p for p in g.parts if p.procedure_id]
    assert bindable, "gearbox-class plan must include ≥1 existing procedure_id"
    assert all(p.procedure_id in _KNOWN for p in bindable)
    payload = g.to_dict()
    assert payload["kind"] == "part_plan"
    assert isinstance(payload["parts"], list)
    assert set(payload["parts"][0].keys()) == _PART_FIELDS


def test_housing_assembly_suggests_known_procedure():
    g = assess_goal("multi-part housing and cover assembly")
    assert isinstance(g, PartPlan)
    for p in g.parts:
        _assert_part_spec(p)
    by_name = {p.local_name: p for p in g.parts}
    if "housing" in by_name:
        assert by_name["housing"].procedure_id == "housing_cover"
    if "cover" in by_name:
        assert by_name["cover"].procedure_id == "housing_cover"
