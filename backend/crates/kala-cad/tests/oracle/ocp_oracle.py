#!/usr/bin/env python3
"""Independent OpenCascade oracle for the native `kala-cad` crate.

This script is test-only. It imports the actual Python Studio compiler,
measurement and mesh modules from `src/`, compiles the same frozen fixture and
boundary scripts, measures them with OCP, reads the native STEP/STL files with
OpenCascade, and compares the results with the native Rust outputs.

The oracle is mandatory in the native test gate: if this interpreter lacks
`OCP`, or the repository `.venv` is missing, the Rust test fails instead of
skipping. It never participates in a product build or runtime.

Usage: ocp_oracle.py REQUEST.json RESULT.json
"""

from __future__ import annotations

import hashlib
import io
import json
import math
import struct
import sys
import traceback
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[5]

sys.path.insert(0, str(ROOT / "src"))

from kala_agent.contracts.construction import (  # noqa: E402
    ConstructionError,
    load_analysis,
    load_construction,
    load_reading,
)
from kala_agent.studio.design.compile_ocp import compile_script  # noqa: E402
from kala_agent.studio.design.importer import (  # noqa: E402
    StepImportError,
    import_step_bytes,
)
from kala_agent.studio.design.measure import measure_shape  # noqa: E402
from kala_agent.studio.design.mesh import tessellate  # noqa: E402

sys.path.insert(0, str(ROOT / "backend" / "compat"))
from corpuslib.common import normalise_step_bytes  # noqa: E402

from OCP.BRepCheck import BRepCheck_Analyzer  # noqa: E402
from OCP.IFSelect import IFSelect_RetDone  # noqa: E402
from OCP.Interface import Interface_Static  # noqa: E402
from OCP.STEPControl import STEPControl_Reader  # noqa: E402
from OCP.StlAPI import StlAPI_Reader  # noqa: E402
from OCP.TopAbs import (  # noqa: E402
    TopAbs_EDGE,
    TopAbs_FACE,
    TopAbs_SHELL,
    TopAbs_SOLID,
    TopAbs_VERTEX,
)
from OCP.TopExp import TopExp, TopExp_Explorer  # noqa: E402
from OCP.TopoDS import TopoDS_Shape  # noqa: E402
from OCP.collections import (  # noqa: E402
    IndexedMap_TopoDS_Shape_TopTools_ShapeMapHasher,
)

VOLUME_RELATIVE = 1e-6
BOUNDS_ABSOLUTE_MM = 0.001
MESH_BOUNDS_ABSOLUTE_MM = 0.11
STEP_VOLUME_RELATIVE = 1e-6
STEP_BOUNDS_ABSOLUTE_MM = 0.001


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def measurement_payload(shape: Any) -> dict[str, Any]:
    measured = measure_shape(shape)
    return {
        "volume": measured.volume_mm3,
        "area": measured.surface_area_mm2,
        "centroid": list(measured.centroid_mm),
        "min": list(measured.bounds.min_mm),
        "max": list(measured.bounds.max_mm),
        "valid": measured.valid,
        "solid_count": measured.solid_count,
    }


def topology_payload(shape: Any) -> dict[str, int]:
    # Unique sub-shape counts (not occurrence counts), matching the native
    # TopTools_IndexedMapOfShape measurement. `TopExp_Explorer` would count a
    # shared edge once per adjacent face and would not describe the B-Rep.
    payload: dict[str, int] = {}
    for name, kind in (
        ("solids", TopAbs_SOLID),
        ("shells", TopAbs_SHELL),
        ("faces", TopAbs_FACE),
        ("edges", TopAbs_EDGE),
        ("vertices", TopAbs_VERTEX),
    ):
        mapping = IndexedMap_TopoDS_Shape_TopTools_ShapeMapHasher()
        TopExp.MapShapes_s(shape, kind, mapping)
        payload[name] = mapping.Extent()
    return payload


