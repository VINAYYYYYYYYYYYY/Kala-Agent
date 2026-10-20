#!/usr/bin/env python3
"""Test-only fake ``FreeCADCmd`` for the native provider's security tests.

The real provider runs this file exactly as it runs ``FreeCADCmd``: an
explicit executable, the generated driver as argv[1], and an explicit
environment. The fake never computes geometry. It exercises the subprocess
hand-off, the result-envelope parser, the artifact containment checks and the
timeout path without pretending to be FreeCAD.

Mode is selected by ``FAKE_FREECAD_MODE`` in the child environment.
"""

from __future__ import annotations

import hashlib
import json
import os
import sys
import tempfile
import time
from pathlib import Path

MODE = os.environ.get("FAKE_FREECAD_MODE", "version_ok")


def _output_path() -> Path:
    return Path(os.environ["KALA_FREECAD_OUTPUT"])


def _workspace() -> Path:
    return Path(os.environ["KALA_FREECAD_WORKSPACE"])


def _write(payload: object) -> None:
    _output_path().write_text(json.dumps(payload, sort_keys=True), encoding="utf-8")


def _digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _artifact(path: Path, export_format: str, data: bytes) -> dict[str, object]:
    path.write_bytes(data)
    return {
        "format": export_format,
        "path": str(path),
        "size_bytes": len(data),
        "digest": _digest(data),
    }


def _success() -> dict[str, object]:
    workspace = _workspace()
    step = _artifact(workspace / "kala-provider.step", "step", b"ISO-10303-21\nFAKE STEP\n")
    native = _artifact(workspace / "kala-provider.FCStd", "fcstd", b"PK\x03\x04FAKE FCSTD")
    return {
        "ok": True,
        "measurement": {"volume_mm3": 120.0, "area_mm2": 148.0},
        "bounds": {"min": [0.0, 0.0, 0.0], "max": [4.0, 5.0, 6.0]},
        "freecad_version": "9.9.9",
        "warnings": ["a fake provider warning"],
        "valid": True,
        "solid_count": 1,
        "artifacts": [step, native],
    }


def _count_run() -> None:
    counter = os.environ.get("FAKE_FREECAD_COUNTER")
    if counter:
        with open(counter, "a", encoding="utf-8") as handle:
            handle.write("run\n")


def _dump_environment() -> None:
    destination = os.environ.get("FAKE_FREECAD_DUMP")
    if destination:
        Path(destination).write_text(
            json.dumps(dict(os.environ), sort_keys=True), encoding="utf-8"
        )
    destination = os.environ.get("FAKE_FREECAD_REQUEST_DUMP")
    if destination:
        Path(destination).write_text(
            Path(os.environ["KALA_FREECAD_REQUEST"]).read_text(encoding="utf-8"),
            encoding="utf-8",
        )


def _version() -> None:
    if MODE == "version_bad":
        print("this is not a FreeCAD banner")
        return
    if MODE == "version_long":
        print("FreeCAD 1.2.3.4-beta.1+local Revision: 99")
        return
    print("FreeCAD 1.1.4 Revision: 45040 (Git)")
    print("FreeCAD 1.1.4, Libs: fake", file=sys.stderr)


def _python_identity() -> None:
    """Answer the provider's Python-identity probe like FreeCADCmd would."""
    abi = os.environ.get("FAKE_FREECAD_PYTHON_ABI", "cp314")
    version = os.environ.get("FAKE_FREECAD_PYTHON_VERSION", "3.14.8")
    cache_tag = os.environ.get("FAKE_FREECAD_PYTHON_CACHE_TAG", "cpython-" + abi[2:])
    print(
        "KALA_FREECAD_PYTHON="
        + json.dumps({"version": version, "cacheTag": cache_tag})
    )


