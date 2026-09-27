from __future__ import annotations

import contextlib
import ipaddress
import math
import posixpath
import re
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import dataclass, field, replace
from datetime import UTC, datetime
from decimal import Decimal, InvalidOperation
from enum import StrEnum
from threading import RLock
from types import MappingProxyType
from typing import TYPE_CHECKING, Final, Protocol, TypeAlias, cast, runtime_checkable
from urllib.parse import urlsplit

from kala_agent.contracts import ApprovalKind, ArtifactClassification, SideEffectLevel
from kala_agent.kernel.budget import (
    BudgetLedger,
    BudgetReservation,
    BudgetReservationUnknownError,
)
from kala_agent.kernel.policy import PolicyEngine, PolicyRequest
from kala_agent.tools.protocol import (
    JsonEnvelopeError,
    JSONValue,
    SchemaValidationError,
    canonical_json_digest,
    canonical_json_dumps,
    check_object_schema,
    sha256_hex,
    strict_json_loads,
    validate_against_schema,
)

if TYPE_CHECKING:
    import httpx

MAX_MESSAGES: Final[int] = 128
MAX_MESSAGE_LENGTH: Final[int] = 32_768
MAX_REQUEST_BYTES: Final[int] = 1_048_576
MAX_TRANSCRIPT_BODY_BYTES: Final[int] = 65_536
MAX_RESPONSE_BYTES: Final[int] = 1_048_576
MAX_OUTPUT_TOKENS: Final[int] = 32_768
MAX_SEED: Final[int] = 2_147_483_647
MIN_TIMEOUT_SECONDS: Final[float] = 0.1
MAX_TIMEOUT_SECONDS: Final[float] = 600.0
CHARS_PER_TOKEN: Final[int] = 4
LOCAL_PROVIDER_ID: Final[str] = "local"
OPENAI_COMPATIBLE_PROVIDER_ID: Final[str] = "openai-compatible"
LOOPBACK_HOSTS: Final[frozenset[str]] = frozenset({"localhost", "ip6-localhost", "ip6-loopback"})
EGRESS_CAPABILITIES: Final[tuple[str, ...]] = ("model.egress",)
MAX_ENDPOINT_LENGTH: Final[int] = 2_048

_SECRET_VALUE: Final[str] = r"[\"']?(?!<redacted>)[^\s\"',;]+[\"']?"
_SECRET_ASSIGNMENT: Final[str] = rf"[\"']?\s*[:=]\s*{_SECRET_VALUE}"
_SECRET_KEYS: Final[str] = (
    r"aws[_-]?(?:secret[_-]?access[_-]?key|access[_-]?key[_-]?id|session[_-]?token)"
    r"|api[_-]?key|api[_-]?secret|client[_-]?secret|secret[_-]?key|private[_-]?key"
    r"|access[_-]?key|passphrase|password|passwd|secret|token|credentials?|authorization"
)
# Underscores and hyphens separate name segments, so OPENAI_API_KEY and
# db-password name secrets; a regex \b would treat them as word characters.
_SECRET_NAME: Final[str] = (
    rf"(?<![A-Za-z0-9])(?:[A-Za-z0-9]+[_-])*(?:{_SECRET_KEYS})(?![A-Za-z0-9])"
)
_SECRET_PATTERNS: Final[tuple[tuple[re.Pattern[str], str], ...]] = (
    (
        re.compile(
            r"-----BEGIN(?: [A-Z0-9]+)* PRIVATE KEY-----[\s\S]*?"
            r"(?:-----END(?: [A-Z0-9]+)* PRIVATE KEY-----|\Z)"
        ),
        "<redacted>",
    ),
    (
        re.compile(
            r"(?im)(?P<header>\b(?:proxy-)?authorization[ \t]*+:[ \t]*+)(?!<redacted>)[^\r\n]+"
        ),
        r"\g<header><redacted>",
    ),
    (re.compile(r"\bsk-[A-Za-z0-9_\-]{12,}"), "<redacted>"),
    (re.compile(r"(?i)\bbearer\s+[A-Za-z0-9._\-]{12,}"), "<redacted>"),
    (re.compile(r"(?i)\bgh[pousr]_[A-Za-z0-9]{16,}"), "<redacted>"),
    (re.compile(r"(?i)\b(?:AKIA|ASIA|AGPA|AIDA|AROA|ANPA)[0-9A-Z]{16}\b"), "<redacted>"),
    (
        re.compile(
            rf"(?i)\"(?P<key>{_SECRET_NAME})\"(?P<sep>\s*:\s*)\"(?!<redacted>\")(?:[^\"\\]|\\.)+\""
        ),
        r'"\g<key>"\g<sep>"<redacted>"',
    ),
    (re.compile(rf"(?i){_SECRET_NAME}{_SECRET_ASSIGNMENT}"), "<redacted>"),
)
_SECRET_NAME_PATTERN: Final[re.Pattern[str]] = re.compile(rf"(?i){_SECRET_NAME}")
_IDENTIFIER_PATTERN: Final[re.Pattern[str]] = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$")

TranscriptCallback: TypeAlias = Callable[["ModelTranscript"], None]
EgressManifestCallback: TypeAlias = Callable[["ModelEgressManifest"], None]
ArtifactMetadata: TypeAlias = Mapping[str, JSONValue]


class ModelError(RuntimeError):
    pass


class ModelRequestError(ModelError, ValueError):
    pass


class ModelCapabilityError(ModelRequestError):
    pass


class StructuredOutputError(ModelError, ValueError):
    pass


class ModelProviderError(ModelError):
    pass


class ProviderUnavailableError(ModelProviderError):
    pass


class ProviderDegradedError(ModelProviderError):
    def __init__(self, provider_id: str, model_id: str, reason: str) -> None:
        self.provider_id = provider_id
        self.model_id = model_id
        self.reason = reason
        self.checkpoint_kind = ApprovalKind.PROVIDER_DEGRADED
        self.substituted = False
        super().__init__(
            f"provider {provider_id} model {model_id} is unavailable: {reason}; "
            "no substitute provider is used"
        )


class ProviderSubstitutionError(ModelProviderError):
    pass


class EgressDeniedError(ModelError, PermissionError):
    pass


class TranscriptError(ModelError):
    pass


class ModelRole(StrEnum):
    SYSTEM = "system"
    USER = "user"
    ASSISTANT = "assistant"


class ModelCapability(StrEnum):
    STRUCTURED_OUTPUT = "structured_output"
    SEED = "seed"
    TOOL_CALLING = "tool_calling"


class ModelFinishReason(StrEnum):
    STOP = "stop"
    LENGTH = "length"
    REFUSAL = "refusal"
    ERROR = "error"


def _bounded_identifier(value: object, field_name: str) -> str:
    if not isinstance(value, str):
        raise ModelRequestError(f"{field_name} must be a string")
    normalized = value.strip()
    if _IDENTIFIER_PATTERN.fullmatch(normalized) is None:
        raise ModelRequestError(f"{field_name} must be a safe identifier")
    return normalized


def _bounded_text(value: object, field_name: str, *, maximum: int = MAX_MESSAGE_LENGTH) -> str:
    if not isinstance(value, str):
        raise ModelRequestError(f"{field_name} must be a string")
    if "\x00" in value:
        raise ModelRequestError(f"{field_name} cannot contain null bytes")
    if len(value) > maximum:
        raise ModelRequestError(f"{field_name} exceeds the bounded length")
    return value


