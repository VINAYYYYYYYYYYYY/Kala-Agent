"""The versioned wire contract of the Solid Edge bridge.

The bridge speaks JSON only. Every request is a :class:`BuildRequest`; every
response is one of :class:`BuildAccepted`, :class:`BuildStatus`,
:class:`BuildResult`, :class:`Health`, or :class:`ErrorResponse`. Artifact
bytes never travel in the request: a request names operations, and a result
names produced artifacts by filename, format, size, and digest. No absolute or
relative disk path is accepted or returned.

Planes follow the measured Solid Edge/FreeCAD convention: ``XY`` has normal
``+Z``, ``XZ`` has normal ``-Y``, and ``YZ`` has normal ``+X``. An extrusion
runs along the plane normal unless ``reversed`` is true. A cut removes material
along the canonical plane normal itself, not the opposite of it, and it has no
``reversed`` flag: its direction is fixed by the contract and is independent of
whether the preceding pad used ``reversed``. ``length=None`` means through-all
in that one direction. A revolve turns around the component-local ``Y`` axis
only; assembly placement is applied separately. Realizing these canonical
directions on a real host still requires native calibration; the contract
states the intended canonical geometry and invents no measured physics.

Wire serialization
------------------
Use ``model_dump_json()`` (or ``Model.model_validate_json()``) for the wire.
Enums serialize as their string values, tuples as JSON arrays, and optional
fields as ``null``. All real-valued fields are finite: ``NaN`` and ``Infinity``
are rejected both on input and in results, and booleans and numeric strings are
never accepted where a real number is expected. Plain JSON integers are
accepted for real-valued fields.

``BuildRequest.canonical_request_bytes()`` is the one deterministic encoding:
UTF-8, sorted object keys, no whitespace, ASCII escapes, and ``request_id``
removed. ``request_id`` is transport correlation only, so a retry of the same
semantic request has the same digest. Everything else, including
``protocol_version``, ``calibration_id``, ``summary``, the ordered procedure,
``timeout_seconds``, and ``density_kg_m3``, is covered.
"""

from __future__ import annotations

import hashlib
import json
import math
from enum import StrEnum
from typing import Annotated, ClassVar, Final, Literal, Self

from pydantic import (
    BaseModel,
    BeforeValidator,
    ConfigDict,
    Field,
    StrictBool,
    StrictInt,
    StringConstraints,
    model_validator,
)

PROTOCOL_VERSION: Final[int] = 1
MAX_PARTS: Final[int] = 8
MAX_OPERATIONS: Final[int] = 600
MIN_SIZE_MM: Final[float] = 0.001
MAX_SIZE_MM: Final[float] = 5_000.0
MAX_OFFSET_MM: Final[float] = 5_000.0
MAX_ANGLE_DEG: Final[float] = 360.0
MIN_TIMEOUT_SECONDS: Final[float] = 1.0
MAX_TIMEOUT_SECONDS: Final[float] = 900.0
MAX_DENSITY_KG_M3: Final[float] = 22_000.0
MAX_TRACE_STEPS: Final[int] = 600
# A full build can save and STEP-export every part (2 per part) and the
# assembly (.asm plus .step), so the cap is derived rather than guessed:
# 2 * MAX_PARTS + 2 == 18 artifacts must always fit.
MAX_ARTIFACTS: Final[int] = 2 * MAX_PARTS + 2
MAX_WARNINGS: Final[int] = 16

_SAFE_ID_PATTERN = r"^[A-Za-z0-9][A-Za-z0-9._:-]*$"
_PART_NAME_PATTERN = r"^[A-Za-z][A-Za-z0-9_-]*$"
_OP_NAME_PATTERN = r"^[a-z][a-z0-9._]*$"
_ARTIFACT_NAME_PATTERN = r"^[A-Za-z0-9][A-Za-z0-9._-]*$"
_MEDIA_TYPE_PATTERN = r"^[a-z0-9][a-z0-9!#$&^_.+-]*/[a-z0-9][a-z0-9!#$&^_.+-]*$"
_SHA256_PATTERN = r"^[0-9a-f]{64}$"
_PLATFORM_PATTERN = r"^[a-z][a-z0-9._-]*$"
_CAPABILITY_PATTERN = r"^[a-z][a-z0-9._-]*$"


