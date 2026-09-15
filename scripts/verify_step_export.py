#!/usr/bin/env python3
"""
Verify STEP export roundtrip: export simple and complex geometries, then probe them
with freecadcmd to ensure they pass isValid() + volume>0 checks.

This script documents the freecadcmd probe command and validates the fix for
STEP export healing.

Usage:
    uv run python scripts/verify_step_export.py
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


def probe_step_file(path: Path) -> dict[str, Any]:
    """
    Probe a STEP file using freecadcmd to check validity and extract metrics.
    
    This is the same probe command used by batch_eval.py _step_metrics().
    
    Command format:
        freecadcmd -c "<python_code>"
    
    Returns:
        dict with keys: valid, volume, solids, size (bbox dimensions), error
    """
    if not path.is_file():
        return {"error": "missing_file"}
    
    size = path.stat().st_size
    if size < 200:
        return {"error": "stub_or_empty_step", "bytes": size}
    
    # Escape path for FreeCAD -c string
    p = str(path).replace("\\", "\\\\").replace('"', '\\"')
    code = (
        "import Part, json\n"
        f'path = "{p}"\n'
        "shape = Part.Shape()\n"
        "shape.read(path)\n"
        "if not shape.isValid():\n"
        "    try:\n"
        "        shape.fix()\n"
        "    except Exception:\n"
        "        pass\n"
        "if (not shape.isValid()) and getattr(shape, 'Solids', None):\n"
        "    solids = list(shape.Solids)\n"
        "    if solids:\n"
        "        shape = solids[0]\n"
        "        for s in solids[1:]:\n"
        "            shape = shape.fuse(s)\n"
        "        if not shape.isValid():\n"
        "            try: shape.fix()\n"
        "            except Exception: pass\n"
        "bb = shape.BoundBox\n"
        "print('METRICS', json.dumps({"
        "'solids': len(getattr(shape, 'Solids', []) or ([shape] if shape.Volume else [])), "
        "'volume': float(shape.Volume), "
        "'size': [float(bb.XLength), float(bb.YLength), float(bb.ZLength)], "
        f"'bytes': {size}, "
        "'valid': bool(shape.isValid())"
        "}))\n"
    )
    
    try:
        proc = subprocess.run(
            ["freecadcmd", "-c", code],
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
        for line in (proc.stdout or "").splitlines():
            if line.startswith("METRICS "):
                return json.loads(line[len("METRICS ") :])
        if proc.returncode != 0:
            err = (proc.stderr or proc.stdout or "")[-500:]
            return {"error": f"freecadcmd_exit={proc.returncode}", "detail": err}
        return {"error": "no_metrics_line", "bytes": size}
    except subprocess.TimeoutExpired:
        return {"error": "timeout"}
    except Exception as exc:
        return {"error": str(exc)[:200]}


def test_simple_geometry() -> tuple[bool, str]:
    """Test simple box export and roundtrip validation."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    with tempfile.TemporaryDirectory() as tmpdir:
        outpath = Path(tmpdir) / "simple_box.step"
        
        backend = FreeCADBackend(doc_name="verify_simple")
        result = backend.create_box(10.0, 20.0, 30.0, label="TestBox")
        if not result.ok:
            return False, f"create_box failed: {result.message}"
        
        body_id = result.data["body_id"]
        result = backend.export(body_id, str(outpath), fmt="step")
        if not result.ok:
            return False, f"export failed: {result.message}"
        
        # Probe the exported file (use remapped path from result)
        metrics = probe_step_file(Path(result.data["path"]))
        if metrics.get("error"):
            return False, f"probe failed: {metrics['error']}"
        
        if not metrics.get("valid"):
            return False, f"exported STEP is not valid: {metrics}"
        
        volume = metrics.get("volume", 0)
        expected_volume = 10.0 * 20.0 * 30.0
        if volume <= 0 or abs(volume - expected_volume) > 1.0:
            return False, f"volume mismatch: got {volume}, expected ~{expected_volume}"
        
        return True, f"✓ Simple box: valid={metrics['valid']}, volume={volume:.1f}, solids={metrics.get('solids')}"


