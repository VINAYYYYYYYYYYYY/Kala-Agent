#!/usr/bin/env python3
"""Live drawing/section oracle for the native `kala-cad` crate.

This script is test-only. It imports the actual Python Studio drawing module
(`src/kala_agent/studio/design/drawing.py`) and runs `hlr_views`, `section` and
`default_section_offset` on:

* the native STEP documents written by the Rust test (so both implementations
  draw the same imported B-Rep);
* Python-built copies of the three shapes in
  `tests/unit/studio_design/test_drawing.py`, written to STEP and re-imported
  before drawing, again so both implementations see the same document.

It also records the exact Python refusal class and message for the argument,
shape and name errors.

The oracle is mandatory: a missing interpreter or OCP is a loud failure in the
Rust test. Usage: drawing_oracle.py REQUEST.json RESULT.json
"""

from __future__ import annotations

import hashlib
import json
import math
import sys
import traceback
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(ROOT / "src"))

from kala_agent.studio.design.drawing import (  # noqa: E402
    DrawingError,
    default_section_offset,
    hlr_views,
    section,
)

from OCP.BRepAlgoAPI import (  # noqa: E402
    BRepAlgoAPI_Cut,
    BRepAlgoAPI_Fuse,
)
from OCP.BRepPrimAPI import (  # noqa: E402
    BRepPrimAPI_MakeBox,
    BRepPrimAPI_MakeCylinder,
)
from OCP.gp import gp_Ax2, gp_Dir, gp_Pnt  # noqa: E402
from OCP.IFSelect import IFSelect_RetDone  # noqa: E402
from OCP.STEPControl import (  # noqa: E402
    STEPControl_AsIs,
    STEPControl_Reader,
    STEPControl_Writer,
)
from OCP.TopoDS import TopoDS_Shape  # noqa: E402


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_step(path: Path) -> Any:
    reader = STEPControl_Reader()
    status = reader.ReadFile(str(path))
    if status != IFSelect_RetDone:
        raise RuntimeError(f"the oracle could not read {path}: {status}")
    reader.TransferRoots()
    shape = reader.OneShape()
    if shape.IsNull():
        raise RuntimeError(f"the oracle read a null shape from {path}")
    return shape


def write_step(shape: Any, path: Path) -> None:
    writer = STEPControl_Writer()
    if writer.Transfer(shape, STEPControl_AsIs) != IFSelect_RetDone:
        raise RuntimeError(f"the oracle could not transfer {path}")
    if writer.Write(str(path)) != IFSelect_RetDone:
        raise RuntimeError(f"the oracle could not write {path}")


def _box(x0: float, y0: float, z0: float, dx: float, dy: float, dz: float) -> Any:
    return BRepPrimAPI_MakeBox(gp_Pnt(x0, y0, z0), dx, dy, dz).Shape()


def _cylinder(
    axis: tuple[float, float, float],
    origin: tuple[float, float, float],
    radius: float,
    height: float,
) -> Any:
    frame = gp_Ax2(gp_Pnt(*origin), gp_Dir(*axis))
    return BRepPrimAPI_MakeCylinder(frame, radius, height).Shape()


def build_l_bracket() -> Any:
    """The ADR-0009 L-bracket, exactly as the Python drawing test builds it."""
    base = _box(-30.0, 0.0, 0.0, 60.0, 45.0, 8.0)
    upright = _box(-30.0, 0.0, 0.0, 60.0, 8.0, 40.0)
    shape = BRepAlgoAPI_Fuse(base, upright).Shape()
    bore = _cylinder((0.0, 1.0, 0.0), (0.0, -1.0, 24.0), 5.0, 10.0)
    shape = BRepAlgoAPI_Cut(shape, bore).Shape()
    for hole_x in (-21.75, 21.75):
        for hole_y in (16.25, 36.75):
            hole = _cylinder((0.0, 0.0, 1.0), (hole_x, hole_y, -1.0), 2.75, 10.0)
            shape = BRepAlgoAPI_Cut(shape, hole).Shape()
    return shape


def build_box_with_through_hole() -> Any:
    box = _box(-20.0, 0.0, 0.0, 40.0, 30.0, 20.0)
    hole = _cylinder((0.0, 1.0, 0.0), (0.0, -1.0, 10.0), 5.0, 40.0)
    return BRepAlgoAPI_Cut(box, hole).Shape()


def build_box_with_an_island_in_a_bore() -> Any:
    box = _box(-20.0, 0.0, 0.0, 40.0, 30.0, 20.0)
    bore = _cylinder((0.0, 1.0, 0.0), (0.0, -1.0, 10.0), 10.0, 40.0)
    shape = BRepAlgoAPI_Cut(box, bore).Shape()
    island = _cylinder((0.0, 1.0, 0.0), (0.0, 5.0, 10.0), 4.0, 10.0)
    return BRepAlgoAPI_Fuse(shape, island).Shape()


BUILDERS = {
    "l_bracket": build_l_bracket,
    "box_with_through_hole": build_box_with_through_hole,
    "box_with_an_island_in_a_bore": build_box_with_an_island_in_a_bore,
}


def parse_float(value: Any) -> float:
    if isinstance(value, str):
        return {"nan": math.nan, "inf": math.inf, "-inf": -math.inf}[value]
    return float(value)


def parse_offset(value: Any) -> float | None:
    if value is None:
        return None
    return parse_float(value)


