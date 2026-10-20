"""The OCP compiler follows the measured construction semantics and refuses the rest."""

from __future__ import annotations

import json
import math
import time
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any, cast

import pytest

from kala_agent.contracts.construction import (
    AnalysisScript,
    ConstructionError,
    ConstructionScript,
    EditStep,
    load_analysis,
    load_construction,
)
from kala_agent.studio.design.compile_ocp import CompilationResult, compile_script
from kala_agent.studio.design.measure import Bounds, build_checks, measure_shape

PROJECT_ROOT = Path(__file__).parents[3]
FIXTURE_PATH = PROJECT_ROOT / "tests" / "fixtures" / "construction_parity.json"
L_BRACKET_VOLUME_MM3 = 35_571.416_047
L_BRACKET_ENVELOPE = Bounds(min_mm=(-30.0, 0.0, 0.0), max_mm=(30.0, 45.0, 40.0))


def _script(
    components: Sequence[Mapping[str, object]],
    assembly: Sequence[Mapping[str, object]] = (),
) -> ConstructionScript:
    return load_construction(
        {
            "summary": "compiler test",
            "components": list(components),
            "assembly": list(assembly),
        }
    )


def _rectangle_component(
    name: str = "plate",
    *,
    plane: str = "XY",
    width: float = 20.0,
    height: float = 20.0,
    length: float = 10.0,
    x: float = 0.0,
    y: float = 0.0,
    reversed_: bool = False,
) -> dict[str, object]:
    return {
        "name": name,
        "steps": [
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
        ],
    }


def _l_bracket_script() -> ConstructionScript:
    """Build the ADR L-bracket from the v1 feature set.

    The upright is an XZ pad so its bore can run along Y: the pad grows from
    ``y = 0`` down to ``y = -T``, the through pocket cuts the same way, and the
    finished upright is placed at ``y = +T`` before the fuse.
    """
    base_steps: list[dict[str, object]] = [
        {
            "op": "sketch",
            "profile": "rectangle",
            "plane": "XY",
            "width": 60.0,
            "height": 45.0,
            "x": -30.0,
            "y": 0.0,
        },
        {"op": "extrude", "length": 8.0},
    ]
    for y in (16.25, 36.75):
        for x in (-21.75, 21.75):
            base_steps.extend(
                [
                    {
                        "op": "sketch",
                        "profile": "circle",
                        "plane": "XY",
                        "diameter": 5.5,
                        "x": x,
                        "y": y,
                    },
                    {"op": "pocket"},
                ]
            )
    upright_steps: list[dict[str, object]] = [
        {
            "op": "sketch",
            "profile": "rectangle",
            "plane": "XZ",
            "width": 60.0,
            "height": 40.0,
            "x": -30.0,
            "y": 0.0,
        },
        {"op": "extrude", "length": 8.0},
        {
            "op": "sketch",
            "profile": "circle",
            "plane": "XZ",
            "diameter": 10.0,
            "x": 0.0,
            "y": 24.0,
        },
        {"op": "pocket"},
    ]
    return _script(
        [
            {"name": "base", "steps": base_steps},
            {"name": "upright", "steps": upright_steps},
        ],
        assembly=[
            {"op": "place", "component": "upright", "y": 8.0},
            {"op": "fuse", "components": ["base", "upright"]},
        ],
    )


def _load_fixture() -> dict[str, Any]:
    assert FIXTURE_PATH.is_file(), (
        f"the frozen parity fixture is missing: {FIXTURE_PATH}; "
        "run tests/fixtures/generate_construction_parity.py"
    )
    return cast("dict[str, Any]", json.loads(FIXTURE_PATH.read_text(encoding="utf-8")))


def _assert_volume(actual: float, expected: float, *, relative: float = 1e-6) -> None:
    assert abs(actual - expected) <= relative * abs(expected)


def _assert_bounds(
    measured: Bounds,
    expected_min: Sequence[float],
    expected_max: Sequence[float],
    *,
    tolerance: float = 0.001,
) -> None:
    for axis in range(3):
        assert abs(measured.min_mm[axis] - expected_min[axis]) <= tolerance
        assert abs(measured.max_mm[axis] - expected_max[axis]) <= tolerance


# --------------------------------------------------------------------- parity


def test_the_fixture_exists_and_records_its_generator_and_oracle_digest() -> None:
    fixture = _load_fixture()
    generator = fixture["generator"]

    assert generator["command"] == (
        "PYTHONPATH=adapters/solidedge/src:src .venv/bin/python "
        "tests/fixtures/generate_construction_parity.py"
    )
    assert generator["oracle_source"] == "adapters/solidedge/src/kala_agent_solidedge/oracle.py"
    digest = generator["oracle_sha256"]
    assert isinstance(digest, str)
    assert len(digest) == 64
    assert all(character in "0123456789abcdef" for character in digest)
    assert len(fixture["cases"]) >= 20


