"""View-only STEP import for the Studio geometry worker (ADR-0011).

OpenCascade's default role is a *viewer*: an external CAD provider (Solid
Edge, FreeCAD) builds the model and exports STEP; this module reads those
bytes with ``STEPControl_Reader``, tessellates the imported shape with
``BRepMesh_IncrementalMesh`` inside the existing mesh budget, and reports the
display view plus the content digest it stored.

The imported shape is never modified and never re-exported. Tessellation adds
a display triangulation to the shape in memory, which is not a geometry edit,
and the original STEP bytes are the only thing stored: the artifact store is
content-addressed, so the digest returned here is the SHA-256 of exactly the
bytes the provider exported.

A provider STEP export can be much larger than an ordinary Studio request
body. The importer therefore has **its own** default ceiling of 64 MiB
(:data:`DEFAULT_MAX_STEP_BYTES`), configurable with
``KALA_STEP_IMPORT_MAX_BYTES``. The HTTP upload route reads the body in bounded
chunks and refuses as soon as the limit is exceeded; this module repeats the
check for direct callers. Every other Studio route keeps the generic body
limit.

A mesh that exceeds the 200k-vertex / 400k-triangle budget is retried once
with a coarser deflection before it is refused with ``step_too_complex``. The
warning states that the coarser view was used; it is never a silent
substitution.

A file that the STEP reader refuses, that carries no transferable root, or
that tessellates past the mesh limits is refused with a typed
:class:`StepImportError`; it is never displayed partially. OCP is imported
lazily so the worker still starts (and reports ``design_available: false``)
on a host without the kernel.
"""

from __future__ import annotations

import hashlib
import math
import os
import tempfile
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, Final

from kala_agent.studio.geometry.limits import (
    MAX_MESH_TRIANGLES,
    MAX_MESH_VERTICES,
)

if TYPE_CHECKING:
    from OCP.BRepCheck import BRepCheck_Analyzer  # type: ignore[import-untyped]
    from OCP.TopExp import TopExp_Explorer  # type: ignore[import-untyped]
    from OCP.TopoDS import TopoDS_Shape  # type: ignore[import-untyped]

    from kala_agent.studio.design.measure import Bounds
    from kala_agent.studio.design.mesh import MeshData
    from kala_agent.studio.session_journal import StudioSessionJournal

STEP_HEADER: Final[bytes] = b"ISO-10303-21"
STEP_IMPORT_MAX_BYTES_ENV: Final[str] = "KALA_STEP_IMPORT_MAX_BYTES"
DEFAULT_MAX_STEP_BYTES: Final[int] = 64 * 1024 * 1024
MIN_STEP_IMPORT_BYTES: Final[int] = 1
MAX_STEP_IMPORT_BYTES: Final[int] = 512 * 1024 * 1024
DEFAULT_MESH_DEFLECTION_MM: Final[float] = 0.1
DEFAULT_ANGULAR_DEFLECTION_RAD: Final[float] = 0.5
COARSER_MESH_DEFLECTION_MM: Final[float] = 0.5
COARSER_ANGULAR_DEFLECTION_RAD: Final[float] = 0.8
COARSENED_WARNING: Final[str] = (
    "the imported shape exceeded the default tessellation budget; it was retried "
    f"once with a coarser {COARSER_MESH_DEFLECTION_MM:g} mm deflection"
)
# The worker response validates view bounds through the Studio ``BBox`` model,
# whose coordinate ceiling is one million millimetres. Refuse a shape outside
# that envelope here so it is a typed 4xx rather than a response-validation 500.
MAX_VIEW_COORDINATE_MM: Final[float] = 1_000_000.0
IMPORT_TEMP_DIRECTORY: Final[str] = "imports"
MAX_WARNING_LENGTH: Final[int] = 256

__all__ = (
    "COARSENED_WARNING",
    "COARSER_MESH_DEFLECTION_MM",
    "DEFAULT_MAX_STEP_BYTES",
    "DEFAULT_MESH_DEFLECTION_MM",
    "MAX_STEP_IMPORT_BYTES",
    "MIN_STEP_IMPORT_BYTES",
    "STEP_HEADER",
    "STEP_IMPORT_MAX_BYTES_ENV",
    "StepImportError",
    "StepImportOutcome",
    "StepMalformedError",
    "StepMeshError",
    "StepTooLargeError",
    "import_step_bytes",
    "resolve_step_import_max_bytes",
    "validate_step_import_max_bytes",
)


