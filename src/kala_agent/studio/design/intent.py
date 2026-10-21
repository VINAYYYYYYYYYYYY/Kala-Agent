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
or bare JSON reply with the same models the HTTP route uses, and
:func:`design_schema_excerpt` gives the repair round-trip the failing field's
constraint instead of the whole schema.
"""

from __future__ import annotations

import json
from typing import Annotated, Final, Literal

from pydantic import (
    AfterValidator,
    AliasChoices,
    BaseModel,
    ConfigDict,
    Field,
    ValidationError,
    model_validator,
)

from kala_agent.contracts.construction import ComponentStep, ConstructionScript
from kala_agent.studio.geometry.limits import MAX_DIMENSION_MM

MAX_TOLERANCES: Final[int] = 16
MAX_MATERIAL_LENGTH: Final[int] = 64
MAX_LABEL_LENGTH: Final[int] = 120
MAX_TOLERANCE_LENGTH: Final[int] = 64
MAX_SEGMENTS: Final[int] = 8
DEFAULT_BRACKET_WIDTH_MM: Final[float] = 60.0
DEFAULT_BRACKET_DEPTH_MM: Final[float] = 45.0

DESIGN_INTENT_INSTRUCTION: Final[str] = (
    "You are Kala Studio's design planner. When the user asks you to design, "
    "size, or draft a part, answer with a short prose explanation and exactly "
    "one fenced ```json block containing a DesignIntent object that validates "
    "against the JSON Schema below. All dimensions are millimetres.\n"
    "\n"
    "Use a template call when one of the templates (l_bracket, stepped_shaft, "
    "housing_envelope) matches the request, and only when every value it needs "
    "is stated. When no template matches, author a ConstructionScript from the "
    "dimensions the user stated, using only this supported subset: sketch a "
    "rectangle (width, height) or "
    "circle (diameter) on XY, XZ or YZ at an optional x/y offset; extrude it by "
    "length (reversed stays false); pocket on the same plane, omitting length "
    "for through-all or giving length for a finite depth; revolve a rectangle "
    "on XY or YZ by angle about the local Y axis (default 360); place each "
    "component once with x/y/z; fuse two or more placed components. A component "
    "is one solid: each sketch must be consumed by an extrude, revolve or "
    "pocket before the next operation, and the component must end with a solid.\n"
    "\n"
    "Sketch coordinates are in the sketch plane: a rectangle's x/y is its "
    "lower-left corner, so it spans x to x+width and y to y+height; a circle's "
    "x/y is its centre. Every pocket must cut into material that its own "
    "component already has at that step; a cut where the component has no "
    "material removes nothing, and the build is rejected. A design may use up "
    "to 500 steps in total across up to 64 components, and one component may "
    "use all of them. Each solid body is one component: do not split a body "
    "into components just to save steps.\n"
    "\n"
    "Never invent a dimension the user did not state: no guessed thickness, "
    "diameter, spacing, radius or angle. You may compute coordinates from "
    "stated positions, such as a hole centre given as an edge distance. Never "
    "state a volume, surface area, or envelope as a result; the geometry kernel "
    "measures those. Include material, tolerances or a label only when the user "
    "stated them.\n"
    "\n"
    "If a value needed to build the part is missing, ask for it in prose and do "
    "not emit a JSON block.\n"
    "\n"
    "Worked examples (each is a complete DesignIntent; do not copy one unless "
    "it matches the request):\n"
    "\n"
    "```json\n"
    '{"script":{"summary":"80 x 40 x 8 mm plate with four 6 mm through holes",'
    '"components":[{"name":"plate","steps":[{"op":"sketch","plane":"XY",'
    '"profile":"rectangle","width":80,"height":40},{"op":"extrude","length":8},'
    '{"op":"sketch","plane":"XY","profile":"circle","diameter":6,"x":8,"y":8},'
    '{"op":"pocket"},{"op":"sketch","plane":"XY","profile":"circle",'
    '"diameter":6,"x":72,"y":8},{"op":"pocket"},{"op":"sketch","plane":"XY",'
    '"profile":"circle","diameter":6,"x":72,"y":32},{"op":"pocket"},'
    '{"op":"sketch","plane":"XY","profile":"circle","diameter":6,"x":8,'
    '"y":32},{"op":"pocket"}]}]}}\n'
    "```\n"
    "\n"
    "```json\n"
    '{"script":{"summary":"stepped 30/20 mm round post, 40 and 25 mm long",'
    '"components":[{"name":"post","steps":[{"op":"sketch","plane":"XY",'
    '"profile":"rectangle","width":15,"height":40},{"op":"revolve","angle":360},'
    '{"op":"sketch","plane":"XY","profile":"rectangle","width":10,"height":25,'
    '"y":40},{"op":"revolve","angle":360}]}]}}\n'
    "```\n"
    "\n"
    "```json\n"
    '{"script":{"summary":"40 x 30 x 5 mm base with a 12 mm boss placed on top",'
    '"components":[{"name":"base","steps":[{"op":"sketch","plane":"XY",'
    '"profile":"rectangle","width":40,"height":30},{"op":"extrude","length":5}]},'
    '{"name":"boss","steps":[{"op":"sketch","plane":"XY","profile":"circle",'
    '"diameter":12},{"op":"extrude","length":10}]}],"assembly":[{"op":"place",'
    '"component":"base"},{"op":"place","component":"boss","x":20,"y":15,"z":5}]}}\n'
    "```"
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
    "design_schema_excerpt",
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


def _check_pocket_planes(script: ConstructionScript) -> ConstructionScript:
    """Refuse a pocket whose sketch plane cannot cut the current solid.

    This is exactly the OCP compiler's rule: the cut follows the pending
    sketch's plane, and that plane must equal the plane of the sketch that
    produced the current solid. A pocket's own ``plane`` field is not part of
    the check because the compiler never reads it, and the shipped L-bracket
    template's canonical wire form states ``plane: XY`` on its XZ upright
    pocket; requiring that field to match would reject the template itself.
    The message names the component, the step index, and both planes; the
    Rust `kala-design-intent` validator emits the same bytes.
    """
    for component in script.components:
        pending: ComponentStep | None = None
        solid_plane: str | None = None
        for index, step in enumerate(component.steps):
            if step.op == "sketch":
                pending = step
                continue
            if step.op in {"extrude", "revolve"}:
                if pending is not None:
                    solid_plane = pending.plane
                pending = None
                continue
            if step.op != "pocket":
                continue
            if pending is None or solid_plane is None:
                continue
            if pending.plane != solid_plane:
                raise ValueError(
                    f"component {component.name!r}: pocket at step index {index} "
                    f"on {pending.plane} over a {solid_plane} solid"
                )
            pending = None
    return script


Script = Annotated[ConstructionScript, AfterValidator(_check_pocket_planes)]


class DesignIntent(_IntentModel):
    """A template call or a raw script, plus declared, non-measured metadata."""

    template: TemplateCall | None = None
    script: Script | None = None
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
    """Parse and validate the JSON design block in a model reply.

    A fenced block is preferred; a bare JSON object is accepted when the reply
    has no fence, including when the object is wrapped in prose. More than one
    conflicting design object is refused instead of guessed.

    Raises:
        DesignIntentParseError: there is no JSON block, the block is not JSON,
            the reply carries conflicting blocks, or the object does not
            validate against :class:`DesignIntent`.
    """
    if not isinstance(text, str) or not text.strip():
        raise DesignIntentParseError("the model reply was empty")
    payload = _extract_design_payload(text)
    try:
        return DesignIntent.model_validate(payload)
    except ValidationError as error:
        raise DesignIntentParseError(_validation_text(error)) from None


def _extract_design_payload(text: str) -> object:
    """Return the one design object in a reply, or raise a parse error."""
    fences = _fence_spans(text)
    if fences:
        spans: list[tuple[int, int]] = []
        for _opening, _closing, content_start, content_end in fences:
            content = text[content_start:content_end]
            spans.extend(
                (content_start + start, content_start + end)
                for start, end in _object_spans(content)
            )
        values, first_error = _parse_object_spans(text, spans)
        if values:
            return _single_payload(values)
        if any(
            "{" in text[content_start:content_end]
            for _opening, _closing, content_start, content_end in fences
        ):
            raise DesignIntentParseError(
                f"the design JSON is not valid JSON: {first_error or 'Expecting value'}"
            )
        raise DesignIntentParseError("the model reply did not contain a fenced json block")

    outside = _outside_fences(text)
    spans = _object_spans(outside)
    values, first_error = _parse_object_spans(outside, spans)
    if values:
        return _single_payload(values)
    if "{" in outside:
        raise DesignIntentParseError(
            f"the design JSON is not valid JSON: {first_error or 'Expecting value'}"
        )
    raise DesignIntentParseError("the model reply did not contain a fenced json block")


def _single_payload(values: list[object]) -> object:
    first = values[0]
    if any(value != first for value in values[1:]):
        raise DesignIntentParseError(
            "the model reply contained more than one conflicting JSON block"
        )
    return first


def _fence_spans(text: str) -> list[tuple[int, int, int, int]]:
    """Return ``(outer_start, outer_end, content_start, content_end)`` per fence.

    The language tag is skipped, so an unlabelled fence and a ```json fence
    behave the same. An unclosed fence runs to the end of the reply.
    """
    spans: list[tuple[int, int, int, int]] = []
    length = len(text)
    index = 0
    while True:
        opening = text.find("```", index)
        if opening < 0:
            break
        cursor = opening + 3
        while cursor < length and text[cursor] in " \t":
            cursor += 1
        while cursor < length and (text[cursor].isalnum() or text[cursor] in "_-"):
            cursor += 1
        while cursor < length and text[cursor] in " \t":
            cursor += 1
        if text.startswith("\r\n", cursor):
            cursor += 2
        elif cursor < length and text[cursor] == "\n":
            cursor += 1
        closing = text.find("```", cursor)
        if closing < 0:
            spans.append((opening, length, cursor, length))
            break
        spans.append((opening, closing + 3, cursor, closing))
        index = closing + 3
    return spans