def mesh_payload(shape: Any) -> dict[str, Any]:
    mesh = tessellate(shape)
    positions = mesh.positions
    if positions:
        minimum = [min(positions[axis::3]) for axis in range(3)]
        maximum = [max(positions[axis::3]) for axis in range(3)]
    else:
        minimum = [math.inf, math.inf, math.inf]
        maximum = [-math.inf, -math.inf, -math.inf]
    return {
        "vertex_count": mesh.vertex_count,
        "triangle_count": mesh.triangle_count,
        "min": minimum,
        "max": maximum,
        "all_finite": all(
            math.isfinite(value) for value in (*mesh.positions, *mesh.normals)
        ),
        "deflection_mm": mesh.deflection_mm,
        "angular_deflection_rad": mesh.angular_deflection_rad,
    }


def close_scalar(
    mismatches: list[dict[str, Any]],
    label: str,
    field: str,
    native: float,
    python: float,
    *,
    relative: float,
    absolute: float,
) -> None:
    if not (math.isfinite(native) and math.isfinite(python)):
        mismatches.append(
            {"case": label, "field": field, "native": native, "python": python,
             "reason": "non-finite"}
        )
        return
    difference = abs(native - python)
    if difference > absolute + relative * abs(python):
        mismatches.append(
            {
                "case": label,
                "field": field,
                "native": native,
                "python": python,
                "difference": difference,
                "tolerance": absolute + relative * abs(python),
            }
        )


def close_vector(
    mismatches: list[dict[str, Any]],
    label: str,
    field: str,
    native: list[float],
    python: list[float],
    *,
    absolute: float,
) -> None:
    if len(native) != len(python):
        mismatches.append(
            {"case": label, "field": field, "native": native, "python": python,
             "reason": "length"}
        )
        return
    for index, (left, right) in enumerate(zip(native, python, strict=True)):
        close_scalar(
            mismatches,
            label,
            f"{field}[{index}]",
            float(left),
            float(right),
            relative=0.0,
            absolute=absolute,
        )


def compare_measurement(
    mismatches: list[dict[str, Any]],
    label: str,
    native: dict[str, Any],
    python: dict[str, Any],
) -> None:
    close_scalar(
        mismatches, label, "volume", native["volume"], python["volume"],
        relative=VOLUME_RELATIVE, absolute=1e-9,
    )
    close_scalar(
        mismatches, label, "area", native["area"], python["area"],
        relative=VOLUME_RELATIVE, absolute=1e-9,
    )
    close_vector(
        mismatches, label, "centroid", native["centroid"], python["centroid"],
        absolute=0.001,
    )
    close_vector(
        mismatches, label, "min", native["min"], python["min"],
        absolute=BOUNDS_ABSOLUTE_MM,
    )
    close_vector(
        mismatches, label, "max", native["max"], python["max"],
        absolute=BOUNDS_ABSOLUTE_MM,
    )
    if bool(native["valid"]) != bool(python["valid"]):
        mismatches.append(
            {"case": label, "field": "valid", "native": native["valid"],
             "python": python["valid"]}
        )
    if int(native["solid_count"]) != int(python["solid_count"]):
        mismatches.append(
            {"case": label, "field": "solid_count",
             "native": native["solid_count"], "python": python["solid_count"]}
        )


def compare_topology(
    mismatches: list[dict[str, Any]],
    label: str,
    native: dict[str, Any],
    python: dict[str, Any],
) -> None:
    for name in ("solids", "shells", "faces", "edges", "vertices"):
        if int(native[name]) != int(python[name]):
            mismatches.append(
                {"case": label, "field": f"topology.{name}",
                 "native": native[name], "python": python[name]}
            )


def read_step(path: Path) -> Any:
    Interface_Static.SetCVal_s("xstep.cascade.unit", "MM")
    reader = STEPControl_Reader()
    status = reader.ReadFile(str(path))
    if status != IFSelect_RetDone:
        raise RuntimeError(f"STEP read status {status}")
    reader.TransferRoots()
    if reader.NbShapes() <= 0:
        raise RuntimeError("STEP reader imported no shapes")
    shape = reader.OneShape()
    if shape.IsNull():
        raise RuntimeError("STEP reader returned a null shape")
    return shape


