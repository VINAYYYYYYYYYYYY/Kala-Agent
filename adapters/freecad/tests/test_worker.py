"""Worker behaviour that does not need a FreeCAD installation.

These tests drive the entry points the executor dispatches, and prove the
properties the kernel depends on: a single response envelope, a typed error
code, no uncaught exception, and no native output on the protocol channel.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Any, Final, cast

import pytest

from conftest import BOX_ARGUMENTS, make_fem_request, make_request
from kala_agent.contracts import ToolRunStatus
from kala_agent.tools.protocol import JSONValue, ToolErrorCode
from kala_agent_freecad import worker
from kala_agent_freecad.cad import geometry
from kala_agent_freecad.cad.geometry import GeometryOutcome
from kala_agent_freecad.cad.runtime import FREECAD_IMPORT_ORDER, native_output_to_stderr
from kala_agent_freecad.contracts import (
    GEOMETRY_OUTPUT_SCHEMA,
    TOOL_CAPABILITIES,
)
from kala_agent_freecad.errors import (
    FreeCADAdapterError,
    FreeCADArtifactError,
    FreeCADDependencyError,
    FreeCADInputError,
    FreeCADOperationError,
    FreeCADOutputTooLargeError,
    FreeCADWorkspaceError,
    error_code_for,
    error_message_for,
)

PACKAGE_ROOT: Final[Path] = Path(__file__).resolve().parent.parent / "src"
ADAPTER_ROOT: Final[Path] = PACKAGE_ROOT / "kala_agent_freecad"
FREECAD_LOADER: Final[Path] = ADAPTER_ROOT / "cad" / "runtime.py"
NATIVE_IMPORT: Final[re.Pattern[str]] = re.compile(
    r"^\s*(?:import|from)\s+(?:FreeCAD|Part|ObjectsFem|femtools)\b"
)
MODULE_LOADER_CALL: Final[str] = "importlib.import_module("
GEOMETRY_RESULT: Final[dict[str, JSONValue]] = {
    "artifacts": [
        {
            "digest": "0" * 64,
            "format": "step",
            "media_type": "model/step",
            "relative_path": "artifacts/kala-model.step",
            "size_bytes": 2_048,
        }
    ],
    "document_name": "kala-freecad",
    "freecad_version": "1.1.3",
    "import_root": "/usr/lib/freecad/lib",
    "measurement": {
        "area_mm2": 16_000.0,
        "bounding_box_mm": [120.0, 40.0, 20.0],
        "edge_count": 12,
        "face_count": 6,
        "solid": True,
        "vertex_count": 8,
        "volume_mm3": 96_000.0,
    },
    "shape": "box",
    "shape_name": "Shape",
    "valid": True,
    "warnings": [],
}


def _sources() -> list[Path]:
    return sorted(path for path in ADAPTER_ROOT.rglob("*.py") if "__pycache__" not in path.parts)


def test_native_modules_are_never_imported_directly() -> None:
    offenders = [
        f"{path.name}:{number}"
        for path in _sources()
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1)
        if NATIVE_IMPORT.match(line)
    ]
    assert offenders == []


def test_only_the_loader_resolves_a_native_module() -> None:
    offenders = [
        path.name
        for path in _sources()
        if path != FREECAD_LOADER and MODULE_LOADER_CALL in path.read_text(encoding="utf-8")
    ]
    assert offenders == []
    loader_lines = FREECAD_LOADER.read_text(encoding="utf-8").splitlines()
    calls = [line for line in loader_lines if MODULE_LOADER_CALL in line]
    assert calls
    for line in calls:
        assert line[:1].isspace()


def test_the_loader_imports_freecad_first() -> None:
    assert FREECAD_IMPORT_ORDER[0] == "FreeCAD"
    assert FREECAD_IMPORT_ORDER[:3] == ("FreeCAD", "Part", "ObjectsFem")
    loader = FREECAD_LOADER.read_text(encoding="utf-8")
    session = loader.split("def freecad_session", 1)[1].split("\ndef ", 1)[0]
    requested = re.findall(r"_import_module\(\s*([^,]+),", session)
    assert requested == ["FREECAD_MODULE", '"Part"', '"ObjectsFem"']


def test_the_adapter_never_imports_a_network_client() -> None:
    forbidden = ("import socket", "import urllib", "import http", "import requests", "import ssl")
    for path in _sources():
        text = path.read_text(encoding="utf-8")
        for needle in forbidden:
            assert needle not in text, f"{path.name} imports {needle}"


def test_importing_the_adapter_does_not_load_freecad(tmp_path: Path) -> None:
    source = (
        f"import sys; sys.path.insert(0, {str(PACKAGE_ROOT)!r});"
        "import kala_agent_freecad, kala_agent_freecad.worker, kala_agent_freecad.registration;"
        "loaded = [name for name in ('FreeCAD', 'Part', 'ObjectsFem') if name in sys.modules];"
        "sys.exit(0 if not loaded else 1)"
    )
    completed = subprocess.run(
        [sys.executable, "-I", "-B", "-c", source],
        capture_output=True,
        check=False,
        cwd=tmp_path,
        env={
            "HOME": str(tmp_path),
            "LANG": "C.UTF-8",
            "PATH": "/usr/local/bin:/usr/bin:/bin",
            "PYTHONDONTWRITEBYTECODE": "1",
        },
        timeout=120.0,
    )
    assert completed.returncode == 0, completed.stderr.decode("utf-8", errors="replace")


@pytest.mark.parametrize(
    "arguments",
    (
        {"shape": "sphere", "export_formats": ["step"]},
        {"export_formats": ["step"]},
        {"shape": "box", "export_formats": ["step"], "unknown": 1},
        {"shape": "box", "export_formats": ["step"], "length_mm": 9_999.0},
        {"shape": "box", "export_formats": ["step"], "artifact_stem": "../escape"},
        {"shape": "box", "export_formats": []},
    ),
)
def test_geometry_refuses_bad_arguments(tmp_path: Path, arguments: dict[str, Any]) -> None:
    response = worker.invoke_geometry(make_request(cast("Any", arguments), workspace=tmp_path))
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.INVALID_ARGUMENTS.value
    assert response.artifacts == ()
    assert response.result is None
    assert response.error_message


def test_geometry_refuses_a_shape_mismatched_argument(tmp_path: Path) -> None:
    arguments: dict[str, Any] = dict(BOX_ARGUMENTS)
    arguments["radius_mm"] = 5.0
    response = worker.invoke_geometry(make_request(cast("Any", arguments), workspace=tmp_path))
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.INVALID_ARGUMENTS.value
    assert "radius_mm" in str(response.error_message)


def test_geometry_refuses_a_cylinder_with_box_dimensions(tmp_path: Path) -> None:
    arguments: dict[str, Any] = {"shape": "cylinder", "export_formats": ["step"], "width_mm": 4.0}
    response = worker.invoke_geometry(make_request(cast("Any", arguments), workspace=tmp_path))
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.INVALID_ARGUMENTS.value


def test_fem_refuses_bad_arguments(tmp_path: Path) -> None:
    response = worker.invoke_fem(
        make_fem_request(cast("Any", {"shape": "box", "poisson_ratio": 0.7}), workspace=tmp_path)
    )
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.INVALID_ARGUMENTS.value


def test_fem_refuses_a_shape_mismatched_argument(tmp_path: Path) -> None:
    response = worker.invoke_fem(
        make_fem_request(
            cast("Any", {"shape": "cylinder", "export_formats": ["step"], "length_mm": 3.0}),
            workspace=tmp_path,
        )
    )
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.INVALID_ARGUMENTS.value


def test_a_missing_freecad_fails_closed(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(worker, "import_roots_from_path", lambda: ())
    response = worker.invoke_geometry(make_request(dict(BOX_ARGUMENTS), workspace=tmp_path))
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.SANDBOX_UNAVAILABLE.value
    assert response.artifacts == ()
    assert list(tmp_path.iterdir()) == []


def test_a_missing_fem_bridge_fails_closed(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(worker, "import_roots_from_path", lambda: ())
    response = worker.invoke_fem(
        make_fem_request({"shape": "box", "length_mm": 10.0}, workspace=tmp_path)
    )
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.SANDBOX_UNAVAILABLE.value


def test_an_unexpected_failure_is_reported_not_raised(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def explode(*_args: object, **_kwargs: object) -> GeometryOutcome:
        raise ZeroDivisionError("a native failure")

    monkeypatch.setattr(geometry, "run", explode)
    response = worker.invoke_geometry(make_request(dict(BOX_ARGUMENTS), workspace=tmp_path))
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.INTERNAL_ERROR.value
    assert "ZeroDivisionError" in str(response.error_message)


def test_a_workspace_escaping_path_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def escape(*_args: object, **_kwargs: object) -> GeometryOutcome:
        raise FreeCADWorkspaceError("the artifact path escapes its run directory")

    monkeypatch.setattr(geometry, "run", escape)
    response = worker.invoke_geometry(make_request(dict(BOX_ARGUMENTS), workspace=tmp_path))
    assert response.error_code == ToolErrorCode.WORKSPACE_VIOLATION.value


def test_a_result_that_breaks_the_schema_is_refused(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def liar(*_args: object, **_kwargs: object) -> GeometryOutcome:
        return GeometryOutcome(result=cast("Any", {"shape": "box"}), artifacts=())

    monkeypatch.setattr(geometry, "run", liar)
    response = worker.invoke_geometry(make_request(dict(BOX_ARGUMENTS), workspace=tmp_path))
    assert response.status is ToolRunStatus.FAILED
    assert response.error_code == ToolErrorCode.INVALID_RESULT.value
    assert response.artifacts == ()


def test_a_valid_result_is_published(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def succeed(*_args: object, **_kwargs: object) -> GeometryOutcome:
        return GeometryOutcome(result=dict(GEOMETRY_RESULT), artifacts=())

    monkeypatch.setattr(geometry, "run", succeed)
    response = worker.invoke_geometry(make_request(dict(BOX_ARGUMENTS), workspace=tmp_path))
    assert response.status is ToolRunStatus.SUCCEEDED
    assert response.error_code is None
    assert response.result == GEOMETRY_RESULT
    assert response.result_json is not None
    assert GEOMETRY_OUTPUT_SCHEMA["required"]


def test_native_output_cannot_reach_the_protocol_channel(capfd: pytest.CaptureFixture[str]) -> None:
    with native_output_to_stderr():
        os.write(1, b"native-banner\n")
    os.write(1, b"protocol-line\n")
    captured = capfd.readouterr()
    assert "native-banner" in captured.err
    assert "native-banner" not in captured.out
    assert "protocol-line" in captured.out


@pytest.mark.parametrize(
    ("error", "code"),
    (
        (FreeCADInputError("bad"), ToolErrorCode.INVALID_ARGUMENTS),
        (FreeCADDependencyError("absent"), ToolErrorCode.SANDBOX_UNAVAILABLE),
        (FreeCADWorkspaceError("escape"), ToolErrorCode.WORKSPACE_VIOLATION),
        (FreeCADArtifactError("empty"), ToolErrorCode.ARTIFACT_REJECTED),
        (FreeCADOutputTooLargeError("huge"), ToolErrorCode.OUTPUT_TOO_LARGE),
        (FreeCADOperationError("crash"), ToolErrorCode.INTERNAL_ERROR),
        (ZeroDivisionError("native"), ToolErrorCode.INTERNAL_ERROR),
    ),
)
def test_every_error_maps_to_an_existing_code(
    error: BaseException,
    code: ToolErrorCode,
) -> None:
    assert error_code_for(error) is code
    assert code in set(ToolErrorCode)
    assert error_message_for(error)


def test_error_messages_are_bounded_and_single_line() -> None:
    message = error_message_for(FreeCADOperationError("a\nb  c" * 4_000), replacement="/")
    assert "\n" not in message
    assert len(message) <= 1_024


def test_error_messages_hide_the_workspace() -> None:
    message = error_message_for(
        FreeCADArtifactError("/tmp/secret/workspace/file"),
        replacement="/tmp/secret/workspace",
    )
    assert "/tmp/secret/workspace" not in message
    assert "<workspace>" in message


def test_capability_constants_are_local_only() -> None:
    assert TOOL_CAPABILITIES == ("artifact.write", "design.write", "tool.execute")
    assert "process.spawn" not in TOOL_CAPABILITIES
    assert "shell.execute" not in TOOL_CAPABILITIES
    assert "system.admin" not in TOOL_CAPABILITIES


def test_adapter_error_hierarchy_is_typed() -> None:
    for error_type in (
        FreeCADInputError,
        FreeCADDependencyError,
        FreeCADWorkspaceError,
        FreeCADArtifactError,
        FreeCADOutputTooLargeError,
        FreeCADOperationError,
    ):
        assert issubclass(error_type, FreeCADAdapterError)
        assert isinstance(error_type("x").error_code, ToolErrorCode)
