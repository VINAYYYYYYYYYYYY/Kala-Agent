"""A construction script every CAD adapter can compile.

The model writes operations and millimetre sizes. It does not write FreeCAD
Python, Fusion Python, or a SolidWorks macro. An adapter lowers the same list
into the script language of the CAD software it owns.
"""

from __future__ import annotations

import math
import re
from collections.abc import Mapping
from enum import StrEnum
from typing import ClassVar, Final, Self, TypeVar

from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    StrictBool,
    ValidationError,
    ValidationInfo,
    field_validator,
    model_validator,
)

MAX_COMPONENTS: Final[int] = 64
MAX_STEPS: Final[int] = 500
#: The whole-script step budget: every component's steps plus the assembly
#: steps. A per-body cap alone would either stop an honest one-body design or
#: leave the model input unbounded.
MAX_TOTAL_STEPS: Final[int] = 500
MAX_DIMENSION_MM: Final[float] = 5_000.0

_MIN_DIMENSION_MM: Final[float] = 0.001
_MAX_ASSEMBLY_STEPS: Final[int] = 16
_MAX_ANALYSIS_STEPS: Final[int] = 8
_MAX_EDITS: Final[int] = 4
_MAX_VERDICT_CHARS: Final[int] = 500
_MAX_FACE: Final[int] = 64
_MAX_FORCE_N: Final[float] = 1_000_000.0
_MAX_PRESSURE_MPA: Final[float] = 5_000.0
_FULL_TURN: Final[float] = 360.0
_NAME_PATTERN: Final[re.Pattern[str]] = re.compile(r"^[A-Za-z][A-Za-z0-9_-]{0,39}$")

COMPONENT_OPS: Final[frozenset[str]] = frozenset(
    {"sketch", "extrude", "revolve", "pocket", "helix"}
)
ASSEMBLY_OPS: Final[frozenset[str]] = frozenset({"place", "fuse", "compound"})
ANALYSIS_OPS: Final[frozenset[str]] = frozenset(
    {"material", "fix", "force", "pressure", "mesh", "solve"}
)
EDIT_OPS: Final[frozenset[str]] = frozenset({"move", "resize", "fillet"})
_PLANES: Final[frozenset[str]] = frozenset({"XY", "XZ", "YZ"})
_PROFILES: Final[frozenset[str]] = frozenset({"rectangle", "circle"})
_PROFILE_ALIASES: Final[dict[str, str]] = {
    "box": "rectangle",
    "rect": "rectangle",
    "rectangle": "rectangle",
    "rectangular": "rectangle",
    "square": "rectangle",
    "circle": "circle",
    "circular": "circle",
    "cylinder": "circle",
    "cylindrical": "circle",
    "disc": "circle",
    "disk": "circle",
    "round": "circle",
}
_SOLID_OPS: Final[frozenset[str]] = frozenset({"extrude", "revolve"})
_ANALYSIS_RANK: Final[dict[str, int]] = {
    "material": 1,
    "fix": 2,
    "force": 3,
    "pressure": 3,
    "mesh": 4,
    "solve": 5,
}
_VALUE_ERROR_PREFIX: Final[str] = "Value error, "


class ConstructionError(ValueError):
    """The construction script is not in the shared protocol."""


class RevisionAction(StrEnum):
    """The decision model's choice after analysis."""

    ACCEPT = "accept"
    TOOL_EDIT = "tool_edit"
    SCRIPT_REWRITE = "script_rewrite"