def _outside_fences(text: str) -> str:
    """Return the reply with every fenced block blanked out."""
    parts: list[str] = []
    last = 0
    for opening, closing, _content_start, _content_end in _fence_spans(text):
        parts.append(text[last:opening])
        last = closing
    parts.append(text[last:])
    return "".join(parts)


def _object_spans(text: str) -> list[tuple[int, int]]:
    """Return the spans of balanced ``{...}`` objects, strings respected."""
    spans: list[tuple[int, int]] = []
    start = 0
    depth = 0
    in_string = False
    escaped = False
    for index, character in enumerate(text):
        if in_string:
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == '"':
                in_string = False
            continue
        if character == '"':
            if depth > 0:
                in_string = True
        elif character == "{":
            if depth == 0:
                start = index
            depth += 1
        elif character == "}":
            if depth > 0:
                depth -= 1
                if depth == 0:
                    spans.append((start, index + 1))
    return spans


def _parse_object_spans(text: str, spans: list[tuple[int, int]]) -> tuple[list[object], str | None]:
    values: list[object] = []
    first_error: str | None = None
    for start, end in spans:
        try:
            values.append(json.loads(text[start:end]))
        except json.JSONDecodeError as error:
            if first_error is None:
                first_error = error.msg
    return values, first_error


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


