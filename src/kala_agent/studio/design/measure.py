"""Measure an OpenCascade B-Rep shape and build the honesty checks.

Every number Studio shows for a design comes from this module. Nothing here
is closed-form or estimated: volume, surface area, centroid, bounds, validity,
and solid count are all read from the kernel. The checks compare those measured
values with a *declared* envelope and with the validity rule; a template may
declare an envelope, but it may never declare a volume.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Final, TypeAlias

from OCP.Bnd import Bnd_Box  # type: ignore[import-untyped]
from OCP.BRepBndLib import BRepBndLib  # type: ignore[import-untyped]
from OCP.BRepCheck import BRepCheck_Analyzer  # type: ignore[import-untyped]
from OCP.BRepGProp import BRepGProp  # type: ignore[import-untyped]
from OCP.GProp import GProp_GProps  # type: ignore[import-untyped]
from OCP.TopAbs import TopAbs_SOLID  # type: ignore[import-untyped]
from OCP.TopExp import TopExp_Explorer  # type: ignore[import-untyped]
from OCP.TopoDS import TopoDS_Shape  # type: ignore[import-untyped]

ENVELOPE_TOLERANCE_MM: Final[float] = 0.01
SINGLE_SOLID_TOLERANCE: Final[float] = 0.0
SOLID_COUNT_TOLERANCE: Final[float] = 0.0
BREP_VALID_TOLERANCE: Final[float] = 0.0

CheckValue: TypeAlias = bool | float | tuple[float, float] | None


class MeasurementError(ValueError):
    """The shape could not be measured honestly."""


@dataclass(frozen=True, slots=True)
class Bounds:
    """An axis-aligned box in millimetres, measured or declared."""

    min_mm: tuple[float, float, float]
    max_mm: tuple[float, float, float]

    def __post_init__(self) -> None:
        if len(self.min_mm) != 3 or len(self.max_mm) != 3:
            raise ValueError("bounds require three minimum and three maximum values")
        for value in (*self.min_mm, *self.max_mm):
            if not math.isfinite(value):
                raise ValueError("bounds must be finite numbers")
        for axis in range(3):
            if self.min_mm[axis] > self.max_mm[axis]:
                raise ValueError("bounds minimum exceeds maximum")

    @property
    def size_mm(self) -> tuple[float, float, float]:
        """Return the box size along X, Y, and Z."""
        return (
            self.max_mm[0] - self.min_mm[0],
            self.max_mm[1] - self.min_mm[1],
            self.max_mm[2] - self.min_mm[2],
        )


@dataclass(frozen=True, slots=True)
class Measurement:
    """Every kernel-measured number for one shape."""

    volume_mm3: float
    surface_area_mm2: float
    centroid_mm: tuple[float, float, float]
    bounds: Bounds
    valid: bool
    solid_count: int


@dataclass(frozen=True, slots=True)
class Check:
    """One declared-versus-measured check row."""

    id: str
    label: str
    expected: CheckValue
    measured: CheckValue
    tolerance: float
    unit: str
    passed: bool

    @property
    def status(self) -> str:
        """Return ``passed``, ``failed``, or the honest ``not_reported``.

        A provider that reports no value is never given a substituted number:
        the row carries ``measured=None`` and this status, and the design is
        not verified.
        """
        if self.measured is None:
            return "not_reported"
        return "passed" if self.passed else "failed"


@dataclass(frozen=True, slots=True)
class CheckReport:
    """The ordered checks and the single verification verdict."""

    checks: tuple[Check, ...]
    verified: bool


def measure_shape(shape: TopoDS_Shape) -> Measurement:
    """Measure ``shape`` with the exact OCP property and bounds algorithms."""
    if shape.IsNull():
        raise MeasurementError("the shape is null and cannot be measured")
    try:
        volume = _volume(shape)
        area = _surface_area(shape)
        bounds = _optimal_bounds(shape)
        centroid = _centroid(shape)
        valid = bool(BRepCheck_Analyzer(shape).IsValid())
        solid_count = _solid_count(shape)
    except MeasurementError:
        raise
    except Exception as error:  # pragma: no cover - OCP raises bare exceptions
        raise MeasurementError(
            f"the geometry kernel could not measure the shape: {type(error).__name__}"
        ) from None
    return Measurement(
        volume_mm3=volume,
        surface_area_mm2=area,
        centroid_mm=centroid,
        bounds=bounds,
        valid=valid,
        solid_count=solid_count,
    )


def build_checks(
    measurement: Measurement,
    *,
    expected_envelope: Bounds | None = None,
    expect_single_solid: bool = True,
) -> CheckReport:
    """Build the mandatory checks and the ``verified`` verdict.

    ``brep_valid`` is always present. ``single_solid`` is present when the
    caller expects one part. The three envelope checks are present only when a
    template declared an envelope; no envelope is invented for a raw script.
    """
    checks: list[Check] = [
        Check(
            id="brep_valid",
            label="B-Rep valid",
            expected=True,
            measured=measurement.valid,
            tolerance=BREP_VALID_TOLERANCE,
            unit="bool",
            passed=measurement.valid,
        )
    ]
    if expect_single_solid:
        checks.append(
            Check(
                id="single_solid",
                label="Single solid",
                expected=1.0,
                measured=float(measurement.solid_count),
                tolerance=SINGLE_SOLID_TOLERANCE,
                unit="solid",
                passed=measurement.solid_count == 1,
            )
        )
    if expected_envelope is not None:
        measured = measurement.bounds
        for axis, name in enumerate(("x", "y", "z")):
            expected_pair = (expected_envelope.min_mm[axis], expected_envelope.max_mm[axis])
            measured_pair = (measured.min_mm[axis], measured.max_mm[axis])
            passed = (
                abs(expected_pair[0] - measured_pair[0]) <= ENVELOPE_TOLERANCE_MM
                and abs(expected_pair[1] - measured_pair[1]) <= ENVELOPE_TOLERANCE_MM
            )
            checks.append(
                Check(
                    id=f"envelope_{name}",
                    label=f"Envelope {name.upper()}",
                    expected=expected_pair,
                    measured=measured_pair,
                    tolerance=ENVELOPE_TOLERANCE_MM,
                    unit="mm",
                    passed=passed,
                )
            )
    return CheckReport(checks=tuple(checks), verified=all(check.passed for check in checks))


def measure_and_check(
    shape: TopoDS_Shape,
    *,
    expected_envelope: Bounds | None = None,
    expect_single_solid: bool = True,
) -> tuple[Measurement, CheckReport]:
    """Measure a shape and build its checks in one call."""
    measurement = measure_shape(shape)
    return measurement, build_checks(
        measurement,
        expected_envelope=expected_envelope,
        expect_single_solid=expect_single_solid,
    )


# --------------------------------------------------------------------- kernel


def _volume(shape: TopoDS_Shape) -> float:
    properties = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, properties)
    return float(properties.Mass())


def _surface_area(shape: TopoDS_Shape) -> float:
    properties = GProp_GProps()
    BRepGProp.SurfaceProperties_s(shape, properties)
    return float(properties.Mass())


def _centroid(shape: TopoDS_Shape) -> tuple[float, float, float]:
    properties = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, properties)
    centre = properties.CentreOfMass()
    return (centre.X(), centre.Y(), centre.Z())


def _optimal_bounds(shape: TopoDS_Shape) -> Bounds:
    box = Bnd_Box()
    box.SetGap(0.0)
    BRepBndLib.AddOptimal_s(shape, box, True, False)
    if box.IsVoid():
        raise MeasurementError("the shape has no finite bounding box")
    minimum = box.CornerMin()
    maximum = box.CornerMax()
    bounds = Bounds(
        min_mm=(minimum.X(), minimum.Y(), minimum.Z()),
        max_mm=(maximum.X(), maximum.Y(), maximum.Z()),
    )
    for value in (*bounds.min_mm, *bounds.max_mm):
        if not math.isfinite(value):
            raise MeasurementError("the shape has no finite bounding box")
    return bounds


def _solid_count(shape: TopoDS_Shape) -> int:
    explorer = TopExp_Explorer(shape, TopAbs_SOLID)
    count = 0
    while explorer.More():
        count += 1
        explorer.Next()
    return count


__all__ = (
    "BREP_VALID_TOLERANCE",
    "ENVELOPE_TOLERANCE_MM",
    "SINGLE_SOLID_TOLERANCE",
    "SOLID_COUNT_TOLERANCE",
    "Bounds",
    "Check",
    "CheckReport",
    "CheckValue",
    "Measurement",
    "MeasurementError",
    "build_checks",
    "measure_and_check",
    "measure_shape",
)
