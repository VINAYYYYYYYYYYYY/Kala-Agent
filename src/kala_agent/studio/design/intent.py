"""The design intent a chat turn or an API caller submits.

``DesignIntent`` is either a template call or a raw
:class:`~kala_agent.contracts.construction.ConstructionScript`, plus the
declared material and tolerance strings and an optional label. Exactly one of
``template`` and ``script`` must be present; ambiguity is refused instead of
guessed.

The module is deliberately kernel-free. It imports pydantic and the shared
construction contract only, so the chat surface, the HTTP schemas, and the
local parser can use it on a machine where the optional ``cadquery-ocp``
geometry package is not installed. The JSON Schema export is what the remote
provider prompt carries; :func:`parse_design_intent_text` validates a fenced
JSON reply with the same models the HTTP route uses.
"""

from __future__ import annotations

import json
import re
from typing import Annotated, Final, Literal

from pydantic import (
    AliasChoices,
    BaseModel,
    ConfigDict,
    Field,
    ValidationError,
    model_validator,
)

from kala_agent.contracts.construction import ConstructionScript
from kala_agent.studio.geometry.limits import MAX_DIMENSION_MM

MAX_TOLERANCES: Final[int] = 16
MAX_MATERIAL_LENGTH: Final[int] = 64
MAX_LABEL_LENGTH: Final[int] = 120
MAX_TOLERANCE_LENGTH: Final[int] = 64
MAX_SEGMENTS: Final[int] = 8
DEFAULT_BRACKET_WIDTH_MM: Final[float] = 60.0
DEFAULT_BRACKET_DEPTH_MM: Final[float] = 45.0
JSON_BLOCK_PATTERN: Final[re.Pattern[str]] = re.compile(
    r"```(?:json)?\s*(\{.*\})\s*```", re.IGNORECASE | re.DOTALL
)

DESIGN_INTENT_INSTRUCTION: Final[str] = (
    "You are Kala Studio's design planner. When the user asks you to design, "
    "size, or draft a part, answer with a short prose explanation and exactly "
    "one fenced ```json block containing a DesignIntent object. The block must "
    "validate against the JSON Schema below. All dimensions are millimetres. "
    "Use a template call when one of the templates matches the request; use a "
    "raw ConstructionScript only when the user supplied a complete script. "
    "Never invent a dimension the user did not state, and never state a volume, "
    "surface area, or envelope as a result: the geometry kernel measures those. "
    "If required values are missing, do not emit a JSON block; ask for the "
    "missing values in prose instead."
)

__all__ = (
    "DEFAULT_BRACKET_DEPTH_MM",
    "DEFAULT_BRACKET_WIDTH_MM",
    "DESIGN_INTENT_INSTRUCTION",
    "DesignIntent",
    "DesignIntentParseError",
    "HousingCall",
    "LbracketCall",
    "ShaftSegment",
    "SteppedShaftCall",
    "TemplateCall",
    "design_intent_json_schema",
    "design_intent_schema_text",
    "parse_design_intent_text",
    "strip_design_json_block",
)


class _IntentModel(BaseModel):
    model_config = ConfigDict(
        extra="forbid",
        frozen=True,
        allow_inf_nan=False,
        str_strip_whitespace=True,
    )


class LbracketCall(_IntentModel):
    """A call to :func:`kala_agent.studio.design.templates.l_bracket`."""

    template: Literal["l_bracket"] = "l_bracket"
    width: Annotated[
        float, Field(gt=0, le=MAX_DIMENSION_MM, validation_alias=AliasChoices("width", "W"))
    ]
    depth: Annotated[
        float, Field(gt=0, le=MAX_DIMENSION_MM, validation_alias=AliasChoices("depth", "D"))
    ]
    height: Annotated[
        float, Field(gt=0, le=MAX_DIMENSION_MM, validation_alias=AliasChoices("height", "H"))
    ]
    wall: Annotated[
        float, Field(gt=0, le=MAX_DIMENSION_MM, validation_alias=AliasChoices("wall", "T"))
    ]
    bore_d: Annotated[
        float | None,
        Field(gt=0, le=MAX_DIMENSION_MM, validation_alias=AliasChoices("bore_d", "B")),
    ] = None
    hole_d: Annotated[
        float | None,
        Field(gt=0, le=MAX_DIMENSION_MM, validation_alias=AliasChoices("hole_d", "h")),
    ] = None
    hole_count: Annotated[
        Literal[0, 2, 4], Field(validation_alias=AliasChoices("hole_count", "holes", "holeCount"))
    ] = 0
    edge: Annotated[
        float | None,
        Field(gt=0, le=MAX_DIMENSION_MM, validation_alias=AliasChoices("edge", "e")),
    ] = None


