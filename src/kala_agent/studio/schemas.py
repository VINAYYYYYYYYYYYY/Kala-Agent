"""Pydantic request and response models for the studio HTTP surface.

Every request body the studio accepts is strict: extras are refused, numeric
values are finite, strings are size-bounded, and unknown ``kind``/``format``
values are a validation failure rather than a silently accepted default. The
responses mirror the frozen views the studio functions produced so the HTTP
surface can never widen, rename, or reinterpret a result.
"""

from __future__ import annotations

from collections.abc import Mapping
from enum import StrEnum
from typing import Annotated, Final, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator

from kala_agent.studio.design.intent import DesignIntent
from kala_agent.studio.design.providers.base import (
    ALLOWED_PROVIDER_IDS,
    CadProviderId,
)
from kala_agent.studio.geometry.limits import (
    MAX_DIMENSION_MM,
    MIN_DIMENSION_MM,
)

MAX_BBOX_COUNT: Final[int] = 6
MAX_INTEGRATION_NAME_LENGTH: Final[int] = 128
MAX_PROMPT_LENGTH: Final[int] = 4_096
MAX_MODEL_ID_LENGTH: Final[int] = 256
MAX_KIND_LENGTH: Final[int] = 32
MAX_FORMAT_LENGTH: Final[int] = 16
MAX_PROVIDER_ID_LENGTH: Final[int] = 64
MAX_PROVIDER_NAME_LENGTH: Final[int] = 128
MAX_PLATFORM_LENGTH: Final[int] = 64
MAX_CAPABILITY_LENGTH: Final[int] = 64
MAX_NOTES_LENGTH: Final[int] = 1_024
MAX_LABEL_LENGTH: Final[int] = 120
MAX_PARAMETERS_ENTRIES: Final[int] = 32
MAX_PARAMETER_KEY_LENGTH: Final[int] = 64
MAX_PARAMETER_STRING_VALUE_LENGTH: Final[int] = 128
MAX_BBOX_NUMBER: Final[float] = 1_000_000.0
INTEGRATION_ID_PATTERN: Final[str] = r"^[a-z0-9][a-z0-9._-]{0,63}$"
PROVIDER_ID_PATTERN: Final[str] = r"^[a-z0-9][a-z0-9._-]{0,63}$"
MODEL_ID_PATTERN: Final[str] = r"^[A-Za-z0-9._:/-]{1,256}$"
DESIGN_ID_PATTERN: Final[str] = r"^[0-9a-f]{64}$"
SERVICE_NAME: Final[str] = "kala-studio"
MAX_JOURNAL_LIMIT: Final[int] = 500
MAX_DRAWING_VIEWS: Final[int] = 4
MAX_SECTION_OFFSET_MM: Final[float] = 10_000.0

DesignId = Annotated[
    str,
    Field(
        min_length=64,
        max_length=64,
        pattern=DESIGN_ID_PATTERN,
        description="The sha256 of the canonical ConstructionScript JSON.",
    ),
]
CheckValue = bool | float | tuple[float, float]


class PrimitiveKind(StrEnum):
    """The primitive kinds the studio mesh generator supports."""

    BOX = "box"
    CYLINDER = "cylinder"
    SPHERE = "sphere"


class ExportFormat(StrEnum):
    """The file formats the studio can render a preview to."""

    STL = "stl"
    STEP = "step"


class IntegrationCategory(StrEnum):
    """The two integration families the registry distinguishes."""

    CAD = "cad"
    ANALYSIS = "analysis"


class IntegrationStatus(StrEnum):
    """The honest capability status a registry entry exposes.

    ``available_via_neutral_file`` records a working capability the studio
    supports today. ``native_api_unavailable`` and ``not_configured`` admit
    that the integration is not actually wired, so the UI and tests can never
    mistake "we'd love to support this" for "this works".
    """

    AVAILABLE_VIA_NEUTRAL_FILE = "available_via_neutral_file"
    NATIVE_API_UNAVAILABLE = "native_api_unavailable"
    NOT_CONFIGURED = "not_configured"