def _nonnegative_int(value: object, field_name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ModelRequestError(f"{field_name} must be an integer")
    if value < 0:
        raise ModelRequestError(f"{field_name} must be nonnegative")
    return value


def _bounded_float(value: object, field_name: str, *, minimum: float, maximum: float) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ModelRequestError(f"{field_name} must be a number")
    parsed = float(value)
    if not math.isfinite(parsed) or not minimum <= parsed <= maximum:
        raise ModelRequestError(f"{field_name} must be finite and within its bounds")
    return parsed


def _nonnegative_decimal(value: object, field_name: str) -> Decimal:
    if isinstance(value, bool):
        raise ModelRequestError(f"{field_name} must be numeric")
    try:
        parsed = value if isinstance(value, Decimal) else Decimal(str(value))
    except (InvalidOperation, TypeError, ValueError) as error:
        raise ModelRequestError(f"{field_name} must be a nonnegative decimal") from error
    if not parsed.is_finite() or parsed < 0:
        raise ModelRequestError(f"{field_name} must be a finite nonnegative decimal")
    return parsed


def _freeze_json(value: object, *, depth: int = 0) -> JSONValue:
    if depth > 32:
        raise ModelRequestError("structured output nesting exceeds the supported depth")
    if value is None or isinstance(value, (bool, int, str)):
        return cast(JSONValue, value)
    if isinstance(value, float):
        if not math.isfinite(value):
            raise ModelRequestError("structured output numbers must be finite")
        return value
    if isinstance(value, Mapping):
        frozen: dict[str, JSONValue] = {}
        for key, item in value.items():
            if not isinstance(key, str):
                raise ModelRequestError("structured output keys must be strings")
            frozen[key] = _freeze_json(item, depth=depth + 1)
        return cast(JSONValue, MappingProxyType(frozen))
    if isinstance(value, (list, tuple)):
        return cast(JSONValue, tuple(_freeze_json(item, depth=depth + 1) for item in value))
    raise ModelRequestError("structured output is not JSON serializable")


def _thaw_json(value: object) -> JSONValue:
    if isinstance(value, Mapping):
        return {key: _thaw_json(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_thaw_json(item) for item in value]
    return cast(JSONValue, value)


def redact_secrets(text: str) -> tuple[str, int]:
    findings = 0
    result = text
    for pattern, replacement in _SECRET_PATTERNS:
        result, count = pattern.subn(replacement, result)
        findings += count
    return result, findings


def truncate_encoded(text: str, maximum_bytes: int) -> str:
    encoded = text.encode("utf-8")
    if len(encoded) <= maximum_bytes:
        return text
    return encoded[:maximum_bytes].decode("utf-8", errors="ignore")


def is_secret_name(value: str) -> bool:
    return _SECRET_NAME_PATTERN.search(value) is not None


def _redact_metadata(value: Mapping[str, JSONValue]) -> tuple[ArtifactMetadata, int]:
    findings = 0
    redacted: dict[str, JSONValue] = {}
    for key, item in value.items():
        if is_secret_name(key):
            findings += 1
            redacted[key] = "<redacted>"
            continue
        if isinstance(item, str):
            redacted_text, text_findings = redact_secrets(item)
            redacted[key] = redacted_text
            findings += text_findings
        elif isinstance(item, Mapping):
            nested, nested_findings = _redact_metadata(cast("Mapping[str, JSONValue]", item))
            redacted[key] = cast(JSONValue, nested)
            findings += nested_findings
        elif isinstance(item, (list, tuple)):
            collected: list[JSONValue] = []
            for element in item:
                if isinstance(element, str):
                    redacted_text, text_findings = redact_secrets(element)
                    collected.append(redacted_text)
                    findings += text_findings
                elif isinstance(element, Mapping):
                    nested, nested_findings = _redact_metadata(
                        cast("Mapping[str, JSONValue]", element)
                    )
                    collected.append(cast(JSONValue, nested))
                    findings += nested_findings
                else:
                    collected.append(cast(JSONValue, element))
            redacted[key] = cast(JSONValue, tuple(collected))
        else:
            redacted[key] = item
    return MappingProxyType(redacted), findings


@dataclass(frozen=True, slots=True)
class ModelMessage:
    role: ModelRole
    content: str

    def __post_init__(self) -> None:
        try:
            role = ModelRole(self.role)
        except ValueError as error:
            raise ModelRequestError("role is not a model message role") from error
        object.__setattr__(self, "role", role)
        object.__setattr__(self, "content", _bounded_text(self.content, "content"))

    def to_payload(self) -> dict[str, str]:
        return {"content": self.content, "role": self.role.value}


@dataclass(frozen=True, slots=True)
class ModelArtifact:
    artifact_id: str
    digest: str
    classification: ArtifactClassification
    media_type: str
    size_bytes: int
    preview_text: str | None = None
    path: str | None = None
    metadata: ArtifactMetadata = field(default_factory=lambda: MappingProxyType({}))

    def __post_init__(self) -> None:
        object.__setattr__(
            self, "artifact_id", _bounded_identifier(self.artifact_id, "artifact_id")
        )
        digest = self.digest.strip().lower()
        if re.fullmatch(r"[0-9a-f]{64}", digest) is None:
            raise ModelRequestError("digest must be a sha256 hexadecimal digest")
        object.__setattr__(self, "digest", digest)
        try:
            classification = ArtifactClassification(self.classification)
        except ValueError as error:
            raise ModelRequestError("classification is not an artifact classification") from error
        object.__setattr__(self, "classification", classification)
        object.__setattr__(
            self,
            "media_type",
            _bounded_text(self.media_type, "media_type", maximum=255).lower(),
        )
        object.__setattr__(self, "size_bytes", _nonnegative_int(self.size_bytes, "size_bytes"))
        if self.preview_text is not None:
            object.__setattr__(
                self,
                "preview_text",
                _bounded_text(self.preview_text, "preview_text", maximum=8_192),
            )
        if self.path is not None:
            object.__setattr__(self, "path", _bounded_text(self.path, "path", maximum=1_024))
        if not isinstance(self.metadata, Mapping):
            raise ModelRequestError("metadata must be a JSON object")
        object.__setattr__(self, "metadata", _freeze_json(dict(self.metadata)))

    @property
    def is_design_binary(self) -> bool:
        return self.classification is ArtifactClassification.DESIGN_BINARY

    def to_payload(self) -> dict[str, JSONValue]:
        payload: dict[str, JSONValue] = {
            "artifact_id": self.artifact_id,
            "classification": self.classification.value,
            "digest": self.digest,
            "media_type": self.media_type,
            "metadata": _thaw_json(self.metadata),
            "size_bytes": self.size_bytes,
        }
        if self.preview_text is not None:
            payload["preview_text"] = self.preview_text
        if self.path is not None:
            payload["path"] = self.path
        return payload

    def redacted(self) -> ModelArtifact:
        return replace(self, preview_text=None, path=None, metadata=MappingProxyType({}))


@dataclass(frozen=True, slots=True)
class ModelRequest:
    request_id: str
    messages: Sequence[ModelMessage]
    capabilities: Iterable[ModelCapability] = frozenset()
    response_schema: Mapping[str, object] | None = None
    artifacts: Sequence[ModelArtifact] = ()
    seed: int | None = None
    max_output_tokens: int = 1_024
    timeout_seconds: float = 60.0

    def __post_init__(self) -> None:
        object.__setattr__(self, "request_id", _bounded_identifier(self.request_id, "request_id"))
        if not isinstance(self.messages, Sequence):
            raise ModelRequestError("messages must be a sequence of model messages")
        messages = tuple(self.messages)
        if not messages:
            raise ModelRequestError("a model request requires at least one message")
        if len(messages) > MAX_MESSAGES:
            raise ModelRequestError("messages exceed the bounded count")
        for message in messages:
            if not isinstance(message, ModelMessage):
                raise ModelRequestError("messages must contain ModelMessage values")
        object.__setattr__(self, "messages", messages)
        capabilities: set[ModelCapability] = set()
        if isinstance(self.capabilities, (str, bytes)) or not isinstance(
            self.capabilities, (set, frozenset, list, tuple)
        ):
            raise ModelRequestError("capabilities must be a collection of model capabilities")
        for capability in self.capabilities:
            try:
                capabilities.add(ModelCapability(capability))
            except ValueError as error:
                raise ModelCapabilityError(f"unknown model capability: {capability!r}") from error
        object.__setattr__(self, "capabilities", frozenset(capabilities))
        if self.response_schema is not None:
            try:
                check_object_schema(self.response_schema, label="response_schema")
            except SchemaValidationError as error:
                raise ModelRequestError(str(error)) from error
        if not isinstance(self.artifacts, Sequence):
            raise ModelRequestError("artifacts must be a sequence of artifact references")
        artifacts = tuple(self.artifacts)
        seen: set[str] = set()
        for artifact in artifacts:
            if not isinstance(artifact, ModelArtifact):
                raise ModelRequestError("artifacts must contain ModelArtifact values")
            if artifact.artifact_id in seen:
                raise ModelRequestError("artifact identifiers must be unique")
            seen.add(artifact.artifact_id)
        object.__setattr__(self, "artifacts", artifacts)
        if self.seed is not None:
            seed = _nonnegative_int(self.seed, "seed")
            if seed > MAX_SEED:
                raise ModelRequestError("seed exceeds the supported bound")
            object.__setattr__(self, "seed", seed)
        object.__setattr__(
            self,
            "max_output_tokens",
            _nonnegative_int(self.max_output_tokens, "max_output_tokens"),
        )
        if self.max_output_tokens > MAX_OUTPUT_TOKENS:
            raise ModelRequestError("max_output_tokens exceeds the supported bound")
        object.__setattr__(
            self,
            "timeout_seconds",
            _bounded_float(
                self.timeout_seconds,
                "timeout_seconds",
                minimum=MIN_TIMEOUT_SECONDS,
                maximum=MAX_TIMEOUT_SECONDS,
            ),
        )
        if ModelCapability.STRUCTURED_OUTPUT in capabilities and self.response_schema is None:
            raise ModelCapabilityError("structured output requires a response schema")
        if ModelCapability.SEED in capabilities and self.seed is None:
            raise ModelCapabilityError("a seeded request requires a seed value")

    @property
    def requires_structured_output(self) -> bool:
        return self.response_schema is not None

    @property
    def request_body(self) -> str:
        return canonical_json_dumps(
            {
                "artifacts": [artifact.to_payload() for artifact in self.artifacts],
                "capabilities": sorted(capability.value for capability in self.capabilities),
                "max_output_tokens": self.max_output_tokens,
                "messages": [message.to_payload() for message in self.messages],
                "request_id": self.request_id,
                "response_schema": None
                if self.response_schema is None
                else _thaw_json(cast(JSONValue, dict(self.response_schema))),
                "seed": self.seed,
            },
            max_bytes=MAX_REQUEST_BYTES,
        )

    @property
    def request_digest(self) -> str:
        return sha256_hex(self.request_body)

    def with_artifacts(self, artifacts: Sequence[ModelArtifact]) -> ModelRequest:
        return replace(self, artifacts=tuple(artifacts))


@dataclass(frozen=True, slots=True)
class ModelResponse:
    request_id: str
    provider_id: str
    model_id: str
    content: str
    structured: Mapping[str, JSONValue] | None = None
    input_tokens: int = 0
    output_tokens: int = 0
    cost: Decimal = Decimal("0")
    finish_reason: ModelFinishReason = ModelFinishReason.STOP
    provider_request_id: str | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "request_id", _bounded_identifier(self.request_id, "request_id"))
        object.__setattr__(
            self, "provider_id", _bounded_identifier(self.provider_id, "provider_id")
        )
        object.__setattr__(self, "model_id", _bounded_identifier(self.model_id, "model_id"))
        object.__setattr__(
            self,
            "content",
            _bounded_text(self.content, "content", maximum=MAX_RESPONSE_BYTES),
        )
        if self.structured is not None:
            if not isinstance(self.structured, Mapping):
                raise ModelProviderError("structured output must be a JSON object")
            object.__setattr__(self, "structured", _freeze_json(dict(self.structured)))
        object.__setattr__(
            self, "input_tokens", _nonnegative_int(self.input_tokens, "input_tokens")
        )
        object.__setattr__(
            self, "output_tokens", _nonnegative_int(self.output_tokens, "output_tokens")
        )
        object.__setattr__(self, "cost", _nonnegative_decimal(self.cost, "cost"))
        try:
            finish_reason = ModelFinishReason(self.finish_reason)
        except ValueError as error:
            raise ModelProviderError("finish_reason is not a model finish reason") from error
        object.__setattr__(self, "finish_reason", finish_reason)
        if self.provider_request_id is not None:
            object.__setattr__(
                self,
                "provider_request_id",
                _bounded_text(
                    self.provider_request_id,
                    "provider_request_id",
                    maximum=256,
                ),
            )

    @property
    def total_tokens(self) -> int:
        return self.input_tokens + self.output_tokens

    @property
    def response_digest(self) -> str:
        return sha256_hex(self.content)

    def canonical_content(self) -> str:
        if self.structured is None:
            return self.content
        return canonical_json_dumps(_thaw_json(self.structured), max_bytes=MAX_RESPONSE_BYTES)


@dataclass(frozen=True, slots=True)
class ModelRedaction:
    kind: str
    subject: str
    reason: str

    def to_payload(self) -> dict[str, str]:
        return {"kind": self.kind, "reason": self.reason, "subject": self.subject}


@dataclass(frozen=True, slots=True)
class ModelTranscript:
    request_id: str
    provider_id: str
    model_id: str
    request_digest: str
    response_digest: str
    request_body: str
    response_body: str
    input_tokens: int
    output_tokens: int
    cost: Decimal
    redactions: tuple[ModelRedaction, ...]
    artifact_digests: tuple[str, ...]
    recorded_at: datetime

    def __post_init__(self) -> None:
        if self.recorded_at.tzinfo is None or self.recorded_at.utcoffset() is None:
            raise ModelRequestError("recorded_at must include a timezone")
        object.__setattr__(self, "recorded_at", self.recorded_at.astimezone(UTC))
        if len(self.request_body.encode("utf-8")) > MAX_TRANSCRIPT_BODY_BYTES:
            raise TranscriptError("the transcript request body exceeds the bounded size")
        if len(self.response_body.encode("utf-8")) > MAX_TRANSCRIPT_BODY_BYTES:
            raise TranscriptError("the transcript response body exceeds the bounded size")

    def to_payload(self) -> dict[str, object]:
        return {
            "artifact_digests": list(self.artifact_digests),
            "cost": str(self.cost),
            "input_tokens": self.input_tokens,
            "model_id": self.model_id,
            "output_tokens": self.output_tokens,
            "provider_id": self.provider_id,
            "recorded_at": self.recorded_at.isoformat(),
            "redactions": [redaction.to_payload() for redaction in self.redactions],
            "request_body": self.request_body,
            "request_digest": self.request_digest,
            "request_id": self.request_id,
            "response_body": self.response_body,
            "response_digest": self.response_digest,
        }


@dataclass(frozen=True, slots=True)
class ModelEgressManifest:
    request_id: str
    egress_request_id: str
    provider_id: str
    model_id: str
    destination: str
    request_digest: str
    state_digest: str
    action_digest: str
    capabilities: tuple[str, ...]
    redactions: tuple[ModelRedaction, ...]
    artifact_digests: tuple[str, ...]
    recorded_at: datetime

    def __post_init__(self) -> None:
        if self.recorded_at.tzinfo is None or self.recorded_at.utcoffset() is None:
            raise ModelRequestError("recorded_at must include a timezone")
        object.__setattr__(self, "recorded_at", self.recorded_at.astimezone(UTC))
        object.__setattr__(
            self,
            "request_id",
            _bounded_identifier(self.request_id, "request_id"),
        )
        object.__setattr__(
            self,
            "egress_request_id",
            _bounded_identifier(self.egress_request_id, "egress_request_id"),
        )
        object.__setattr__(
            self, "provider_id", _bounded_identifier(self.provider_id, "provider_id")
        )
        object.__setattr__(self, "model_id", _bounded_identifier(self.model_id, "model_id"))
        object.__setattr__(self, "destination", _bounded_text(self.destination, "destination"))
        for digest_name, digest in (
            ("request_digest", self.request_digest),
            ("state_digest", self.state_digest),
            ("action_digest", self.action_digest),
        ):
            if re.fullmatch(r"[0-9a-f]{64}", digest) is None:
                raise ModelRequestError(f"{digest_name} must be a sha256 hexadecimal digest")
        object.__setattr__(self, "capabilities", tuple(sorted(self.capabilities)))
        object.__setattr__(self, "redactions", tuple(self.redactions))
        object.__setattr__(self, "artifact_digests", tuple(self.artifact_digests))

    def to_payload(self) -> dict[str, object]:
        return {
            "action_digest": self.action_digest,
            "artifact_digests": list(self.artifact_digests),
            "capabilities": list(self.capabilities),
            "destination": self.destination,
            "egress_request_id": self.egress_request_id,
            "model_id": self.model_id,
            "provider_id": self.provider_id,
            "recorded_at": self.recorded_at.isoformat(),
            "redactions": [redaction.to_payload() for redaction in self.redactions],
            "request_digest": self.request_digest,
            "request_id": self.request_id,
            "state_digest": self.state_digest,
        }


@runtime_checkable
class LLMClient(Protocol):
    @property
    def provider_id(self) -> str: ...

    @property
    def model_id(self) -> str: ...

    @property
    def requires_egress(self) -> bool: ...

    def supports(self, capability: ModelCapability) -> bool: ...

    def complete(self, request: ModelRequest) -> ModelResponse: ...


def estimate_tokens(text: str) -> int:
    if not text:
        return 0
    return max(1, (len(text) + CHARS_PER_TOKEN - 1) // CHARS_PER_TOKEN)


def _schema_type_names(schema: Mapping[str, object]) -> tuple[str, ...]:
    declared = schema.get("type")
    if declared is None:
        return ("object",)
    if isinstance(declared, (list, tuple)):
        return tuple(cast(Sequence[str], declared))
    return (cast(str, declared),)


def _schema_properties(schema: Mapping[str, object]) -> Mapping[str, object]:
    properties = schema.get("properties")
    if isinstance(properties, Mapping):
        return properties
    return {}


def _synthesize_value(schema: Mapping[str, object], *, depth: int) -> JSONValue:
    if depth > 16:
        raise ModelProviderError("the response schema nesting exceeds the synthesis bound")
    if "const" in schema:
        return _freeze_json(schema["const"])
    options = schema.get("enum")
    if isinstance(options, (list, tuple)) and options:
        return _freeze_json(options[0])
    if "default" in schema:
        try:
            return _freeze_json(schema["default"])
        except ModelRequestError:
            pass
    for keyword in ("allOf", "anyOf", "oneOf"):
        branches = schema.get(keyword)
        if isinstance(branches, (list, tuple)) and branches and isinstance(branches[0], Mapping):
            return _synthesize_value(cast(Mapping[str, object], branches[0]), depth=depth + 1)
    names = _schema_type_names(schema)
    if "object" in names:
        properties = _schema_properties(schema)
        required = schema.get("required")
        wanted = (
            tuple(cast(Sequence[str], required))
            if isinstance(required, (list, tuple))
            else tuple(properties)
        )
        synthesized: dict[str, JSONValue] = {}
        for key in wanted:
            subschema = properties.get(key)
            if isinstance(subschema, Mapping):
                synthesized[key] = _synthesize_value(
                    cast(Mapping[str, object], subschema), depth=depth + 1
                )
            else:
                synthesized[key] = None
        return _freeze_json(synthesized)
    if "array" in names:
        minimum_items = schema.get("minItems")
        count = int(minimum_items) if isinstance(minimum_items, int) else 1
        items = schema.get("items")
        if not isinstance(items, Mapping):
            return _freeze_json([])
        return _freeze_json(
            [
                _synthesize_value(cast(Mapping[str, object], items), depth=depth + 1)
                for _ in range(max(0, min(count, 4)))
            ]
        )
    if "string" in names:
        return "local-offline-deterministic"
    if "integer" in names:
        return 0
    if "number" in names:
        return 0.0
    if "boolean" in names:
        return False
    return None


class LocalModelClient:
    def __init__(
        self,
        *,
        model_id: str = "local-deterministic",
        provider_id: str = LOCAL_PROVIDER_ID,
        capabilities: Iterable[ModelCapability] = (
            ModelCapability.STRUCTURED_OUTPUT,
            ModelCapability.SEED,
        ),
    ) -> None:
        self._provider_id = _bounded_identifier(provider_id, "provider_id")
        self._model_id = _bounded_identifier(model_id, "model_id")
        try:
            self._capabilities = frozenset(ModelCapability(item) for item in capabilities)
        except ValueError as error:
            raise ModelCapabilityError("unknown model capability") from error
        self._lock = RLock()
        self._calls = 0

    @property
    def provider_id(self) -> str:
        return self._provider_id

    @property
    def model_id(self) -> str:
        return self._model_id

    @property
    def requires_egress(self) -> bool:
        return False

    @property
    def calls(self) -> int:
        with self._lock:
            return self._calls

    def supports(self, capability: ModelCapability) -> bool:
        return ModelCapability(capability) in self._capabilities

    def complete(self, request: ModelRequest) -> ModelResponse:
        if not isinstance(request, ModelRequest):
            raise TypeError("request must be a ModelRequest")
        for capability in sorted(request.capabilities, key=lambda item: item.value):
            if not self.supports(capability):
                raise ModelCapabilityError(
                    f"provider {self._provider_id} does not declare {capability.value}"
                )
        prompt_tokens = sum(estimate_tokens(message.content) for message in request.messages)
        if request.response_schema is None:
            content = canonical_json_dumps({"request_digest": request.request_digest})
        else:
            synthesized = _synthesize_value(request.response_schema, depth=0)
            if not isinstance(synthesized, Mapping):
                raise ModelProviderError("the response schema does not describe an object")
            content = canonical_json_dumps(_thaw_json(synthesized))
        with self._lock:
            self._calls += 1
        return ModelResponse(
            request_id=request.request_id,
            provider_id=self._provider_id,
            model_id=self._model_id,
            content=content,
            structured=None,
            input_tokens=prompt_tokens,
            output_tokens=estimate_tokens(content),
            cost=Decimal("0"),
            finish_reason=ModelFinishReason.STOP,
            provider_request_id=f"local-{request.request_digest[:16]}",
        )

    def __repr__(self) -> str:
        return (
            f"{type(self).__name__}(provider_id={self._provider_id!r}, model_id={self._model_id!r})"
        )


@dataclass(frozen=True, slots=True)
class _Destination:
    scheme: str
    host: str
    port: int
    path: str

    @property
    def normalized(self) -> str:
        return f"{self.scheme}://{self.host}:{self.port}{self.path}"

    @property
    def is_loopback(self) -> bool:
        return is_loopback_host(self.host)

    @property
    def is_tls(self) -> bool:
        return self.scheme == "https"


def _parse_destination(value: object) -> _Destination:
    if not isinstance(value, str):
        raise EgressDeniedError("the provider endpoint must be a string")
    candidate = value.strip()
    if not candidate or len(candidate) > MAX_ENDPOINT_LENGTH:
        raise EgressDeniedError("the provider endpoint must be nonempty and bounded")
    try:
        parts = urlsplit(candidate)
        hostname = parts.hostname
        port = parts.port
    except ValueError as error:
        raise EgressDeniedError("the provider endpoint is not a valid URL") from error
    scheme = parts.scheme.lower()
    if scheme not in {"http", "https"}:
        raise EgressDeniedError("the provider endpoint must use http or https")
    if parts.username is not None or parts.password is not None:
        raise EgressDeniedError("the provider endpoint cannot contain credentials")
    if parts.query or parts.fragment:
        raise EgressDeniedError("the provider endpoint cannot contain a query or fragment")
    if not hostname:
        raise EgressDeniedError("the provider endpoint must contain a host")
    host = hostname.lower()
    resolved_port = port if port is not None else (443 if scheme == "https" else 80)
    if not 1 <= resolved_port <= 65_535:
        raise EgressDeniedError("the provider endpoint port is out of range")
    path = posixpath.normpath(parts.path or "/")
    if not path.startswith("/"):
        path = f"/{path}"
    return _Destination(scheme=scheme, host=host, port=resolved_port, path=path)


def _normalized_destination(value: object) -> str:
    return _parse_destination(value).normalized


def normalize_endpoint(
    value: object,
    *,
    allowed_endpoints: Iterable[object] = (),
    consent: bool = False,
) -> str:
    destination = _parse_destination(value)
    if destination.is_loopback:
        return destination.normalized
    if not destination.is_tls:
        raise EgressDeniedError("plaintext provider endpoints are limited to loopback hosts")
    if not consent:
        raise EgressDeniedError("hosted provider endpoints require explicit egress consent")
    allowlist = frozenset(_normalized_destination(item) for item in allowed_endpoints)
    if destination.normalized not in allowlist:
        raise EgressDeniedError("the hosted provider endpoint is not in the egress allowlist")
    return destination.normalized


def is_loopback_host(host: str) -> bool:
    normalized = host.lower()
    if normalized in LOOPBACK_HOSTS:
        return True
    try:
        return ipaddress.ip_address(normalized).is_loopback
    except ValueError:
        return False


class OpenAICompatibleClient:
    def __init__(
        self,
        *,
        endpoint: str,
        model_id: str,
        allowed_endpoints: Iterable[str] = (),
        api_key: str | None = None,
        consent: bool = False,
        provider_id: str = OPENAI_COMPATIBLE_PROVIDER_ID,
        timeout_seconds: float = 30.0,
        max_response_bytes: int = MAX_RESPONSE_BYTES,
        transport: httpx.BaseTransport | None = None,
        input_cost_per_million_tokens: Decimal = Decimal("0"),
        output_cost_per_million_tokens: Decimal = Decimal("0"),
    ) -> None:
        self._model_id = _bounded_identifier(model_id, "model_id")
        self._provider_id = _bounded_identifier(provider_id, "provider_id")
        self._allowed = frozenset(_normalized_destination(item) for item in allowed_endpoints)
        self._endpoint = normalize_endpoint(
            endpoint,
            allowed_endpoints=self._allowed,
            consent=bool(consent),
        )
        self._consent = bool(consent)
        self._api_key = api_key
        self._timeout = _bounded_float(
            timeout_seconds,
            "timeout_seconds",
            minimum=MIN_TIMEOUT_SECONDS,
            maximum=MAX_TIMEOUT_SECONDS,
        )
        if isinstance(max_response_bytes, bool) or not isinstance(max_response_bytes, int):
            raise ModelRequestError("max_response_bytes must be an integer")
        if not 1 <= max_response_bytes <= MAX_RESPONSE_BYTES:
            raise ModelRequestError("max_response_bytes is out of range")
        self._max_response_bytes = max_response_bytes
        self._transport = transport
        self._input_cost = _nonnegative_decimal(
            input_cost_per_million_tokens, "input_cost_per_million_tokens"
        )
        self._output_cost = _nonnegative_decimal(
            output_cost_per_million_tokens, "output_cost_per_million_tokens"
        )
        if api_key is not None and (not api_key.strip() or len(api_key) > 1_024):
            raise ModelRequestError("api_key must be a bounded nonempty string")

    @property
    def provider_id(self) -> str:
        return self._provider_id

    @property
    def model_id(self) -> str:
        return self._model_id

    @property
    def endpoint(self) -> str:
        return self._endpoint

    @property
    def allowed_endpoints(self) -> frozenset[str]:
        return self._allowed

    @property
    def requires_egress(self) -> bool:
        return True

    def supports(self, capability: ModelCapability) -> bool:
        return ModelCapability(capability) in {
            ModelCapability.STRUCTURED_OUTPUT,
            ModelCapability.SEED,
        }

    def _authorize_destination(self) -> None:
        destination = _parse_destination(self._endpoint)
        if not self._consent:
            raise EgressDeniedError("outbound provider calls require explicit egress consent")
        if not destination.is_loopback and not destination.is_tls:
            raise EgressDeniedError("plaintext provider endpoints are limited to loopback hosts")
        if self._endpoint not in self._allowed:
            raise EgressDeniedError("the provider endpoint is not in the egress allowlist")

    def _headers(self) -> dict[str, str]:
        headers = {"content-type": "application/json"}
        if self._api_key is not None:
            headers["authorization"] = f"Bearer {self._api_key}"
        return headers

    def _payload(self, request: ModelRequest) -> bytes:
        messages: list[dict[str, str]] = [message.to_payload() for message in request.messages]
        payload: dict[str, JSONValue] = {
            "max_tokens": request.max_output_tokens,
            "messages": [cast(JSONValue, message) for message in messages],
            "model": self._model_id,
        }
        if request.seed is not None:
            payload["seed"] = request.seed
        if request.response_schema is not None:
            if not self.supports(ModelCapability.STRUCTURED_OUTPUT):
                raise ModelCapabilityError("the provider does not declare structured output")
            payload["response_format"] = {
                "json_schema": {
                    "name": "kala_response",
                    "schema": _thaw_json(cast(JSONValue, dict(request.response_schema))),
                    "strict": True,
                },
                "type": "json_schema",
            }
        if request.artifacts:
            raise EgressDeniedError("artifact payloads are never sent to a remote provider")
        return canonical_json_dumps(payload, max_bytes=MAX_REQUEST_BYTES).encode("utf-8")

    def complete(self, request: ModelRequest) -> ModelResponse:
        import httpx

        if not isinstance(request, ModelRequest):
            raise TypeError("request must be a ModelRequest")
        self._authorize_destination()
        for capability in sorted(request.capabilities, key=lambda item: item.value):
            if not self.supports(capability):
                raise ModelCapabilityError(
                    f"provider {self._provider_id} does not declare {capability.value}"
                )
        body = self._payload(request)
        timeout = httpx.Timeout(min(request.timeout_seconds, self._timeout))
        try:
            with httpx.Client(
                transport=self._transport,
                timeout=timeout,
                follow_redirects=False,
                trust_env=False,
            ) as client:
                with client.stream(
                    "POST",
                    self._endpoint,
                    content=body,
                    headers=self._headers(),
                ) as response:
                    if response.status_code != 200:
                        raise ProviderUnavailableError(
                            f"provider {self._provider_id} returned status {response.status_code}"
                        )
                    collected = bytearray()
                    for chunk in response.iter_bytes():
                        if len(collected) + len(chunk) > self._max_response_bytes:
                            raise ModelProviderError(
                                "the provider response exceeds the bounded size"
                            )
                        collected.extend(chunk)
        except (httpx.TimeoutException, httpx.TransportError) as error:
            raise ProviderUnavailableError(
                f"provider {self._provider_id} is unreachable: {type(error).__name__}"
            ) from None
        return self._parse_response(request, bytes(collected))

    def _parse_response(self, request: ModelRequest, payload: bytes) -> ModelResponse:
        try:
            decoded = strict_json_loads(payload, max_bytes=self._max_response_bytes)
        except (JsonEnvelopeError, TypeError) as error:
            raise ModelProviderError("the provider response is not strict JSON") from error
        if not isinstance(decoded, Mapping):
            raise ModelProviderError("the provider response must be a JSON object")
        choices = decoded.get("choices")
        if not isinstance(choices, list) or not choices:
            raise ModelProviderError("the provider response contains no choices")
        first = choices[0]
        if not isinstance(first, Mapping):
            raise ModelProviderError("the provider choice is malformed")
        message = first.get("message")
        if not isinstance(message, Mapping):
            raise ModelProviderError("the provider choice has no message")
        content = message.get("content")
        if not isinstance(content, str) or not content.strip():
            raise ModelProviderError("the provider returned no textual content")
        usage = decoded.get("usage")
        prompt_tokens = 0
        completion_tokens = 0
        if isinstance(usage, Mapping):
            prompt_tokens = _provider_int(usage.get("prompt_tokens"))
            completion_tokens = _provider_int(usage.get("completion_tokens"))
        finish_reason = first.get("finish_reason")
        try:
            resolved_finish = ModelFinishReason(
                finish_reason if isinstance(finish_reason, str) else "stop"
            )
        except ValueError:
            resolved_finish = ModelFinishReason.STOP
        provider_request_id = decoded.get("id")
        cost = self._cost(prompt_tokens, completion_tokens)
        return ModelResponse(
            request_id=request.request_id,
            provider_id=self._provider_id,
            model_id=str(decoded.get("model", self._model_id)),
            content=content,
            structured=None,
            input_tokens=prompt_tokens,
            output_tokens=completion_tokens,
            cost=cost,
            finish_reason=resolved_finish,
            provider_request_id=(
                str(provider_request_id) if isinstance(provider_request_id, str) else None
            ),
        )

    def _cost(self, input_tokens: int, output_tokens: int) -> Decimal:
        million = Decimal(1_000_000)
        return (
            Decimal(input_tokens) * self._input_cost + Decimal(output_tokens) * self._output_cost
        ) / million

    def _sanitize(self, text: str) -> str:
        if self._api_key is None:
            return text
        return text.replace(self._api_key, "<redacted>")

    def __repr__(self) -> str:
        return (
            f"{type(self).__name__}(provider_id={self._provider_id!r}, "
            f"model_id={self._model_id!r}, endpoint={self._endpoint!r}, "
            f"api_key={'<redacted>' if self._api_key is not None else None}, "
            f"consent={self._consent})"
        )

    def __str__(self) -> str:
        return self._sanitize(repr(self))


def _provider_int(value: object) -> int:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return 0
    if isinstance(value, float) and not math.isfinite(value):
        return 0
    return max(0, int(value))


class ModelGateway:
    def __init__(
        self,
        client: LLMClient,
        *,
        ledger: BudgetLedger | None = None,
        transcript: TranscriptCallback | None = None,
        policy: PolicyEngine | None = None,
        egress_manifest: EgressManifestCallback | None = None,
        clock: Callable[[], datetime] | None = None,
    ) -> None:
        for name in ("provider_id", "model_id", "complete", "supports", "requires_egress"):
            if not hasattr(client, name):
                raise TypeError("client must implement the LLMClient protocol")
        if not callable(client.complete):
            raise TypeError("client.complete must be callable")
        if transcript is not None and not callable(transcript):
            raise TypeError("transcript must be callable")
        if egress_manifest is not None and not callable(egress_manifest):
            raise TypeError("egress_manifest must be callable")
        if policy is not None and not isinstance(policy, PolicyEngine):
            raise TypeError("policy must be a PolicyEngine")
        if clock is not None and not callable(clock):
            raise TypeError("clock must be callable")
        self._client = client
        self._ledger = ledger
        self._transcript = transcript
        self._policy = policy
        self._egress_manifest = egress_manifest
        self._clock = clock if clock is not None else lambda: datetime.now(UTC)
        self._lock = RLock()
        self._transcripts: list[ModelTranscript] = []
        self._manifests: list[ModelEgressManifest] = []

    @property
    def client(self) -> LLMClient:
        return self._client

    @property
    def policy(self) -> PolicyEngine | None:
        return self._policy

    @property
    def provider_id(self) -> str:
        return str(self._client.provider_id)

    @property
    def model_id(self) -> str:
        return str(self._client.model_id)

    @property
    def requires_egress(self) -> bool:
        return bool(self._client.requires_egress)

    @property
    def destination(self) -> str:
        endpoint: object = getattr(self._client, "endpoint", None)
        if isinstance(endpoint, str) and endpoint:
            return endpoint
        return f"provider:{self.provider_id}"

    @property
    def transcripts(self) -> tuple[ModelTranscript, ...]:
        with self._lock:
            return tuple(self._transcripts)

    @property
    def manifests(self) -> tuple[ModelEgressManifest, ...]:
        with self._lock:
            return tuple(self._manifests)

    def complete(self, request: ModelRequest) -> ModelResponse:
        if not isinstance(request, ModelRequest):
            raise TypeError("request must be a ModelRequest")
        sanitized, redactions = self._sanitize(request)
        manifest = self._authorize_egress(sanitized, redactions)
        self._check_capabilities(sanitized)
        reservation = self._reserve(sanitized)
        try:
            return self._complete_reserved(sanitized, redactions, manifest, reservation)
        except BaseException:
            self._release_if_held(reservation)
            raise

    def _complete_reserved(
        self,
        sanitized: ModelRequest,
        redactions: Sequence[ModelRedaction],
        manifest: ModelEgressManifest | None,
        reservation: BudgetReservation | None,
    ) -> ModelResponse:
        if manifest is not None:
            self._record_manifest(manifest)
        started = self._now()
        try:
            raw = self._client.complete(sanitized)
        except ProviderUnavailableError as error:
            raise ProviderDegradedError(
                self.provider_id, self.model_id, self._safe_reason(error)
            ) from None
        except ModelError:
            raise
        except Exception as error:
            raise ProviderDegradedError(
                self.provider_id, self.model_id, type(error).__name__
            ) from None
        elapsed = max(0.0, (self._now() - started).total_seconds())
        response = self._verify_identity(raw)
        if sanitized.response_schema is not None:
            response = self._validate_structured(response, sanitized.response_schema)
        self._record(
            sanitized,
            response,
            redactions,
            reservation,
            elapsed_seconds=elapsed,
        )
        return response

    def _check_capabilities(self, request: ModelRequest) -> None:
        for capability in sorted(request.capabilities, key=lambda item: item.value):
            if not self._client.supports(capability):
                raise ModelCapabilityError(
                    f"provider {self.provider_id} does not declare {capability.value}"
                )

    def egress_policy_request(self, request: ModelRequest) -> PolicyRequest:
        if not isinstance(request, ModelRequest):
            raise TypeError("request must be a ModelRequest")
        sanitized, redactions = self._sanitize(request)
        return self._egress_policy_request(sanitized, redactions)

    def _egress_policy_request(
        self,
        request: ModelRequest,
        redactions: Sequence[ModelRedaction],
    ) -> PolicyRequest:
        return PolicyRequest(
            request_id=self._egress_request_id(request),
            state_digest=self._egress_state_digest(request, redactions),
            action_digest=self._egress_action_digest(request, redactions),
            requested_capabilities=EGRESS_CAPABILITIES,
            side_effect_level=SideEffectLevel.EGRESS,
            destructive=False,
            egress=True,
            checkpoint_type=ApprovalKind.EGRESS,
        )

    def egress_state_digest(self, request: ModelRequest) -> str:
        if not isinstance(request, ModelRequest):
            raise TypeError("request must be a ModelRequest")
        sanitized, redactions = self._sanitize(request)
        return self._egress_state_digest(sanitized, redactions)

    def egress_action_digest(self, request: ModelRequest) -> str:
        if not isinstance(request, ModelRequest):
            raise TypeError("request must be a ModelRequest")
        sanitized, redactions = self._sanitize(request)
        return self._egress_action_digest(sanitized, redactions)

    @staticmethod
    def _egress_request_id(request: ModelRequest) -> str:
        return f"egress-{request.request_id}"

    @staticmethod
    def _redaction_digest(redactions: Sequence[ModelRedaction]) -> str:
        return canonical_json_digest(
            sorted(
                (
                    {
                        "kind": redaction.kind,
                        "reason": redaction.reason,
                        "subject": redaction.subject,
                    }
                    for redaction in redactions
                ),
                key=canonical_json_dumps,
            )
        )

    def _egress_state_digest(
        self,
        request: ModelRequest,
        redactions: Sequence[ModelRedaction],
    ) -> str:
        return canonical_json_digest(
            {
                "destination": self.destination,
                "model_id": self.model_id,
                "provider": self.provider_id,
                "redactions": self._redaction_digest(redactions),
                "request": self._egress_request_id(request),
            }
        )

    def _egress_action_digest(
        self,
        request: ModelRequest,
        redactions: Sequence[ModelRedaction],
    ) -> str:
        return canonical_json_digest(
            {
                "capabilities": list(EGRESS_CAPABILITIES),
                "destination": self.destination,
                "model_id": self.model_id,
                "provider": self.provider_id,
                "redactions": self._redaction_digest(redactions),
                "request_digest": request.request_digest,
            }
        )

    def _authorize_egress(
        self,
        request: ModelRequest,
        redactions: Sequence[ModelRedaction],
    ) -> ModelEgressManifest | None:
        if not self.requires_egress:
            return None
        policy = self._policy
        if policy is None:
            raise EgressDeniedError(
                f"provider {self.provider_id} requires egress and no policy engine is attached"
            )
        decision = policy.decide(self._egress_policy_request(request, redactions))
        if not decision.allowed:
            raise EgressDeniedError(
                f"egress policy denied the provider call: {decision.reason.value}"
            )
        return ModelEgressManifest(
            request_id=request.request_id,
            egress_request_id=self._egress_request_id(request),
            provider_id=self.provider_id,
            model_id=self.model_id,
            destination=self.destination,
            request_digest=request.request_digest,
            state_digest=self._egress_state_digest(request, redactions),
            action_digest=self._egress_action_digest(request, redactions),
            capabilities=EGRESS_CAPABILITIES,
            redactions=tuple(redactions),
            artifact_digests=tuple(artifact.digest for artifact in request.artifacts),
            recorded_at=self._now(),
        )

    def _record_manifest(self, manifest: ModelEgressManifest) -> None:
        callback = self._egress_manifest
        if callback is None:
            raise EgressDeniedError(
                f"provider {self.provider_id} requires egress and no egress manifest is attached"
            )
        with self._lock:
            self._manifests.append(manifest)
        try:
            callback(manifest)
        except Exception as error:
            raise EgressDeniedError(
                f"the egress manifest could not be recorded: {type(error).__name__}"
            ) from error

    def _sanitize(
        self,
        request: ModelRequest,
    ) -> tuple[ModelRequest, tuple[ModelRedaction, ...]]:
        redactions: list[ModelRedaction] = []
        artifacts: list[ModelArtifact] = []
        for artifact in request.artifacts:
            sanitized, artifact_redactions = self._sanitize_artifact(artifact)
            artifacts.append(sanitized)
            redactions.extend(artifact_redactions)
        messages: list[ModelMessage] = []
        for message in request.messages:
            content, findings = redact_secrets(message.content)
            if findings:
                redactions.append(
                    ModelRedaction(
                        kind="secret",
                        subject=message.role.value,
                        reason="credential-shaped text was removed from the transcript",
                    )
                )
            messages.append(ModelMessage(role=message.role, content=content))
        sanitized_request = replace(
            request,
            messages=tuple(messages),
            artifacts=tuple(artifacts),
        )
        return sanitized_request, tuple(redactions)

    @staticmethod
    def _sanitize_artifact(
        artifact: ModelArtifact,
    ) -> tuple[ModelArtifact, tuple[ModelRedaction, ...]]:
        if artifact.is_design_binary:
            return artifact.redacted(), (
                ModelRedaction(
                    kind="design_binary",
                    subject=artifact.artifact_id,
                    reason="design binaries are referenced by digest only",
                ),
            )
        preview_text = artifact.preview_text
        preview_findings = 0
        if preview_text is not None:
            preview_text, preview_findings = redact_secrets(preview_text)
        metadata, metadata_findings = _redact_metadata(artifact.metadata)
        if preview_findings == 0 and metadata_findings == 0:
            return artifact, ()
        redactions: list[ModelRedaction] = []
        if preview_findings:
            redactions.append(
                ModelRedaction(
                    kind="secret",
                    subject=f"{artifact.artifact_id}.preview_text",
                    reason="credential-shaped artifact preview text was redacted",
                )
            )
        if metadata_findings:
            redactions.append(
                ModelRedaction(
                    kind="secret",
                    subject=f"{artifact.artifact_id}.metadata",
                    reason="credential-shaped artifact metadata was redacted",
                )
            )
        return (
            replace(
                artifact,
                preview_text=preview_text,
                metadata=metadata,
            ),
            tuple(redactions),
        )

    def _verify_identity(self, response: ModelResponse) -> ModelResponse:
        if not isinstance(response, ModelResponse):
            raise ModelProviderError("the client did not return a ModelResponse")
        if response.provider_id != self.provider_id or response.model_id != self.model_id:
            raise ProviderSubstitutionError(
                f"provider {response.provider_id} model {response.model_id} does not match the "
                f"configured provider {self.provider_id} model {self.model_id}"
            )
        return response

    def _validate_structured(
        self,
        response: ModelResponse,
        schema: Mapping[str, object],
    ) -> ModelResponse:
        try:
            decoded = strict_json_loads(response.content, max_bytes=MAX_RESPONSE_BYTES)
        except (JsonEnvelopeError, TypeError) as error:
            raise StructuredOutputError("the model response is not strict JSON") from error
        if not isinstance(decoded, Mapping):
            raise StructuredOutputError("the model response is not a JSON object")
        try:
            validate_against_schema(decoded, schema, path="$")
        except SchemaValidationError as error:
            raise StructuredOutputError(f"the model response failed its schema: {error}") from error
        return replace(
            response,
            structured=cast(Mapping[str, JSONValue], decoded),
            content=canonical_json_dumps(
                _thaw_json(cast(JSONValue, decoded)), max_bytes=MAX_RESPONSE_BYTES
            ),
        )

    def _estimate(self, request: ModelRequest) -> tuple[int, Decimal]:
        prompt_tokens = sum(estimate_tokens(message.content) for message in request.messages)
        return prompt_tokens, Decimal("0")

    def _reserve(self, request: ModelRequest) -> BudgetReservation | None:
        if self._ledger is None:
            return None
        prompt_tokens, cost = self._estimate(request)
        return self._ledger.reserve(
            model_calls=1,
            total_tokens=prompt_tokens + request.max_output_tokens,
            cost=cost,
        )

    def _release_if_held(self, reservation: BudgetReservation | None) -> None:
        if self._ledger is None or reservation is None:
            return
        with contextlib.suppress(BudgetReservationUnknownError):
            self._ledger.release(reservation)

    def _record(
        self,
        request: ModelRequest,
        response: ModelResponse,
        redactions: Sequence[ModelRedaction],
        reservation: BudgetReservation | None,
        *,
        elapsed_seconds: float,
    ) -> None:
        if self._ledger is not None and reservation is not None:
            self._ledger.commit(
                reservation,
                model_calls=1,
                total_tokens=response.total_tokens,
                cost=response.cost,
                duration_seconds=elapsed_seconds,
            )
        request_body, request_findings = redact_secrets(request.request_body)
        response_body, response_findings = redact_secrets(response.canonical_content())
        entries = list(redactions)
        if request_findings:
            entries.append(
                ModelRedaction(
                    kind="secret",
                    subject="request",
                    reason="credential-shaped text was removed from the transcript",
                )
            )
        if response_findings:
            entries.append(
                ModelRedaction(
                    kind="secret",
                    subject="response",
                    reason="credential-shaped text was removed from the transcript",
                )
            )
        if len(request_body.encode("utf-8")) > MAX_TRANSCRIPT_BODY_BYTES:
            entries.append(
                ModelRedaction(
                    kind="truncated",
                    subject="request",
                    reason="the transcript request body was truncated",
                )
            )
            request_body = truncate_encoded(request_body, MAX_TRANSCRIPT_BODY_BYTES)
        if len(response_body.encode("utf-8")) > MAX_TRANSCRIPT_BODY_BYTES:
            entries.append(
                ModelRedaction(
                    kind="truncated",
                    subject="response",
                    reason="the transcript response body was truncated",
                )
            )
            response_body = truncate_encoded(response_body, MAX_TRANSCRIPT_BODY_BYTES)
        transcript = ModelTranscript(
            request_id=request.request_id,
            provider_id=response.provider_id,
            model_id=response.model_id,
            request_digest=request.request_digest,
            response_digest=response.response_digest,
            request_body=request_body,
            response_body=response_body,
            input_tokens=response.input_tokens,
            output_tokens=response.output_tokens,
            cost=response.cost,
            redactions=tuple(entries),
            artifact_digests=tuple(artifact.digest for artifact in request.artifacts),
            recorded_at=self._now(),
        )
        with self._lock:
            self._transcripts.append(transcript)
        if self._transcript is not None:
            try:
                self._transcript(transcript)
            except Exception as error:
                raise TranscriptError(
                    f"the model transcript could not be persisted: {type(error).__name__}"
                ) from error

    def _now(self) -> datetime:
        value = self._clock()
        if value.tzinfo is None or value.utcoffset() is None:
            raise ModelRequestError("the gateway clock must return an aware datetime")
        return value.astimezone(UTC)

    def _safe_reason(self, error: ProviderUnavailableError) -> str:
        text = str(error)
        _, findings = redact_secrets(text)
        return text if findings == 0 else "<redacted provider error>"


__all__ = (
    "EGRESS_CAPABILITIES",
    "MAX_RESPONSE_BYTES",
    "MAX_TRANSCRIPT_BODY_BYTES",
    "OPENAI_COMPATIBLE_PROVIDER_ID",
    "EgressDeniedError",
    "EgressManifestCallback",
    "LLMClient",
    "LocalModelClient",
    "ModelArtifact",
    "ModelCapability",
    "ModelCapabilityError",
    "ModelEgressManifest",
    "ModelError",
    "ModelFinishReason",
    "ModelGateway",
    "ModelMessage",
    "ModelProviderError",
    "ModelRedaction",
    "ModelRequest",
    "ModelRequestError",
    "ModelResponse",
    "ModelRole",
    "ModelTranscript",
    "OpenAICompatibleClient",
    "ProviderDegradedError",
    "ProviderSubstitutionError",
    "ProviderUnavailableError",
    "StructuredOutputError",
    "TranscriptCallback",
    "TranscriptError",
    "estimate_tokens",
    "is_loopback_host",
    "is_secret_name",
    "normalize_endpoint",
    "redact_secrets",
    "truncate_encoded",
)
