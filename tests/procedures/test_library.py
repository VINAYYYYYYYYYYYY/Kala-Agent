"""Procedure library packaging and discovery."""

from __future__ import annotations

from kala.procedures import (
    library_dir,
    list_procedure_ids,
    list_procedures,
    load_default_procedure,
    require_known_procedure,
)


def test_library_dir_is_packaged():
    root = library_dir()
    assert root.name == "library"
    assert (root / "simple_bracket.json").is_file()
    assert list(root.glob("*.json"))


def test_list_procedures_includes_core_and_new():
    ids = set(list_procedure_ids())
    for required in (
        "simple_bracket",
        "machine_assembly",
        "stepped_shaft",
        "plate_with_holes",
        "housing_cover",
    ):
        assert required in ids
    procs = list_procedures()
    assert len(procs) >= 5
    assert all(p.steps for p in procs)


def test_load_stepped_shaft():
    p = load_default_procedure("stepped_shaft")
    assert p.id == "stepped_shaft"
    assert p.steps[0].id == "envelope"


def test_require_known_procedure_ok():
    require_known_procedure("simple_bracket")
    require_known_procedure("machine_assembly")


def test_require_known_procedure_unknown_fails():
    try:
        require_known_procedure("nonexistent_procedure_xyz")
    except ValueError as exc:
        assert "nonexistent_procedure_xyz" in str(exc)
        assert "Known:" in str(exc)
    else:
        raise AssertionError("Expected ValueError for unknown procedure")