class ProviderId(StrEnum):
    """The provider identifiers the studio chat surface recognises.

    ``codex`` is the explicit ChatGPT subscription route, ``openai`` and
    ``openrouter`` are the API routes, and ``local`` is the deterministic
    offline design helper. Unknown identifiers are a validation failure
    rather than a silent no-op.

    ``claude_code``, ``codex_cli`` and ``opencode_cli`` are the three
    subscription routes of the native Studio product: each one runs the
    operator's own, already-authenticated command-line tool as a sandboxed
    subprocess. The Python reference runs the same three CLIs with the same
    contract, so a design turn can be built through any of them.
    """

    LOCAL = "local"
    OPENAI = "openai"
    OPENROUTER = "openrouter"
    CODEX = "codex"
    CLAUDE_CODE = "claude_code"
    CODEX_CLI = "codex_cli"
    OPENCODE_CLI = "opencode_cli"


class CredentialProviderId(StrEnum):
    """Providers whose API keys can be configured in Studio settings."""

    OPENAI = "openai"
    OPENROUTER = "openrouter"


class CredentialSource(StrEnum):
    """Where a provider's active API key is sourced from."""

    NONE = "none"
    ENVIRONMENT = "environment"
    STORED = "stored"
    SUBSCRIPTION = "subscription"


class _StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)


class _RequestModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, str_strip_whitespace=True)


class DesignRequest(DesignIntent):
    """A design intent plus the provider that must build it.

    ``provider`` is a required field by ADR-0011. A missing value is a 422
    whose detail names every allowed value, which a plain "field required"
    framework error would not.
    """

    provider: CadProviderId

    @model_validator(mode="before")
    @classmethod
    def check_provider_selected(cls, data: object) -> object:
        if isinstance(data, Mapping) and "provider" not in data:
            allowed = ", ".join(ALLOWED_PROVIDER_IDS)
            raise ValueError(f"provider is required (choose {allowed})")
        return data


def _validate_parameters_block(value: dict[str, object] | None) -> dict[str, object] | None:
    """Validate a primitive parameters block.

    The studio only ever feeds a finite, non-empty numbers-as-strings-or-floats
    object into its own geometry code. Booleans are refused because Python
    silently casts them through ``int`` and a stray True could double a size
    calculation. Lists and nested objects are refused because the studio's
    geometry is keyed on a fixed set of numbers per primitive.
    """
    if value is None:
        return None
    if len(value) > MAX_PARAMETERS_ENTRIES:
        raise ValueError(f"parameters cannot declare more than {MAX_PARAMETERS_ENTRIES} entries")
    for key in value:
        if not key or len(key) > MAX_PARAMETER_KEY_LENGTH:
            raise ValueError("parameter names must be nonempty and bounded")
        entry = value[key]
        if isinstance(entry, bool):
            raise ValueError("parameter values must not be booleans")
        if isinstance(entry, (int, float)):
            continue
        if isinstance(entry, str):
            if not entry or len(entry) > MAX_PARAMETER_STRING_VALUE_LENGTH:
                raise ValueError("string parameter values are bounded")
            continue
        raise ValueError("parameter values must be numbers or bounded strings")
    return value


ParametersField = Annotated[
    dict[str, object] | None,
    Field(
        default=None,
        description=(
            "Parameters for the requested primitive. Only numbers and bounded "
            "strings are accepted; list and object values are refused."
        ),
    ),
]


class PreviewRequest(_RequestModel):
    """Ask the studio for a parametric mesh preview."""

    kind: PrimitiveKind
    parameters: ParametersField = None

    @field_validator("parameters")
    @classmethod
    def check_parameters(cls, value: dict[str, object] | None) -> dict[str, object] | None:
        return _validate_parameters_block(value)