class StepImportError(ValueError):
    """The STEP bytes could not be imported for display."""

    code: str = "invalid_step"
    message: str = "the STEP file could not be imported"


class StepTooLargeError(StepImportError):
    """The STEP bytes exceed the bounded upload size."""

    code = "payload_too_large"
    message = "the STEP file exceeds the configured size limit"


class StepMalformedError(StepImportError):
    """The bytes are not a readable ISO-10303-21 exchange file."""

    code = "invalid_step"
    message = "the STEP file is malformed or carries no transferable shape"


class StepMeshError(StepImportError):
    """The imported shape cannot be tessellated inside the mesh budget."""

    code = "step_too_complex"
    message = "the imported STEP shape exceeds the mesh limits"


@dataclass(frozen=True, slots=True)
class StepImportOutcome:
    """One view-only import: the stored digest and the display view."""

    step_sha256: str
    mesh: MeshData
    bounds: Bounds
    solids: int
    warnings: tuple[str, ...]
    shape: TopoDS_Shape


def validate_step_import_max_bytes(value: int) -> int:
    """Return a usable STEP import ceiling or raise ``ValueError``."""
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError("the STEP import limit must be an integer number of bytes")
    if not MIN_STEP_IMPORT_BYTES <= value <= MAX_STEP_IMPORT_BYTES:
        raise ValueError(
            f"the STEP import limit must be between {MIN_STEP_IMPORT_BYTES} and "
            f"{MAX_STEP_IMPORT_BYTES} bytes"
        )
    return value


def resolve_step_import_max_bytes(
    environ: Mapping[str, str] | None = None,
    *,
    default: int = DEFAULT_MAX_STEP_BYTES,
) -> int:
    """Resolve the STEP import ceiling from the environment.

    ``KALA_STEP_IMPORT_MAX_BYTES`` is the operator-facing setting. An absent
    or blank value keeps the 64 MiB default; a malformed or out-of-range value
    refuses startup with a clear error rather than silently changing a security
    boundary.
    """
    env = os.environ if environ is None else environ
    raw = env.get(STEP_IMPORT_MAX_BYTES_ENV)
    if raw is None or not raw.strip():
        return validate_step_import_max_bytes(default)
    try:
        value = int(raw.strip())
    except ValueError:
        raise ValueError(
            f"{STEP_IMPORT_MAX_BYTES_ENV} must be an integer number of bytes"
        ) from None
    return validate_step_import_max_bytes(value)


