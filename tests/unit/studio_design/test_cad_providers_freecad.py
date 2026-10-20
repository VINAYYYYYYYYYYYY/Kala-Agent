"""Real FreeCADCmd provider tests (ADR-0007/0011).

Every test in this module is marked ``freecad`` and runs a real
``FreeCADCmd`` subprocess on this host (FreeCAD 1.1.4). When FreeCAD is absent
the module skips with a clear reason; it never fakes a build.

The corpus is ``tests/fixtures/construction_parity.json``, the frozen oracle
the ADR-0009 compiler also uses. Every case is built through the adapter's
official ``worker.invoke_design`` entry point. A place-only assembly exports
its placed occurrences as one STEP assembly, and an assembly keeps the
fixture's ``assembly_bounds`` (0.001 mm) and summed part volume (1e-6); it is
never fused or silently reduced to one component.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, cast

import pytest

from kala_agent.contracts.construction import ConstructionScript
from kala_agent.studio.design.importer import import_step_bytes
from kala_agent.studio.design.intent import DesignIntent
from kala_agent.studio.design.providers.freecad import FreeCadProvider
from kala_agent.studio.design.service import DesignService, design_available

pytestmark = [
    pytest.mark.freecad,
    pytest.mark.skipif(
        not design_available(),
        reason=(
            "the OpenCascade view importer is required for the FreeCAD STEP "
            "parity check; install 'cadquery-ocp'"
        ),
    ),
]

PARITY_FIXTURE = Path(__file__).parents[2] / "fixtures" / "construction_parity.json"
ADR_VOLUME_MM3 = 35_571.416047
VOLUME_RELATIVE_TOLERANCE = 1e-6
BOUNDS_ABSOLUTE_TOLERANCE_MM = 0.001
EXPECTED_CORPUS_CASES = 28
# The two rectangle-revolve cases have no closed-form volume oracle in the
# fixture; their documented status is a build with bounds only.
EXPECTED_VOLUME_CHECKED_CASES = 26


def _case(case_id: str) -> dict[str, Any]:
    data = cast("dict[str, Any]", json.loads(PARITY_FIXTURE.read_text(encoding="utf-8")))
    cases = cast("list[dict[str, Any]]", data["cases"])
    return next(case for case in cases if case["id"] == case_id)


@pytest.fixture(scope="module")
def provider() -> FreeCadProvider:
    instance = FreeCadProvider(probe_cache_seconds=0)
    availability = instance.probe()
    if not availability.available:
        pytest.skip(
            f"a real FreeCADCmd is required for the FreeCAD provider tests: {availability.reason}"
        )
    return instance


def _intent(script: ConstructionScript) -> DesignIntent:
    return DesignIntent(script=script)


def _service(data_dir: Path, provider: FreeCadProvider) -> DesignService:
    return DesignService(
        data_dir,
        providers={"freecad": provider},
        max_step_bytes=64 * 1024 * 1024,
    )


def _lbracket_script() -> ConstructionScript:
    from kala_agent.studio.design.templates import l_bracket

    return l_bracket(
        width=60,
        depth=45,
        height=40,
        wall=8,
        bore_d=10,
        hole_d=5.5,
        hole_count=4,
    ).script


def test_probe_reports_the_real_freecad_1_1_installation(provider: FreeCadProvider) -> None:
    availability = provider.probe()
    assert availability.available is True
    assert availability.version is not None
    assert availability.version.startswith("1.1.")


def test_l_bracket_builds_in_freecad_and_reports_its_own_volume(
    provider: FreeCadProvider,
) -> None:
    script = _lbracket_script()
    build = provider.build(script, _intent(script))
    assert build.step_bytes.startswith(b"ISO-10303-21")
    assert build.native_kind == "fcstd"
    assert build.native_bytes
    assert build.measured.volume_mm3 == pytest.approx(ADR_VOLUME_MM3, rel=VOLUME_RELATIVE_TOLERANCE)
    assert build.measured.bounds is not None
    assert build.measured.bounds.min_mm == pytest.approx(
        (-30.0, 0.0, 0.0), abs=BOUNDS_ABSOLUTE_TOLERANCE_MM
    )
    assert build.measured.bounds.max_mm == pytest.approx(
        (30.0, 45.0, 40.0), abs=BOUNDS_ABSOLUTE_TOLERANCE_MM
    )
    assert build.valid is True
    assert build.solid_count == 1

    imported = import_step_bytes(build.step_bytes, store_artifact=False)
    assert imported.bounds.min_mm == pytest.approx(
        (-30.0, 0.0, 0.0), abs=BOUNDS_ABSOLUTE_TOLERANCE_MM
    )
    assert imported.bounds.max_mm == pytest.approx(
        (30.0, 45.0, 40.0), abs=BOUNDS_ABSOLUTE_TOLERANCE_MM
    )
    assert imported.solids == 1


def test_every_supported_parity_case_builds_and_matches_the_fixture(
    provider: FreeCadProvider,
) -> None:
    data = cast("dict[str, Any]", json.loads(PARITY_FIXTURE.read_text(encoding="utf-8")))
    cases = cast("list[dict[str, Any]]", data["cases"])
    supported = 0
    volume_checked = 0
    for case in cases:
        script = ConstructionScript.model_validate(case["script"])
        oracle = cast("dict[str, Any]", case["oracle"])
        parts = cast("list[dict[str, Any]]", oracle["parts"])
        build = provider.build(script, _intent(script))
        assert build.step_bytes.startswith(b"ISO-10303-21"), case["id"]
        assert build.native_bytes is not None, case["id"]
        assert build.native_kind == "fcstd", case["id"]

        imported = import_step_bytes(build.step_bytes, store_artifact=False)
        assembly_bounds = oracle.get("assembly_bounds")
        if isinstance(assembly_bounds, dict):
            expected_min = cast("list[float]", assembly_bounds["min_mm"])
            expected_max = cast("list[float]", assembly_bounds["max_mm"])
        else:
            expected_min = cast("list[float]", parts[0]["min_mm"])
            expected_max = cast("list[float]", parts[0]["max_mm"])
        if expected_min and expected_max:
            assert imported.bounds.min_mm == pytest.approx(
                expected_min, abs=BOUNDS_ABSOLUTE_TOLERANCE_MM
            ), case["id"]
            assert imported.bounds.max_mm == pytest.approx(
                expected_max, abs=BOUNDS_ABSOLUTE_TOLERANCE_MM
            ), case["id"]
            assert build.measured.bounds is not None, case["id"]
            assert build.measured.bounds.min_mm == pytest.approx(
                expected_min, abs=BOUNDS_ABSOLUTE_TOLERANCE_MM
            ), case["id"]
            assert build.measured.bounds.max_mm == pytest.approx(
                expected_max, abs=BOUNDS_ABSOLUTE_TOLERANCE_MM
            ), case["id"]

        checked_parts = [
            part
            for part in parts
            if part.get("checked") is True and part.get("volume_mm3") is not None
        ]
        if len(checked_parts) == len(parts) and checked_parts:
            expected_volume = sum(float(part["volume_mm3"]) for part in checked_parts)
            assert build.measured.volume_mm3 == pytest.approx(
                expected_volume, rel=VOLUME_RELATIVE_TOLERANCE
            ), case["id"]
            volume_checked += 1
        supported += 1

    assert supported == EXPECTED_CORPUS_CASES
    assert volume_checked == EXPECTED_VOLUME_CHECKED_CASES


def test_freecad_exports_a_multi_component_place_only_assembly(
    provider: FreeCadProvider,
) -> None:
    case = _case("assembly_two_parts_placed_once")
    script = ConstructionScript.model_validate(case["script"])
    build = provider.build(script, _intent(script))

    imported = import_step_bytes(build.step_bytes, store_artifact=False)
    assert imported.solids == 2
    assert imported.bounds.min_mm == pytest.approx(
        (0.0, 0.0, 0.0), abs=BOUNDS_ABSOLUTE_TOLERANCE_MM
    )
    assert imported.bounds.max_mm == pytest.approx(
        (30.0, 10.0, 5.0), abs=BOUNDS_ABSOLUTE_TOLERANCE_MM
    )
    # FreeCAD reports the compound's total volume, not one component's.
    assert build.measured.volume_mm3 == pytest.approx(1_000.0, rel=VOLUME_RELATIVE_TOLERANCE)
    assert build.measured.bounds is not None
    assert build.measured.bounds.min_mm == pytest.approx(
        (0.0, 0.0, 0.0), abs=BOUNDS_ABSOLUTE_TOLERANCE_MM
    )
    assert build.measured.bounds.max_mm == pytest.approx(
        (30.0, 10.0, 5.0), abs=BOUNDS_ABSOLUTE_TOLERANCE_MM
    )
    assert build.solid_count == 2


def test_freecad_exports_a_single_placed_component_at_its_placement(
    provider: FreeCadProvider,
) -> None:
    case = _case("assembly_single_translated_part")
    script = ConstructionScript.model_validate(case["script"])
    build = provider.build(script, _intent(script))

    expected_min = (5.0, 6.0, 7.0)
    expected_max = (15.0, 16.0, 12.0)
    imported = import_step_bytes(build.step_bytes, store_artifact=False)
    assert imported.solids == 1
    assert imported.bounds.min_mm == pytest.approx(expected_min, abs=BOUNDS_ABSOLUTE_TOLERANCE_MM)
    assert imported.bounds.max_mm == pytest.approx(expected_max, abs=BOUNDS_ABSOLUTE_TOLERANCE_MM)
    assert build.measured.volume_mm3 == pytest.approx(500.0, rel=VOLUME_RELATIVE_TOLERANCE)
    assert build.measured.bounds is not None
    assert build.measured.bounds.min_mm == pytest.approx(
        expected_min, abs=BOUNDS_ABSOLUTE_TOLERANCE_MM
    )
    assert build.measured.bounds.max_mm == pytest.approx(
        expected_max, abs=BOUNDS_ABSOLUTE_TOLERANCE_MM
    )
    assert build.solid_count == 1


def test_the_l_bracket_keeps_its_single_solid_check(
    provider: FreeCadProvider,
    tmp_path: Path,
) -> None:
    script = _lbracket_script()
    service = _service(tmp_path, provider)
    try:
        outcome = service.design(_intent(script), provider="freecad")
    finally:
        service.close()

    checks = {check.id: check for check in outcome.checks}
    assert set(checks) == {"brep_valid", "single_solid"}
    single = checks["single_solid"]
    assert single.expected == 1.0
    assert single.measured == 1.0
    assert single.status == "passed"
    assert "solid_count" not in checks
    assert outcome.solid_count == 1
    assert outcome.verified is True


@pytest.mark.parametrize(
    ("case_id", "expected_solids"),
    [
        ("assembly_two_parts_placed_once", 2),
        ("assembly_three_parts_placed_once", 3),
    ],
)
def test_a_place_only_assembly_verifies_with_its_own_solid_count(
    provider: FreeCadProvider,
    tmp_path: Path,
    case_id: str,
    expected_solids: int,
) -> None:
    case = _case(case_id)
    script = ConstructionScript.model_validate(case["script"])
    service = _service(tmp_path, provider)
    try:
        outcome = service.design(_intent(script), provider="freecad")
    finally:
        service.close()

    checks = {check.id: check for check in outcome.checks}
    assert set(checks) == {"brep_valid", "solid_count"}
    row = checks["solid_count"]
    assert row.label == "Solid count"
    assert row.expected == float(expected_solids)
    assert row.measured == float(expected_solids)
    assert row.status == "passed"
    assert "single_solid" not in checks
    assert outcome.solid_count == expected_solids
    assert outcome.component_count == expected_solids
    assert outcome.fused is False
    assert outcome.verified is True

    # A cold reload derives the same expected count from the stored script.
    reopened = _service(tmp_path, provider)
    try:
        loaded = reopened.load(outcome.design_id)
    finally:
        reopened.close()
    reloaded = {check.id: check for check in loaded.checks}["solid_count"]
    assert reloaded.expected == float(expected_solids)
    assert reloaded.measured == float(expected_solids)
    assert reloaded.status == "passed"
    assert loaded.verified is True