def test_every_corpus_case_matches_the_oracle() -> None:
    fixture = _load_fixture()
    checked_parts = 0
    documented_cases = 0

    for case in fixture["cases"]:
        script = load_construction(case["script"])
        result = compile_script(script)
        assert isinstance(result, CompilationResult)
        assert result.warnings == ()
        measured_parts = {
            component.name: measure_shape(component.local_shape) for component in result.components
        }

        for expected in case["oracle"]["parts"]:
            measured = measured_parts[expected["name"]]
            assert measured.valid, case["id"]
            if expected["checked"]:
                checked_parts += 1
                assert measured.solid_count == 1, case["id"]
                _assert_volume(measured.volume_mm3, float(expected["volume_mm3"]))
                _assert_bounds(measured.bounds, expected["min_mm"], expected["max_mm"])
            else:
                # The oracle deliberately does not predict a rectangle revolve;
                # the compiler must still produce a valid, non-empty solid.
                assert measured.solid_count == 1, case["id"]
                assert measured.volume_mm3 > 0.0, case["id"]
                assert "revolve" in expected["reason"], case["id"]

        assembly_bounds = case["oracle"]["assembly_bounds"]
        if assembly_bounds is not None:
            assembly = measure_shape(result.assembly_shape)
            assert assembly.valid, case["id"]
            _assert_bounds(
                assembly.bounds,
                assembly_bounds["min_mm"],
                assembly_bounds["max_mm"],
            )

        documented = case["documented"]
        if documented is not None:
            documented_cases += 1
            expected_part = case["oracle"]["parts"][0]
            assert expected_part["checked"], case["id"]
            assert (
                abs(float(expected_part["volume_mm3"]) - float(documented["volume_mm3"])) <= 1e-6
            ), case["id"]
            for key in ("min_mm", "max_mm"):
                for axis in range(3):
                    assert (
                        abs(float(expected_part[key][axis]) - float(documented[key][axis])) <= 1e-6
                    ), case["id"]

    assert checked_parts >= 20
    assert documented_cases >= 4


def test_documented_plane_case_reproduces_the_measured_boxes() -> None:
    fixture = _load_fixture()
    documented = {
        case["id"]: case
        for case in fixture["cases"]
        if case["id"]
        in {
            "rectangle_pad_xy_offset_forward",
            "rectangle_pad_xz_offset_forward",
            "rectangle_pad_yz_offset_forward",
        }
    }
    expected = {
        "rectangle_pad_xy_offset_forward": ((2.0, 3.0, 0.0), (6.0, 8.0, 6.0)),
        "rectangle_pad_xz_offset_forward": ((2.0, -6.0, 3.0), (6.0, 0.0, 8.0)),
        "rectangle_pad_yz_offset_forward": ((0.0, 2.0, 3.0), (6.0, 6.0, 8.0)),
    }
    for case_id, (minimum, maximum) in expected.items():
        script = load_construction(documented[case_id]["script"])
        measured = measure_shape(compile_script(script).components[0].local_shape)
        _assert_volume(measured.volume_mm3, 120.0)
        _assert_bounds(measured.bounds, minimum, maximum)


# --------------------------------------------------------------------- L-bracket


def test_l_bracket_matches_the_adr_oracle() -> None:
    result = compile_script(_l_bracket_script())

    assert result.fused is True
    assert result.warnings == ()
    measurement = measure_shape(result.assembly_shape)
    assert measurement.valid is True
    assert measurement.solid_count == 1
    _assert_volume(measurement.volume_mm3, L_BRACKET_VOLUME_MM3)
    _assert_bounds(
        measurement.bounds,
        L_BRACKET_ENVELOPE.min_mm,
        L_BRACKET_ENVELOPE.max_mm,
    )
    expected_closed_form = (
        60.0 * 45.0 * 8.0
        + 60.0 * 8.0 * (40.0 - 8.0)
        - math.pi * (10.0 / 2.0) ** 2 * 8.0
        - 4.0 * math.pi * (5.5 / 2.0) ** 2 * 8.0
    )
    assert measurement.volume_mm3 == pytest.approx(expected_closed_form, rel=1e-9)


def test_l_bracket_checks_verify() -> None:
    measurement = measure_shape(compile_script(_l_bracket_script()).assembly_shape)
    report = build_checks(measurement, expected_envelope=L_BRACKET_ENVELOPE)

    assert report.verified is True
    assert [check.id for check in report.checks] == [
        "brep_valid",
        "single_solid",
        "envelope_x",
        "envelope_y",
        "envelope_z",
    ]
    envelope = {check.id: check for check in report.checks}
    assert envelope["envelope_x"].expected == (-30.0, 30.0)
    assert envelope["envelope_x"].measured == (-30.0, 30.0)
    assert envelope["envelope_x"].tolerance == 0.01
    assert envelope["envelope_x"].unit == "mm"
    assert all(check.passed for check in report.checks)