def import_step_bytes(
    data: bytes,
    *,
    journal: StudioSessionJournal | None = None,
    max_step_bytes: int = DEFAULT_MAX_STEP_BYTES,
    max_vertices: int = MAX_MESH_VERTICES,
    max_triangles: int = MAX_MESH_TRIANGLES,
    store_artifact: bool = True,
) -> StepImportOutcome:
    """Read STEP bytes, tessellate them for display, and optionally store them.

    The shape is read and tessellated under the Studio's process-wide geometry
    lock. With ``store_artifact`` (the default), the original bytes are written
    to the session artifact store by content digest and two journal events
    record the result: ``artifact.stored`` for the STEP blob and
    ``view.imported`` for the derived display view. A service that already
    stored the provider's STEP passes ``store_artifact=False`` and journals its
    own provider-labelled events; the view is still the provider's bytes.

    Raises:
        StepTooLargeError: the bytes exceed ``max_step_bytes``.
        StepMalformedError: the reader refuses the file or finds no shape.
        StepMeshError: the triangulation exceeds the mesh budget even after
            the single coarser retry.
    """
    if not isinstance(data, bytes):
        raise TypeError("STEP data must be bytes")
    if not data:
        raise StepMalformedError("the STEP file is empty")
    limit = validate_step_import_max_bytes(max_step_bytes)
    if len(data) > limit:
        raise StepTooLargeError(f"the STEP file must not exceed {limit} bytes")
    if not data.startswith(STEP_HEADER):
        raise StepMalformedError("the file does not start with an ISO-10303-21 header")
    if store_artifact and journal is None:
        raise ValueError("storing a STEP import requires a session journal")

    from kala_agent.studio.design.service import _GEOMETRY_LOCK

    with _GEOMETRY_LOCK:
        mesh, bounds, solids, warnings, shape, coarsened = _read_and_tessellate(
            data,
            journal=journal,
            max_vertices=max_vertices,
            max_triangles=max_triangles,
        )
    if coarsened:
        warnings = (*warnings, COARSENED_WARNING)

    digest = hashlib.sha256(data).hexdigest()
    if store_artifact:
        assert journal is not None
        stored = journal.put_artifact(data)
        if stored != digest:
            raise StepImportError("the stored STEP digest did not match its bytes")
        journal.append(
            "artifact.stored",
            {
                "kind": "step",
                "sha256": stored,
                "size_bytes": len(data),
                "source": "step_import",
            },
            aggregate_id=stored,
        )
        journal.append(
            "view.imported",
            {
                "step_sha256": stored,
                "viewer": "opencascade",
                "role": "view-only",
                "solids": solids,
                "vertices": mesh.vertex_count,
                "triangles": mesh.triangle_count,
                "bounds": {
                    "min": list(bounds.min_mm),
                    "max": list(bounds.max_mm),
                },
                "warnings": list(warnings),
            },
            aggregate_id=stored,
        )
    return StepImportOutcome(
        step_sha256=digest,
        mesh=mesh,
        bounds=bounds,
        solids=solids,
        warnings=warnings,
        shape=shape,
    )


def _read_and_tessellate(
    data: bytes,
    *,
    journal: StudioSessionJournal | None,
    max_vertices: int,
    max_triangles: int,
) -> tuple[MeshData, Bounds, int, tuple[str, ...], TopoDS_Shape, bool]:
    """Read one STEP buffer in a temporary file and tessellate its shape."""
    from OCP.BRepCheck import BRepCheck_Analyzer
    from OCP.IFSelect import IFSelect_RetDone  # type: ignore[import-untyped]
    from OCP.STEPControl import STEPControl_Reader  # type: ignore[import-untyped]
    from OCP.TopAbs import TopAbs_SOLID  # type: ignore[import-untyped]
    from OCP.TopExp import TopExp_Explorer

    from kala_agent.studio.design.measure import Bounds
    from kala_agent.studio.design.mesh import MeshError, tessellate

    import_dir = _import_directory(journal)
    try:
        descriptor, path_str = tempfile.mkstemp(
            prefix="kala-step-import-",
            suffix=".step",
            dir=str(import_dir),
        )
    except OSError as error:
        raise StepImportError(
            f"the STEP file could not be staged: {type(error).__name__}"
        ) from None
    path = Path(path_str)
    try:
        os.close(descriptor)
        try:
            path.write_bytes(data)
        except OSError as error:
            raise StepImportError(
                f"the STEP file could not be staged: {type(error).__name__}"
            ) from None

        reader = STEPControl_Reader()
        try:
            status = reader.ReadFile(str(path))
        except Exception as error:  # pragma: no cover - OCP raises bare exceptions
            raise StepMalformedError(f"the STEP reader failed: {type(error).__name__}") from None
        if status != IFSelect_RetDone:
            raise StepMalformedError("the STEP reader refused the file")
        if int(reader.NbRootsForTransfer()) < 1:
            raise StepMalformedError("the STEP file carries no transferable root")
        try:
            transferred = int(reader.TransferRoots())
        except Exception as error:  # pragma: no cover - OCP raises bare exceptions
            raise StepMalformedError(
                f"the STEP reader could not transfer the shape: {type(error).__name__}"
            ) from None
        if transferred < 1:
            raise StepMalformedError("the STEP reader transferred no shape")
        shape = reader.OneShape()
        if shape is None or shape.IsNull():
            raise StepMalformedError("the STEP file produced an empty shape")

        coarsened = False
        try:
            mesh = tessellate(
                shape,
                max_vertices=max_vertices,
                max_triangles=max_triangles,
            )
        except MeshError:
            try:
                mesh = tessellate(
                    shape,
                    deflection_mm=COARSER_MESH_DEFLECTION_MM,
                    angular_deflection_rad=COARSER_ANGULAR_DEFLECTION_RAD,
                    max_vertices=max_vertices,
                    max_triangles=max_triangles,
                )
            except MeshError as error:
                raise StepMeshError(str(error)) from None
            coarsened = True
        except Exception as error:  # pragma: no cover - OCP raises bare exceptions
            raise StepMeshError(
                f"the geometry kernel could not tessellate the STEP shape: {type(error).__name__}"
            ) from None

        bounds = _shape_bounds(shape, Bounds)
        solids = _solid_count(shape, TopAbs_SOLID, TopExp_Explorer)
        warnings = _warnings(shape, solids, BRepCheck_Analyzer)
        return mesh, bounds, solids, warnings, shape, coarsened
    finally:
        try:
            path.unlink()
        except OSError:
            pass


