"""suggest_procedure keyword map."""

from __future__ import annotations

from kala.procedures import list_procedure_ids, suggest_procedure


def test_suggest_shaft():
    assert suggest_procedure("stepped shaft OD 20mm") == "stepped_shaft"


def test_suggest_plate():
    assert suggest_procedure("plate with four holes") == "plate_with_holes"


def test_suggest_housing():
    assert suggest_procedure("housing cover with bore") == "housing_cover"


def test_suggest_default_bracket():
    pid = suggest_procedure("L-bracket base 60x40")
    assert pid in list_procedure_ids()
    assert pid == "simple_bracket"
