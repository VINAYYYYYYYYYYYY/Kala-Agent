#!/usr/bin/env python3
"""Test-only Python oracle for the native Rust geometry crate.

This script is invoked by `tests/python_differential.rs`. It is never used by
the Rust library or a production service. It:

1. Re-renders Rust-emitted float bit patterns with CPython `json.dumps` and
   reports SHA-256 digests, so fixture compact-JSON digests are checked
   against Python's own formatter rather than only a Rust reimplementation.
2. Compares CPython `repr(float)` output for random finite bit patterns.
3. Builds requested meshes with the live Python product source and reports
   exact float bit patterns, indices, STL digest/length, bounds, and volume.
4. Builds invalid requests and reports the actual exception type/message.
"""

from __future__ import annotations

import hashlib
import json
import struct
import sys
from pathlib import Path


def bits_to_float(text: str) -> float:
    return struct.unpack("<d", int(text, 16).to_bytes(8, "little"))[0]


def float_to_bits(value: float) -> str:
    return format(struct.unpack("<Q", struct.pack("<d", value))[0], "016x")


def array_digest(bits: list[str]) -> str:
    values = [bits_to_float(bit) for bit in bits]
    text = json.dumps(
        values,
        allow_nan=False,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    )
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: geometry_oracle.py <input.json> <output.json>", file=sys.stderr)
        return 2
    input_path = Path(sys.argv[1])
    output_path = Path(sys.argv[2])
    request = json.loads(input_path.read_text(encoding="utf-8"))

    from kala_agent.studio.geometry import (
        build_primitive,
        compute_bounds,
        signed_volume,
        write_binary_stl,
    )

    response: dict[str, object] = {}

    canonical = request.get("canonical", {})
    canonical_response: dict[str, object] = {"arrays": {}, "float_mismatches": []}
    for record in canonical.get("arrays", []):
        canonical_response["arrays"][record["id"]] = array_digest(record["bits"])
    for record in canonical.get("floats", []):
        actual = repr(bits_to_float(record["bits"]))
        if actual != record["repr"]:
            canonical_response["float_mismatches"].append(
                {"bits": record["bits"], "expected": record["repr"], "actual": actual}
            )
    response["canonical"] = canonical_response

    mesh_response: dict[str, object] = {}
    for case in request.get("meshes", []):
        mesh = build_primitive(case["kind"], case["parameters"])
        bounds = compute_bounds(mesh.positions)
        stl = write_binary_stl(mesh)
        mesh_response[case["id"]] = {
            "positions": [float_to_bits(value) for value in mesh.positions],
            "normals": [float_to_bits(value) for value in mesh.normals],
            "indices": list(mesh.indices),
            "stl_sha256": hashlib.sha256(stl).hexdigest(),
            "stl_length": len(stl),
            "bounds": {
                "min": [
                    float_to_bits(bounds.min_x),
                    float_to_bits(bounds.min_y),
                    float_to_bits(bounds.min_z),
                ],
                "max": [
                    float_to_bits(bounds.max_x),
                    float_to_bits(bounds.max_y),
                    float_to_bits(bounds.max_z),
                ],
            },
            "signed_volume": float_to_bits(signed_volume(mesh)),
        }
    response["meshes"] = mesh_response

    invalid_response: dict[str, object] = {}
    for case in request.get("invalid", []):
        try:
            build_primitive(case["kind"], case["parameters"])
        except Exception as error:  # noqa: BLE001 - oracle captures the real type
            invalid_response[case["id"]] = {
                "type": type(error).__name__,
                "message": str(error),
            }
        else:
            invalid_response[case["id"]] = {"type": None, "message": None}
    response["invalid"] = invalid_response

    output_path.write_text(json.dumps(response, sort_keys=True), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