class ExportRequest(_RequestModel):
    """Ask the studio to render a primitive or a stored design into a file.

    Exactly one source is required: a primitive ``kind`` with optional
    ``parameters``, or a stored ``design_id``. Mixing the two is refused
    rather than silently preferring one.
    """

    kind: PrimitiveKind | None = None
    parameters: ParametersField = None
    format: ExportFormat
    design_id: DesignId | None = None

    @field_validator("parameters")
    @classmethod
    def check_parameters(cls, value: dict[str, object] | None) -> dict[str, object] | None:
        return _validate_parameters_block(value)

    @model_validator(mode="after")
    def check_source(self) -> ExportRequest:
        if (self.kind is None) == (self.design_id is None):
            raise ValueError("exactly one of kind or design_id is required")
        if self.design_id is not None and self.parameters is not None:
            raise ValueError("parameters are only valid together with a primitive kind")
        return self


class HandoffRequest(_RequestModel):
    """Ask the studio to render a neutral-file handoff for a CAD target."""

    integration_id: Annotated[
        str,
        Field(
            min_length=1,
            max_length=64,
            pattern=INTEGRATION_ID_PATTERN,
            description="Identifier of the integration to hand off to.",
        ),
    ]
    kind: PrimitiveKind
    parameters: ParametersField = None
    format: ExportFormat

    @field_validator("parameters")
    @classmethod
    def check_parameters(cls, value: dict[str, object] | None) -> dict[str, object] | None:
        return _validate_parameters_block(value)


class ChatRequest(_RequestModel):
    """Ask a chat provider to elaborate a short prompt.

    The studio promises nothing about model output beyond the bounded buffer
    the route returns. ``model`` is required for API providers; the local
    provider ignores it. ``allow_remote`` is opt-in and gates every external
    HTTPS request this surface makes.
    """

    provider: ProviderId
    model: Annotated[
        str | None,
        Field(
            default=None,
            max_length=MAX_MODEL_ID_LENGTH,
            pattern=MODEL_ID_PATTERN,
        ),
    ] = None
    prompt: Annotated[
        str,
        Field(min_length=1, max_length=MAX_PROMPT_LENGTH),
    ]
    allow_remote: bool = False
    cad_provider: CadProviderId | None = None


