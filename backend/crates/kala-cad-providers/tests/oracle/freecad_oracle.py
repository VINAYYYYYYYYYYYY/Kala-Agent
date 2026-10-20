"""Live Python oracle for the native FreeCAD provider differential.

This script is **test-only**. It imports the repository's own
``kala_agent.studio.design.providers.freecad`` and
``kala_agent.studio.design.importer``, builds the requested construction
scripts with a real ``FreeCADCmd``, and prints one JSON document describing
what the Python provider produced. The Rust test compares its own provider
against that document.

The script is run with an isolated interpreter (``python -I``). Isolated mode
ignores ``PYTHONPATH``, so the repository paths are inserted here. It reads a
JSON request from stdin:

    {
      "cases": [{"id": "...", "script": {...}, "rust_step_path": "...|null"}],
      "include_driver_source": true
    }

and writes a JSON response to stdout. It never writes a file the product
reads.
"""

from __future__ import annotations

import hashlib
import json
import os
import sys
from typing import Any


def _prepare(root: str) -> None:
    for part in ("src", os.path.join("adapters", "freecad", "src")):
        path = os.path.join(root, part)
        if path not in sys.path:
            sys.path.insert(0, path)


def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _bounds(bounds: Any) -> dict[str, list[float]] | None:
    if bounds is None:
        return None
    return {"min": list(bounds.min_mm), "max": list(bounds.max_mm)}


def _measured(measured: Any) -> dict[str, Any]:
    return {
        "volume_mm3": measured.volume_mm3,
        "area_mm2": measured.area_mm2,
        "bounds": _bounds(measured.bounds),
        "mass_g": measured.mass_g,
        "centroid_mm": list(measured.centroid_mm)
        if measured.centroid_mm is not None
        else None,
    }


def _imported(data: bytes) -> dict[str, Any]:
    from kala_agent.studio.design.importer import import_step_bytes

    outcome = import_step_bytes(data, store_artifact=False)
    return {
        "min": list(outcome.bounds.min_mm),
        "max": list(outcome.bounds.max_mm),
        "solids": outcome.solids,
    }


def _build_case(provider: Any, case: dict[str, Any]) -> dict[str, Any]:
    from kala_agent.contracts.construction import ConstructionScript
    from kala_agent.studio.design.intent import DesignIntent
    from kala_agent.studio.design.providers.base import CadProviderBuildError

    script = ConstructionScript.model_validate(case["script"])
    request_json = json.dumps(
        {"script": script.model_dump(mode="json", exclude_none=True)},
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=True,
    )
    result: dict[str, Any] = {
        "id": case["id"],
        "request_json": request_json,
    }
    try:
        build = provider.build(script, DesignIntent(script=script))
    except CadProviderBuildError as error:
        result["refused"] = {"message": str(error), "op": error.op}
        return result
    imported = None
    import_error = None
    try:
        imported = _imported(build.step_bytes)
    except Exception as error:  # noqa: BLE001 - report the oracle's failure
        import_error = type(error).__name__ + ": " + str(error)
    rust_imported: dict[str, Any] | None = None
    rust_step_path = case.get("rust_step_path")
    if isinstance(rust_step_path, str):
        try:
            with open(rust_step_path, "rb") as handle:
                rust_imported = _imported(handle.read())
        except Exception as error:  # noqa: BLE001 - report the oracle's failure
            rust_imported = {"error": type(error).__name__ + ": " + str(error)}
    result["build"] = {
        "provider_version": build.provider_version,
        "measured": _measured(build.measured),
        "warnings": list(build.warnings),
        "valid": build.valid,
        "solid_count": build.solid_count,
        "native_kind": build.native_kind,
        "native_present": build.native_bytes is not None,
        "native_size": len(build.native_bytes) if build.native_bytes else 0,
        "native_sha256": _sha256(build.native_bytes) if build.native_bytes else None,
        "step_size": len(build.step_bytes),
        "step_sha256": _sha256(build.step_bytes),
        "imported": imported,
        "import_error": import_error,
        "rust_imported": rust_imported,
    }
    return result


def main() -> int:
    root = (
        os.path.abspath(sys.argv[1])
        if len(sys.argv) > 1
        else os.path.abspath(
            os.path.join(os.path.dirname(__file__), "..", "..", "..", "..", "..")
        )
    )
    _prepare(root)
    request = json.loads(sys.stdin.read())

    from kala_agent.studio.design.providers import freecad
    from kala_agent.studio.design.providers.freecad import FreeCadProvider

    provider = FreeCadProvider(probe_cache_seconds=0)
    availability = provider.probe()
    if not availability.available:
        print(
            json.dumps(
                {"unavailable": availability.reason},
                sort_keys=True,
                separators=(",", ":"),
            )
        )
        return 0

    response: dict[str, Any] = {"cases": []}
    if request.get("include_driver_source"):
        response["driver_source"] = freecad._DRIVER_SOURCE
    for case in request.get("cases") or []:
        response["cases"].append(_build_case(provider, case))
    print(
        json.dumps(
            response,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=True,
            allow_nan=False,
            default=str,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
