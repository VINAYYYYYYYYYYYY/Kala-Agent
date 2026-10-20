"""Test-only FreeCAD importer cross-check for the native `kala-step` exporter.

OpenCascade through `cadquery-ocp` is the required independent oracle
(`ocp_oracle.py`). FreeCAD is a second, application-level consumer with its own
import path, so this script re-reads the same emitted files there and reports
whether a different CAD application accepts them as one closed analytic solid.

Usage:
    FreeCADCmd freecad_oracle.py <request.json> <report.json>
"""

from __future__ import annotations

import json
import sys

import FreeCAD
import Part

_SURFACE_NAMES = {
    "Plane": "plane",
    "Cylinder": "cylinder",
    "Sphere": "sphere",
    "Cone": "cone",
    "Toroid": "torus",
    "BSplineSurface": "bspline",
}


def main() -> int:
    request_path, report_path = sys.argv[-2], sys.argv[-1]
    request = json.loads(open(request_path, encoding="utf-8").read())

    report = {
        "freecad_version": ".".join(FreeCAD.Version()[0:3]),
        "cases": [],
    }
    for case in request["cases"]:
        entry = {"id": case["id"]}
        try:
            shape = Part.Shape()
            shape.read(case["step_path"])
            surfaces: dict[str, int] = {}
            for face in shape.Faces:
                name = _SURFACE_NAMES.get(
                    face.Surface.__class__.__name__, face.Surface.__class__.__name__
                )
                surfaces[name] = surfaces.get(name, 0) + 1
            bounds = shape.BoundBox
            entry.update(
                {
                    "valid": bool(shape.isValid()),
                    "closed": bool(shape.isClosed()),
                    "solids": len(shape.Solids),
                    "shells": len(shape.Shells),
                    "faces": len(shape.Faces),
                    "volume": shape.Volume,
                    "area": shape.Area,
                    "surface_types": surfaces,
                    "bounds_min": [bounds.XMin, bounds.YMin, bounds.ZMin],
                    "bounds_max": [bounds.XMax, bounds.YMax, bounds.ZMax],
                }
            )
        except Exception as error:  # reported to the Rust test, never fatal
            entry["oracle_error"] = f"{type(error).__name__}: {error}"
        report["cases"].append(entry)

    with open(report_path, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2)
    return 0


# FreeCADCmd executes this file with `__name__` set to the script stem (not
# "__main__"), so the entry point runs unconditionally: this file is only ever
# started by FreeCADCmd from the Rust differential test.
raise SystemExit(main())
