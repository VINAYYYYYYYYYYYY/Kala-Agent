"""Test-only OpenCascade oracle for the STEP bodies the Studio HTTP routes emit.

This script is *not* part of the product runtime and is never executed by the
service. `tests/step_export_ocp.rs` writes the real HTTP response bodies of
`POST /api/studio/export` and `POST /api/studio/handoff` to disk and runs this
script under the repository virtualenv, so an independent CAD kernel
(`cadquery-ocp`, the same bindings the Python studio used) verifies:

* the transfer status and shape validity;
* exactly one closed shell with a correct orientation and no free edges;
* the analytic surface histogram of the primitive;
* volume, surface area and bounds in millimetres;
* the same primitives exported by the current Python studio exporter, so the
  native bytes can be compared numerically against the implementation being
  replaced (OpenCascade's own STEP bytes are not comparable byte-for-byte).

Usage:
    python step_ocp_oracle.py <request.json> <report.json>

The request carries the cases, the repository source root and a toggle for the
Python reference export.
"""

from __future__ import annotations

import json
import sys
import tempfile
import traceback
from pathlib import Path


def _surface_name(kind) -> str:
    names = {
        "GeomAbs_Plane": "plane",
        "GeomAbs_Cylinder": "cylinder",
        "GeomAbs_Sphere": "sphere",
        "GeomAbs_Cone": "cone",
        "GeomAbs_Torus": "torus",
        "GeomAbs_BezierSurface": "bezier",
        "GeomAbs_BSplineSurface": "bspline",
        "GeomAbs_SurfaceOfRevolution": "revolution",
        "GeomAbs_SurfaceOfExtrusion": "extrusion",
        "GeomAbs_OffsetSurface": "offset",
        "GeomAbs_OtherSurface": "other",
    }
    return names.get(kind.name, kind.name)


def _status_name(status) -> str:
    return getattr(status, "name", str(status))


def measure(path: Path) -> dict:
    """Read one STEP file and report its geometry metrics in millimetres."""
    from OCP.BRepAdaptor import BRepAdaptor_Surface
    from OCP.BRepBndLib import BRepBndLib
    from OCP.BRepCheck import BRepCheck_Analyzer, BRepCheck_Shell
    from OCP.BRepGProp import BRepGProp
    from OCP.Bnd import Bnd_Box
    from OCP.GProp import GProp_GProps
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.ShapeAnalysis import ShapeAnalysis_Shell
    from OCP.STEPControl import STEPControl_Reader
    from OCP.TopAbs import TopAbs_EDGE, TopAbs_FACE, TopAbs_SHELL, TopAbs_SOLID
    from OCP.TopExp import TopExp_Explorer
    from OCP.TopoDS import TopoDS

    reader = STEPControl_Reader()
    read_status = reader.ReadFile(str(path))
    if read_status != IFSelect_RetDone:
        return {"read_status": int(read_status), "transfer_status": None, "null_shape": True}
    transfer_status = int(reader.TransferRoots())
    shape = reader.OneShape()
    if shape.IsNull():
        return {
            "read_status": int(read_status),
            "transfer_status": transfer_status,
            "null_shape": True,
        }

    surface_types: dict[str, int] = {}
    faces = 0
    explorer = TopExp_Explorer(shape, TopAbs_FACE)
    while explorer.More():
        faces += 1
        surface = BRepAdaptor_Surface(TopoDS.Face(explorer.Current()))
        name = _surface_name(surface.GetType())
        surface_types[name] = surface_types.get(name, 0) + 1
        explorer.Next()

    solids = 0
    explorer = TopExp_Explorer(shape, TopAbs_SOLID)
    while explorer.More():
        solids += 1
        explorer.Next()

    shells = 0
    shell_closed = []
    shell_orientation = []
    free_edges = 0
    explorer = TopExp_Explorer(shape, TopAbs_SHELL)
    while explorer.More():
        shells += 1
        shell = TopoDS.Shell(explorer.Current())
        check = BRepCheck_Shell(shell)
        shell_closed.append(_status_name(check.Closed()))
        shell_orientation.append(_status_name(check.Orientation()))
        analysis = ShapeAnalysis_Shell()
        analysis.LoadShells(shell)
        free = TopExp_Explorer(analysis.FreeEdges(), TopAbs_EDGE)
        while free.More():
            free_edges += 1
            free.Next()
        explorer.Next()

    edges = 0
    explorer = TopExp_Explorer(shape, TopAbs_EDGE)
    while explorer.More():
        edges += 1
        explorer.Next()

    valid = bool(BRepCheck_Analyzer(shape).IsValid())

    volume_props = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, volume_props)
    area_props = GProp_GProps()
    BRepGProp.SurfaceProperties_s(shape, area_props)

    box = Bnd_Box()
    BRepBndLib.Add_s(shape, box)
    minimum = box.CornerMin()
    maximum = box.CornerMax()

    return {
        "read_status": int(read_status),
        "transfer_status": transfer_status,
        "null_shape": False,
        "valid": valid,
        "solids": solids,
        "shells": shells,
        "shell_closed": shell_closed,
        "shell_orientation": shell_orientation,
        "free_edges": free_edges,
        "faces": faces,
        "edges": edges,
        "surface_types": surface_types,
        "volume": volume_props.Mass(),
        "area": area_props.Mass(),
        "bounds_min": [minimum.X(), minimum.Y(), minimum.Z()],
        "bounds_max": [maximum.X(), maximum.Y(), maximum.Z()],
    }


def python_reference(case: dict, source_root: Path) -> dict:
    """Export the same primitive with the current Python studio exporter."""
    sys.path.insert(0, str(source_root / "src"))
    from kala_agent.studio.geometry.step import export_step_primitive

    parameters = case.get("parameters") or {}
    with tempfile.TemporaryDirectory(prefix="kala-studio-ref-") as reference_dir:
        data = export_step_primitive(
            case["kind"], dict(parameters), workdir=Path(reference_dir)
        )
        reference_path = Path(reference_dir) / "reference.step"
        reference_path.write_bytes(data)
        metrics = measure(reference_path)
    metrics["bytes"] = len(data)
    return metrics


def main() -> int:
    request_path = Path(sys.argv[1])
    report_path = Path(sys.argv[2])
    request = json.loads(request_path.read_text(encoding="utf-8"))
    source_root = Path(request["source_root"])
    want_reference = bool(request.get("python_reference", True))

    import importlib.metadata

    try:
        ocp_version = importlib.metadata.version("cadquery-ocp")
    except Exception:  # pragma: no cover - reported, never fatal
        ocp_version = "unknown"

    report = {
        "python_version": sys.version,
        "ocp_version": ocp_version,
        "source_root": str(source_root),
        "cases": [],
    }
    for case in request["cases"]:
        entry = {"id": case["id"], "kind": case["kind"]}
        try:
            entry.update(measure(Path(case["step_path"])))
            if want_reference:
                entry["reference"] = python_reference(case, source_root)
        except Exception as error:  # pragma: no cover - reported to the Rust test
            entry["oracle_error"] = f"{type(error).__name__}: {error}"
            entry["traceback"] = traceback.format_exc()
        report["cases"].append(entry)
    report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