class ShaftSegment(_IntentModel):
    """One diameter/length pair of a stepped shaft."""

    diameter: Annotated[float, Field(gt=0, le=MAX_DIMENSION_MM)]
    length: Annotated[float, Field(gt=0, le=MAX_DIMENSION_MM)]


class SteppedShaftCall(_IntentModel):
    """A call to :func:`kala_agent.studio.design.templates.stepped_shaft`."""

    template: Literal["stepped_shaft"] = "stepped_shaft"
    segments: Annotated[tuple[ShaftSegment, ...], Field(min_length=1, max_length=MAX_SEGMENTS)]


class HousingCall(_IntentModel):
    """A call to :func:`kala_agent.studio.design.templates.housing_envelope`."""

    template: Literal["housing_envelope"] = "housing_envelope"
    width: Annotated[
        float, Field(gt=0, le=MAX_DIMENSION_MM, validation_alias=AliasChoices("width", "W"))
    ]
    depth: Annotated[
        float, Field(gt=0, le=MAX_DIMENSION_MM, validation_alias=AliasChoices("depth", "D"))
    ]
    height: Annotated[
        float, Field(gt=0, le=MAX_DIMENSION_MM, validation_alias=AliasChoices("height", "H"))
    ]
    wall: Annotated[
        float, Field(gt=0, le=MAX_DIMENSION_MM, validation_alias=AliasChoices("wall", "T"))
    ]


TemplateCall = Annotated[
    LbracketCall | SteppedShaftCall | HousingCall,
    Field(discriminator="template"),
]

Tolerance = Annotated[str, Field(min_length=1, max_length=MAX_TOLERANCE_LENGTH)]
Material = Annotated[str, Field(min_length=1, max_length=MAX_MATERIAL_LENGTH)]
Label = Annotated[str, Field(min_length=1, max_length=MAX_LABEL_LENGTH)]


class DesignIntent(_IntentModel):
    """A template call or a raw script, plus declared, non-measured metadata."""

    template: TemplateCall | None = None
    script: ConstructionScript | None = None
    material: Material | None = None
    tolerances: Annotated[tuple[Tolerance, ...], Field(max_length=MAX_TOLERANCES)] = ()
    label: Label | None = None

    @model_validator(mode="after")
    def check_exactly_one_source(self) -> DesignIntent:
        if (self.template is None) == (self.script is None):
            raise ValueError("a design intent must carry exactly one of template or script")
        return self


class DesignIntentParseError(ValueError):
    """The model reply did not contain a valid DesignIntent."""


def design_intent_json_schema() -> dict[str, object]:
    """Return the JSON Schema the remote provider prompt carries."""
    schema: dict[str, object] = DesignIntent.model_json_schema()
    return schema


def design_intent_schema_text() -> str:
    """Return the schema as readable, bounded JSON for a prompt."""
    return json.dumps(design_intent_json_schema(), indent=2, sort_keys=True)


def parse_design_intent_text(text: str) -> DesignIntent:
    """Parse and validate the fenced JSON block in a model reply.

    Raises:
        DesignIntentParseError: there is no JSON block, the block is not JSON,
            or the object does not validate against :class:`DesignIntent`.
    """
    if not isinstance(text, str) or not text.strip():
        raise DesignIntentParseError("the model reply was empty")
    match = JSON_BLOCK_PATTERN.search(text)
    candidate = match.group(1) if match is not None else text.strip()
    if not candidate.startswith("{"):
        raise DesignIntentParseError("the model reply did not contain a fenced json block")
    try:
        payload = json.loads(candidate)
    except json.JSONDecodeError as error:
        raise DesignIntentParseError(f"the design JSON is not valid JSON: {error.msg}") from None
    try:
        return DesignIntent.model_validate(payload)
    except ValidationError as error:
        raise DesignIntentParseError(_validation_text(error)) from None


def _validation_text(error: ValidationError) -> str:
    parts: list[str] = []
    for detail in error.errors()[:6]:
        location = ".".join(str(item) for item in detail.get("loc", ())) or "intent"
        message = str(detail.get("msg", "invalid value")).splitlines()[0].strip()
        if message.startswith("Value error, "):
            message = message[len("Value error, ") :]
        parts.append(f"{location}: {message[:160]}")
    if not parts:
        return "the design intent was refused"
    return "; ".join(parts)


def strip_design_json_block(text: str) -> str:
    """Return the prose of a reply with its fenced design block removed."""
    stripped = JSON_BLOCK_PATTERN.sub("", text).strip()
    return stripped or "Design intent received."