class _ScriptModel(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(
        extra="forbid",
        frozen=True,
        validate_default=True,
    )


def _finite_number(value: object, label: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{label} must be a finite number")
    number = float(value)
    if not math.isfinite(number):
        raise ValueError(f"{label} must be a finite number")
    return number


def _optional_size(value: object, info: ValidationInfo) -> float | None:
    if value is None:
        return None
    label = info.field_name or "size"
    number = _finite_number(value, label)
    if number < _MIN_DIMENSION_MM or number > MAX_DIMENSION_MM:
        raise ValueError(f"{label} is outside the model bounds")
    return number


def _coordinate(value: object, info: ValidationInfo) -> float:
    label = info.field_name or "coordinate"
    number = _finite_number(value, label)
    if abs(number) > MAX_DIMENSION_MM:
        raise ValueError(f"{label} is outside the model bounds")
    return number


def _optional_open_interval(value: object, label: str, high: float) -> float | None:
    if value is None:
        return None
    number = _finite_number(value, label)
    if not 0.0 < number <= high:
        raise ValueError(f"{label} is outside the model bounds")
    return number


def _known_operation(value: object, allowed: frozenset[str]) -> str:
    if isinstance(value, str) and value in allowed:
        return value
    if isinstance(value, str) and value:
        raise ValueError(f"unknown operation {value}")
    raise ValueError("unknown operation")


def _component_name(value: str) -> str:
    if _NAME_PATTERN.fullmatch(value) is None:
        raise ValueError("a component name must match ^[A-Za-z][A-Za-z0-9_-]{0,39}$")
    return value


def _text(value: object, label: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{label} must be nonempty text")
    text = value.strip()
    if not text:
        raise ValueError(f"{label} must be nonempty text")
    return text


def _bounded_text(value: object, label: str, limit: int) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{label} must be nonempty text")
    text = value.strip()
    if not text or len(text) > limit:
        raise ValueError(f"{label} must be nonempty and at most {limit} characters")
    return text


def _require(value: float | None, message: str) -> None:
    if value is None:
        raise ValueError(message)


class ComponentStep(_ScriptModel):
    """One operation inside a component."""

    op: str
    plane: str = "XY"
    profile: str | None = None
    width: float | None = None
    height: float | None = None
    diameter: float | None = None
    length: float | None = None
    pitch: float | None = None
    angle: float | None = None
    x: float = 0
    y: float = 0
    reversed: StrictBool = False

    @model_validator(mode="before")
    @classmethod
    def flatten_profile(cls, value: object) -> object:
        return _flatten_profile(value)

    @field_validator("op", mode="before")
    @classmethod
    def check_op(cls, value: object) -> str:
        return _known_operation(value, COMPONENT_OPS)

    @field_validator("plane")
    @classmethod
    def check_plane(cls, value: str) -> str:
        if value not in _PLANES:
            raise ValueError("plane must be XY, XZ, or YZ")
        return value

    @field_validator("profile", mode="before")
    @classmethod
    def check_profile(cls, value: object) -> str | None:
        if value is None:
            return None
        if not isinstance(value, str) or not value.strip():
            raise ValueError("profile must be rectangle or circle")
        return _PROFILE_ALIASES.get(value.strip().casefold(), value.strip().casefold())

    @field_validator(
        "width",
        "height",
        "diameter",
        "length",
        "pitch",
        "angle",
        mode="before",
    )
    @classmethod
    def check_sizes(cls, value: object, info: ValidationInfo) -> float | None:
        return _optional_size(value, info)

    @field_validator("x", "y", mode="before")
    @classmethod
    def check_coordinates(cls, value: object, info: ValidationInfo) -> float:
        return _coordinate(value, info)

    @model_validator(mode="after")
    def check_operation(self) -> Self:
        if self.op == "sketch":
            _normalize_profile(self)
            _check_sketch(self)
        elif self.op == "extrude":
            _require(self.length, "extrude requires a length")
        elif self.op == "revolve":
            if self.angle is None:
                object.__setattr__(self, "angle", _FULL_TURN)
            elif self.angle > _FULL_TURN:
                raise ValueError("revolve angle must be in (0, 360]")
        elif self.op == "helix":
            _require(self.diameter, "helix requires a diameter")
            _require(self.pitch, "helix requires a pitch")
            _require(self.length, "helix requires a length")
        if self.reversed and self.op != "extrude":
            raise ValueError("reversed is only meaningful on extrude")
        return self


def _flatten_profile(value: object) -> object:
    """Lift a nested profile object onto the sketch step.

    The model writes ``profile: {circle: {diameter}}`` rather than a profile name.
    """
    if not isinstance(value, dict):
        return value
    profile = value.get("profile")
    if not isinstance(profile, Mapping):
        return value
    flat = dict(value)
    shape: str | None = None
    body: Mapping[str, object] | None = None
    for key, item in profile.items():
        if not isinstance(key, str):
            continue
        name = _PROFILE_ALIASES.get(key.casefold())
        if name is not None and isinstance(item, Mapping):
            shape = name
            body = item
            break
    if shape is None:
        named = profile.get("type") or profile.get("kind") or profile.get("shape")
        if isinstance(named, str) and named.strip():
            shape = _PROFILE_ALIASES.get(named.strip().casefold(), named.strip().casefold())
            body = profile
    if shape is not None:
        flat["profile"] = shape
    if body is not None:
        for size in ("width", "height", "diameter", "length", "pitch"):
            if size in body and flat.get(size) is None:
                flat[size] = body[size]
    for coord in ("x", "y"):
        if coord in profile and coord not in flat:
            flat[coord] = profile[coord]
    return flat


def _normalize_profile(step: ComponentStep) -> None:
    """Keep a sketch the builder can draw when the model names the shape loosely."""
    if step.profile in _PROFILES:
        return
    if step.diameter is not None and (step.width is None or step.height is None):
        object.__setattr__(step, "profile", "circle")
        return
    if step.width is not None and step.height is not None:
        object.__setattr__(step, "profile", "rectangle")


def _check_sketch(step: ComponentStep) -> None:
    if step.profile == "rectangle":
        if step.width is None or step.height is None:
            raise ValueError("a rectangle sketch requires width and height")
        return
    if step.profile == "circle":
        if step.diameter is None:
            raise ValueError("a circle sketch requires a diameter")
        return
    raise ValueError("profile must be rectangle or circle")


class ComponentScript(_ScriptModel):
    """The operations that build one named solid."""

    name: str
    steps: tuple[ComponentStep, ...] = Field(min_length=1, max_length=MAX_STEPS)

    @field_validator("name")
    @classmethod
    def check_name(cls, value: str) -> str:
        return _component_name(value)

    @model_validator(mode="after")
    def check_sequence(self) -> Self:
        pending = False
        solid = False
        for step in self.steps:
            if step.op == "sketch":
                if pending:
                    raise ValueError("a sketch must be consumed before another sketch")
                pending = True
                continue
            if step.op == "helix":
                if not solid:
                    raise ValueError("a helix must follow a solid")
                if pending:
                    raise ValueError("a sketch must be consumed before a helix")
                continue
            if step.op == "pocket":
                if not solid:
                    raise ValueError("a pocket must follow a solid")
                if not pending:
                    raise ValueError("a pocket must follow a sketch")
                pending = False
                continue
            if step.op in _SOLID_OPS:
                if not pending:
                    raise ValueError(f"{step.op} must follow a sketch")
                pending = False
                solid = True
                continue
            raise ValueError(f"unknown operation {step.op}")
        if not solid:
            raise ValueError("a component must produce a solid")
        if pending:
            raise ValueError("a sketch must be consumed")
        return self


class AssemblyStep(_ScriptModel):
    """Place a component, or fuse or compound several of them."""

    op: str
    component: str | None = None
    components: tuple[str, ...] = ()
    x: float = 0
    y: float = 0
    z: float = 0

    @field_validator("op", mode="before")
    @classmethod
    def check_op(cls, value: object) -> str:
        return _known_operation(value, ASSEMBLY_OPS)

    @field_validator("component")
    @classmethod
    def check_component(cls, value: str | None) -> str | None:
        if value is None:
            return None
        return _component_name(value)

    @field_validator("components")
    @classmethod
    def check_components(cls, value: tuple[str, ...]) -> tuple[str, ...]:
        return tuple(_component_name(name) for name in value)

    @field_validator("x", "y", "z", mode="before")
    @classmethod
    def check_coordinates(cls, value: object, info: ValidationInfo) -> float:
        return _coordinate(value, info)

    @model_validator(mode="after")
    def check_operation(self) -> Self:
        if self.op == "place":
            _require_text(self.component, "place requires a component")
        elif len(self.components) < 2:
            raise ValueError(f"{self.op} requires at least two components")
        return self


def _require_text(value: str | None, message: str) -> None:
    if value is None:
        raise ValueError(message)


class ConstructionScript(_ScriptModel):
    """Components and the assembly steps that join them."""

    summary: str
    components: tuple[ComponentScript, ...] = Field(min_length=1, max_length=MAX_COMPONENTS)
    assembly: tuple[AssemblyStep, ...] = Field(default=(), max_length=_MAX_ASSEMBLY_STEPS)

    @field_validator("summary", mode="before")
    @classmethod
    def check_summary(cls, value: object) -> str:
        return _text(value, "summary")

    @model_validator(mode="after")
    def check_references(self) -> Self:
        names = [component.name for component in self.components]
        if len(names) != len(set(names)):
            raise ValueError("component names must be unique")
        known = set(names)
        for step in self.assembly:
            referenced = _assembly_names(step)
            for name in referenced:
                if name not in known:
                    raise ValueError(f"assembly names a missing component {name}")
        total = sum(len(component.steps) for component in self.components) + len(self.assembly)
        if total > MAX_TOTAL_STEPS:
            raise ValueError(
                f"a script may use at most {MAX_TOTAL_STEPS} steps in total; "
                f"this one uses {total}"
            )
        return self


def _assembly_names(step: AssemblyStep) -> tuple[str, ...]:
    names: list[str] = []
    if step.component is not None:
        names.append(step.component)
    names.extend(step.components)
    return tuple(names)


class AnalysisStep(_ScriptModel):
    """One analysis operation."""

    op: str
    name: str | None = None
    face: int | None = None
    force_n: float | None = None
    pressure_mpa: float | None = None

    @field_validator("op", mode="before")
    @classmethod
    def check_op(cls, value: object) -> str:
        return _known_operation(value, ANALYSIS_OPS)

    @field_validator("face", mode="before")
    @classmethod
    def check_face(cls, value: object) -> int | None:
        if value is None:
            return None
        if isinstance(value, bool) or not isinstance(value, int):
            raise ValueError("face must be an integer from 1 to 64")
        if not 1 <= value <= _MAX_FACE:
            raise ValueError("face must be an integer from 1 to 64")
        return value

    @field_validator("force_n", mode="before")
    @classmethod
    def check_force(cls, value: object) -> float | None:
        return _optional_open_interval(value, "force_n", _MAX_FORCE_N)

    @field_validator("pressure_mpa", mode="before")
    @classmethod
    def check_pressure(cls, value: object) -> float | None:
        return _optional_open_interval(value, "pressure_mpa", _MAX_PRESSURE_MPA)

    @model_validator(mode="after")
    def check_operation(self) -> Self:
        if self.op == "force":
            _require(self.force_n, "force requires force_n")
        elif self.op == "pressure":
            _require(self.pressure_mpa, "pressure requires pressure_mpa")
        return self


class AnalysisScript(_ScriptModel):
    """The analysis operations, in the order a solver expects."""

    steps: tuple[AnalysisStep, ...] = Field(min_length=1, max_length=_MAX_ANALYSIS_STEPS)

    @model_validator(mode="after")
    def check_order(self) -> Self:
        rank = 0
        solved = False
        for step in self.steps:
            step_rank = _ANALYSIS_RANK.get(step.op)
            if step_rank is None:
                raise ValueError(f"unknown operation {step.op}")
            if step_rank < rank:
                raise ValueError("analysis steps are out of order")
            rank = step_rank
            solved = solved or step.op == "solve"
        if not solved:
            raise ValueError("an analysis script must include solve")
        return self


class EditStep(_ScriptModel):
    """A small change applied with tool calls."""

    op: str
    component: str
    x: float = 0
    y: float = 0
    z: float = 0
    length: float | None = None
    diameter: float | None = None
    radius: float | None = None

    @field_validator("op", mode="before")
    @classmethod
    def check_op(cls, value: object) -> str:
        return _known_operation(value, EDIT_OPS)

    @field_validator("component")
    @classmethod
    def check_component(cls, value: str) -> str:
        return _component_name(value)

    @field_validator("x", "y", "z", mode="before")
    @classmethod
    def check_coordinates(cls, value: object, info: ValidationInfo) -> float:
        return _coordinate(value, info)

    @field_validator("length", "diameter", "radius", mode="before")
    @classmethod
    def check_sizes(cls, value: object, info: ValidationInfo) -> float | None:
        return _optional_size(value, info)

    @model_validator(mode="after")
    def check_operation(self) -> Self:
        if self.op == "resize" and self.length is None and self.diameter is None:
            raise ValueError("resize requires a length or a diameter")
        if self.op == "fillet":
            _require(self.radius, "fillet requires a radius")
        return self


class AnalysisReading(_ScriptModel):
    """What the design model says about the numbers, and the change it proposes."""

    verdict: str
    tool_edit: tuple[EditStep, ...] = Field(default=(), max_length=_MAX_EDITS)
    rewrite: ComponentScript | None = None

    @field_validator("verdict", mode="before")
    @classmethod
    def check_verdict(cls, value: object) -> str:
        return _bounded_text(value, "verdict", _MAX_VERDICT_CHARS)


def load_construction(raw: object) -> ConstructionScript:
    """Validate a design object from the model."""
    return _load(ConstructionScript, raw)


def load_analysis(raw: object) -> AnalysisScript:
    """Validate an analysis script from the model."""
    return _load(AnalysisScript, raw)


def load_reading(raw: object) -> AnalysisReading:
    """Validate the model's reading of an analysis."""
    return _load(AnalysisReading, raw)


_ModelT = TypeVar("_ModelT", bound=BaseModel)


def _load(kind: type[_ModelT], raw: object) -> _ModelT:
    if not isinstance(raw, Mapping):
        raise ConstructionError("the model did not return an object")
    try:
        return kind.model_validate(raw)
    except ValidationError as error:
        raise ConstructionError(_validation_text(error)) from None


def _validation_text(error: ValidationError) -> str:
    messages = [_clean_message(detail["msg"]) for detail in error.errors()]
    for message in messages:
        if "unknown operation" in message:
            return message
    if messages and messages[0]:
        return messages[0]
    return "the construction script was refused"


def _clean_message(message: str) -> str:
    if message.startswith(_VALUE_ERROR_PREFIX):
        message = message[len(_VALUE_ERROR_PREFIX) :]
    return message.splitlines()[0].strip()


__all__ = (
    "ANALYSIS_OPS",
    "ASSEMBLY_OPS",
    "COMPONENT_OPS",
    "EDIT_OPS",
    "MAX_COMPONENTS",
    "MAX_DIMENSION_MM",
    "MAX_STEPS",
    "MAX_TOTAL_STEPS",
    "AnalysisReading",
    "AnalysisScript",
    "AnalysisStep",
    "AssemblyStep",
    "ComponentScript",
    "ComponentStep",
    "ConstructionError",
    "ConstructionScript",
    "EditStep",
    "RevisionAction",
    "load_analysis",
    "load_construction",
    "load_reading",
)
