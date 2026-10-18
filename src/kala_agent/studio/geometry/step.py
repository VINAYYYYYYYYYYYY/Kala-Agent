"""Exact B-rep STEP export for studio preview primitives.

When the OCP bindings (``cadquery-ocp``) are installed, this module builds
the requested primitive directly as a B-rep using
``BRepPrimAPI_MakeBox`` / ``BRepPrimAPI_MakeCylinder`` / ``BRepPrimAPI_MakeSphere``
and writes the result as STEP through ``STEPControl_Writer``. The exported
shape is an exact analytic solid, not a triangulated shell.

The export route path preserves the original primitive parameters end-to-end
so that the operator's ``size_x``, ``radius``, ``height`` map to a single
OCP builder call. When OCP is absent the surface returns a 503 with an
actionable message rather than fabricating geometry.
"""

from __future__ import annotations

import os
import tempfile
from collections.abc import Mapping
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Final, Protocol

from kala_agent.studio.geometry.primitives import resolve_dimension_parameters

OCP_MODULE_NAMES: Final[tuple[str, ...]] = ("OCP",)
STEP_UNAVAILABLE_MESSAGE: Final[str] = (
    "STEP export requires the OCP bindings (the 'cadquery-ocp' package). "
    "The studio never fabricates a STEP representation; install "
    "'cadquery-ocp' and restart the studio to enable it."
)
STEP_OUTPUT_MODE: Final[str] = "exact_brep"


class PrimitiveKind(StrEnum):
    BOX = "box"
    CYLINDER = "cylinder"
    SPHERE = "sphere"


@dataclass(frozen=True, slots=True)
class StepCapability:
    """Reports whether OCP is importable in this process."""

    available: bool
    module_name: str | None

    @classmethod
    def probe(cls) -> StepCapability:
        for name in OCP_MODULE_NAMES:
            try:
                __import__(name)
            except Exception:
                continue
            return cls(available=True, module_name=name)
        return cls(available=False, module_name=None)


def probe_step_capability() -> StepCapability:
    """Return the current STEP capability for this process."""
    return StepCapability.probe()


class StepUnavailableError(RuntimeError):
    """Raised when a STEP export is requested but OCP is unavailable."""


class StepExportError(RuntimeError):
    """Raised when the OCP writer cannot complete a STEP export."""


class _OcpShape(Protocol):
    """Structural type for the OCP shape handed to the STEP writer.

    The OCP bindings ship without stubs, so the studio describes only the
    one method it calls instead of importing ``Any`` across the module.
    """

    def IsNull(self) -> bool: ...


class _OcpPrimitiveBuilder(Protocol):
    """Structural type for an OCP ``BRepPrimAPI_Make*`` builder."""

    def Shape(self) -> _OcpShape | None: ...


def _require_ocp() -> None:
    capability = probe_step_capability()
    if not capability.available:
        raise StepUnavailableError(STEP_UNAVAILABLE_MESSAGE)


def _ocp_shape_for(
    kind: str,
    dimensions: Mapping[str, float],
) -> _OcpShape:
    """Build an OCP B-rep shape for ``kind`` from canonical dimensions.

    ``dimensions`` is the result of
    :func:`~kala_agent.studio.geometry.primitives.resolve_dimension_parameters`,
    so every value is already a validated finite float: the writer never
    re-validates or applies a second, subtly different default.
    """
    from OCP.BRepPrimAPI import (  # type: ignore[import-untyped]
        BRepPrimAPI_MakeBox,
        BRepPrimAPI_MakeCylinder,
        BRepPrimAPI_MakeSphere,
    )
    from OCP.gp import (  # type: ignore[import-untyped]
        gp_Ax2,
        gp_Dir,
        gp_Pnt,
    )

    if kind == "box":
        sx = dimensions["size_x"]
        sy = dimensions["size_y"]
        sz = dimensions["size_z"]
        corner = gp_Pnt(-sx / 2.0, -sy / 2.0, -sz / 2.0)
        builder = BRepPrimAPI_MakeBox(corner, sx, sy, sz)
        return _check_and_return(builder, kind=kind)
    if kind == "cylinder":
        radius = dimensions["radius"]
        height = dimensions["height"]
        axis = gp_Ax2(gp_Pnt(0.0, 0.0, -height / 2.0), gp_Dir(0.0, 0.0, 1.0))
        builder = BRepPrimAPI_MakeCylinder(axis, radius, height)
        return _check_and_return(builder, kind=kind)
    if kind == "sphere":
        radius = dimensions["radius"]
        builder = BRepPrimAPI_MakeSphere(gp_Pnt(0.0, 0.0, 0.0), radius)
        return _check_and_return(builder, kind=kind)
    raise StepExportError(f"unsupported primitive kind for STEP export: {kind}")