def polyline_payload(polyline: Any) -> dict[str, Any]:
    return {
        "points": [[float(point[0]), float(point[1])] for point in polyline.points],
        "closed": bool(polyline.closed),
    }


def view_payload(view: Any) -> dict[str, Any]:
    return {
        "name": view.name,
        "visible": [polyline_payload(line) for line in view.visible],
        "hidden": [polyline_payload(line) for line in view.hidden],
        "truncated": bool(view.truncated),
        "point_count": int(view.point_count),
    }


def section_payload(cut: Any) -> dict[str, Any]:
    return {
        "plane": cut.plane,
        "offset_mm": float(cut.offset_mm),
        "loops": [
            {
                "points": [
                    [float(point[0]), float(point[1])] for point in loop.points
                ],
                "area_mm2": float(loop.area_mm2),
                "outer": bool(loop.outer),
                "area_exact": bool(loop.area_exact),
            }
            for loop in cut.loops
        ],
        "area_mm2": float(cut.area_mm2),
        "chord_tolerance_mm": float(cut.chord_tolerance_mm),
        "truncated": bool(cut.truncated),
        "area_exact": bool(cut.area_exact),
    }


def draw_shape(shape: Any, spec: dict[str, Any]) -> dict[str, Any]:
    views = list(spec["views"])
    tolerance = float(spec["chord_tolerance_mm"])
    max_points = int(spec["max_points"])
    drawing = hlr_views(
        shape,
        views=views,
        chord_tolerance_mm=tolerance,
        max_points=max_points,
    )
    sections = [
        section(
            shape,
            entry["plane"],
            parse_offset(entry.get("offset_mm")),
            chord_tolerance_mm=float(entry["chord_tolerance_mm"]),
        )
        for entry in spec["sections"]
    ]
    return {
        "id": spec["id"],
        "views": [view_payload(view) for view in drawing.views],
        "sections": [section_payload(cut) for cut in sections],
        "default_section_offset": float(default_section_offset(shape)),
        "chord_tolerance_mm": float(drawing.chord_tolerance_mm),
        "max_points": int(drawing.max_points),
        "point_count": int(drawing.total_point_count),
    }


def run_refusal(spec: dict[str, Any]) -> dict[str, Any]:
    try:
        if spec.get("shape") == "null":
            shape = TopoDS_Shape()
        else:
            shape = load_step(Path(spec["shape_path"]))
        call = spec["call"]
        if call == "hlr_views":
            hlr_views(
                shape,
                views=list(spec["views"]),
                chord_tolerance_mm=parse_float(spec["chord_tolerance_mm"]),
                max_points=int(spec["max_points"]),
            )
        elif call == "section":
            section(
                shape,
                spec["plane"],
                parse_offset(spec.get("offset_mm")),
                chord_tolerance_mm=parse_float(spec["chord_tolerance_mm"]),
            )
        elif call == "default_section_offset":
            default_section_offset(shape)
        else:
            raise RuntimeError(f"unknown refusal call {call}")
    except Exception as exc:  # noqa: BLE001 - the class and message are the data
        return {
            "id": spec["id"],
            "error_type": type(exc).__name__,
            "message": str(exc),
        }
    return {"id": spec["id"], "error_type": None, "message": None}


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: drawing_oracle.py REQUEST.json RESULT.json", file=sys.stderr)
        return 2
    request_path = Path(sys.argv[1])
    result_path = Path(sys.argv[2])
    request = json.loads(request_path.read_text())
    result: dict[str, Any] = {
        "ok": False,
        "checked": {
            "shapes": 0,
            "views": 0,
            "sections": 0,
            "oracle_shapes": 0,
            "refusals": 0,
        },
        "shapes": [],
        "oracle_shapes": [],
        "refusals": [],
        "mismatches": [],
        "versions": {},
        "source_digests": {},
    }
    try:
        import OCP

        result["versions"] = {
            "ocp": getattr(OCP, "__version__", "unknown"),
            "python": sys.version.split()[0],
        }
        result["source_digests"] = {
            "drawing.py": sha256(
                ROOT / "src" / "kala_agent" / "studio" / "design" / "drawing.py"
            ),
            "test_drawing.py": sha256(
                ROOT / "tests" / "unit" / "studio_design" / "test_drawing.py"
            ),
        }

        for spec in request.get("shapes", []):
            shape = load_step(Path(spec["step_path"]))
            payload = draw_shape(shape, spec)
            result["shapes"].append(payload)
            result["checked"]["shapes"] += 1
            result["checked"]["views"] += len(payload["views"])
            result["checked"]["sections"] += len(payload["sections"])

        for spec in request.get("oracle_shapes", []):
            builder = BUILDERS[spec["builder"]]
            step_path = Path(spec["step_path"])
            write_step(builder(), step_path)
            # Both implementations draw the re-imported document, not the
            # native Python object, so the input is byte-for-byte the same.
            shape = load_step(step_path)
            payload = draw_shape(shape, spec)
            payload["step_path"] = str(step_path)
            result["oracle_shapes"].append(payload)
            result["checked"]["oracle_shapes"] += 1
            result["checked"]["views"] += len(payload["views"])
            result["checked"]["sections"] += len(payload["sections"])

        for spec in request.get("refusals", []):
            result["refusals"].append(run_refusal(spec))
            result["checked"]["refusals"] += 1

        result["ok"] = True
    except Exception:  # noqa: BLE001 - report the failure to the Rust test
        result["mismatches"].append(traceback.format_exc())
    result_path.write_text(json.dumps(result, indent=2))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