class UpdateProviderKeyRequest(BaseModel):
    """Store or clear an API key for a remote chat provider in Studio settings."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    provider: CredentialProviderId
    api_key: Annotated[str | None, Field(default=None, max_length=256)] = None


class IntegrationRecord(_StrictModel):
    """One row in the integration registry.

    ``platforms`` describes where the tool runs (operating system and
    licensing model) so the UI can surface honest setup notes. ``status``
    never claims native execution the studio cannot perform; ``capabilities``
    lists the neutral-file formats the studio can hand off to.
    """

    id: Annotated[str, Field(min_length=1, max_length=64, pattern=INTEGRATION_ID_PATTERN)]
    name: Annotated[str, Field(min_length=1, max_length=MAX_INTEGRATION_NAME_LENGTH)]
    category: IntegrationCategory
    platforms: tuple[Annotated[str, Field(min_length=1, max_length=MAX_PLATFORM_LENGTH)], ...]
    status: IntegrationStatus
    capabilities: tuple[Annotated[str, Field(min_length=1, max_length=MAX_CAPABILITY_LENGTH)], ...]
    notes: Annotated[str, Field(max_length=MAX_NOTES_LENGTH)] = ""


class IntegrationsResponse(_StrictModel):
    integrations: tuple[IntegrationRecord, ...]


class ProviderRecord(_StrictModel):
    """One row in the provider list.

    ``configured`` is reported by inspecting what the environment actually
    exposes, so it can never claim an API key that is not set.

    ``available`` is the availability probe's verdict for a subscription
    route and is ``null`` for a route the studio did not probe. ``reason``
    names the honest reason a subscription route is unavailable, and
    ``source`` names where the credential comes from; a subscription route's
    credential lives in the operator's own CLI, never in Kala.
    """

    id: Annotated[str, Field(min_length=1, max_length=64, pattern=PROVIDER_ID_PATTERN)]
    name: Annotated[str, Field(min_length=1, max_length=MAX_PROVIDER_NAME_LENGTH)]
    configured: bool
    notes: Annotated[str, Field(max_length=MAX_NOTES_LENGTH)] = ""
    available: bool | None = None
    reason: Annotated[str | None, Field(max_length=MAX_NOTES_LENGTH)] = None
    source: Annotated[str | None, Field(max_length=64)] = None


class ProvidersResponse(_StrictModel):
    providers: tuple[ProviderRecord, ...]


class ProviderCredentialStatus(_StrictModel):
    """Non-secret status of a configurable provider credential.

    The two API-key rows name the environment variable the key comes from;
    a subscription row names no variable at all, because its credential is
    the operator's own CLI login.
    """

    provider: Annotated[str, Field(min_length=1, max_length=64)]
    env_var: Annotated[str, Field(max_length=64)] = ""
    configured: bool
    source: CredentialSource


class StudioSettingsResponse(_StrictModel):
    """Current non-secret provider credential settings and resulting provider list."""

    credentials: tuple[ProviderCredentialStatus, ...]
    providers: tuple[ProviderRecord, ...]
    offline: bool


class BBox(_StrictModel):
    """An axis-aligned bounding box expressed in millimetres."""

    min: Annotated[tuple[float, float, float], Field(min_length=3, max_length=3)]
    max: Annotated[tuple[float, float, float], Field(min_length=3, max_length=3)]

    @field_validator("min", "max")
    @classmethod
    def check_finite(cls, value: tuple[float, float, float]) -> tuple[float, float, float]:
        for component in value:
            if isinstance(component, bool) or not isinstance(component, (int, float)):
                raise ValueError("bounding box coordinates must be finite numbers")
            if abs(component) > MAX_BBOX_NUMBER:
                raise ValueError("bounding box coordinates are out of range")
        return (float(value[0]), float(value[1]), float(value[2]))


class PreviewResponse(_StrictModel):
    """The mesh returned by the studio preview pipeline."""

    positions: tuple[float, ...]
    normals: tuple[float, ...]
    indices: tuple[int, ...]
    bounds: BBox
    engine: Annotated[str, Field(min_length=1, max_length=64)]
    warnings: tuple[Annotated[str, Field(min_length=1, max_length=256)], ...]


class ChatResponse(_StrictModel):
    """A short model reply, never a credential or a long traceback.

    ``design`` is present only when the turn produced a compiled, measured,
    verified design. Its absence is the honest "no design was built" signal.
    """

    text: Annotated[str, Field(min_length=0, max_length=MAX_PROMPT_LENGTH)]
    provider: ProviderId
    model: Annotated[str | None, Field(default=None, max_length=MAX_MODEL_ID_LENGTH)] = None
    offline: bool
    design: DesignResult | None = None


class HealthResponse(_StrictModel):
    """Process liveness, with no credentials or endpoints echoed."""

    status: Annotated[str, Field(min_length=1, max_length=64)] = "ok"
    service: Annotated[str, Field(min_length=1, max_length=64)] = SERVICE_NAME
    version: Annotated[str, Field(min_length=1, max_length=64)]
    offline: bool
    #: The backend's own worst-case design-turn wall time, in seconds. The
    #: client derives its deadline from this value plus a margin instead of
    #: pinning one that the design budget can outgrow.
    design_turn_budget_seconds: Annotated[int, Field(ge=1, le=86_400)]
    step_available: bool
    design_available: bool = False
    integrations: tuple[str, ...]
    providers: tuple[str, ...]


class DesignBounds(_StrictModel):
    """An axis-aligned envelope in millimetres."""

    min: Annotated[tuple[float, float, float], Field(min_length=3, max_length=3)]
    max: Annotated[tuple[float, float, float], Field(min_length=3, max_length=3)]

    @field_validator("min", "max")
    @classmethod
    def check_finite(cls, value: tuple[float, float, float]) -> tuple[float, float, float]:
        for component in value:
            if isinstance(component, bool) or not isinstance(component, (int, float)):
                raise ValueError("envelope coordinates must be finite numbers")
            if abs(float(component)) > MAX_BBOX_NUMBER:
                raise ValueError("envelope coordinates are out of range")
        return (float(value[0]), float(value[1]), float(value[2]))


class DesignMeasured(_StrictModel):
    """Every provider-measured number of one design.

    A provider that does not report a property leaves it ``null``. The UI
    shows an em dash for that value; OpenCascade never substitutes one.
    """

    volume_mm3: float | None = None
    surface_area_mm2: float | None = None
    centroid_mm: Annotated[tuple[float, float, float], Field(min_length=3, max_length=3)] | None = (
        None
    )
    bounds: DesignBounds | None = None
    size_mm: Annotated[tuple[float, float, float], Field(min_length=3, max_length=3)] | None = None
    valid: bool | None = None
    solid_count: Annotated[int, Field(ge=0)] | None = None
    mass_g: float | None = None


class DesignCheck(_StrictModel):
    """One declared-versus-measured check row.

    ``status`` distinguishes an actual failure from a value the provider did
    not report. ``passed`` is false in both cases so a single boolean remains
    safe for a client that ignores ``status``.
    """

    id: Annotated[str, Field(min_length=1, max_length=64)]
    label: Annotated[str, Field(min_length=1, max_length=128)]
    expected: CheckValue
    measured: CheckValue | None = None
    tolerance: float
    unit: Annotated[str, Field(min_length=1, max_length=16)]
    passed: bool
    status: Literal["passed", "failed", "not_reported"] = "passed"


class DesignMesh(_StrictModel):
    """A bounded OCP triangulation of the compiled shape."""

    positions: tuple[float, ...]
    normals: tuple[float, ...]
    indices: tuple[int, ...]
    vertex_count: Annotated[int, Field(ge=0)]
    triangle_count: Annotated[int, Field(ge=0)]
    deflection_mm: float
    engine: Annotated[str, Field(min_length=1, max_length=32)]


class DesignView(DesignMesh):
    """The display view imported from the provider's exact STEP bytes."""

    bounds: DesignBounds
    solids: Annotated[int, Field(ge=0)]
    warnings: tuple[Annotated[str, Field(min_length=1, max_length=256)], ...] = ()