def design_schema_excerpt(error: str, limit: int = 700) -> str | None:
    """Return a bounded schema excerpt for the field an error names.

    The repair prompt carries this so the model sees the exact constraint of
    the field it got wrong instead of the whole 11 KB schema again.
    """
    location = _error_location(error)
    tokens = location.split(".") if location else []
    node, progressed = _walk_schema(design_intent_json_schema(), tokens)
    if not progressed:
        schema = design_intent_json_schema()
        node = {
            "type": "object",
            "required": list(schema.get("required", [])),
            "properties": {name: {} for name in schema.get("properties", {})},
        }
    text = json.dumps(node, indent=2, sort_keys=True, ensure_ascii=False)
    return text[:limit]


def _error_location(error: str) -> str:
    head = error.split(": ", 1)[0].strip()
    if not head:
        return ""
    for token in head.split("."):
        if not token or not all(character.isalnum() or character == "_" for character in token):
            return ""
    return head


def _walk_schema(schema: dict[str, object], tokens: list[str]) -> tuple[object, bool]:
    node: object = schema
    progressed = False
    index = 0
    while index < len(tokens):
        node = _resolve(schema, node)
        token = tokens[index]
        if not token:
            index += 1
            continue
        properties = node.get("properties") if isinstance(node, dict) else None
        if isinstance(properties, dict) and token in properties:
            node = properties[token]
            progressed = True
            index += 1
            continue
        items = node.get("items") if isinstance(node, dict) else None
        if token.isdigit() and items is not None:
            node = items
            progressed = True
            index += 1
            continue
        mapping = _discriminator_mapping(node)
        if token in mapping:
            node = {"$ref": mapping[token]}
            progressed = True
            index += 1
            continue
        branch = _select_branch(schema, node, token)
        if branch is not None:
            node = branch
            continue
        index += 1
    return _resolve(schema, node), progressed