# --------------------------------------------------------------------- semantics


def test_reversed_extrusion_flips_the_bounds_and_keeps_the_volume() -> None:
    forward = compile_script(
        _script([_rectangle_component("plate", width=4.0, height=5.0, length=6.0)])
    )
    reverse = compile_script(
        _script(
            [
                _rectangle_component(
                    "plate",
                    width=4.0,
                    height=5.0,
                    length=6.0,
                    reversed_=True,
                )
            ]
        )
    )

    forward_measure = measure_shape(forward.components[0].local_shape)
    reverse_measure = measure_shape(reverse.components[0].local_shape)
    _assert_volume(forward_measure.volume_mm3, 120.0)
    _assert_volume(reverse_measure.volume_mm3, 120.0)
    _assert_bounds(forward_measure.bounds, (0.0, 0.0, 0.0), (4.0, 5.0, 6.0))
    _assert_bounds(reverse_measure.bounds, (0.0, 0.0, -6.0), (4.0, 5.0, 0.0))


def test_xz_extrudes_along_negative_y() -> None:
    result = compile_script(
        _script(
            [
                _rectangle_component(
                    "plate",
                    plane="XZ",
                    width=4.0,
                    height=5.0,
                    length=6.0,
                    x=2.0,
                    y=3.0,
                )
            ]
        )
    )
    measured = measure_shape(result.components[0].local_shape)
    _assert_bounds(measured.bounds, (2.0, -6.0, 3.0), (6.0, 0.0, 8.0))


def test_a_circle_revolve_is_lowered_to_an_extrusion() -> None:
    result = compile_script(
        _script(
            [
                {
                    "name": "pin",
                    "steps": [
                        {
                            "op": "sketch",
                            "profile": "circle",
                            "plane": "XY",
                            "diameter": 6.0,
                        },
                        {"op": "revolve"},
                    ],
                }
            ]
        )
    )
    measured = measure_shape(result.components[0].local_shape)
    _assert_volume(measured.volume_mm3, math.pi * 3.0**2 * 6.0)
    _assert_bounds(measured.bounds, (-3.0, -3.0, 0.0), (3.0, 3.0, 6.0))


def test_a_rectangle_revolve_builds_a_ring() -> None:
    result = compile_script(
        _script(
            [
                {
                    "name": "bush",
                    "steps": [
                        {
                            "op": "sketch",
                            "profile": "rectangle",
                            "plane": "XY",
                            "width": 4.0,
                            "height": 5.0,
                            "x": 2.0,
                            "y": 0.0,
                        },
                        {"op": "revolve", "angle": 360.0},
                    ],
                }
            ]
        )
    )
    measured = measure_shape(result.components[0].local_shape)
    _assert_volume(measured.volume_mm3, math.pi * (6.0**2 - 2.0**2) * 5.0)
    _assert_bounds(measured.bounds, (-6.0, 0.0, -6.0), (6.0, 5.0, 6.0), tolerance=1e-6)


def test_place_translates_the_assembly_but_not_the_local_shape() -> None:
    script = _script(
        [_rectangle_component("plate", width=10.0, height=10.0, length=5.0)],
        assembly=[{"op": "place", "component": "plate", "x": 5.0, "y": 6.0, "z": 7.0}],
    )
    result = compile_script(script)

    local = measure_shape(result.components[0].local_shape)
    placed = measure_shape(result.components[0].placed_shape)
    _assert_bounds(local.bounds, (0.0, 0.0, 0.0), (10.0, 10.0, 5.0))
    _assert_bounds(placed.bounds, (5.0, 6.0, 7.0), (15.0, 16.0, 12.0))
    _assert_volume(placed.volume_mm3, local.volume_mm3)
    assert result.fused is False


def test_an_unfused_assembly_keeps_two_solids() -> None:
    script = _script(
        [
            _rectangle_component("a", width=10.0, height=10.0, length=5.0),
            _rectangle_component("b", width=10.0, height=10.0, length=5.0),
        ],
        assembly=[
            {"op": "place", "component": "a", "x": 0.0},
            {"op": "place", "component": "b", "x": 20.0},
        ],
    )
    result = compile_script(script)
    measured = measure_shape(result.assembly_shape)

    assert result.fused is False
    assert measured.solid_count == 2
    _assert_volume(measured.volume_mm3, 1000.0)
    _assert_bounds(measured.bounds, (0.0, 0.0, 0.0), (30.0, 10.0, 5.0))


