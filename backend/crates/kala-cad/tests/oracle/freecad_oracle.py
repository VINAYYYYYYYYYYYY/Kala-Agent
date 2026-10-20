#!/usr/bin/env python3
"""Optional second-reader oracle through FreeCAD.

Run by `tests/freecad_differential.rs` inside `freecadcmd -c` (FreeCAD's CLI
loads Python from stdin). It re-imports the native STEP and STL files with
FreeCAD 1.1.4 and reports geometry for comparison with the native measurement.

This is a second, application-level reader, not the mandatory OCP oracle. A
missing FreeCAD installation is reported by the Rust test; when FreeCAD is
present, a failure is a test failure.
"""

from __future__ import annotations

import json
import math
import sys

import FreeCAD
import Mesh
import Part

VOLUME_RELATIVE = 1e-6
BOUNDS_ABSOLUTE_MM = 0.001
STL_BOUNDS_ABSOLUTE_MM = 0.001


def close(mismatches, case, field, native, freecad, *, relative, absolute):
    difference = abs(float(native) - float(freecad))
    if not (
        math.isfinite(float(native))
        and math.isfinite(float(freecad))
    ) or difference > absolute + relative * abs(float(native)):
        mismatches.append(
            {
                "case": case,
                "field": field,
                "native": native,
                "freecad": freecad,
                "difference": difference,
            }
        )


def main() -> int:
    request = json.loads(open(sys.argv[1], encoding="utf-8").read())
    mismatches: list[dict[str, object]] = []
    checked = {"steps": 0, "stls": 0}

    for entry in request["steps"]:
        checked["steps"] += 1
        shape = Part.Shape()
        try:
            shape.read(entry["path"])
        except Exception as error:  # noqa: BLE001
            mismatches.append(
                {"case": entry["id"], "field": "step.read",
                 "freecad": f"{type(error).__name__}: {error}"}
            )
            continue
        native = entry["native"]
        if shape.isNull():
            mismatches.append(
                {"case": entry["id"], "field": "step.null", "freecad": True}
            )
            continue
        box = shape.BoundBox
        close(
            mismatches, entry["id"], "step.volume",
            native["volume"], shape.Volume,
            relative=VOLUME_RELATIVE, absolute=1e-9,
        )
        for axis, (low, high) in enumerate(
            [
                (box.XMin, box.XMax),
                (box.YMin, box.YMax),
                (box.ZMin, box.ZMax),
            ]
        ):
            close(
                mismatches, entry["id"], f"step.min[{axis}]",
                native["min"][axis], low,
                relative=0.0, absolute=BOUNDS_ABSOLUTE_MM,
            )
            close(
                mismatches, entry["id"], f"step.max[{axis}]",
                native["max"][axis], high,
                relative=0.0, absolute=BOUNDS_ABSOLUTE_MM,
            )
        if int(native["solid_count"]) != len(shape.Solids):
            mismatches.append(
                {"case": entry["id"], "field": "step.solid_count",
                 "native": native["solid_count"], "freecad": len(shape.Solids)}
            )
        if not shape.isValid():
            mismatches.append(
                {"case": entry["id"], "field": "step.valid", "freecad": False}
            )

    for entry in request["stls"]:
        checked["stls"] += 1
        try:
            mesh = Mesh.Mesh(entry["path"])
        except Exception as error:  # noqa: BLE001
            mismatches.append(
                {"case": entry["id"], "field": "stl.read",
                 "freecad": f"{type(error).__name__}: {error}"}
            )
            continue
        native = entry["native"]
        if len(mesh.Facets) != int(native["triangle_count"]):
            mismatches.append(
                {"case": entry["id"], "field": "stl.facets",
                 "native": native["triangle_count"], "freecad": len(mesh.Facets)}
            )
        box = mesh.BoundBox
        for axis, (low, high) in enumerate(
            [
                (box.XMin, box.XMax),
                (box.YMin, box.YMax),
                (box.ZMin, box.ZMax),
            ]
        ):
            close(
                mismatches, entry["id"], f"stl.min[{axis}]",
                native["min"][axis], low,
                relative=0.0, absolute=STL_BOUNDS_ABSOLUTE_MM,
            )
            close(
                mismatches, entry["id"], f"stl.max[{axis}]",
                native["max"][axis], high,
                relative=0.0, absolute=STL_BOUNDS_ABSOLUTE_MM,
            )

    result = {
        "ok": not mismatches,
        "mismatches": mismatches,
        "checked": checked,
        "versions": {
            "freecad": ".".join(FreeCAD.Version()[0:3]),
            "python": sys.version,
        },
    }
    with open(sys.argv[2], "w", encoding="utf-8") as handle:
        json.dump(result, handle, indent=2)
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