def _run_build() -> int:
    _dump_environment()
    if MODE == "sleep_build":
        time.sleep(30.0)
    if MODE == "no_response":
        print("boom   on two\nlines", file=sys.stderr)
        print("x" * 600, file=sys.stderr)
        return 3
    if MODE == "no_response_empty_stderr":
        return 4
    if MODE == "invalid_json":
        _output_path().write_text("{not json", encoding="utf-8")
        return 0
    if MODE == "non_dict":
        _output_path().write_text("[1, 2, 3]", encoding="utf-8")
        return 0
    if MODE == "oversize":
        _output_path().write_bytes(b" " * (4 * 1024 * 1024 + 1))
        return 0
    if MODE == "refuse":
        _write({"ok": False, "error": "FreeCADOperationError: did not produce a solid"})
        return 0
    if MODE == "refuse_no_op":
        _write({"ok": False, "error": "FreeCAD refused this script"})
        return 0
    if MODE == "refuse_none":
        _write({"ok": False, "error": None})
        return 0
    if MODE == "bad_artifacts":
        payload = _success()
        payload["artifacts"] = 5
        _write(payload)
        return 0
    if MODE == "missing_step":
        payload = _success()
        payload["artifacts"] = [
            entry for entry in payload["artifacts"] if entry["format"] != "step"
        ]
        _write(payload)
        return 0
    if MODE == "missing_native":
        payload = _success()
        payload["artifacts"] = [
            entry for entry in payload["artifacts"] if entry["format"] != "fcstd"
        ]
        _write(payload)
        return 0
    if MODE == "escape":
        outside = Path(tempfile.gettempdir()) / f"kala-fake-outside-{os.getpid()}.step"
        outside.write_bytes(b"ISO-10303-21\nOUTSIDE\n")
        payload = _success()
        payload["artifacts"][0] = {
            "format": "step",
            "path": str(outside),
            "size_bytes": outside.stat().st_size,
            "digest": _digest(outside.read_bytes()),
        }
        _write(payload)
        return 0
    if MODE == "symlink_escape":
        workspace = _workspace()
        outside = Path(tempfile.gettempdir()) / f"kala-fake-target-{os.getpid()}.step"
        outside.write_bytes(b"ISO-10303-21\nOUTSIDE\n")
        link = workspace / "link.step"
        if link.exists() or link.is_symlink():
            link.unlink()
        link.symlink_to(outside)
        payload = _success()
        payload["artifacts"][0] = {
            "format": "step",
            "path": str(link),
            "size_bytes": outside.stat().st_size,
            "digest": _digest(outside.read_bytes()),
        }
        _write(payload)
        return 0
    if MODE == "size_over":
        payload = _success()
        payload["artifacts"][0]["size_bytes"] = 10**30
        _write(payload)
        return 0
    if MODE == "size_mismatch":
        payload = _success()
        payload["artifacts"][0]["size_bytes"] = 1
        _write(payload)
        return 0
    if MODE == "digest_mismatch":
        payload = _success()
        payload["artifacts"][0]["digest"] = "0" * 64
        _write(payload)
        return 0
    if MODE == "descriptor_incomplete":
        payload = _success()
        del payload["artifacts"][0]["digest"]
        _write(payload)
        return 0
    if MODE == "edge_metrics":
        payload = _success()
        payload["measurement"] = {"volume_mm3": "12", "area_mm2": True}
        payload["bounds"] = {
            "min": ["-1.5", "-2.5", "-3.5"],
            "max": ["4.5", "5.5", "6.5"],
        }
        payload["valid"] = "yes"
        payload["solid_count"] = -1
        payload["freecad_version"] = "9.9.9-" + "9" * 80
        payload["warnings"] = [1, "kept", "", "x" * 300] + ["w" * 3] * 20
        _write(payload)
        return 0
    if MODE == "bounds_bad":
        payload = _success()
        payload["bounds"] = {"min": [0.0, 0.0], "max": [4.0, 5.0, 6.0]}
        _write(payload)
        return 0
    if MODE == "bounds_inverted":
        payload = _success()
        payload["bounds"] = {"min": [5.0, 0.0, 0.0], "max": [4.0, 5.0, 6.0]}
        _write(payload)
        return 0
    if MODE == "bounds_nan":
        payload = _success()
        payload["bounds"] = {"min": ["NaN", 0.0, 0.0], "max": [4.0, 5.0, 6.0]}
        _write(payload)
        return 0
    if MODE == "warning_truncation":
        payload = _success()
        payload["warnings"] = ["y" * 400] * 20
        _write(payload)
        return 0
    if MODE != "success":
        raise SystemExit(f"unknown fake mode {MODE!r}")
    _write(_success())
    return 0


def main() -> int:
    _count_run()
    if "--version" in sys.argv[1:]:
        _version()
        return 0
    if (
        sys.argv[1:]
        and Path(sys.argv[-1]).name == "kala_freecad_python_probe.py"
    ):
        _python_identity()
        return 0
    return _run_build()


if __name__ == "__main__":
    raise SystemExit(main())