def _finite(value: object) -> float:
    """Return a finite float, rejecting booleans, strings, and non-finite numbers."""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError("value must be a finite number, not a boolean or string")
    number = float(value)
    if not math.isfinite(number):
        raise ValueError("value must be finite")
    return number


def _protocol_version(value: object) -> Literal[1]:
    if isinstance(value, bool) or not isinstance(value, int) or value != PROTOCOL_VERSION:
        raise ValueError(f"protocol_version must be the integer {PROTOCOL_VERSION}")
    return 1


def _size_mm(value: object) -> float:
    number = _finite(value)
    if number < MIN_SIZE_MM or number > MAX_SIZE_MM:
        raise ValueError(f"size must be between {MIN_SIZE_MM} and {MAX_SIZE_MM} mm")
    return number


def _offset_mm(value: object) -> float:
    number = _finite(value)
    if abs(number) > MAX_OFFSET_MM:
        raise ValueError(f"offset must be between -{MAX_OFFSET_MM} and {MAX_OFFSET_MM} mm")
    return number


def _angle_deg(value: object) -> float:
    number = _finite(value)
    if number <= 0.0 or number > MAX_ANGLE_DEG:
        raise ValueError(f"angle must be greater than 0 and at most {MAX_ANGLE_DEG} degrees")
    return number


def _timeout_seconds(value: object) -> float:
    number = _finite(value)
    if number < MIN_TIMEOUT_SECONDS or number > MAX_TIMEOUT_SECONDS:
        raise ValueError("timeout_seconds must be between 1 and 900 seconds")
    return number


def _density_kg_m3(value: object) -> float:
    number = _finite(value)
    if number <= 0.0 or number > MAX_DENSITY_KG_M3:
        raise ValueError(f"density_kg_m3 must be greater than 0 and at most {MAX_DENSITY_KG_M3}")
    return number


def _nonnegative_float(value: object) -> float:
    number = _finite(value)
    if number < 0.0:
        raise ValueError("value must be nonnegative")
    return number


FiniteFloat = Annotated[float, BeforeValidator(_finite)]
SizeMm = Annotated[float, BeforeValidator(_size_mm)]
OffsetMm = Annotated[float, BeforeValidator(_offset_mm)]
AngleDeg = Annotated[float, BeforeValidator(_angle_deg)]
TimeoutSeconds = Annotated[float, BeforeValidator(_timeout_seconds)]
DensityKgM3 = Annotated[float, BeforeValidator(_density_kg_m3)]
NonNegativeFloat = Annotated[float, BeforeValidator(_nonnegative_float)]
NonNegativeInt = Annotated[StrictInt, Field(ge=0)]
ProtocolVersion = Annotated[Literal[1], BeforeValidator(_protocol_version)]
SafeId = Annotated[
    str,
    StringConstraints(min_length=1, max_length=64, pattern=_SAFE_ID_PATTERN),
]
CalibrationId = Annotated[
    str,
    StringConstraints(min_length=1, max_length=128, pattern=_SAFE_ID_PATTERN),
]
PartName = Annotated[
    str,
    StringConstraints(min_length=1, max_length=40, pattern=_PART_NAME_PATTERN),
]
OpName = Annotated[
    str,
    StringConstraints(min_length=1, max_length=64, pattern=_OP_NAME_PATTERN),
]
Summary = Annotated[str, StringConstraints(max_length=500)]
Detail = Annotated[str, StringConstraints(max_length=500)]
Warning = Annotated[str, StringConstraints(max_length=500)]
VersionText = Annotated[str, StringConstraints(min_length=1, max_length=64)]
Platform = Annotated[
    str,
    StringConstraints(min_length=1, max_length=32, pattern=_PLATFORM_PATTERN),
]
Capability = Annotated[
    str,
    StringConstraints(min_length=1, max_length=64, pattern=_CAPABILITY_PATTERN),
]
ArtifactName = Annotated[
    str,
    StringConstraints(min_length=1, max_length=128, pattern=_ARTIFACT_NAME_PATTERN),
]
MediaType = Annotated[
    str,
    StringConstraints(min_length=3, max_length=127, pattern=_MEDIA_TYPE_PATTERN),
]
Sha256 = Annotated[
    str,
    StringConstraints(min_length=64, max_length=64, pattern=_SHA256_PATTERN),
]
MmTriple = tuple[FiniteFloat, FiniteFloat, FiniteFloat]