class DesignArtifacts(_StrictModel):
    """The content digests stored for a design.

    ``step_sha256`` is the provider's STEP export. ``native_sha256`` is the
    provider's native file (``fcstd`` for FreeCAD); it is null for the
    OpenCascade builder, which has no separate native format.
    """

    script_sha256: DesignId
    step_sha256: Annotated[str | None, Field(default=None, max_length=64)] = None
    native_sha256: Annotated[str | None, Field(default=None, max_length=64)] = None
    native_kind: Annotated[str | None, Field(default=None, max_length=16)] = None


class DesignResult(_StrictModel):
    """The complete, provider-measured response to a design turn."""

    design_id: DesignId
    label: Annotated[str, Field(min_length=1, max_length=MAX_LABEL_LENGTH)]
    provider: CadProviderId
    provider_version: Annotated[str | None, Field(default=None, max_length=64)] = None
    measured_by: Annotated[str, Field(min_length=1, max_length=64)]
    script: dict[str, object]
    measured: DesignMeasured
    checks: tuple[DesignCheck, ...]
    verified: bool
    view: DesignView
    artifacts: DesignArtifacts
    journal_head: Annotated[str | None, Field(default=None, max_length=64)] = None
    material: Annotated[str | None, Field(default=None, max_length=64)] = None
    tolerances: tuple[Annotated[str, Field(min_length=1, max_length=64)], ...] = ()
    declared_envelope: DesignBounds | None = None
    cached: bool = False
    warnings: tuple[Annotated[str, Field(min_length=1, max_length=256)], ...] = ()
    component_count: Annotated[int, Field(ge=0)] = 0
    fused: bool = False