def read_stl(path: Path) -> Any:
    data = path.read_bytes()
    header_count = struct.unpack("<I", data[80:84])[0]
    reader = StlAPI_Reader()
    shape = TopoDS_Shape()
    if not reader.Read(shape, io.BytesIO(data)):
        raise RuntimeError("STL read failed")
    if shape.IsNull():
        raise RuntimeError("STL reader returned a null shape")
    return shape, header_count


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: ocp_oracle.py REQUEST.json RESULT.json", file=sys.stderr)
        return 2
    request_path = Path(sys.argv[1])
    result_path = Path(sys.argv[2])
    request = json.loads(request_path.read_text(encoding="utf-8"))
    mismatches: list[dict[str, Any]] = []
    checked = {
        "cases": 0,
        "parts": 0,
        "refusals": 0,
        "meshes": 0,
        "steps": 0,
        "stls": 0,
        "imports": 0,
        "import_failures": 0,
        "normalisations": 0,
    }

    try:
        for case in request["cases"]:
            case_id = case["id"]
            native = case["native"]
            script = load_construction(case["script"])
            result = compile_script(script)
            checked["cases"] += 1

            if list(result.warnings) != list(native["warnings"]):
                mismatches.append(
                    {"case": case_id, "field": "warnings",
                     "native": native["warnings"], "python": list(result.warnings)}
                )
            if bool(result.fused) != bool(native["fused"]):
                mismatches.append(
                    {"case": case_id, "field": "fused",
                     "native": native["fused"], "python": bool(result.fused)}
                )
            python_parts = {
                component.name: component for component in result.components
            }
            if list(python_parts) != [part["name"] for part in native["parts"]]:
                mismatches.append(
                    {"case": case_id, "field": "component_names",
                     "native": [part["name"] for part in native["parts"]],
                     "python": list(python_parts)}
                )
            for part in native["parts"]:
                name = part["name"]
                if name not in python_parts:
                    continue
                checked["parts"] += 1
                compare_measurement(
                    mismatches,
                    f"{case_id}/{name}",
                    part["measurement"],
                    measurement_payload(python_parts[name].local_shape),
                )
                compare_topology(
                    mismatches,
                    f"{case_id}/{name}",
                    part["topology"],
                    topology_payload(python_parts[name].local_shape),
                )
            compare_measurement(
                mismatches,
                f"{case_id}/assembly",
                native["assembly"],
                measurement_payload(result.assembly_shape),
            )
            compare_topology(
                mismatches,
                f"{case_id}/assembly",
                native["assembly_topology"],
                topology_payload(result.assembly_shape),
            )

        for refusal in request["refusals"]:
            checked["refusals"] += 1
            analysis = (
                load_analysis(refusal["analysis"])
                if refusal["analysis"] is not None
                else None
            )
            edits: tuple[Any, ...] = ()
            if refusal["edits"]:
                reading = load_reading(
                    {
                        "verdict": "apply the edit",
                        "tool_edit": refusal["edits"],
                        "rewrite": None,
                    }
                )
                edits = reading.tool_edit
            fillets = refusal["fillets"] or None
            try:
                compile_script(
                    load_construction(refusal["script"]),
                    analysis=analysis,
                    edits=edits,
                    fillets=fillets,
                )
                message = "NO ERROR"
            except ConstructionError as error:
                message = str(error)
            except Exception as error:  # noqa: BLE001
                message = f"{type(error).__name__}: {error}"
            if message != refusal["native_error"]:
                mismatches.append(
                    {"case": refusal["id"], "field": "refusal_message",
                     "native": refusal["native_error"], "python": message}
                )

        for mesh_case in request["meshes"]:
            checked["meshes"] += 1
            result = compile_script(load_construction(mesh_case["script"]))
            python = mesh_payload(result.assembly_shape)
            native = mesh_case["native"]
            for name in ("vertex_count", "triangle_count"):
                if int(native[name]) != int(python[name]):
                    mismatches.append(
                        {"case": mesh_case["id"], "field": f"mesh.{name}",
                         "native": native[name], "python": python[name]}
                    )
            close_vector(
                mismatches,
                mesh_case["id"],
                "mesh.min",
                native["min"],
                python["min"],
                absolute=MESH_BOUNDS_ABSOLUTE_MM,
            )
            close_vector(
                mismatches,
                mesh_case["id"],
                "mesh.max",
                native["max"],
                python["max"],
                absolute=MESH_BOUNDS_ABSOLUTE_MM,
            )
            if bool(native["all_finite"]) != bool(python["all_finite"]):
                mismatches.append(
                    {"case": mesh_case["id"], "field": "mesh.all_finite",
                     "native": native["all_finite"],
                     "python": python["all_finite"]}
                )

        for step_case in request["steps"]:
            checked["steps"] += 1
            try:
                shape = read_step(Path(step_case["path"]))
                python = measurement_payload(shape)
                python_topology = topology_payload(shape)
            except Exception as error:  # noqa: BLE001
                mismatches.append(
                    {"case": step_case["id"], "field": "step.read",
                     "python": f"{type(error).__name__}: {error}"}
                )
                continue
            native = step_case["native_measurement"]
            close_scalar(
                mismatches, step_case["id"], "step.volume",
                native["volume"], python["volume"],
                relative=STEP_VOLUME_RELATIVE, absolute=1e-9,
            )
            close_vector(
                mismatches, step_case["id"], "step.min",
                native["min"], python["min"],
                absolute=STEP_BOUNDS_ABSOLUTE_MM,
            )
            close_vector(
                mismatches, step_case["id"], "step.max",
                native["max"], python["max"],
                absolute=STEP_BOUNDS_ABSOLUTE_MM,
            )
            if int(native["solid_count"]) != int(python["solid_count"]):
                mismatches.append(
                    {"case": step_case["id"], "field": "step.solid_count",
                     "native": native["solid_count"],
                     "python": python["solid_count"]}
                )
            if not python["valid"]:
                mismatches.append(
                    {"case": step_case["id"], "field": "step.valid",
                     "python": python["valid"]}
                )
            if bool(BRepCheck_Analyzer(shape).IsValid()) != bool(python["valid"]):
                mismatches.append(
                    {"case": step_case["id"], "field": "step.analyzer",
                     "python": python["valid"]}
                )
            compare_topology(
                mismatches,
                step_case["id"],
                step_case["native_topology"],
                python_topology,
            )

        for stl_case in request["stls"]:
            checked["stls"] += 1
            try:
                shape, header_count = read_stl(Path(stl_case["path"]))
                python_topology = topology_payload(shape)
                python = measurement_payload(shape)
            except Exception as error:  # noqa: BLE001
                mismatches.append(
                    {"case": stl_case["id"], "field": "stl.read",
                     "python": f"{type(error).__name__}: {error}"}
                )
                continue
            native = stl_case["native_mesh"]
            if header_count != native["triangle_count"]:
                mismatches.append(
                    {"case": stl_case["id"], "field": "stl.header_count",
                     "native": native["triangle_count"], "python": header_count}
                )
            if python_topology["faces"] != native["triangle_count"]:
                mismatches.append(
                    {"case": stl_case["id"], "field": "stl.face_count",
                     "native": native["triangle_count"],
                     "python": python_topology["faces"]}
                )
            close_vector(
                mismatches, stl_case["id"], "stl.min",
                native["min"], python["min"],
                absolute=MESH_BOUNDS_ABSOLUTE_MM,
            )
            close_vector(
                mismatches, stl_case["id"], "stl.max",
                native["max"], python["max"],
                absolute=MESH_BOUNDS_ABSOLUTE_MM,
            )
            if python_topology["faces"] <= 0:
                mismatches.append(
                    {"case": stl_case["id"], "field": "stl.empty",
                     "python": python_topology["faces"]}
                )

        for import_case in request.get("imports", []):
            checked["imports"] += 1
            data = Path(import_case["path"]).read_bytes()
            outcome = import_step_bytes(
                data,
                store_artifact=False,
                max_vertices=import_case["max_vertices"],
                max_triangles=import_case["max_triangles"],
            )
            native = import_case["native"]
            close_vector(
                mismatches, import_case["id"], "import.min",
                native["min"], list(outcome.bounds.min_mm),
                absolute=BOUNDS_ABSOLUTE_MM,
            )
            close_vector(
                mismatches, import_case["id"], "import.max",
                native["max"], list(outcome.bounds.max_mm),
                absolute=BOUNDS_ABSOLUTE_MM,
            )
            if int(native["solids"]) != int(outcome.solids):
                mismatches.append(
                    {"case": import_case["id"], "field": "import.solids",
                     "native": native["solids"], "python": int(outcome.solids)}
                )
            if list(native["warnings"]) != list(outcome.warnings):
                mismatches.append(
                    {"case": import_case["id"], "field": "import.warnings",
                     "native": native["warnings"], "python": list(outcome.warnings)}
                )
            if int(native["vertex_count"]) != int(outcome.mesh.vertex_count):
                mismatches.append(
                    {"case": import_case["id"], "field": "import.vertex_count",
                     "native": native["vertex_count"],
                     "python": int(outcome.mesh.vertex_count)}
                )
            if int(native["triangle_count"]) != int(outcome.mesh.triangle_count):
                mismatches.append(
                    {"case": import_case["id"], "field": "import.triangle_count",
                     "native": native["triangle_count"],
                     "python": int(outcome.mesh.triangle_count)}
                )
            if bool(native["coarsened"]) != bool(outcome.warnings and
                                                 "retried once with a coarser" in outcome.warnings[-1]):
                mismatches.append(
                    {"case": import_case["id"], "field": "import.coarsened",
                     "native": native["coarsened"], "python": list(outcome.warnings)}
                )

        for failure in request.get("import_failures", []):
            checked["import_failures"] += 1
            data = Path(failure["path"]).read_bytes()
            try:
                import_step_bytes(
                    data,
                    store_artifact=False,
                    max_step_bytes=failure["max_step_bytes"],
                )
                observed = {"code": "NO ERROR", "message": "NO ERROR"}
            except StepImportError as error:
                observed = {"code": error.code, "message": str(error)}
            if observed != failure["native_error"]:
                mismatches.append(
                    {"case": failure["id"], "field": "import.error",
                     "native": failure["native_error"], "python": observed}
                )

        for normalisation in request.get("normalisations", []):
            checked["normalisations"] += 1
            raw = Path(normalisation["raw_path"]).read_bytes()
            expected = Path(normalisation["native_path"]).read_bytes()
            actual = normalise_step_bytes(raw)
            if actual != expected:
                mismatches.append(
                    {"case": normalisation["id"],
                     "field": "normalisation.bytes",
                     "python_length": len(actual),
                     "native_length": len(expected)}
                )

    except Exception as error:  # noqa: BLE001
        mismatches.append(
            {
                "case": "oracle",
                "field": "exception",
                "python": f"{type(error).__name__}: {error}",
                "traceback": traceback.format_exc(),
            }
        )

    import OCP

    result = {
        "ok": not mismatches,
        "mismatches": mismatches,
        "checked": checked,
        "versions": {
            "python": sys.version,
            "ocp": getattr(OCP, "__version__", "unknown"),
            "executable": sys.executable,
        },
        "source_digests": {
            "compile_ocp.py": sha256(ROOT / "src/kala_agent/studio/design/compile_ocp.py"),
            "measure.py": sha256(ROOT / "src/kala_agent/studio/design/measure.py"),
            "mesh.py": sha256(ROOT / "src/kala_agent/studio/design/mesh.py"),
            "importer.py": sha256(ROOT / "src/kala_agent/studio/design/importer.py"),
            "corpus_common.py": sha256(ROOT / "backend/compat/corpuslib/common.py"),
            "construction.py": sha256(ROOT / "src/kala_agent/contracts/construction.py"),
            "fixture": sha256(ROOT / "tests/fixtures/construction_parity.json"),
        },
    }
    result_path.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps({"ok": result["ok"], "mismatches": len(mismatches)}))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
