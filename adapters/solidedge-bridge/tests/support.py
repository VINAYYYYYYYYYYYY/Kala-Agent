"""Shared test data builders. Never used by production code."""

from __future__ import annotations

from kala_solidedge_bridge.protocol import (
    BuildRequest,
    PartMeasure,
    PartNew,
    Plane,
    SketchRectangle,
    SolidExtrude,
)


def build_request(
    request_id: str = "req-1",
    *,
    timeout_seconds: float = 30.0,
    summary: str = "",
    calibration_id: str | None = "cal-test",
) -> BuildRequest:
    return BuildRequest(
        request_id=request_id,
        calibration_id=calibration_id,
        summary=summary,
        timeout_seconds=timeout_seconds,
        procedure=(
            PartNew(name="plate"),
            SketchRectangle(plane=Plane.XY, width=20.0, height=20.0),
            SolidExtrude(length=10.0),
            PartMeasure(),
        ),
    )