def _select_branch(schema: dict[str, object], node: object, token: str) -> object | None:
    if not isinstance(node, dict):
        return None
    for key in ("anyOf", "oneOf"):
        branches = node.get(key)
        if not isinstance(branches, list):
            continue
        for branch in branches:
            resolved = _resolve(schema, branch)
            if not isinstance(resolved, dict):
                continue
            properties = resolved.get("properties")
            if isinstance(properties, dict) and token in properties:
                return branch
            if token.isdigit() and "items" in resolved:
                return branch
            if token in _discriminator_mapping(resolved):
                return branch
    return None


def _resolve(schema: dict[str, object], node: object) -> object:
    seen = 0
    while isinstance(node, dict) and isinstance(node.get("$ref"), str):
        pointer = node["$ref"]
        if not pointer.startswith("#/"):
            return node
        target: object = schema
        for part in pointer[2:].split("/"):
            part = part.replace("~1", "/").replace("~0", "~")
            if not isinstance(target, dict) or part not in target:
                return node
            target = target[part]
        node = target
        seen += 1
        if seen > 16:
            break
    return node


def _discriminator_mapping(node: object) -> dict[str, str]:
    if not isinstance(node, dict):
        return {}
    discriminator = node.get("discriminator")
    if isinstance(discriminator, dict):
        mapping = discriminator.get("mapping")
        if isinstance(mapping, dict):
            return {str(key): str(value) for key, value in mapping.items()}
    for key in ("anyOf", "oneOf"):
        branches = node.get(key)
        if isinstance(branches, list):
            for branch in branches:
                mapping = _discriminator_mapping(branch)
                if mapping:
                    return mapping
    return {}


def strip_design_json_block(text: str) -> str:
    """Return the prose of a reply with its design JSON removed."""
    without_fences = _outside_fences(text)
    spans = _object_spans(without_fences)
    for start, end in reversed(spans):
        try:
            json.loads(without_fences[start:end])
        except json.JSONDecodeError:
            continue
        without_fences = without_fences[:start] + without_fences[end:]
    stripped = without_fences.strip()
    return stripped or "Design intent received."