def _import_directory(journal: StudioSessionJournal | None) -> Path:
    if journal is not None:
        root = journal.data_dir / IMPORT_TEMP_DIRECTORY
    else:
        root = Path(tempfile.gettempdir()) / "kala-studio" / IMPORT_TEMP_DIRECTORY
    try:
        root.mkdir(parents=True, exist_ok=True)
    except OSError as error:
        raise StepImportError(
            f"the STEP import directory is unusable: {type(error).__name__}"
        ) from None
    return root


def _shape_bounds(shape: TopoDS_Shape, bounds_type: type[Bounds]) -> Bounds:
    """Return the imported B-Rep's optimal envelope without mass properties.

    The bounds are the envelope of the imported shape, not the vertices of its
    display triangulation. A tessellated circle's extreme point can lie between
    two triangle vertices, so mesh-vertex bounds under-report a curved edge by
    up to the deflection (0.1 mm); a provider export's envelope is exact. This
    is still a view fact: no volume, area, or centroid is computed here, and
    OpenCascade is never presented as the provider's measuring authority.
    """
    from OCP.Bnd import Bnd_Box  # type: ignore[import-untyped]
    from OCP.BRepBndLib import BRepBndLib  # type: ignore[import-untyped]

    box = Bnd_Box()
    box.SetGap(0.0)
    BRepBndLib.AddOptimal_s(shape, box, False, False)
    if box.IsVoid():
        raise StepMalformedError("the STEP shape has no finite bounding box")
    minimum = box.CornerMin()
    maximum = box.CornerMax()
    coordinates = (
        minimum.X(),
        minimum.Y(),
        minimum.Z(),
        maximum.X(),
        maximum.Y(),
        maximum.Z(),
    )
    if not all(math.isfinite(value) for value in coordinates):
        raise StepMalformedError("the STEP shape has no finite bounding box")
    if not all(abs(value) <= MAX_VIEW_COORDINATE_MM for value in coordinates):
        raise StepMalformedError("the STEP shape is outside the display coordinate range")
    try:
        return bounds_type(
            min_mm=(coordinates[0], coordinates[1], coordinates[2]),
            max_mm=(coordinates[3], coordinates[4], coordinates[5]),
        )
    except ValueError:
        raise StepMalformedError("the STEP shape has an invalid bounding box") from None


def _solid_count(
    shape: TopoDS_Shape,
    solid_type: object,
    explorer_type: type[TopExp_Explorer],
) -> int:
    """Count the solids in the imported shape without measuring it."""
    count = 0
    explorer = explorer_type(shape, solid_type)
    while explorer.More():
        count += 1
        explorer.Next()
    return count


def _warnings(
    shape: TopoDS_Shape,
    solids: int,
    analyzer_type: type[BRepCheck_Analyzer],
) -> tuple[str, ...]:
    """Return honest display warnings about the imported shape."""
    warnings: list[str] = []
    if solids == 0:
        warnings.append("the STEP file contains no solids; only its surfaces are shown")
    try:
        valid = bool(analyzer_type(shape).IsValid())
    except Exception:  # pragma: no cover - OCP raises bare exceptions
        valid = False
    if not valid:
        warnings.append(
            "the imported shape did not pass the kernel validity check; "
            "it is displayed exactly as exported"
        )
    return tuple(warning[:MAX_WARNING_LENGTH] for warning in warnings)
