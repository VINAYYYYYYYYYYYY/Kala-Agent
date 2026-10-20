"""Geometry and STL oracle cases.

Meshes are built by ``kala_agent.studio.geometry`` exactly as the studio
builds them. Default STL files are stored byte-for-byte because they are
small. Maximum-tessellation STL files are represented by length and SHA-256
only; the self-check regenerates the mesh and recomputes the digest rather
than checking megabytes into the repository.
"""

from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path
from typing import Final

from corpuslib.common import (
    normalise_step_bytes,
    sha256_bytes,
    sha256_file,
    silenced_fds,
    step_normalisation_facts,
)

STL_STORE_IDS: Final[frozenset[str]] = frozenset(
    {"box_default", "cylinder_default", "sphere_default"}
)
ISO_TIMESTAMP_RE: Final[re.Pattern[str]] = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}")


def _array_record(values: tuple[float, ...] | tuple[int, ...]) -> dict[str, object]:
    # ``canonical_json_dumps`` refuses collections above 2,048 items, which
    # preview meshes routinely exceed. The compact encoder below is the same
    # CPython float/string formatting with the same separators and ASCII
    # escaping, applied to a raw array rather than a protocol envelope.
    text = json.dumps(
        list(values),
        allow_nan=False,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    )
    return {
        "length": len(values),
        "sha256_of_compact_json": hashlib.sha256(text.encode("utf-8")).hexdigest(),
        "first": list(values[:6]),
        "last": list(values[-6:]),
    }


def _mesh_record(kind: str, parameters: dict[str, object]) -> tuple[dict[str, object], bytes]:
    from kala_agent.studio.geometry import (
        ANALYTIC_WARNING,
        MESH_ENGINE,
        build_primitive,
        compute_bounds,
        signed_volume,
        welded_edges_by_position_pairs,
        write_binary_stl,
    )

    mesh = build_primitive(kind, parameters)
    bounds = compute_bounds(mesh.positions)
    edge_uses = welded_edges_by_position_pairs(mesh)
    watertight = bool(edge_uses) and all(count == 2 for count in edge_uses.values())
    stl = write_binary_stl(mesh)
    triangle_count_from_header = int.from_bytes(stl[80:84], "little") if len(stl) >= 84 else 0

    record: dict[str, object] = {
        "kind": kind,
        "parameters": parameters,
        "engine": MESH_ENGINE,
        "warning": ANALYTIC_WARNING,
        "vertex_count": mesh.vertex_count,
        "triangle_count": mesh.triangle_count,
        "positions": _array_record(mesh.positions),
        "normals": _array_record(tuple(mesh.normals)),
        "indices": _array_record(tuple(mesh.indices)),
        "bounds": {
            "min": [bounds.min_x, bounds.min_y, bounds.min_z],
            "max": [bounds.max_x, bounds.max_y, bounds.max_z],
        },
        "signed_volume": signed_volume(mesh),
        "signed_volume_repr": repr(signed_volume(mesh)),
        "signed_volume_note": (
            "diagnostic oracle; compare with a small tolerance because accumulation "
            "order is not a product contract"
        ),
        "welded_edge_symmetric_counts": {
            "distinct_edges": len(edge_uses),
            "all_edges_used_twice": watertight,
        },
        "stl": {
            "sha256": sha256_bytes(stl),
            "length": len(stl),
            "header_ascii": stl[:80].decode("ascii", errors="replace").rstrip(),
            "triangle_count_from_header": triangle_count_from_header,
            "triangle_count_matches_mesh": triangle_count_from_header == mesh.triangle_count,
        },
    }
    return record, stl


def _step_record(workdir: Path) -> dict[str, object]:
    from kala_agent.studio.geometry import probe_step_capability
    from kala_agent.studio.geometry.step import export_step_primitive

    capability = probe_step_capability()
    if not capability.available:
        return {
            "available": False,
            "module_name": None,
            "note": "OCP is not importable; the route would return 503 step_unavailable",
        }
    try:
        with silenced_fds():
            data = export_step_primitive("box", {}, workdir=workdir)
    except Exception as error:
        return {
            "available": True,
            "module_name": capability.module_name,
            "exported": False,
            "error_type": type(error).__name__,
            "message": str(error)[:512],
        }
    normalised = normalise_step_bytes(data)
    normalised_header = normalised[:2048].decode("latin-1", errors="replace")
    normalised_header_lines = normalised_header.splitlines()[:12]
    normalised_timestamp_line = next(
        (line for line in normalised_header_lines if ISO_TIMESTAMP_RE.search(line)),
        None,
    )
    return {
        "available": True,
        "module_name": capability.module_name,
        "exported": True,
        "kind": "box",
        "parameters": {},
        "length": len(data),
        "sha256": sha256_bytes(normalised),
        "sha256_role": (
            "digest of the STEP body after the writer's run-specific values are "
            "normalised; the raw bytes carry a wall-clock timestamp and a "
            "per-process translator counter, so the raw digest is not a parity "
            "contract"
        ),
        "starts_with_iso_10303_21": data.startswith(b"ISO-10303-21"),
        "header_lines": normalised_header_lines,
        "timestamp_line": normalised_timestamp_line,
        "header_timestamp_detected": normalised_timestamp_line is not None,
        "manifold_solid_brep_count": data.count(b"MANIFOLD_SOLID_BREP"),
        "byte_equality_contract": False,
        "normalisation": step_normalisation_facts(),
        "structural_contract": [
            "starts with ISO-10303-21",
            "has STEP HEADER and DATA sections",
            "contains at least one MANIFOLD_SOLID_BREP entity",
            "header timestamp is not part of parity comparison",
        ],
        "oracle_stdout_note": (
            "the OCP writer prints transfer statistics on stdout; captured and not "
            "included in the fixture"
        ),
        "workdir_is_ephemeral": True,
    }