class DrawingPolyline(_StrictModel):
    """One HLR polyline in view-plane millimetres."""

    points: tuple[tuple[float, float], ...]
    closed: bool
    point_count: Annotated[int, Field(ge=0)]


class DrawingView(_StrictModel):
    """The visible and hidden lines of one view."""

    name: Annotated[str, Field(min_length=1, max_length=16)]
    visible: tuple[DrawingPolyline, ...]
    hidden: tuple[DrawingPolyline, ...]
    truncated: bool
    point_count: Annotated[int, Field(ge=0)]
    polyline_count: Annotated[int, Field(ge=0)]


class DrawingResponse(_StrictModel):
    """The HLR drawing of one stored design."""

    design_id: DesignId
    views: tuple[DrawingView, ...]
    chord_tolerance_mm: float
    max_points: Annotated[int, Field(ge=1)]
    point_count: Annotated[int, Field(ge=0)]


class SectionBounds2D(_StrictModel):
    """An axis-aligned box in section-plane millimetres."""

    min_u: float
    min_v: float
    max_u: float
    max_v: float
    width: float
    height: float
    center_u: float
    center_v: float


class SectionLoopResponse(_StrictModel):
    """One closed loop of a section cut."""

    points: tuple[tuple[float, float], ...]
    area_mm2: float
    area_exact: bool
    outer: bool
    bounds: SectionBounds2D
    centroid: tuple[float, float]
    point_count: Annotated[int, Field(ge=0)]


class SectionResponse(_StrictModel):
    """The closed loops and measured net area of one section cut."""

    design_id: DesignId
    plane: Annotated[str, Field(min_length=2, max_length=2)]
    offset_mm: float
    loops: tuple[SectionLoopResponse, ...]
    area_mm2: float
    area_exact: bool
    chord_tolerance_mm: float
    truncated: bool
    outer_loop_count: Annotated[int, Field(ge=0)]
    inner_loop_count: Annotated[int, Field(ge=0)]
    point_count: Annotated[int, Field(ge=0)]


class JournalEventRecord(_StrictModel):
    """One stored Studio journal event without its raw payload text."""

    sequence: Annotated[int, Field(ge=1)]
    event_type: Annotated[str, Field(min_length=1, max_length=64)]
    aggregate_id: Annotated[str, Field(min_length=1, max_length=64)]
    payload: dict[str, object] | None
    digest: Annotated[str, Field(min_length=64, max_length=64)]
    occurred_at: Annotated[str, Field(min_length=1, max_length=64)]
    previous_event_digest: Annotated[str, Field(min_length=64, max_length=64)]
    event_digest: Annotated[str, Field(min_length=64, max_length=64)]


class JournalPageResponse(_StrictModel):
    """A bounded page of Studio journal events."""

    events: tuple[JournalEventRecord, ...]
    next_after: Annotated[int, Field(ge=0)]
    has_more: bool
    limit: Annotated[int, Field(ge=1, le=MAX_JOURNAL_LIMIT)]


class JournalVerifyResponse(_StrictModel):
    """The hash-chain verdict for the Studio journal."""

    ok: bool
    event_count: Annotated[int, Field(ge=0)]
    head_digest: Annotated[str | None, Field(default=None, max_length=64)] = None
    first_bad_sequence: Annotated[int | None, Field(default=None, ge=1)] = None


class CadProviderRecord(_StrictModel):
    """One provider row: a live probe result plus static identity."""

    id: CadProviderId
    name: Annotated[str, Field(min_length=1, max_length=64)]
    available: bool
    version: Annotated[str | None, Field(default=None, max_length=64)] = None
    reason: Annotated[str | None, Field(default=None, max_length=512)] = None
    measured_by: Annotated[str, Field(min_length=1, max_length=64)]
    mode: Annotated[str, Field(min_length=1, max_length=16)]


class CadProvidersResponse(_StrictModel):
    """The probe result of every CAD provider, in a fixed order."""

    providers: tuple[CadProviderRecord, ...]


