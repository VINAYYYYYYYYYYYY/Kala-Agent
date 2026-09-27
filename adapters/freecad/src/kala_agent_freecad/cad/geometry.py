"""Geometry work that runs inside a FreeCAD worker.

Nothing in this module imports FreeCAD at module scope. Every entry point
opens a :func:`~kala_agent_freecad.cad.runtime.freecad_session`, builds the
solid, measures it, and writes exports into the worker workspace only.
"""

from __future__ import annotations

import math
from collections.abc import Mapping, Sequence, Sized
from dataclasses import dataclass
from pathlib import Path
from typing import Final

from kala_agent.contracts import ArtifactRef
from kala_agent.tools.protocol import JSONValue
from kala_agent_freecad.cad.arguments import (
    MAX_DIMENSION_MM,
    MAX_RADIUS_MM,
    dimension,
    flag,
    member,
    text,
    unique_members,
)
from kala_agent_freecad.cad.artifacts import (
    PublishedArtifact,
    bounded_label,
    discard,
    finalize,
    prepare_target,
    resolve_target,
)
from kala_agent_freecad.cad.runtime import (
    FreeCADModules,
    FreeCADObject,
    freecad_session,
)
from kala_agent_freecad.contracts import (
    ARTIFACT_DIRECTORY,
    DEFAULT_ARTIFACT_STEM,
    DEFAULT_DOCUMENT_NAME,
    EXPORT_SUFFIXES,
    GEOMETRY_EXPORT_FORMATS,
    SHAPE_KINDS,
)
from kala_agent_freecad.errors import (
    FreeCADAdapterError,
    FreeCADInputError,
    FreeCADOperationError,
)

MIN_DIMENSION_MM: Final[float] = 0.001
MAX_DOCUMENT_NAME_LENGTH: Final[int] = 64
MAX_SHAPE_NAME_LENGTH: Final[int] = 64
MAX_VERSION_LENGTH: Final[int] = 64
MAX_PATH_LENGTH: Final[int] = 1_024
ROUNDING_DIGITS: Final[int] = 6
SHAPE_OBJECT_NAME: Final[str] = "Shape"


@dataclass(frozen=True, slots=True)
class GeometryRequest:
    shape: str
    export_formats: tuple[str, ...]
    document_name: str
    artifact_stem: str
    length_mm: float
    width_mm: float
    height_mm: float
    radius_mm: float
    measure: bool


@dataclass(frozen=True, slots=True)
class GeometryOutcome:
    result: dict[str, JSONValue]
    artifacts: tuple[ArtifactRef, ...]


def parse_request(arguments: Mapping[str, object]) -> GeometryRequest:
    """Turn validated arguments into a request, refusing anything ambiguous."""
    if not isinstance(arguments, Mapping):
        raise FreeCADInputError("arguments must be a JSON object")
    shape = member(arguments, "shape", SHAPE_KINDS)
    export_formats = unique_members(arguments, "export_formats", GEOMETRY_EXPORT_FORMATS)
    if shape == "box" and "radius_mm" in arguments:
        raise FreeCADInputError("radius_mm only applies to a cylinder")
    if shape == "cylinder" and ("length_mm" in arguments or "width_mm" in arguments):
        raise FreeCADInputError("length_mm and width_mm only apply to a box")
    return GeometryRequest(
        shape=shape,
        export_formats=export_formats,
        document_name=text(arguments, "document_name", DEFAULT_DOCUMENT_NAME),
        artifact_stem=text(arguments, "artifact_stem", DEFAULT_ARTIFACT_STEM),
        length_mm=dimension(arguments, "length_mm", MAX_DIMENSION_MM),
        width_mm=dimension(arguments, "width_mm", MAX_DIMENSION_MM),
        height_mm=dimension(arguments, "height_mm", MAX_DIMENSION_MM),
        radius_mm=dimension(arguments, "radius_mm", MAX_RADIUS_MM),
        measure=flag(arguments, "measure", default=True),
    )


def run(
    request: GeometryRequest,
    *,
    workspace: Path,
    import_roots: Sequence[Path],
    max_artifact_bytes: int,
    call_id: str,
) -> GeometryOutcome:
    """Build the solid, measure it, and publish the requested exports."""
    with freecad_session(import_roots) as modules:
        document = _new_document(modules.freecad, request.document_name)
        try:
            feature = _add_feature(modules, document, request)
            _recompute(document)
            shape = getattr(feature, "Shape", None)
            if shape is None:
                raise FreeCADOperationError("FreeCAD did not build a shape for the request")
            published = _export_all(
                modules,
                document,
                feature,
                request,
                workspace=workspace,
                max_artifact_bytes=max_artifact_bytes,
                call_id=call_id,
            )
            result: dict[str, JSONValue] = {
                "artifacts": [item.entry() for item in published],
                "document_name": bounded_label(request.document_name, MAX_DOCUMENT_NAME_LENGTH),
                "freecad_version": bounded_label(modules.version, MAX_VERSION_LENGTH),
                "import_root": bounded_label(modules.import_root, MAX_PATH_LENGTH),
                "measurement": _measure(shape) if request.measure else _unmeasured(),
                "shape": request.shape,
                "shape_name": bounded_label(SHAPE_OBJECT_NAME, MAX_SHAPE_NAME_LENGTH),
                "valid": bool(_attribute(shape, "isValid")),
                "warnings": [],
            }
        finally:
            _close(modules.freecad, document)
    return GeometryOutcome(
        result=result,
        artifacts=tuple(item.artifact for item in published),
    )


