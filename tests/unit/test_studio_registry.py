"""Tests for the static integration registry."""

from __future__ import annotations

from collections.abc import Iterator

import pytest

from kala_agent.studio.integrations.registry import (
    build_registry,
    integration_by_id,
    list_capabilities,
)


@pytest.fixture(autouse=True)
def _forbid_network(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    def blocked(*args: object, **kwargs: object) -> None:
        raise AssertionError("registry tests must not open network connections")

    monkeypatch.setattr("socket.socket.connect", blocked)
    yield


def test_registry_includes_six_cad_targets() -> None:
    ids = {row.id for row in build_registry()}
    assert {"solidworks", "solid_edge", "autocad", "fusion360", "freecad", "openscad"} <= ids


def test_registry_includes_five_analysis_targets() -> None:
    ids = {row.id for row in build_registry()}
    expected = {
        "ansys_mechanical",
        "abaqus",
        "comsol",
        "simcenter_nastran",
        "altair_optistruct",
    }
    assert expected <= ids
    # Sized to 5 by the brief ("representative not ranked top-five").
    analysis_rows = [row for row in build_registry() if row.category.value == "analysis"]
    assert len(analysis_rows) == 5


def test_native_api_unavailable_for_proprietary_targets() -> None:
    by_id = {row.id: row for row in build_registry()}
    # Native API for proprietary hosts is not bound by the studio:
    for target in (
        "solidworks",
        "solid_edge",
        "autocad",
        "fusion360",
        "ansys_mechanical",
        "abaqus",
        "comsol",
        "simcenter_nastran",
        "altair_optistruct",
    ):
        assert by_id[target].status.value == "native_api_unavailable"


def test_freecad_is_listed_as_neutral_file_capability() -> None:
    by_id = {row.id: row for row in build_registry()}
    assert by_id["freecad"].status.value == "available_via_neutral_file"


def test_openscad_is_not_configured_because_scripting_is_unsupported() -> None:
    by_id = {row.id: row for row in build_registry()}
    assert by_id["openscad"].status.value == "not_configured"


def test_nastran_does_not_ingest_step_directly() -> None:
    by_id = {row.id: row for row in build_registry()}
    assert "Femap" in by_id["simcenter_nastran"].notes


def test_autocad_step_support_depends_on_release() -> None:
    by_id = {row.id: row for row in build_registry()}
    assert "STEP" in by_id["autocad"].notes
    assert "release" in by_id["autocad"].notes.lower()
    assert "edition" in by_id["autocad"].notes.lower()


def test_every_registry_row_has_platforms_and_capabilities() -> None:
    for row in build_registry():
        assert row.platforms, f"{row.id} has no platforms"
        assert row.capabilities, f"{row.id} has no capabilities"
        assert row.notes, f"{row.id} has no notes"


def test_capabilities_only_contain_supported_formats() -> None:
    for row in build_registry():
        for cap in row.capabilities:
            assert cap in {"step", "stl", "iges"}, f"{row.id} lists unsupported capability: {cap}"


def test_no_row_claims_solver_or_native_connection() -> None:
    """The studio never pretends to run a solver or talk to a native API."""
    for row in build_registry():
        text = (row.notes + " " + " ".join(row.platforms)).lower()
        # No row should advertise "the studio runs X" or "studio drives X";
        # "is a separate" / "is the analysis product" are informational
        # only and are tracked through the phrasing whitelist.
        forbidden_native_phrases = (
            "studio runs",
            "studio drives",
            "studio invokes",
            "studio automates",
            "studio calls",
        )
        for phrase in forbidden_native_phrases:
            assert phrase not in text, (
                f"{row.id} says {phrase!r}; the studio does not drive native APIs"
            )
        # No row should claim FEA in a way that names the studio as the
        # analysis. Phrases like "is the analysis product" or "is a
        # separate analysis" are kept because they explicitly say *the
        # other tool* runs the analysis.
        assert "studio runs the" not in text
        assert "studio runs an" not in text


def test_integration_by_id_returns_none_for_unknown() -> None:
    assert integration_by_id("") is None
    assert integration_by_id("not-a-real-target") is None


def test_integration_by_id_round_trip() -> None:
    for row in build_registry():
        assert integration_by_id(row.id) is not None
        assert list_capabilities(row.id) == row.capabilities