def build(stl_dir: Path, step_workdir: Path) -> dict[str, object]:
    from kala_agent.studio.geometry.limits import (
        DEFAULT_BOX_EDGE_SUBDIVISIONS,
        DEFAULT_CYLINDER_HEIGHT_SEGMENTS,
        DEFAULT_CYLINDER_RADIAL_SEGMENTS,
        DEFAULT_SPHERE_RINGS,
        DEFAULT_SPHERE_SECTORS,
        MAX_BOX_EDGE_SUBDIVISIONS,
        MAX_CYLINDER_HEIGHT_SEGMENTS,
        MAX_CYLINDER_RADIAL_SEGMENTS,
        MAX_MESH_TRIANGLES,
        MAX_MESH_VERTICES,
        MAX_SPHERE_RINGS,
        MAX_SPHERE_SECTORS,
    )
    from kala_agent.studio.geometry.primitives import (
        DEFAULT_BOX_SIZE_MM,
        DEFAULT_CYLINDER_HEIGHT_MM,
        DEFAULT_CYLINDER_RADIUS_MM,
        DEFAULT_SPHERE_RADIUS_MM,
    )

    specs: list[tuple[str, str, dict[str, object]]] = [
        ("box_default", "box", {}),
        ("box_max", "box", {"subdivisions": MAX_BOX_EDGE_SUBDIVISIONS}),
        ("cylinder_default", "cylinder", {}),
        (
            "cylinder_max",
            "cylinder",
            {
                "radial_segments": MAX_CYLINDER_RADIAL_SEGMENTS,
                "height_segments": MAX_CYLINDER_HEIGHT_SEGMENTS,
            },
        ),
        ("sphere_default", "sphere", {}),
        (
            "sphere_max",
            "sphere",
            {"rings": MAX_SPHERE_RINGS, "sectors": MAX_SPHERE_SECTORS},
        ),
    ]

    stl_dir.mkdir(parents=True, exist_ok=True)
    meshes: list[dict[str, object]] = []
    for case_id, kind, parameters in specs:
        record, stl = _mesh_record(kind, parameters)
        record["id"] = case_id
        if case_id in STL_STORE_IDS:
            path = stl_dir / f"{case_id}.stl"
            path.write_bytes(stl)
            record["stl"]["stored_file"] = path.name
            record["stl"]["stored_file_sha256"] = sha256_file(path)
        else:
            record["stl"]["stored_file"] = None
            record["stl"]["stored_file_note"] = (
                "large mesh: regenerate the mesh and compare sha256/length"
            )
        meshes.append(record)

    return {
        "corpus_schema_version": 1,
        "kind": "geometry_stl",
        "oracle_modules": (
            "kala_agent.studio.geometry.meshes",
            "kala_agent.studio.geometry.stl",
            "kala_agent.studio.geometry.step",
        ),
        "defaults": {
            "box_size_mm": DEFAULT_BOX_SIZE_MM,
            "box_edge_subdivisions": DEFAULT_BOX_EDGE_SUBDIVISIONS,
            "cylinder_radius_mm": DEFAULT_CYLINDER_RADIUS_MM,
            "cylinder_height_mm": DEFAULT_CYLINDER_HEIGHT_MM,
            "cylinder_radial_segments": DEFAULT_CYLINDER_RADIAL_SEGMENTS,
            "cylinder_height_segments": DEFAULT_CYLINDER_HEIGHT_SEGMENTS,
            "sphere_radius_mm": DEFAULT_SPHERE_RADIUS_MM,
            "sphere_rings": DEFAULT_SPHERE_RINGS,
            "sphere_sectors": DEFAULT_SPHERE_SECTORS,
        },
        "maxima": {
            "box_edge_subdivisions": MAX_BOX_EDGE_SUBDIVISIONS,
            "cylinder_radial_segments": MAX_CYLINDER_RADIAL_SEGMENTS,
            "cylinder_height_segments": MAX_CYLINDER_HEIGHT_SEGMENTS,
            "sphere_rings": MAX_SPHERE_RINGS,
            "sphere_sectors": MAX_SPHERE_SECTORS,
            "mesh_vertices": MAX_MESH_VERTICES,
            "mesh_triangles": MAX_MESH_TRIANGLES,
        },
        "stl_layout": {
            "header_bytes": 80,
            "triangle_record_bytes": 50,
            "triangle_record_format": "<12fH",
            "attribute_byte_count": 0,
            "header_fill": "kala-studio analytic tessellation, binary STL export",
            "normal_source": "per-triangle geometric normal, not the smooth vertex normals",
        },
        "meshes": meshes,
        "step": _step_record(step_workdir),
    }


__all__ = ("build",)