class Plane(StrEnum):
    """The measured sketch-plane orientation and its canonical normal."""

    XY = "XY"
    XZ = "XZ"
    YZ = "YZ"


class JobState(StrEnum):
    """The only states a bridge job may report."""

    QUEUED = "queued"
    RUNNING = "running"
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    CANCELLED = "cancelled"
    INDETERMINATE = "indeterminate"
    INTERRUPTED = "interrupted"


class ArtifactFormat(StrEnum):
    """Artifact formats the bridge may publish."""

    PAR = "par"
    ASM = "asm"
    STEP = "step"


class ErrorCode(StrEnum):
    """Stable error codes. An unknown code is never accepted on the wire."""

    INVALID_REQUEST = "invalid_request"
    INVALID_SEQUENCE = "invalid_sequence"
    UNKNOWN_OPERATION = "unknown_operation"
    UNSUPPORTED_OPERATION = "unsupported_operation"
    UNAVAILABLE = "unavailable"
    CALIBRATION_REQUIRED = "calibration_required"
    BUSY = "busy"
    CONFLICT = "conflict"
    TIMEOUT_INDETERMINATE = "timeout_indeterminate"
    CANCELLED = "cancelled"
    INTERNAL_ERROR = "internal_error"


class _BridgeModel(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(
        extra="forbid",
        frozen=True,
        validate_default=True,
    )


class PartNew(_BridgeModel):
    op: Literal["part.new"] = "part.new"
    name: PartName


class SketchRectangle(_BridgeModel):
    op: Literal["sketch.rectangle"] = "sketch.rectangle"
    plane: Plane
    width: SizeMm
    height: SizeMm
    x: OffsetMm = 0
    y: OffsetMm = 0


class SketchCircle(_BridgeModel):
    op: Literal["sketch.circle"] = "sketch.circle"
    plane: Plane
    diameter: SizeMm
    x: OffsetMm = 0
    y: OffsetMm = 0


class SolidExtrude(_BridgeModel):
    op: Literal["solid.extrude"] = "solid.extrude"
    length: SizeMm
    reversed: StrictBool = False


class SolidCut(_BridgeModel):
    """Remove material along the canonical sketch-plane normal.

    The direction is the plane's canonical normal (``+Z`` for ``XY``, ``-Y``
    for ``XZ``, ``+X`` for ``YZ``). It is fixed by the contract and is
    independent of whether the preceding extrusion used ``reversed``.
    ``length=None`` means through-all. There is deliberately no ``reversed``
    field; a native executor must calibrate this canonical direction before it
    can build a through-cut.
    """

    op: Literal["solid.cut"] = "solid.cut"
    length: SizeMm | None = None


class SolidRevolve(_BridgeModel):
    """Revolve a profile around the component-local ``Y`` axis.

    The axis is local to the part; assembly placement is applied separately and
    is never part of this operation.
    """

    op: Literal["solid.revolve"] = "solid.revolve"
    angle: AngleDeg = 360
    axis: Literal["Y"] = "Y"


class PartMeasure(_BridgeModel):
    op: Literal["part.measure"] = "part.measure"


class PartSave(_BridgeModel):
    op: Literal["part.save"] = "part.save"


class PartExportStep(_BridgeModel):
    op: Literal["part.export_step"] = "part.export_step"


class AssemblyNew(_BridgeModel):
    op: Literal["assembly.new"] = "assembly.new"
    name: PartName


class AssemblyPlace(_BridgeModel):
    op: Literal["assembly.place"] = "assembly.place"
    component: PartName
    x: OffsetMm = 0
    y: OffsetMm = 0
    z: OffsetMm = 0


class AssemblySave(_BridgeModel):
    op: Literal["assembly.save"] = "assembly.save"


class AssemblyExportStep(_BridgeModel):
    op: Literal["assembly.export_step"] = "assembly.export_step"


BridgeOp = Annotated[
    PartNew
    | SketchRectangle
    | SketchCircle
    | SolidExtrude
    | SolidCut
    | SolidRevolve
    | PartMeasure
    | PartSave
    | PartExportStep
    | AssemblyNew
    | AssemblyPlace
    | AssemblySave
    | AssemblyExportStep,
    Field(discriminator="op"),
]


def _validate_procedure(procedure: tuple[BridgeOp, ...]) -> None:
    """Reject any procedure that would compile into the wrong feature order.

    Part selection is implicit: the ``part.new`` that most recently succeeded
    is the current part. No operation can name a document or arbitrate over a
    part other than that current part, except ``assembly.place``, which may
    reference only a finished part.

    A part may carry several additive features: the first extrude/revolve
    creates its solid, and a later profile+extrude/revolve extends the solid
    that already exists. This validator only fixes ordering and profile
    consumption; how a native executor chains the additive calls is its own
    concern and is not a reason to shrink the wire capability here.
    """
    created: dict[str, bool] = {}
    current: str | None = None
    profile_pending = False
    solid_built = False
    assembly_started = False
    placed: set[str] = set()

    for index, op in enumerate(procedure):
        where = f"operation {index} ({op.op})"
        if assembly_started and not isinstance(
            op, (AssemblyPlace, AssemblySave, AssemblyExportStep)
        ):
            raise ValueError(f"{where} cannot follow assembly.new")

        if isinstance(op, PartNew):
            if profile_pending:
                raise ValueError(f"{where}: finish the pending profile before a new part")
            if current is not None and not solid_built:
                raise ValueError(f"{where}: part {current!r} has no solid yet")
            if op.name in created:
                raise ValueError(f"{where}: duplicate part name {op.name!r}")
            if len(created) >= MAX_PARTS:
                raise ValueError(f"{where}: at most {MAX_PARTS} parts are allowed")
            created[op.name] = False
            current = op.name
            profile_pending = False
            solid_built = False
            continue

        if isinstance(op, (SketchRectangle, SketchCircle)):
            if current is None:
                raise ValueError(f"{where}: a sketch requires a current part")
            if profile_pending:
                raise ValueError(f"{where}: consume the pending profile before another sketch")
            profile_pending = True
            continue

        if isinstance(op, (SolidExtrude, SolidRevolve)):
            if current is None:
                raise ValueError(f"{where}: a solid requires a current part")
            if not profile_pending:
                raise ValueError(f"{where}: a solid requires a pending profile")
            # The first additive feature creates the part's solid; later ones
            # extend the solid that already exists, so its state is preserved
            # rather than rejected or reset.
            profile_pending = False
            solid_built = True
            created[current] = True
            continue

        if isinstance(op, SolidCut):
            if current is None or not solid_built:
                raise ValueError(f"{where}: a cut requires an existing solid")
            if not profile_pending:
                raise ValueError(f"{where}: a cut requires a pending profile")
            profile_pending = False
            continue

        if isinstance(op, (PartMeasure, PartSave, PartExportStep)):
            if current is None or not solid_built:
                raise ValueError(f"{where}: the current part has no solid")
            if profile_pending:
                raise ValueError(f"{where}: consume the pending profile before {op.op}")
            continue

        if isinstance(op, AssemblyNew):
            if assembly_started:
                raise ValueError(f"{where}: only one assembly may be created")
            if current is None or not solid_built:
                raise ValueError(f"{where}: an assembly requires at least one finished part")
            assembly_started = True
            continue

        if isinstance(op, AssemblyPlace):
            if not assembly_started:
                raise ValueError(f"{where}: assembly.place requires assembly.new first")
            if op.component not in created or not created[op.component]:
                raise ValueError(f"{where}: {op.component!r} is not a finished part")
            if op.component in placed:
                raise ValueError(f"{where}: {op.component!r} is already placed")
            placed.add(op.component)
            continue

        if isinstance(op, (AssemblySave, AssemblyExportStep)):
            if not assembly_started:
                raise ValueError(f"{where}: the assembly has not been created")
            if not placed:
                raise ValueError(f"{where}: place at least one component before saving")
            continue

        raise ValueError(f"{where}: unknown operation")

    if current is None:
        raise ValueError("the procedure must create a part")
    if profile_pending:
        raise ValueError("the procedure ends with an unconsumed profile")
    if not solid_built:
        raise ValueError("the procedure ends with a part that has no solid")


class BuildRequest(_BridgeModel):
    """One bounded, deterministic build request.

    ``procedure`` is capped at ``MAX_OPERATIONS`` independently of the core
    construction IR, which allows up to 500 steps in one component and 500 in
    total, and at ``MAX_PARTS`` parts. The bridge never receives artifact
    paths; ``density_kg_m3`` is optional and, when present, makes mass fields
    meaningful in :class:`PartResult`.
    """

    protocol_version: ProtocolVersion = 1
    request_id: SafeId
    calibration_id: CalibrationId | None = None
    summary: Summary = ""
    procedure: tuple[BridgeOp, ...] = Field(min_length=1, max_length=MAX_OPERATIONS)
    timeout_seconds: TimeoutSeconds = 120
    density_kg_m3: DensityKgM3 | None = None

    @model_validator(mode="after")
    def check_procedure(self) -> Self:
        _validate_procedure(self.procedure)
        return self

    def canonical_request_bytes(self) -> bytes:
        """Return the deterministic digest input, excluding ``request_id``."""
        payload = self.model_dump(mode="json", exclude={"request_id"})
        encoded = json.dumps(
            payload,
            allow_nan=False,
            ensure_ascii=True,
            separators=(",", ":"),
            sort_keys=True,
        )
        return encoded.encode("utf-8")

    def request_digest(self) -> str:
        """Return the lowercase SHA-256 hex digest of the canonical request."""
        return hashlib.sha256(self.canonical_request_bytes()).hexdigest()


class StepTrace(_BridgeModel):
    seq: NonNegativeInt
    op: OpName
    status: Annotated[
        str,
        StringConstraints(min_length=1, max_length=32, pattern=r"^[a-z][a-z0-9._-]*$"),
    ]
    message: Detail = ""
    duration_ms: NonNegativeFloat


class RangeBox(_BridgeModel):
    """An axis-aligned range in part-local coordinates, in millimetres."""

    min_mm: MmTriple
    max_mm: MmTriple

    @model_validator(mode="after")
    def check_order(self) -> Self:
        if any(low > high for low, high in zip(self.min_mm, self.max_mm, strict=True)):
            raise ValueError("min_mm must be less than or equal to max_mm on every axis")
        return self


class PartResult(_BridgeModel):
    """Local part metrics. ``mass_kg`` is set only for an explicit density."""

    name: PartName
    valid: StrictBool
    volume_mm3: NonNegativeFloat
    surface_area_mm2: NonNegativeFloat
    range_box: RangeBox
    mass_kg: NonNegativeFloat | None = None


class Artifact(_BridgeModel):
    """A produced file named by metadata only; disk paths are never exchanged."""

    name: ArtifactName
    format: ArtifactFormat
    media_type: MediaType
    size_bytes: NonNegativeInt
    sha256: Sha256


class ErrorResponse(_BridgeModel):
    code: ErrorCode
    message: Annotated[str, StringConstraints(min_length=1, max_length=500)]


class BuildResult(_BridgeModel):
    build_id: SafeId
    state: JobState
    parts: tuple[PartResult, ...] = Field(default=(), max_length=MAX_PARTS)
    assembly_bounds: RangeBox | None = None
    artifacts: tuple[Artifact, ...] = Field(default=(), max_length=MAX_ARTIFACTS)
    trace: tuple[StepTrace, ...] = Field(default=(), max_length=MAX_TRACE_STEPS)
    warnings: tuple[Warning, ...] = Field(default=(), max_length=MAX_WARNINGS)
    error: ErrorResponse | None = None


class BuildStatus(_BridgeModel):
    build_id: SafeId
    state: JobState
    step_index: Annotated[StrictInt, Field(ge=0, le=MAX_OPERATIONS)]
    total: Annotated[StrictInt, Field(ge=1, le=MAX_OPERATIONS)]
    trace: tuple[StepTrace, ...] = Field(default=(), max_length=MAX_TRACE_STEPS)
    result: BuildResult | None = None
    message: Detail | None = None


class Health(_BridgeModel):
    """Honest bridge health.

    ``capabilities`` are bounded names, never a readiness claim: a capability
    name does not mean Solid Edge can build anything on this host.
    """

    protocol_version: ProtocolVersion = 1
    bridge_version: VersionText
    platform: Platform
    supported: StrictBool
    available: StrictBool
    busy: StrictBool
    se_version: VersionText | None = None
    calibration_id: CalibrationId | None = None
    capabilities: tuple[Capability, ...] = Field(default=(), max_length=32)
    detail: Detail = ""


class BuildAccepted(_BridgeModel):
    build_id: SafeId
    state: JobState = JobState.QUEUED


__all__ = (
    "MAX_ANGLE_DEG",
    "MAX_ARTIFACTS",
    "MAX_DENSITY_KG_M3",
    "MAX_OFFSET_MM",
    "MAX_OPERATIONS",
    "MAX_PARTS",
    "MAX_SIZE_MM",
    "MAX_TIMEOUT_SECONDS",
    "MAX_TRACE_STEPS",
    "MAX_WARNINGS",
    "MIN_SIZE_MM",
    "MIN_TIMEOUT_SECONDS",
    "PROTOCOL_VERSION",
    "AngleDeg",
    "Artifact",
    "ArtifactFormat",
    "ArtifactName",
    "AssemblyExportStep",
    "AssemblyNew",
    "AssemblyPlace",
    "AssemblySave",
    "BridgeOp",
    "BuildAccepted",
    "BuildRequest",
    "BuildResult",
    "BuildStatus",
    "CalibrationId",
    "Capability",
    "DensityKgM3",
    "ErrorCode",
    "ErrorResponse",
    "FiniteFloat",
    "Health",
    "JobState",
    "MediaType",
    "MmTriple",
    "NonNegativeFloat",
    "NonNegativeInt",
    "OffsetMm",
    "OpName",
    "PartExportStep",
    "PartMeasure",
    "PartName",
    "PartNew",
    "PartResult",
    "PartSave",
    "Plane",
    "ProtocolVersion",
    "RangeBox",
    "SafeId",
    "Sha256",
    "SizeMm",
    "SketchCircle",
    "SketchRectangle",
    "SolidCut",
    "SolidExtrude",
    "SolidRevolve",
    "StepTrace",
    "Summary",
    "TimeoutSeconds",
    "VersionText",
    "Warning",
)
