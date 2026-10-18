"""Pydantic request and response models for the studio HTTP surface.

Every request body the studio accepts is strict: extras are refused, numeric
values are finite, strings are size-bounded, and unknown ``kind``/``format``
values are a validation failure rather than a silently accepted default. The
responses mirror the frozen views the studio functions produced so the HTTP
surface can never widen, rename, or reinterpret a result.
"""

from __future__ import annotations

from enum import StrEnum
from typing import Annotated, Final

from pydantic import BaseModel, ConfigDict, Field, field_validator

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
MAX_PARAMETERS_ENTRIES: Final[int] = 32
MAX_PARAMETER_KEY_LENGTH: Final[int] = 64
MAX_PARAMETER_STRING_VALUE_LENGTH: Final[int] = 128
MAX_BBOX_NUMBER: Final[float] = 1_000_000.0
INTEGRATION_ID_PATTERN: Final[str] = r"^[a-z0-9][a-z0-9._-]{0,63}$"
PROVIDER_ID_PATTERN: Final[str] = r"^[a-z0-9][a-z0-9._-]{0,63}$"
MODEL_ID_PATTERN: Final[str] = r"^[A-Za-z0-9._:/-]{1,256}$"
SERVICE_NAME: Final[str] = "kala-studio"


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
    """

    LOCAL = "local"
    OPENAI = "openai"
    OPENROUTER = "openrouter"
    CODEX = "codex"


class CredentialProviderId(StrEnum):
    """Providers whose API keys can be configured in Studio settings."""

    OPENAI = "openai"
    OPENROUTER = "openrouter"


class CredentialSource(StrEnum):
    """Where a provider's active API key is sourced from."""

    NONE = "none"
    ENVIRONMENT = "environment"
    STORED = "stored"


class _StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)


class _RequestModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, str_strip_whitespace=True)


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
    """Ask the studio to render a parametric preview into a downloadable file."""

    kind: PrimitiveKind
    parameters: ParametersField = None
    format: ExportFormat

    @field_validator("parameters")
    @classmethod
    def check_parameters(cls, value: dict[str, object] | None) -> dict[str, object] | None:
        return _validate_parameters_block(value)


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
    """

    id: Annotated[str, Field(min_length=1, max_length=64, pattern=PROVIDER_ID_PATTERN)]
    name: Annotated[str, Field(min_length=1, max_length=MAX_PROVIDER_NAME_LENGTH)]
    configured: bool
    notes: Annotated[str, Field(max_length=MAX_NOTES_LENGTH)] = ""


class ProvidersResponse(_StrictModel):
    providers: tuple[ProviderRecord, ...]


class ProviderCredentialStatus(_StrictModel):
    """Non-secret status of a configurable provider credential."""

    provider: CredentialProviderId
    env_var: Annotated[str, Field(min_length=1, max_length=64)]
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
    """A short model reply, never a credential or a long traceback."""

    text: Annotated[str, Field(min_length=0, max_length=MAX_PROMPT_LENGTH)]
    provider: ProviderId
    model: Annotated[str | None, Field(default=None, max_length=MAX_MODEL_ID_LENGTH)] = None
    offline: bool


class HealthResponse(_StrictModel):
    """Process liveness, with no credentials or endpoints echoed."""

    status: Annotated[str, Field(min_length=1, max_length=64)] = "ok"
    service: Annotated[str, Field(min_length=1, max_length=64)] = SERVICE_NAME
    version: Annotated[str, Field(min_length=1, max_length=64)]
    offline: bool
    step_available: bool
    integrations: tuple[str, ...]
    providers: tuple[str, ...]


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
    "MAX_FORMAT_LENGTH",
    "MAX_INTEGRATION_NAME_LENGTH",
    "MAX_KIND_LENGTH",
    "MAX_MODEL_ID_LENGTH",
    "MAX_NOTES_LENGTH",
    "MAX_PARAMETERS_ENTRIES",
    "MAX_PARAMETER_KEY_LENGTH",
    "MAX_PARAMETER_STRING_VALUE_LENGTH",
    "MAX_PLATFORM_LENGTH",
    "MAX_PROMPT_LENGTH",
    "MAX_PROVIDER_ID_LENGTH",
    "MAX_PROVIDER_NAME_LENGTH",
    "SERVICE_NAME",
    "BBox",
    "ChatRequest",
    "ChatResponse",
    "CredentialProviderId",
    "CredentialSource",
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
    "PreviewRequest",
    "PreviewResponse",
    "PrimitiveKind",
    "ProviderCredentialStatus",
    "ProviderId",
    "ProviderRecord",
    "ProvidersResponse",
    "StudioSettingsResponse",
    "UpdateProviderKeyRequest",
    "_StrictModel",
    "_validate_parameters_block",
    "dimension_bounds_payload",
    "error_response",
)
