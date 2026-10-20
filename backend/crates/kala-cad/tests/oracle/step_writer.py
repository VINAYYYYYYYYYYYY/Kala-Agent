#!/usr/bin/env python3
"""Test-only Python STEP writer for the native `kala-cad` import gate.

Compiles one ConstructionScript with the real Python reference compiler
(`compile_ocp.py`), measures it with `measure.py`, and writes the same STEP
export the opt-in OpenCascade provider produces. The native test imports those
bytes and compares its view bounds with the Python measurement. This script is
never part of the product build or runtime.

Usage: step_writer.py REQUEST.json RESULT.json
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(ROOT / "src"))

from kala_agent.contracts.construction import load_construction  # noqa: E402
from kala_agent.studio.design.compile_ocp import compile_script  # noqa: E402
from kala_agent.studio.design.measure import measure_shape  # noqa: E402

from OCP.IFSelect import IFSelect_RetDone  # noqa: E402
from OCP.STEPControl import (  # noqa: E402
    STEPControl_AsIs,
    STEPControl_Writer,
)


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: step_writer.py REQUEST.json RESULT.json", file=sys.stderr)
        return 2
    request = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    result_path = Path(sys.argv[2])
    step_path = Path(request["step_path"])
    payload: dict[str, Any]
    try:
        script = load_construction(request["script"])
        compiled = compile_script(script)
        measured = measure_shape(compiled.assembly_shape)
        writer = STEPControl_Writer()
        transfer = writer.Transfer(compiled.assembly_shape, STEPControl_AsIs)
        if transfer != IFSelect_RetDone:
            raise RuntimeError("the Python STEP writer refused the compiled shape")
        write = writer.Write(str(step_path))
        if write != IFSelect_RetDone:
            raise RuntimeError("the Python STEP writer did not finish serialising")
        data = step_path.read_bytes()
        payload = {
            "ok": True,
            "step_size": len(data),
            "measurement": {
                "volume": measured.volume_mm3,
                "area": measured.surface_area_mm2,
                "centroid": list(measured.centroid_mm),
                "min": list(measured.bounds.min_mm),
                "max": list(measured.bounds.max_mm),
                "valid": measured.valid,
                "solid_count": measured.solid_count,
            },
        }
    except Exception as error:  # noqa: BLE001
        payload = {"ok": False, "error": f"{type(error).__name__}: {error}"}
    result_path.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    print(json.dumps({"ok": payload["ok"]}))
    return 0 if payload["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