def _new_document(freecad: FreeCADObject, name: str) -> FreeCADObject:
    try:
        return freecad.newDocument(name)
    except Exception as error:
        raise FreeCADOperationError(
            f"FreeCAD could not open a document: {type(error).__name__}: {error}"
        ) from error


def _add_feature(
    modules: FreeCADModules,
    document: FreeCADObject,
    request: GeometryRequest,
) -> FreeCADObject:
    try:
        feature = document.addObject("Part::Feature", SHAPE_OBJECT_NAME)
        feature.Shape = _build_shape(modules.part, request)
    except FreeCADAdapterError:
        raise
    except Exception as error:
        raise FreeCADOperationError(
            f"FreeCAD could not build the {request.shape} solid: {type(error).__name__}: {error}"
        ) from error
    return feature


def _build_shape(part: FreeCADObject, request: GeometryRequest) -> FreeCADObject:
    factory_name = "makeBox" if request.shape == "box" else "makeCylinder"
    factory = getattr(part, factory_name, None)
    if not callable(factory):
        raise FreeCADOperationError(f"the FreeCAD Part module does not offer {factory_name}")
    try:
        if request.shape == "box":
            return factory(request.length_mm, request.width_mm, request.height_mm)
        return factory(request.radius_mm, request.height_mm)
    except Exception as error:
        raise FreeCADOperationError(
            f"FreeCAD refused the {request.shape} dimensions: {type(error).__name__}: {error}"
        ) from error


def _recompute(document: FreeCADObject) -> None:
    try:
        document.recompute()
    except Exception as error:
        raise FreeCADOperationError(
            f"FreeCAD could not recompute the document: {type(error).__name__}: {error}"
        ) from error


def _measure(shape: FreeCADObject) -> dict[str, JSONValue]:
    bounds = _attribute(shape, "BoundBox")
    return {
        "area_mm2": _rounded(_attribute(shape, "Area")),
        "bounding_box_mm": [
            _rounded(_attribute(bounds, "XLength")),
            _rounded(_attribute(bounds, "YLength")),
            _rounded(_attribute(bounds, "ZLength")),
        ],
        "edge_count": _count(shape, "Edges"),
        "face_count": _count(shape, "Faces"),
        "solid": _attribute(shape, "ShapeType") == "Solid",
        "vertex_count": _count(shape, "Vertexes"),
        "volume_mm3": _rounded(_attribute(shape, "Volume")),
    }


def _unmeasured() -> dict[str, JSONValue]:
    return {
        "area_mm2": 0.0,
        "bounding_box_mm": [0.0, 0.0, 0.0],
        "edge_count": 0,
        "face_count": 0,
        "solid": False,
        "vertex_count": 0,
        "volume_mm3": 0.0,
    }


def _attribute(source: FreeCADObject, name: str) -> FreeCADObject:
    value = getattr(source, name, None)
    if value is None:
        raise FreeCADOperationError(f"the FreeCAD shape has no {name}")
    return value


def _count(source: FreeCADObject, name: str) -> int:
    collection = getattr(source, name, None)
    if not isinstance(collection, Sized):
        return 0
    try:
        return max(len(collection), 0)
    except (TypeError, ValueError):  # pragma: no cover - defensive
        return 0


def _rounded(value: FreeCADObject) -> float:
    try:
        number = float(value)
    except (TypeError, ValueError) as error:
        raise FreeCADOperationError("a measured value was not a number") from error
    if not math.isfinite(number):
        raise FreeCADOperationError("a measured value was not finite")
    return round(number, ROUNDING_DIGITS)


def _export_all(
    modules: FreeCADModules,
    document: FreeCADObject,
    feature: FreeCADObject,
    request: GeometryRequest,
    *,
    workspace: Path,
    max_artifact_bytes: int,
    call_id: str,
) -> tuple[PublishedArtifact, ...]:
    published: list[PublishedArtifact] = []
    for export_format in request.export_formats:
        relative = f"{ARTIFACT_DIRECTORY}/{request.artifact_stem}{EXPORT_SUFFIXES[export_format]}"
        target = resolve_target(workspace, relative)
        prepare_target(target)
        try:
            if export_format == "fcstd":
                document.saveAs(str(target))
            else:
                modules.part.export([feature], str(target))
        except Exception as error:
            discard(target)
            raise FreeCADOperationError(
                f"FreeCAD could not write the {export_format} export: "
                f"{type(error).__name__}: {error}"
            ) from error
        published.append(
            finalize(
                target,
                relative_path=relative,
                export_format=export_format,
                artifact_id=f"{call_id}-{export_format}",
                max_artifact_bytes=max_artifact_bytes,
            )
        )
    return tuple(published)


def _close(freecad: FreeCADObject, document: FreeCADObject) -> None:
    name = getattr(document, "Name", None)
    if not isinstance(name, str) or not name:
        return
    try:
        freecad.closeDocument(name)
    except Exception:  # pragma: no cover - teardown noise
        return


__all__ = (
    "MAX_DIMENSION_MM",
    "MAX_RADIUS_MM",
    "ROUNDING_DIGITS",
    "SHAPE_KINDS",
    "GeometryOutcome",
    "GeometryRequest",
    "parse_request",
    "run",
)