class HandoffResponse(_StrictModel):
    """The metadata for a successful neutral-file handoff."""

    integration_id: Annotated[
        str, Field(min_length=1, max_length=64, pattern=INTEGRATION_ID_PATTERN)
    ]
    format: ExportFormat
    bytes: Annotated[int, Field(ge=0)]
    filename: Annotated[str, Field(min_length=1, max_length=MAX_NOTES_LENGTH)]


class ErrorResponse(_StrictModel):
    """The flat studio error envelope.

    The shape is intentionally flat (``code`` / ``message`` / ``details``)
    so the browser UI can render a single message without unwrapping a
    nested object. The envelope never carries a traceback, a submitted
    value, or credential-shaped text.
    """

    code: Annotated[str, Field(min_length=1, max_length=64)]
    message: Annotated[str, Field(min_length=1, max_length=MAX_NOTES_LENGTH)]
    details: tuple[Annotated[str, Field(min_length=1, max_length=256)], ...] = ()


def error_response(code: str, message: str, details: tuple[str, ...] = ()) -> ErrorResponse:
    """Build a bounded, secret-free flat error body."""
    bounded_details = tuple(detail[:256] for detail in details[:16] if detail)
    return ErrorResponse(
        code=code,
        message=message[:MAX_NOTES_LENGTH],
        details=bounded_details,
    )


def dimension_bounds_payload() -> dict[str, float]:
    """Expose dimension bounds as JSON-serialisable bounds for documentation."""
    return {
        "min_millimetres": MIN_DIMENSION_MM,
        "max_millimetres": MAX_DIMENSION_MM,
    }


__all__ = (
    "MAX_BBOX_COUNT",
    "MAX_BBOX_NUMBER",
    "MAX_CAPABILITY_LENGTH",
    "MAX_DRAWING_VIEWS",
    "MAX_FORMAT_LENGTH",
    "MAX_INTEGRATION_NAME_LENGTH",
    "MAX_JOURNAL_LIMIT",
    "MAX_KIND_LENGTH",
    "MAX_LABEL_LENGTH",
    "MAX_MODEL_ID_LENGTH",
    "MAX_NOTES_LENGTH",
    "MAX_PARAMETERS_ENTRIES",
    "MAX_PARAMETER_KEY_LENGTH",
    "MAX_PARAMETER_STRING_VALUE_LENGTH",
    "MAX_PLATFORM_LENGTH",
    "MAX_PROMPT_LENGTH",
    "MAX_PROVIDER_ID_LENGTH",
    "MAX_PROVIDER_NAME_LENGTH",
    "MAX_SECTION_OFFSET_MM",
    "SERVICE_NAME",
    "BBox",
    "CadProviderId",
    "CadProviderRecord",
    "CadProvidersResponse",
    "ChatRequest",
    "ChatResponse",
    "CheckValue",
    "CredentialProviderId",
    "CredentialSource",
    "DesignArtifacts",
    "DesignBounds",
    "DesignCheck",
    "DesignId",
    "DesignMeasured",
    "DesignMesh",
    "DesignRequest",
    "DesignResult",
    "DesignView",
    "DrawingPolyline",
    "DrawingResponse",
    "DrawingView",
    "ErrorResponse",
    "ExportFormat",
    "ExportRequest",
    "HandoffRequest",
    "HandoffResponse",
    "HealthResponse",
    "IntegrationCategory",
    "IntegrationRecord",
    "IntegrationStatus",
    "IntegrationsResponse",
    "JournalEventRecord",
    "JournalPageResponse",
    "JournalVerifyResponse",
    "PreviewRequest",
    "PreviewResponse",
    "PrimitiveKind",
    "ProviderCredentialStatus",
    "ProviderId",
    "ProviderRecord",
    "ProvidersResponse",
    "SectionBounds2D",
    "SectionLoopResponse",
    "SectionResponse",
    "StudioSettingsResponse",
    "UpdateProviderKeyRequest",
    "_StrictModel",
    "_validate_parameters_block",
    "dimension_bounds_payload",
    "error_response",
)
