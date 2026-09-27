from __future__ import annotations

import math
import os
import re
from collections.abc import Mapping
from pathlib import Path
from typing import Annotated, ClassVar, Self, TypeAlias, cast

from pydantic import (
    AnyHttpUrl,
    BaseModel,
    ConfigDict,
    Field,
    StringConstraints,
    field_validator,
)

_PROVIDER_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
_WORKER_PATTERN = re.compile(r"^[0-9]+$")
_FloatConvertible: TypeAlias = str | bytes | bytearray | int | float
ProviderName = Annotated[
    str,
    StringConstraints(strip_whitespace=True, min_length=1, max_length=64),
]
ModelIdentifier = Annotated[
    str,
    StringConstraints(strip_whitespace=True, min_length=1, max_length=256),
]


def _default_data_dir() -> Path:
    return Path.home() / ".local" / "share" / "kala-agent"


class Settings(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(
        extra="forbid",
        frozen=True,
        validate_default=True,
    )

    data_dir: Path = Field(default_factory=_default_data_dir)
    model_provider: ProviderName = Field(default="local")
    model_id: ModelIdentifier = Field(default="local")
    model_endpoint: AnyHttpUrl | None = None
    request_timeout_seconds: float = Field(default=120.0, ge=0.0)
    tool_timeout_seconds: float = Field(default=300.0, ge=0.0)
    worker_count: int = Field(default=4, ge=1, le=8)

    _environment_fields: ClassVar[tuple[tuple[str, str], ...]] = (
        ("KALA_AGENT_DATA_DIR", "data_dir"),
        ("KALA_AGENT_MODEL_PROVIDER", "model_provider"),
        ("KALA_AGENT_MODEL_ID", "model_id"),
        ("KALA_AGENT_MODEL_ENDPOINT", "model_endpoint"),
        ("KALA_AGENT_REQUEST_TIMEOUT_SECONDS", "request_timeout_seconds"),
        ("KALA_AGENT_TOOL_TIMEOUT_SECONDS", "tool_timeout_seconds"),
        ("KALA_AGENT_WORKER_COUNT", "worker_count"),
    )

    @field_validator("data_dir", mode="before")
    @classmethod
    def normalize_data_dir(cls, value: object) -> Path:
        if isinstance(value, Path):
            candidate = value
        elif isinstance(value, str):
            candidate = Path(value)
        else:
            raise ValueError("data_dir must be a path")

        if "\x00" in str(candidate):
            raise ValueError("data_dir contains an invalid character")
        expanded = candidate.expanduser()
        if not expanded.is_absolute():
            raise ValueError("data_dir must be absolute")
        try:
            resolved = expanded.resolve(strict=False)
        except (OSError, RuntimeError) as error:
            raise ValueError("data_dir cannot be resolved") from error
        if resolved == Path(resolved.anchor):
            raise ValueError("data_dir cannot be a filesystem root")
        if resolved.exists() and not resolved.is_dir():
            raise ValueError("data_dir must identify a directory")
        return resolved

    @field_validator("model_provider")
    @classmethod
    def validate_model_provider(cls, value: str) -> str:
        normalized = value.strip().lower()
        if _PROVIDER_PATTERN.fullmatch(normalized) is None:
            raise ValueError("model_provider contains unsupported characters")
        return normalized

    @field_validator("model_endpoint")
    @classmethod
    def validate_model_endpoint(cls, value: AnyHttpUrl | None) -> AnyHttpUrl | None:
        if value is None:
            return None
        if value.username is not None or value.password is not None:
            raise ValueError("model_endpoint cannot contain credentials")
        if value.query or value.fragment:
            raise ValueError("model_endpoint cannot contain query parameters or fragments")
        return value

    @field_validator("request_timeout_seconds", "tool_timeout_seconds", mode="before")
    @classmethod
    def parse_timeout(cls, value: object) -> float:
        if isinstance(value, bool):
            raise ValueError("timeouts must be numeric")
        try:
            parsed = float(cast(_FloatConvertible, value))
        except (OverflowError, TypeError, ValueError) as error:
            raise ValueError("timeouts must be numeric") from error
        if not math.isfinite(parsed) or parsed < 0.0:
            raise ValueError("timeouts must be finite and nonnegative")
        return parsed

    @field_validator("worker_count", mode="before")
    @classmethod
    def parse_worker_count(cls, value: object) -> int:
        if isinstance(value, bool):
            raise ValueError("worker_count must be an integer")
        if isinstance(value, int):
            return value
        if isinstance(value, str) and _WORKER_PATTERN.fullmatch(value.strip()) is not None:
            return int(value)
        raise ValueError("worker_count must be an integer")

    @property
    def artifact_dir(self) -> Path:
        return self.data_dir / "artifacts"

    @property
    def journal_path(self) -> Path:
        return self.data_dir / "journal.sqlite3"

    @property
    def database_path(self) -> Path:
        return self.data_dir / "kala.sqlite3"

    @property
    def transcript_dir(self) -> Path:
        return self.data_dir / "transcripts"

    @classmethod
    def from_env(cls, environ: Mapping[str, str] | None = None) -> Self:
        source = os.environ if environ is None else environ
        values = {
            field_name: source[environment_name]
            for environment_name, field_name in cls._environment_fields
            if environment_name in source
        }
        return cls.model_validate(values)