def _check_and_return(builder: _OcpPrimitiveBuilder, *, kind: str) -> _OcpShape:
    """Run an OCP primitive builder and return the resulting shape."""
    shape = builder.Shape()
    if shape is None or shape.IsNull():
        raise StepExportError(f"the OCP {kind} builder returned a null shape")
    return shape


def export_step_primitive(
    kind: str,
    parameters: Mapping[str, object],
    *,
    workdir: Path,
) -> bytes:
    """Serialise one primitive B-rep to a STEP byte string.

    The byte string is the literal contents of a STEP file written through
    ``STEPControl_Writer`` with the ``STEPControl_AsIs`` mode and read back
    from a temporary file inside ``workdir`` that is unlinked before this
    function returns. The result is an exact B-rep shape. Missing
    parameters take the same canonical defaults the mesh preview uses.
    """
    # The availability probe runs before the import so a machine without
    # OCP returns the honest 503 instead of an ImportError traceback.
    _require_ocp()
    dimensions = resolve_dimension_parameters(kind, parameters)
    from OCP.IFSelect import (  # type: ignore[import-untyped]
        IFSelect_RetDone,
    )
    from OCP.STEPControl import (  # type: ignore[import-untyped]
        STEPControl_AsIs,
        STEPControl_Writer,
    )

    shape = _ocp_shape_for(kind, dimensions)

    workdir.mkdir(parents=True, exist_ok=True)
    fd, path_str = tempfile.mkstemp(prefix="kala-studio-step-", suffix=".step", dir=str(workdir))
    os.close(fd)
    path = Path(path_str)
    try:
        writer = STEPControl_Writer()
        transfer_status = writer.Transfer(shape, STEPControl_AsIs)
        if transfer_status != IFSelect_RetDone:
            raise StepExportError("the OCP STEP writer refused the analytic shape")
        write_status = writer.Write(str(path))
        if write_status != IFSelect_RetDone:
            raise StepExportError("the OCP STEP writer did not finish serialising")
        data = path.read_bytes()
        if not data.startswith(b"ISO-10303-21"):
            raise StepExportError("the STEP writer produced a non-ISO file; refusing to ship it")
        return data
    finally:
        try:
            path.unlink()
        except OSError:
            pass


def export_step(*, kind: str, parameters: Mapping[str, object]) -> bytes:
    """Serialise one primitive to a STEP byte string.

    The function picks a project-local data directory for the temporary
    file: callers can use this directly because the only on-disk artefact
    is the temporary STEP file, which is removed before the call returns.
    Production callers should prefer :func:`export_step_primitive` with an
    explicit ``workdir`` so the route can pass the studio data directory.
    """
    workdir = Path(tempfile.gettempdir()) / "kala-studio-step"
    return export_step_primitive(kind, parameters, workdir=workdir)


__all__ = (
    "OCP_MODULE_NAMES",
    "STEP_OUTPUT_MODE",
    "STEP_UNAVAILABLE_MESSAGE",
    "PrimitiveKind",
    "StepCapability",
    "StepExportError",
    "StepUnavailableError",
    "export_step",
    "export_step_primitive",
    "probe_step_capability",
)