def test_a_no_op_pocket_is_reported_not_claimed_as_removal() -> None:
    script = _script(
        [
            {
                "name": "plate",
                "steps": [
                    {
                        "op": "sketch",
                        "profile": "rectangle",
                        "plane": "XY",
                        "width": 20.0,
                        "height": 20.0,
                    },
                    {"op": "extrude", "length": 10.0, "reversed": True},
                    {
                        "op": "sketch",
                        "profile": "circle",
                        "plane": "XY",
                        "diameter": 2.0,
                        "x": 5.0,
                        "y": 5.0,
                    },
                    {"op": "pocket", "length": 4.0},
                ],
            }
        ]
    )
    result = compile_script(script)
    measured = measure_shape(result.assembly_shape)

    assert result.warnings
    assert "removed no material" in result.warnings[0]
    _assert_volume(measured.volume_mm3, 4000.0)


# --------------------------------------------------------------------- refusals


def test_helix_is_refused_by_name() -> None:
    script = _script(
        [
            {
                "name": "screw",
                "steps": [
                    {"op": "sketch", "profile": "rectangle", "width": 4.0, "height": 4.0},
                    {"op": "extrude", "length": 10.0},
                    {"op": "helix", "diameter": 4.0, "pitch": 1.0, "length": 5.0},
                ],
            }
        ]
    )
    with pytest.raises(ConstructionError, match="unsupported by the Studio OCP compiler v1: helix"):
        compile_script(script)


def test_compound_is_refused_by_name() -> None:
    script = _script(
        [
            _rectangle_component("a"),
            _rectangle_component("b"),
        ],
        assembly=[{"op": "compound", "components": ["a", "b"]}],
    )
    with pytest.raises(
        ConstructionError, match="unsupported by the Studio OCP compiler v1: compound"
    ):
        compile_script(script)


def test_analysis_edits_and_fillets_are_refused_by_name() -> None:
    plate = _script([_rectangle_component("plate")])
    analysis: AnalysisScript = load_analysis(
        {"steps": [{"op": "material"}, {"op": "mesh"}, {"op": "solve"}]}
    )
    with pytest.raises(
        ConstructionError, match="unsupported by the Studio OCP compiler v1: material"
    ):
        compile_script(plate, analysis=analysis)

    for edit, op in (
        (EditStep(op="move", component="plate", x=1.0), "move"),
        (EditStep(op="resize", component="plate", length=5.0), "resize"),
        (EditStep(op="fillet", component="plate", radius=1.0), "fillet"),
    ):
        with pytest.raises(
            ConstructionError, match=f"unsupported by the Studio OCP compiler v1: {op}"
        ):
            compile_script(plate, edits=(edit,))

    with pytest.raises(
        ConstructionError, match="unsupported by the Studio OCP compiler v1: fillet"
    ):
        compile_script(plate, fillets={"plate": 1.0})


def test_a_rectangle_revolve_on_xz_is_refused_by_name() -> None:
    script = _script(
        [
            {
                "name": "wheel",
                "steps": [
                    {
                        "op": "sketch",
                        "profile": "rectangle",
                        "plane": "XZ",
                        "width": 4.0,
                        "height": 5.0,
                    },
                    {"op": "revolve"},
                ],
            }
        ]
    )
    with pytest.raises(
        ConstructionError, match="unsupported by the Studio OCP compiler v1: revolve"
    ):
        compile_script(script)


def test_a_cross_plane_pocket_is_refused_by_name() -> None:
    script = _script(
        [
            {
                "name": "plate",
                "steps": [
                    {
                        "op": "sketch",
                        "profile": "rectangle",
                        "plane": "XY",
                        "width": 20.0,
                        "height": 20.0,
                    },
                    {"op": "extrude", "length": 10.0},
                    {
                        "op": "sketch",
                        "profile": "circle",
                        "plane": "XZ",
                        "diameter": 2.0,
                        "x": 5.0,
                        "y": 5.0,
                    },
                    {"op": "pocket"},
                ],
            }
        ]
    )
    with pytest.raises(
        ConstructionError, match="unsupported by the Studio OCP compiler v1: pocket"
    ):
        compile_script(script)


def test_a_duplicate_placement_is_refused() -> None:
    script = _script(
        [_rectangle_component("plate")],
        assembly=[
            {"op": "place", "component": "plate", "x": 1.0},
            {"op": "place", "component": "plate", "x": 2.0},
        ],
    )
    with pytest.raises(ConstructionError, match="placed more than once"):
        compile_script(script)


def test_the_compile_budget_is_enforced(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    moments = iter((0.0, 6.0, 6.0, 6.0))
    monkeypatch.setattr(time, "monotonic", lambda: next(moments))

    with pytest.raises(ConstructionError, match="compile budget"):
        compile_script(_script([_rectangle_component("plate")]))

    with pytest.raises(ConstructionError, match="non-negative finite"):
        compile_script(_script([_rectangle_component("plate")]), timeout_seconds=-1.0)