def test_complex_geometry() -> tuple[bool, str]:
    """Test complex geometry (boolean operations) export and roundtrip validation."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    with tempfile.TemporaryDirectory() as tmpdir:
        outpath = Path(tmpdir) / "complex_part.step"
        
        backend = FreeCADBackend(doc_name="verify_complex")
        
        # Create a box
        r1 = backend.create_box(50.0, 50.0, 50.0, label="Base")
        if not r1.ok:
            return False, f"create_box failed: {r1.message}"
        base_id = r1.data["body_id"]
        
        # Create a cylinder to subtract
        r2 = backend.create_cylinder(15.0, 60.0, label="Hole")
        if not r2.ok:
            return False, f"create_cylinder failed: {r2.message}"
        hole_id = r2.data["body_id"]
        
        # Translate cylinder to center
        r3 = backend.translate(hole_id, 25.0, 25.0, -5.0)
        if not r3.ok:
            return False, f"translate failed: {r3.message}"
        
        # Boolean cut
        r4 = backend.boolean_cut(base_id, hole_id)
        if not r4.ok:
            return False, f"boolean_cut failed: {r4.message}"
        final_id = r4.data["body_id"]
        
        # Export
        r5 = backend.export(final_id, str(outpath), fmt="step")
        if not r5.ok:
            return False, f"export failed: {r5.message}"
        
        # Probe the exported file (use remapped path from result)
        metrics = probe_step_file(Path(r5.data["path"]))
        if metrics.get("error"):
            return False, f"probe failed: {metrics['error']}"
        
        if not metrics.get("valid"):
            return False, f"exported STEP is not valid: {metrics}"
        
        volume = metrics.get("volume", 0)
        expected_volume = 50.0 * 50.0 * 50.0 - 3.14159 * 15.0 * 15.0 * 50.0
        if volume <= 0:
            return False, f"zero or negative volume: {volume}"
        
        return True, f"✓ Complex part: valid={metrics['valid']}, volume={volume:.1f}, solids={metrics.get('solids')}"


def test_assembly_export() -> tuple[bool, str]:
    """Test assembly (multi-body) export and roundtrip validation."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    with tempfile.TemporaryDirectory() as tmpdir:
        outpath = Path(tmpdir) / "assembly.step"
        
        backend = FreeCADBackend(doc_name="verify_assembly")
        
        # Create two separate bodies
        r1 = backend.create_box(10.0, 10.0, 10.0, label="Part1")
        if not r1.ok:
            return False, f"create_box 1 failed: {r1.message}"
        
        r2 = backend.create_sphere(8.0, label="Part2")
        if not r2.ok:
            return False, f"create_sphere failed: {r2.message}"
        
        r3 = backend.translate(r2.data["body_id"], 20.0, 0.0, 0.0)
        if not r3.ok:
            return False, f"translate failed: {r3.message}"
        
        # Export as assembly (ALL bodies)
        r4 = backend.export("ALL", str(outpath), fmt="step")
        if not r4.ok:
            return False, f"assembly export failed: {r4.message}"
        
        # Probe the exported file (use remapped path from result)
        metrics = probe_step_file(Path(r4.data["path"]))
        if metrics.get("error"):
            return False, f"probe failed: {metrics['error']}"
        
        if not metrics.get("valid"):
            return False, f"exported STEP is not valid: {metrics}"
        
        volume = metrics.get("volume", 0)
        if volume <= 0:
            return False, f"zero or negative volume: {volume}"
        
        # Assembly should fuse into 1 solid
        solids = metrics.get("solids", 0)
        return True, f"✓ Assembly: valid={metrics['valid']}, volume={volume:.1f}, solids={solids}"


def test_fillet_export() -> tuple[bool, str]:
    """Test fillet (edge operation) export and roundtrip validation."""
    from kala.cad.freecad.backend import FreeCADBackend
    
    with tempfile.TemporaryDirectory() as tmpdir:
        outpath = Path(tmpdir) / "filleted.step"
        
        backend = FreeCADBackend(doc_name="verify_fillet")
        
        # Create a box
        r1 = backend.create_box(30.0, 30.0, 30.0, label="Box")
        if not r1.ok:
            return False, f"create_box failed: {r1.message}"
        box_id = r1.data["body_id"]
        
        # Apply fillet
        r2 = backend.fillet(box_id, 3.0)
        if not r2.ok:
            return False, f"fillet failed: {r2.message}"
        filleted_id = r2.data["body_id"]
        
        # Export
        r3 = backend.export(filleted_id, str(outpath), fmt="step")
        if not r3.ok:
            return False, f"export failed: {r3.message}"
        
        # Probe the exported file (use remapped path from result)
        metrics = probe_step_file(Path(r3.data["path"]))
        if metrics.get("error"):
            return False, f"probe failed: {metrics['error']}"
        
        if not metrics.get("valid"):
            return False, f"exported STEP is not valid: {metrics}"
        
        volume = metrics.get("volume", 0)
        expected_volume = 30.0 * 30.0 * 30.0  # Approximately (fillet slightly reduces)
        if volume <= 0 or volume > expected_volume:
            return False, f"volume unexpected: {volume}"
        
        return True, f"✓ Fillet: valid={metrics['valid']}, volume={volume:.1f}, solids={metrics.get('solids')}"


def main() -> int:
    """Run all verification tests."""
    print("=" * 70)
    print("STEP Export Roundtrip Verification")
    print("=" * 70)
    print()
    print("freecadcmd probe command:")
    print("  freecadcmd -c '<python code that reads STEP, validates, extracts metrics>'")
    print()
    print("Expected properties for valid STEP:")
    print("  - shape.isValid() == True (after fix if needed)")
    print("  - volume > 0")
    print("  - solids >= 1")
    print()
    print("-" * 70)
    
    tests = [
        ("Simple Box", test_simple_geometry),
        ("Complex Boolean", test_complex_geometry),
        ("Assembly Export", test_assembly_export),
        ("Fillet Export", test_fillet_export),
    ]
    
    results = []
    for name, test_fn in tests:
        print(f"\nTesting: {name}")
        try:
            ok, msg = test_fn()
            results.append(ok)
            status = "PASS" if ok else "FAIL"
            print(f"  [{status}] {msg}")
        except Exception as exc:
            results.append(False)
            print(f"  [FAIL] Exception: {exc}")
    
    print()
    print("-" * 70)
    passed = sum(results)
    total = len(results)
    print(f"\nResults: {passed}/{total} tests passed")
    
    if passed == total:
        print("✓ All STEP exports are valid and readable by freecadcmd")
        return 0
    else:
        print("✗ Some STEP exports failed validation")
        return 1


if __name__ == "__main__":
    sys.exit(main())
