"""Measurement and checks are read from OpenCascade, never guessed."""

from __future__ import annotations

import math
from collections.abc import Sequence
from typing import cast

import pytest
from OCP.BRep import BRep_Builder  # type: ignore[import-untyped]
from OCP.BRepBuilderAPI import (  # type: ignore[import-untyped]
    BRepBuilderAPI_MakeEdge,
    BRepBuilderAPI_MakeFace,
    BRepBuilderAPI_MakeWire,
    BRepBuilderAPI_Transform,
)
from OCP.BRepPrimAPI import (  # type: ignore[import-untyped]
    BRepPrimAPI_MakeBox,
    BRepPrimAPI_MakeCylinder,
)
from OCP.gp import (  # type: ignore[import-untyped]
    gp_Ax2,
    gp_Circ,
    gp_Dir,
    gp_Pnt,
    gp_Trsf,
    gp_Vec,
)
from OCP.TopoDS import TopoDS_Compound, TopoDS_Shape  # type: ignore[import-untyped]

from kala_agent.studio.design.measure import (
    Bounds,
    MeasurementError,
    build_checks,
    measure_and_check,
    measure_shape,
)


def _box(size_x: float = 20.0, size_y: float = 20.0, size_z: float = 10.0) -> TopoDS_Shape:
    shape: TopoDS_Shape = BRepPrimAPI_MakeBox(size_x, size_y, size_z).Shape()
    return shape


def _cylinder(radius: float = 2.0, height: float = 5.0) -> TopoDS_Shape:
    axis = gp_Ax2(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0))
    shape: TopoDS_Shape = BRepPrimAPI_MakeCylinder(axis, radius, height).Shape()
    return shape


def _compound(shapes: Sequence[TopoDS_Shape]) -> TopoDS_Shape:
    compound = TopoDS_Compound()
    builder = BRep_Builder()
    builder.MakeCompound(compound)
    for shape in shapes:
        builder.Add(compound, shape)
    result: TopoDS_Shape = compound
    return result


def _disc(radius: float = 1.0) -> TopoDS_Shape:
    circle = gp_Circ(gp_Ax2(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0)), radius)
    edge = BRepBuilderAPI_MakeEdge(circle).Edge()
    wire = BRepBuilderAPI_MakeWire(edge).Wire()
    face: TopoDS_Shape = BRepBuilderAPI_MakeFace(wire).Face()
    return face


def test_a_box_is_measured_exactly() -> None:
    measurement = measure_shape(_box())

    assert measurement.volume_mm3 == pytest.approx(4000.0, rel=1e-12)
    assert measurement.surface_area_mm2 == pytest.approx(1600.0, rel=1e-12)
    assert measurement.centroid_mm == pytest.approx((10.0, 10.0, 5.0))
    assert measurement.bounds.min_mm == pytest.approx((0.0, 0.0, 0.0))
    assert measurement.bounds.max_mm == pytest.approx((20.0, 20.0, 10.0))
    assert measurement.bounds.size_mm == pytest.approx((20.0, 20.0, 10.0))
    assert measurement.valid is True
    assert measurement.solid_count == 1


def test_a_cylinder_is_measured_exactly() -> None:
    measurement = measure_shape(_cylinder())

    assert measurement.volume_mm3 == pytest.approx(math.pi * 2.0**2 * 5.0, rel=1e-12)
    assert measurement.surface_area_mm2 == pytest.approx(
        2.0 * math.pi * 2.0**2 + 2.0 * math.pi * 2.0 * 5.0, rel=1e-12
    )
    assert measurement.centroid_mm == pytest.approx((0.0, 0.0, 2.5))
    assert measurement.bounds.min_mm == pytest.approx((-2.0, -2.0, 0.0))
    assert measurement.bounds.max_mm == pytest.approx((2.0, 2.0, 5.0))
    assert measurement.valid is True
    assert measurement.solid_count == 1


def test_validity_and_single_solid_are_always_checked() -> None:
    measurement = measure_shape(_box())
    report = build_checks(measurement)

    assert [check.id for check in report.checks] == ["brep_valid", "single_solid"]
    assert report.verified is True
    brep = report.checks[0]
    assert brep.expected is True
    assert brep.measured is True
    assert brep.tolerance == 0.0
    assert brep.unit == "bool"
    single = report.checks[1]
    assert single.expected == 1.0
    assert single.measured == 1.0
    assert single.tolerance == 0.0
    assert single.unit == "solid"


