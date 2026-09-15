"""GeometryProbe backend — lightweight validation via live FreeCAD backend handle."""

from __future__ import annotations

from typing import Any

from kala.analysis.base import AnalysisReport, AnalysisRequest


class GeometryProbeBackend:
    """Geometry probe using live FreeCAD backend handle for validation."""

    def analyze(self, request: AnalysisRequest) -> AnalysisReport:
        """Probe geometry validity, volume, and bounding box via FreeCAD backend.

        Soft-fail on all errors — never raises exceptions.
        """
        body_id = request.body_id
        backend_handle = request.backend_handle

        if backend_handle is None:
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message="No backend handle provided",
                metrics={},
            )

        try:
            return self._probe_freecad(body_id, backend_handle)
        except Exception as exc:  # noqa: BLE001
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message=f"Analysis failed: {exc}",
                metrics={},
            )

    def _probe_freecad(self, body_id: str, backend: Any) -> AnalysisReport:
        """Probe FreeCAD body for basic geometric properties."""
        doc = getattr(backend, "_doc", None)
        if doc is None:
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message="Backend has no _doc attribute",
                metrics={},
            )

        obj = doc.getObject(body_id)
        if obj is None:
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message=f"Body not found: {body_id}",
                metrics={},
            )

        shape = getattr(obj, "Shape", None)
        if shape is None:
            return AnalysisReport(
                body_id=body_id,
                ok=False,
                message=f"Body {body_id} has no Shape attribute",
                metrics={},
            )

        metrics: dict[str, Any] = {}

        try:
            is_valid = shape.isValid()
            metrics["is_valid"] = is_valid
        except Exception:  # noqa: BLE001
            metrics["is_valid"] = None

        try:
            is_null = shape.isNull()
            metrics["is_null"] = is_null
        except Exception:  # noqa: BLE001
            metrics["is_null"] = None

        try:
            volume = float(shape.Volume)
            metrics["volume"] = volume
        except Exception:  # noqa: BLE001
            metrics["volume"] = None

        try:
            bbox = shape.BoundBox
            metrics["bbox"] = {
                "x_min": float(bbox.XMin),
                "y_min": float(bbox.YMin),
                "z_min": float(bbox.ZMin),
                "x_max": float(bbox.XMax),
                "y_max": float(bbox.YMax),
                "z_max": float(bbox.ZMax),
                "x_length": float(bbox.XLength),
                "y_length": float(bbox.YLength),
                "z_length": float(bbox.ZLength),
            }
        except Exception:  # noqa: BLE001
            metrics["bbox"] = None

        is_valid = metrics.get("is_valid")
        is_null = metrics.get("is_null")
        volume = metrics.get("volume")

        if is_null:
            message = f"Geometry is null for {body_id}"
            ok = False
        elif is_valid is False:
            message = f"Geometry is invalid for {body_id}"
            ok = False
        elif volume is not None and volume <= 0.0:
            message = f"Geometry has zero or negative volume: {volume}"
            ok = False
        else:
            message = f"Geometry probe OK for {body_id}"
            ok = True

        return AnalysisReport(
            body_id=body_id,
            ok=ok,
            message=message,
            metrics=metrics,
        )