def test_the_envelope_is_compared_to_the_measured_bounds() -> None:
    measurement = measure_shape(_box())
    envelope = Bounds(min_mm=(0.0, 0.0, 0.0), max_mm=(20.0, 20.0, 10.0))
    report = build_checks(measurement, expected_envelope=envelope)

    assert report.verified is True
    assert [check.id for check in report.checks] == [
        "brep_valid",
        "single_solid",
        "envelope_x",
        "envelope_y",
        "envelope_z",
    ]
    for check in report.checks[2:]:
        assert check.tolerance == 0.01
        assert check.unit == "mm"
        assert check.passed is True


def test_a_wrong_envelope_fails_verification() -> None:
    measurement = measure_shape(_box())
    envelope = Bounds(min_mm=(0.0, 0.0, 0.0), max_mm=(20.0, 20.0, 11.0))
    report = build_checks(measurement, expected_envelope=envelope)

    assert report.verified is False
    failed = [check for check in report.checks if not check.passed]
    assert [check.id for check in failed] == ["envelope_z"]
    assert failed[0].expected == (0.0, 11.0)
    assert failed[0].measured == (0.0, 10.0)


def test_a_multi_solid_compound_is_measured_and_fails_single_solid() -> None:
    first = _box(10.0, 10.0, 5.0)
    second = _box(10.0, 10.0, 5.0)
    transform = gp_Trsf()
    transform.SetTranslation(gp_Vec(20.0, 0.0, 0.0))
    moved: TopoDS_Shape = BRepBuilderAPI_Transform(second, transform, True).Shape()
    measurement = measure_shape(_compound([first, moved]))

    assert measurement.volume_mm3 == pytest.approx(1000.0, rel=1e-12)
    assert measurement.bounds.min_mm == pytest.approx((0.0, 0.0, 0.0))
    assert measurement.bounds.max_mm == pytest.approx((30.0, 10.0, 5.0))
    assert measurement.solid_count == 2
    assert measurement.valid is True

    report = build_checks(measurement)
    assert report.verified is False
    assert [check.id for check in report.checks if not check.passed] == ["single_solid"]

    without_single = build_checks(measurement, expect_single_solid=False)
    assert [check.id for check in without_single.checks] == ["brep_valid"]
    assert without_single.verified is True


def test_measure_and_check_returns_the_same_measurement() -> None:
    shape = _box()
    measurement, report = measure_and_check(
        shape,
        expected_envelope=Bounds((0.0, 0.0, 0.0), (20.0, 20.0, 10.0)),
    )

    assert measurement.volume_mm3 == pytest.approx(4000.0, rel=1e-12)
    assert report.verified is True
    assert report == build_checks(
        measurement,
        expected_envelope=Bounds((0.0, 0.0, 0.0), (20.0, 20.0, 10.0)),
    )


def test_a_zero_volume_face_is_measured_honestly_but_is_not_a_part() -> None:
    measurement = measure_shape(_disc())
    report = build_checks(measurement)

    assert measurement.volume_mm3 == pytest.approx(0.0, abs=1e-12)
    assert measurement.solid_count == 0
    assert measurement.bounds.min_mm == pytest.approx((-1.0, -1.0, 0.0))
    assert measurement.bounds.max_mm == pytest.approx((1.0, 1.0, 0.0))
    assert report.verified is False


def test_a_null_shape_raises_a_measurement_error() -> None:
    with pytest.raises(MeasurementError, match="null"):
        measure_shape(TopoDS_Shape())


def test_bounds_reject_inverted_and_non_finite_values() -> None:
    with pytest.raises(ValueError, match="minimum exceeds maximum"):
        Bounds(min_mm=(1.0, 0.0, 0.0), max_mm=(0.0, 1.0, 1.0))
    with pytest.raises(ValueError, match="finite"):
        Bounds(min_mm=(0.0, 0.0, 0.0), max_mm=(math.inf, 1.0, 1.0))
    with pytest.raises(ValueError, match="three"):
        Bounds(
            min_mm=cast("tuple[float, float, float]", (0.0, 0.0)),
            max_mm=(1.0, 1.0, 1.0),
        )
