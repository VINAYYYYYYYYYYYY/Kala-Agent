from __future__ import annotations

import hashlib
import json
import math
import os
import re
import sys
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import dataclass
from datetime import datetime
from enum import StrEnum
from io import TextIOBase
from pathlib import Path, PurePosixPath
from types import MappingProxyType
from typing import IO, Final, NoReturn, Protocol, TextIO, TypeAlias, cast, runtime_checkable

from kala_agent.contracts import (
    ArtifactRef,
    ToolCall,
    ToolDescriptor,
    ToolResult,
    ToolRunStatus,
)

PROTOCOL_VERSION: Final[str] = "1.0"
MAX_JSON_BYTES: Final[int] = 1_048_576
MAX_ARGUMENT_BYTES: Final[int] = 262_144
MAX_STRING_LENGTH: Final[int] = 32_768
MAX_COLLECTION_ITEMS: Final[int] = 2_048
MAX_KEY_LENGTH: Final[int] = 256
MAX_JSON_DEPTH: Final[int] = 48
MAX_SCHEMA_DEPTH: Final[int] = 32
MAX_PATH_SEGMENTS: Final[int] = 32
MAX_PATH_LENGTH: Final[int] = 1_024
MAX_PATTERN_LENGTH: Final[int] = 256
MAX_PATTERN_INPUT_LENGTH: Final[int] = 1_024
MAX_PATTERN_REPEAT: Final[int] = 1_000
MAX_PATTERN_UNBOUNDED_REPEATS: Final[int] = 1
MAX_PATTERN_BACKTRACK_STATES: Final[int] = 2_048
MAX_SANITIZED_DETAIL_LENGTH: Final[int] = 512

JSONValue: TypeAlias = "bool | int | float | str | list[JSONValue] | dict[str, JSONValue] | None"
JsonObject: TypeAlias = "dict[str, JSONValue]"

_ENVIRONMENT_KEY_PATTERN: Final[re.Pattern[str]] = re.compile(r"^[A-Za-z_][A-Za-z0-9_]{0,63}$")

_IDENTIFIER_PATTERN: Final[re.Pattern[str]] = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$")
_VERSION_PATTERN: Final[re.Pattern[str]] = re.compile(r"^[A-Za-z0-9][A-Za-z0-9.+_-]{0,63}$")
_ERROR_CODE_PATTERN: Final[re.Pattern[str]] = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$")
_SEGMENT_PATTERN: Final[re.Pattern[str]] = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
_BASE64_RUN_PATTERN: Final[re.Pattern[str]] = re.compile(r"[A-Za-z0-9+/]{96,}={0,2}")
_PATTERN_REPEAT_SPEC: Final[re.Pattern[str]] = re.compile(r"\{(\d{1,9})(?:(,)(\d{1,9})?)?\}")
_UNSAFE_PATTERN_CONSTRUCTS: Final[tuple[tuple[str, re.Pattern[str]], ...]] = (
    ("backreferences", re.compile(r"\\(?:[1-9]|k<|g<)|\(\?P=")),
    ("lookaround assertions", re.compile(r"\(\?(?:=|!|<=|<!)")),
    ("conditional groups", re.compile(r"\(\?\(")),
    ("inline flags", re.compile(r"\(\?[a-zA-Z]+(?:-[a-zA-Z]+)?[):]")),
    ("atomic and possessive groups", re.compile(r"\(\?>|\\[+*]")),
    ("wildcard repetition", re.compile(r"\.\s*(?:[*+]|\{\d+,\})")),
)

_ANNOTATION_KEYWORDS: Final[frozenset[str]] = frozenset(
    {
        "$comment",
        "$id",
        "$schema",
        "default",
        "deprecated",
        "description",
        "examples",
        "format",
        "readOnly",
        "title",
        "unit",
        "writeOnly",
    }
)
_SUPPORTED_KEYWORDS: Final[frozenset[str]] = frozenset(
    {
        "additionalProperties",
        "allOf",
        "anyOf",
        "const",
        "enum",
        "exclusiveMaximum",
        "exclusiveMinimum",
        "items",
        "maxItems",
        "maxLength",
        "maxProperties",
        "maximum",
        "minItems",
        "minLength",
        "minProperties",
        "minimum",
        "multipleOf",
        "oneOf",
        "pattern",
        "properties",
        "required",
        "type",
        "uniqueItems",
    }
)


class ToolProtocolError(RuntimeError):
    pass


class JsonEnvelopeError(ToolProtocolError, ValueError):
    pass


class ToolEnvelopeError(ToolProtocolError, ValueError):
    pass


class ToolWorkspaceError(ToolProtocolError, ValueError):
    pass


class SchemaValidationError(ToolProtocolError, ValueError):
    def __init__(self, message: str, *, path: str = "$") -> None:
        self.reason = message
        self.path = path
        super().__init__(f"{path}: {message}")


class ToolErrorCode(StrEnum):
    UNKNOWN_TOOL = "unknown_tool"
    VERSION_NOT_FOUND = "version_not_found"
    DESCRIPTOR_MISMATCH = "descriptor_mismatch"
    SCHEMA_DIGEST_MISMATCH = "schema_digest_mismatch"
    INVALID_ARGUMENTS = "invalid_arguments"
    INVALID_RESULT = "invalid_result"
    CAPABILITY_DENIED = "capability_denied"
    CAPABILITY_FORBIDDEN = "capability_forbidden"
    NETWORK_DENIED = "network_denied"
    EGRESS_DENIED = "egress_denied"
    APPROVAL_REQUIRED = "approval_required"
    SIDE_EFFECT_MISMATCH = "side_effect_mismatch"
    ISOLATION_REQUIRED = "isolation_required"
    ISOLATION_UNAVAILABLE = "isolation_unavailable"
    TIMEOUT = "timeout"
    CANCELLED = "cancelled"
    CRASHED = "tool_crashed"
    PROTOCOL_ERROR = "protocol_error"
    OUTPUT_TOO_LARGE = "output_too_large"
    WORKSPACE_VIOLATION = "workspace_violation"
    ARTIFACT_REJECTED = "artifact_rejected"
    BUDGET_EXHAUSTED = "budget_exhausted"
    WORKER_CAPACITY_EXHAUSTED = "worker_capacity_exhausted"
    SANDBOX_UNAVAILABLE = "sandbox_unavailable"
    INTERNAL_ERROR = "tool_error"


class ToolEnvelopeKind(StrEnum):
    REQUEST = "request"
    RESPONSE = "response"


_FAILURE_STATUSES: Final[frozenset[ToolRunStatus]] = frozenset(
    {ToolRunStatus.FAILED, ToolRunStatus.DENIED, ToolRunStatus.TIMED_OUT}
)
_TYPE_CHECKS: Final[Mapping[str, Callable[[object], bool]]] = MappingProxyType(
    {
        "array": lambda value: isinstance(value, (list, tuple)),
        "boolean": lambda value: isinstance(value, bool),
        "integer": lambda value: isinstance(value, int) and not isinstance(value, bool),
        "null": lambda value: value is None,
        "number": lambda value: isinstance(value, (int, float)) and not isinstance(value, bool),
        "object": lambda value: isinstance(value, Mapping),
        "string": lambda value: isinstance(value, str),
    }
)


def _json_equal(left: object, right: object) -> bool:
    if isinstance(left, bool) != isinstance(right, bool):
        return False
    if isinstance(left, Mapping) and isinstance(right, Mapping):
        if set(left) != set(right):
            return False
        return all(_json_equal(left[key], right[key]) for key in left)
    if isinstance(left, (list, tuple)) and isinstance(right, (list, tuple)):
        if len(left) != len(right):
            return False
        return all(_json_equal(first, second) for first, second in zip(left, right, strict=True))
    return bool(left == right)


def _reject_json_constant(value: str) -> NoReturn:
    raise JsonEnvelopeError(f"JSON constant {value} is not permitted")


def _parse_json_float(value: str) -> float:
    parsed = float(value)
    if not math.isfinite(parsed):
        raise JsonEnvelopeError("JSON numbers must be finite")
    return parsed


def _object_without_duplicate_keys(pairs: list[tuple[str, object]]) -> JsonObject:
    result: JsonObject = {}
    for key, value in pairs:
        if key in result:
            raise JsonEnvelopeError("duplicate JSON object keys are not permitted")
        result[key] = cast(JSONValue, value)
    return result


def _normalize_json(value: object, *, depth: int) -> JSONValue:
    if depth > MAX_JSON_DEPTH:
        raise JsonEnvelopeError("JSON nesting exceeds the supported depth")
    if value is None or isinstance(value, (bool, int)):
        return cast(JSONValue, value)
    if isinstance(value, str):
        if len(value) > MAX_STRING_LENGTH:
            raise JsonEnvelopeError("JSON string exceeds the bounded length")
        return value
    if isinstance(value, float):
        if not math.isfinite(value):
            raise JsonEnvelopeError("JSON numbers must be finite")
        return value
    if isinstance(value, Mapping):
        normalized: dict[str, JSONValue] = {}
        for key, item in value.items():
            if not isinstance(key, str):
                raise JsonEnvelopeError("JSON object keys must be strings")
            if len(key) > MAX_KEY_LENGTH:
                raise JsonEnvelopeError("JSON object key exceeds the bounded length")
            normalized[key] = _normalize_json(item, depth=depth + 1)
        return normalized
    if isinstance(value, (list, tuple)):
        if len(value) > MAX_COLLECTION_ITEMS:
            raise JsonEnvelopeError("JSON collection exceeds the bounded item count")
        return [_normalize_json(item, depth=depth + 1) for item in value]
    raise JsonEnvelopeError("value is not JSON serializable")


def canonical_json_dumps(value: object, *, max_bytes: int = MAX_JSON_BYTES) -> str:
    normalized = _normalize_json(value, depth=0)
    try:
        text = json.dumps(
            normalized,
            allow_nan=False,
            ensure_ascii=True,
            separators=(",", ":"),
            sort_keys=True,
        )
    except (RecursionError, TypeError, ValueError) as error:
        raise JsonEnvelopeError("value cannot be canonicalized as JSON") from error
    if len(text.encode("utf-8")) > max_bytes:
        raise JsonEnvelopeError("canonical JSON exceeds the bounded size")
    return text


def strict_json_loads(payload: str | bytes, *, max_bytes: int = MAX_JSON_BYTES) -> JSONValue:
    if isinstance(payload, (bytes, bytearray)):
        raw = bytes(payload)
    elif isinstance(payload, str):
        raw = payload.encode("utf-8")
    else:
        raise TypeError("payload must be str or bytes")
    if len(raw) > max_bytes:
        raise JsonEnvelopeError("payload exceeds the bounded size")
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise JsonEnvelopeError("payload is not valid UTF-8") from error
    try:
        decoded = json.loads(
            text,
            parse_constant=_reject_json_constant,
            parse_float=_parse_json_float,
            object_pairs_hook=_object_without_duplicate_keys,
        )
    except (JsonEnvelopeError, RecursionError, ValueError) as error:
        if isinstance(error, JsonEnvelopeError):
            raise
        raise JsonEnvelopeError("payload is not strict JSON") from error
    return _normalize_json(decoded, depth=0)


def sha256_hex(data: str | bytes) -> str:
    if isinstance(data, str):
        payload = data.encode("utf-8")
    elif isinstance(data, (bytes, bytearray)):
        payload = bytes(data)
    else:
        raise TypeError("digest input must be str or bytes")
    return hashlib.sha256(payload).hexdigest()


def canonical_json_digest(value: object) -> str:
    return sha256_hex(canonical_json_dumps(value))


def _schema_keywords(schema: Mapping[str, object], *, path: str) -> None:
    for keyword in schema:
        if not isinstance(keyword, str):
            raise SchemaValidationError("schema keywords must be strings", path=path)
        if keyword in _SUPPORTED_KEYWORDS or keyword in _ANNOTATION_KEYWORDS:
            continue
        raise SchemaValidationError(f"unsupported schema keyword: {keyword}", path=path)


def _schema_bound(
    schema: Mapping[str, object],
    keyword: str,
    *,
    path: str,
    integral: bool,
) -> float | int | None:
    if keyword not in schema:
        return None
    value = schema[keyword]
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise SchemaValidationError(f"{keyword} must be a number", path=path)
    if not math.isfinite(float(value)):
        raise SchemaValidationError(f"{keyword} must be finite", path=path)
    if integral:
        if isinstance(value, float) and not value.is_integer():
            raise SchemaValidationError(f"{keyword} must be an integer", path=path)
        return int(value)
    return float(value)


def _schema_int(schema: Mapping[str, object], keyword: str, *, path: str) -> int | None:
    bound = _schema_bound(schema, keyword, path=path, integral=True)
    return None if bound is None else int(bound)


def _schema_float(schema: Mapping[str, object], keyword: str, *, path: str) -> float | None:
    bound = _schema_bound(schema, keyword, path=path, integral=False)
    return None if bound is None else float(bound)


def _schema_branch(value: object, branch: object, *, path: str, depth: int) -> bool:
    if not isinstance(branch, Mapping):
        raise SchemaValidationError("schema branches must be objects", path=path)
    try:
        validate_against_schema(value, branch, path=path, depth=depth + 1)
    except SchemaValidationError:
        return False
    return True


def _validate_object(
    value: Mapping[object, object],
    schema: Mapping[str, object],
    *,
    path: str,
) -> None:
    properties = schema.get("properties", {})
    if not isinstance(properties, Mapping):
        raise SchemaValidationError("properties must be an object", path=path)
    normalized_properties: dict[str, Mapping[str, object]] = {}
    for key, subschema in properties.items():
        if not isinstance(key, str) or not isinstance(subschema, Mapping):
            raise SchemaValidationError("properties must map names to schemas", path=path)
        normalized_properties[key] = subschema
    required = schema.get("required", [])
    if not isinstance(required, (list, tuple)):
        raise SchemaValidationError("required must be an array", path=path)
    normalized_required: list[str] = []
    for name in required:
        if not isinstance(name, str):
            raise SchemaValidationError("required entries must be strings", path=path)
        if name in normalized_required:
            raise SchemaValidationError("required entries must be unique", path=path)
        normalized_required.append(name)
        if normalized_properties and name not in normalized_properties:
            raise SchemaValidationError(f"required property {name!r} is not declared", path=path)
    additional = schema.get("additionalProperties", True)
    if not isinstance(additional, (bool, Mapping)):
        raise SchemaValidationError("additionalProperties must be a boolean or schema", path=path)
    for name in normalized_required:
        if name not in value:
            raise SchemaValidationError(f"required property {name!r} is missing", path=path)
    minimum_properties = _schema_int(schema, "minProperties", path=path)
    maximum_properties = _schema_int(schema, "maxProperties", path=path)
    if minimum_properties is not None and len(value) < minimum_properties:
        raise SchemaValidationError("object has too few properties", path=path)
    if maximum_properties is not None and len(value) > maximum_properties:
        raise SchemaValidationError("object has too many properties", path=path)
    extras: list[str] = []
    for key, item in value.items():
        if not isinstance(key, str):
            raise SchemaValidationError("object keys must be strings", path=path)
        subschema = normalized_properties.get(key)
        if subschema is not None:
            validate_against_schema(item, subschema, path=f"{path}.{key}")
            continue
        if additional is False:
            extras.append(key)
            continue
        if isinstance(additional, Mapping):
            validate_against_schema(item, additional, path=f"{path}.{key}")
    if extras:
        raise SchemaValidationError(
            f"unpermitted properties: {', '.join(sorted(extras))}", path=path
        )


def _validate_array(value: Sequence[object], schema: Mapping[str, object], *, path: str) -> None:
    if len(value) > MAX_COLLECTION_ITEMS:
        raise SchemaValidationError("array exceeds the bounded item count", path=path)
    minimum_items = _schema_int(schema, "minItems", path=path)
    maximum_items = _schema_int(schema, "maxItems", path=path)
    if minimum_items is not None and len(value) < minimum_items:
        raise SchemaValidationError("array has too few items", path=path)
    if maximum_items is not None and len(value) > maximum_items:
        raise SchemaValidationError("array has too many items", path=path)
    items = schema.get("items")
    if items is not None:
        if not isinstance(items, Mapping):
            raise SchemaValidationError("items must be a schema", path=path)
        for index, item in enumerate(value):
            validate_against_schema(item, items, path=f"{path}[{index}]")
    if schema.get("uniqueItems") is True:
        seen: list[str] = []
        for item in value:
            fingerprint = canonical_json_dumps(item)
            if fingerprint in seen:
                raise SchemaValidationError("array items must be unique", path=path)
            seen.append(fingerprint)


def _validate_string(value: str, schema: Mapping[str, object], *, path: str) -> None:
    minimum_length = _schema_int(schema, "minLength", path=path)
    maximum_length = _schema_int(schema, "maxLength", path=path)
    if minimum_length is not None and len(value) < minimum_length:
        raise SchemaValidationError("string is shorter than the permitted length", path=path)
    if maximum_length is not None and len(value) > maximum_length:
        raise SchemaValidationError("string is longer than the permitted length", path=path)
    pattern = schema.get("pattern")
    if pattern is None:
        return
    if not isinstance(pattern, str):
        raise SchemaValidationError("pattern must be a string", path=path)
    compiled = _bounded_pattern(pattern, maximum_length=maximum_length, path=path)
    if compiled.search(value) is None:
        raise SchemaValidationError("string does not match the declared pattern", path=path)


def _pattern_rejects(schema: Mapping[str, object], *, path: str) -> None:
    """Reject a pattern that is not provably bounded at schema registration."""
    pattern = schema.get("pattern")
    if pattern is None:
        return
    if not isinstance(pattern, str):
        raise SchemaValidationError("pattern must be a string", path=path)
    _bounded_pattern(pattern, maximum_length=_schema_int(schema, "maxLength", path=path), path=path)


def _bounded_pattern(
    pattern: str,
    *,
    maximum_length: int | None,
    path: str,
) -> re.Pattern[str]:
    """Compile a pattern that cannot be made to run long on a bounded input.

    Admission is decided from the pattern's own text, before anything is
    compiled and before the pattern is ever run against a value, and the same
    decision is reached on every call: the schema must cap the string, the
    pattern must be short, and the pattern must fit a bounded backtracking
    budget. A refused pattern is never compiled and never executed, so a tool
    author cannot buy an unbounded regex by registering a hostile schema.
    """
    if maximum_length is None:
        raise SchemaValidationError("pattern requires a maxLength bound", path=path)
    if not 0 < maximum_length <= MAX_PATTERN_INPUT_LENGTH:
        raise SchemaValidationError(
            f"pattern requires a maxLength between 1 and {MAX_PATTERN_INPUT_LENGTH}", path=path
        )
    if len(pattern) > MAX_PATTERN_LENGTH:
        raise SchemaValidationError("pattern exceeds the bounded length", path=path)
    for name, expression in _UNSAFE_PATTERN_CONSTRUCTS:
        if expression.search(pattern) is not None:
            raise SchemaValidationError(f"pattern uses an unsafe construct: {name}", path=path)
    _reject_nested_repetition(pattern, path=path)
    _reject_ambiguous_pattern(pattern, path=path)
    try:
        return re.compile(pattern)
    except re.error as error:
        raise SchemaValidationError(
            "pattern is not a valid regular expression", path=path
        ) from error


def _reject_nested_repetition(pattern: str, *, path: str) -> None:
    """Refuse a quantified group whose body already contains a quantifier.

    ``(a+)+`` and friends make the matching cost grow exponentially with the
    length of the subject string, which is a denial of service in the validator
    itself. A single level of repetition, such as ``(ab)+`` or ``(a+)$``, stays
    linear and is accepted.
    """
    open_groups: list[bool] = []
    index = 0
    length = len(pattern)
    while index < length:
        character = pattern[index]
        if character == "\\":
            index += 2
            continue
        if character == "[":
            index += 1
            while index < length and pattern[index] != "]":
                index += 2 if pattern[index] == "\\" else 1
            index += 1
            continue
        if character == "(":
            open_groups.append(False)
            index += 3 if pattern[index : index + 3] == "(?:" else 1
            continue
        if character == ")":
            if not open_groups:
                raise SchemaValidationError("pattern has an unbalanced group", path=path)
            body_is_quantified = open_groups.pop()
            index += 1
            quantified = _quantifier_at(pattern, index)
            if quantified is None:
                continue
            if body_is_quantified:
                raise SchemaValidationError(
                    "pattern repeats a group that already contains a quantifier", path=path
                )
            if open_groups:
                open_groups[-1] = True
            index = quantified
            continue
        if open_groups and _quantifier_at(pattern, index) is not None:
            open_groups[-1] = True
        index += 1
    if open_groups:
        raise SchemaValidationError("pattern has an unbalanced group", path=path)


def _quantifier_at(pattern: str, index: int) -> int | None:
    """Return the index just past a quantifier at ``index``, or ``None``."""
    character = pattern[index : index + 1]
    if character in {"*", "+"} or (
        character == "?" and pattern[index - 1 : index] not in {"?", ":"}
    ):
        return index + 1
    if character == "{":
        match = _PATTERN_REPEAT_SPEC.match(pattern, index)
        if match is not None and match.group(1) is not None:
            return match.end()
    return None


class _PatternBudget:
    """The total match cost a pattern is allowed to describe.

    ``consumed`` counts the characters a pattern must match whatever the input
    is, so a pattern that can only ever describe a string longer than the schema
    admits is refused instead of searched. ``states`` counts the number of ways
    the repetitions in a pattern can split a string, so a pattern that multiplies
    its split points instead of matching is refused before it runs.
    """

    __slots__ = ("consumed", "states", "unbounded")

    def __init__(self) -> None:
        self.consumed = 0
        self.states = 1
        self.unbounded = 0

    def _admit(self, *, path: str) -> None:
        if self.states > MAX_PATTERN_BACKTRACK_STATES:
            raise SchemaValidationError("pattern exceeds the backtracking budget", path=path)
        if self.consumed > MAX_PATTERN_INPUT_LENGTH:
            raise SchemaValidationError(
                "pattern can match a string beyond the bounded input", path=path
            )

    def literal(self, width: int, *, path: str) -> None:
        self.consumed += width
        self._admit(path=path)

    def repeat(self, width: int, maximum: int | None, *, path: str) -> None:
        if maximum is None:
            self.unbounded += 1
            if self.unbounded > MAX_PATTERN_UNBOUNDED_REPEATS:
                raise SchemaValidationError(
                    "pattern repeats more than one unbounded atom", path=path
                )
            self.states *= MAX_PATTERN_INPUT_LENGTH + 1
        else:
            if maximum > MAX_PATTERN_REPEAT:
                raise SchemaValidationError("pattern repetition exceeds the bound", path=path)
            self.states *= maximum + 1
            self.consumed += width * maximum
        self._admit(path=path)


class _PatternScope:
    """What the body of one open group contains."""

    __slots__ = ("alternative", "atoms", "width")

    def __init__(self) -> None:
        self.alternative = False
        self.atoms = 0
        self.width = 0


def _invalid_pattern(*, path: str) -> SchemaValidationError:
    return SchemaValidationError("pattern is not a valid regular expression", path=path)


def _escaped_atom_end(pattern: str, index: int, *, path: str) -> int:
    if index + 1 >= len(pattern):
        raise _invalid_pattern(path=path)
    return index + 2


def _character_class_end(pattern: str, index: int, *, path: str) -> int:
    cursor = index + 1
    if pattern[cursor : cursor + 1] == "^":
        cursor += 1
    if pattern[cursor : cursor + 1] == "]":
        cursor += 1
    while cursor < len(pattern) and pattern[cursor] != "]":
        cursor += 2 if pattern[cursor] == "\\" else 1
    if cursor >= len(pattern):
        raise _invalid_pattern(path=path)
    return cursor + 1


def _group_body_start(pattern: str, index: int, *, path: str) -> int:
    if pattern[index + 1 : index + 2] != "?":
        return index + 1
    if pattern[index + 2 : index + 3] == ":":
        return index + 3
    if pattern[index + 2 : index + 4] == "P<":
        name_end = pattern.find(">", index + 4)
        if name_end < 0:
            raise _invalid_pattern(path=path)
        return name_end + 1
    raise SchemaValidationError(
        "pattern uses a group construct outside the admitted subset", path=path
    )


def _modifier_end(pattern: str, index: int, *, path: str) -> int:
    marker = pattern[index : index + 1]
    if marker == "+":
        raise SchemaValidationError("pattern uses a possessive quantifier", path=path)
    return index + 1 if marker == "?" else index


def _repeated(
    pattern: str,
    index: int,
    *,
    width: int,
    path: str,
) -> tuple[int, int | None, int] | None:
    """Return the width, upper bound, and end of the quantifier at ``index``.

    ``width`` is how many characters one repetition of the atom consumes, and an
    upper bound of ``None`` means the repetition is unbounded. The caller has
    already parsed the atom, so a quantifier here always belongs to it.
    """
    quantifier = _quantifier_at(pattern, index)
    if quantifier is None:
        return None
    character = pattern[index]
    if character in {"*", "+"}:
        return (width, None, _modifier_end(pattern, quantifier, path=path))
    if character == "?":
        return (width, 1, _modifier_end(pattern, quantifier, path=path))
    match = _PATTERN_REPEAT_SPEC.match(pattern, index)
    if match is None:
        raise _invalid_pattern(path=path)
    minimum = int(match.group(1))
    if match.group(2) is None:
        return (width, minimum, _modifier_end(pattern, quantifier, path=path))
    if match.group(3) is None:
        return (width, None, _modifier_end(pattern, quantifier, path=path))
    maximum = int(match.group(3))
    if maximum < minimum:
        raise _invalid_pattern(path=path)
    return (width, maximum, _modifier_end(pattern, quantifier, path=path))


def _reject_ambiguous_pattern(pattern: str, *, path: str) -> None:
    """Refuse a pattern that can be matched in more than one bounded way.

    The admitted subset is deliberately small: literals, character classes, the
    wildcard, anchors, and a quantifier applied to a single character atom or to
    a group whose body is a plain concatenation. What makes a regular expression
    expensive is that neighbouring matches can claim the same text in more than
    one way, so every construct that creates such an overlap is refused instead
    of executed: quantifiers on adjacent atoms, alternatives under a quantifier,
    a quantifier nested in a quantified group, more than one unbounded repeat,
    and any repetition whose number of splits pushes the total number of split
    points past a fixed budget. The remaining patterns are matched in time
    bounded by the schema's own ``maxLength``.
    """
    scopes: list[_PatternScope] = []
    budget = _PatternBudget()
    quantified = False
    index = 0
    length = len(pattern)
    while index < length:
        character = pattern[index]
        if character == "(":
            scopes.append(_PatternScope())
            index = _group_body_start(pattern, index, path=path)
            quantified = False
            continue
        if character == ")":
            if not scopes:
                raise SchemaValidationError("pattern has an unbalanced group", path=path)
            scope = scopes.pop()
            repeated = _repeated(pattern, index + 1, width=scope.width, path=path)
            if repeated is None:
                index += 1
                quantified = False
                continue
            if scope.atoms == 0:
                raise SchemaValidationError("pattern repeats an empty group", path=path)
            if scope.alternative:
                raise SchemaValidationError(
                    "pattern repeats a group whose body has alternatives", path=path
                )
            if quantified:
                raise SchemaValidationError("pattern repeats adjacent atoms", path=path)
            budget.repeat(repeated[0], repeated[1], path=path)
            index = repeated[2]
            quantified = True
            continue
        if character == "|":
            if scopes:
                scopes[-1].alternative = True
            index += 1
            quantified = False
            continue
        if character in "^$":
            index += 1
            quantified = False
            continue
        if character == "\\":
            index = _escaped_atom_end(pattern, index, path=path)
        elif character == "[":
            index = _character_class_end(pattern, index, path=path)
        elif character == ".":
            index += 1
        elif character in "*+?" or _PATTERN_REPEAT_SPEC.match(pattern, index) is not None:
            raise _invalid_pattern(path=path)
        else:
            index += 1
        if scopes:
            open_scope = scopes[-1]
            open_scope.atoms += 1
            open_scope.width += 1
        repeated = _repeated(pattern, index, width=1, path=path)
        if repeated is None:
            budget.literal(1, path=path)
            quantified = False
        else:
            if quantified:
                raise SchemaValidationError("pattern repeats adjacent atoms", path=path)
            budget.repeat(repeated[0], repeated[1], path=path)
            index = repeated[2]
            quantified = True
    if scopes:
        raise SchemaValidationError("pattern has an unbalanced group", path=path)


def _validate_number(value: float, schema: Mapping[str, object], *, path: str) -> None:
    minimum = _schema_float(schema, "minimum", path=path)
    maximum = _schema_float(schema, "maximum", path=path)
    exclusive_minimum = _schema_float(schema, "exclusiveMinimum", path=path)
    exclusive_maximum = _schema_float(schema, "exclusiveMaximum", path=path)
    multiple_of = _schema_float(schema, "multipleOf", path=path)
    if minimum is not None and value < minimum:
        raise SchemaValidationError("number is below the permitted minimum", path=path)
    if maximum is not None and value > maximum:
        raise SchemaValidationError("number is above the permitted maximum", path=path)
    if exclusive_minimum is not None and value <= exclusive_minimum:
        raise SchemaValidationError("number is not above the exclusive minimum", path=path)
    if exclusive_maximum is not None and value >= exclusive_maximum:
        raise SchemaValidationError("number is not below the exclusive maximum", path=path)
    if multiple_of is not None:
        if multiple_of <= 0.0:
            raise SchemaValidationError("multipleOf must be positive", path=path)
        if not math.isclose(value % multiple_of, 0.0, abs_tol=1e-9):
            raise SchemaValidationError("number is not a multiple of the declared value", path=path)


def validate_against_schema(
    value: object,
    schema: Mapping[str, object],
    *,
    path: str = "$",
    depth: int = 0,
) -> None:
    if not isinstance(schema, Mapping):
        raise SchemaValidationError("schema must be an object", path=path)
    if depth > MAX_SCHEMA_DEPTH:
        raise SchemaValidationError("schema nesting exceeds the supported depth", path=path)
    _schema_keywords(schema, path=path)
    declared = schema.get("type")
    if declared is not None:
        names = tuple(declared) if isinstance(declared, (list, tuple)) else (declared,)
        for name in names:
            if not isinstance(name, str) or name not in _TYPE_CHECKS:
                raise SchemaValidationError(f"unsupported schema type: {name!r}", path=path)
        if not any(_TYPE_CHECKS[name](value) for name in names):
            raise SchemaValidationError(
                f"value does not match the declared type {'|'.join(names)}", path=path
            )
    if "const" in schema and not _json_equal(value, schema["const"]):
        raise SchemaValidationError("value does not match the declared const", path=path)
    if "enum" in schema:
        options = schema["enum"]
        if not isinstance(options, (list, tuple)) or not options:
            raise SchemaValidationError("enum must be a nonempty array", path=path)
        if not any(_json_equal(value, option) for option in options):
            raise SchemaValidationError("value is not a permitted enum member", path=path)
    for keyword in ("allOf",):
        branches = schema.get(keyword)
        if branches is None:
            continue
        if not isinstance(branches, (list, tuple)) or not branches:
            raise SchemaValidationError(f"{keyword} must be a nonempty array", path=path)
        for branch in branches:
            if not _schema_branch(value, branch, path=path, depth=depth):
                raise SchemaValidationError("value does not match a required subschema", path=path)
    for keyword in ("anyOf", "oneOf"):
        branches = schema.get(keyword)
        if branches is None:
            continue
        if not isinstance(branches, (list, tuple)) or not branches:
            raise SchemaValidationError(f"{keyword} must be a nonempty array", path=path)
        matched = sum(
            1 for branch in branches if _schema_branch(value, branch, path=path, depth=depth)
        )
        if matched == 0:
            raise SchemaValidationError(f"value does not match any {keyword} subschema", path=path)
        if keyword == "oneOf" and matched > 1:
            raise SchemaValidationError("value matches more than one oneOf subschema", path=path)
    if isinstance(value, Mapping):
        _validate_object(value, schema, path=path)
    elif isinstance(value, (list, tuple)):
        _validate_array(value, schema, path=path)
    elif isinstance(value, str):
        _validate_string(value, schema, path=path)
    elif isinstance(value, (int, float)) and not isinstance(value, bool):
        _validate_number(float(value), schema, path=path)


def check_object_schema(schema: object, *, label: str = "schema") -> Mapping[str, object]:
    if not isinstance(schema, Mapping):
        raise SchemaValidationError(f"{label} must be a JSON object")
    declared = schema.get("type")
    if declared != "object":
        raise SchemaValidationError(f"{label} must declare an object type")
    if schema.get("additionalProperties") is not False:
        raise SchemaValidationError(f"{label} must forbid additional properties")
    properties = schema.get("properties")
    if not isinstance(properties, Mapping) or not properties:
        raise SchemaValidationError(f"{label} must declare a nonempty properties map")
    for key, subschema in properties.items():
        if not isinstance(key, str) or not isinstance(subschema, Mapping):
            raise SchemaValidationError(f"{label} properties must map names to schemas")
        _schema_keywords(subschema, path=f"{label}.{key}")
        _pattern_rejects(subschema, path=f"{label}.{key}")
    required = schema.get("required", [])
    if not isinstance(required, (list, tuple)):
        raise SchemaValidationError(f"{label} required entries must be an array")
    for name in required:
        if not isinstance(name, str) or name not in properties:
            raise SchemaValidationError(f"{label} requires an undeclared property")
    _schema_keywords(schema, path=label)
    _pattern_rejects(schema, path=label)
    return schema


def _bounded_identifier(value: object, field_name: str) -> str:
    if not isinstance(value, str):
        raise ToolEnvelopeError(f"{field_name} must be a string")
    normalized = value.strip()
    if _IDENTIFIER_PATTERN.fullmatch(normalized) is None:
        raise ToolEnvelopeError(f"{field_name} must be a safe identifier")
    return normalized


def _bounded_version(value: object, field_name: str) -> str:
    if not isinstance(value, str):
        raise ToolEnvelopeError(f"{field_name} must be a string")
    normalized = value.strip()
    if _VERSION_PATTERN.fullmatch(normalized) is None:
        raise ToolEnvelopeError(f"{field_name} must be a safe version string")
    return normalized


def _bounded_text(value: object, field_name: str, *, maximum: int = MAX_STRING_LENGTH) -> str:
    if not isinstance(value, str):
        raise ToolEnvelopeError(f"{field_name} must be a string")
    normalized = value.strip()
    if not normalized:
        raise ToolEnvelopeError(f"{field_name} must be nonempty")
    if len(normalized) > maximum:
        raise ToolEnvelopeError(f"{field_name} exceeds the bounded length")
    return normalized


def _positive_number(value: object, field_name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ToolEnvelopeError(f"{field_name} must be a number")
    parsed = float(value)
    if not math.isfinite(parsed) or parsed <= 0.0:
        raise ToolEnvelopeError(f"{field_name} must be finite and positive")
    return parsed


def _positive_integer(value: object, field_name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ToolEnvelopeError(f"{field_name} must be an integer")
    if value < 1:
        raise ToolEnvelopeError(f"{field_name} must be positive")
    return value


def _frozen_object(value: object, field_name: str) -> Mapping[str, JSONValue]:
    if not isinstance(value, Mapping):
        raise ToolEnvelopeError(f"{field_name} must be a JSON object")
    for key in value:
        if not isinstance(key, str):
            raise ToolEnvelopeError(f"{field_name} keys must be strings")
    try:
        return MappingProxyType(
            {key: cast(JSONValue, item) for key, item in value.items()},
        )
    except (RecursionError, TypeError, ValueError) as error:
        raise ToolEnvelopeError(f"{field_name} is not a JSON object") from error


def _capability_tuple(value: object, field_name: str) -> tuple[str, ...]:
    if isinstance(value, (str, bytes)) or not isinstance(value, (Sequence, set, frozenset)):
        raise ToolEnvelopeError(f"{field_name} must be a collection of capability names")
    names: set[str] = set()
    for item in value:
        names.add(_bounded_identifier(item, f"{field_name} entry"))
    return tuple(sorted(names))


def _run_status(value: object, field_name: str) -> ToolRunStatus:
    if isinstance(value, ToolRunStatus):
        return value
    try:
        return ToolRunStatus(cast(str, value))
    except ValueError as error:
        raise ToolEnvelopeError(f"{field_name} is not a tool run status") from error


def resolve_workspace_path(workspace: Path, relative: str) -> Path:
    if not isinstance(relative, str) or not relative.strip():
        raise ToolWorkspaceError("workspace paths must be nonempty strings")
    if "\x00" in relative or "\\" in relative:
        raise ToolWorkspaceError("workspace paths cannot contain null bytes or backslashes")
    if len(relative) > MAX_PATH_LENGTH:
        raise ToolWorkspaceError("workspace path exceeds the bounded length")
    candidate = PurePosixPath(relative)
    if candidate.is_absolute():
        raise ToolWorkspaceError("workspace paths must be relative")
    segments = [segment for segment in candidate.parts if segment not in {"", "."}]
    if not segments or len(segments) > MAX_PATH_SEGMENTS:
        raise ToolWorkspaceError("workspace path has an unusable segment count")
    for segment in segments:
        if segment == ".." or _SEGMENT_PATTERN.fullmatch(segment) is None:
            raise ToolWorkspaceError("workspace path contains an unsafe segment")
    base = workspace.resolve(strict=False)
    if not base.is_dir():
        raise ToolWorkspaceError("workspace directory does not exist")
    target = base.joinpath(*segments)
    current = base
    for segment in segments:
        current = current / segment
        if current.is_symlink():
            raise ToolWorkspaceError("workspace paths cannot traverse symlinks")
    resolved = target.resolve(strict=False)
    if not resolved.is_relative_to(base):
        raise ToolWorkspaceError("workspace path escapes its run directory")
    return target


@dataclass(frozen=True, slots=True)
class ToolRequestEnvelope:
    protocol_version: str
    kind: ToolEnvelopeKind
    call_id: str
    tool_id: str
    tool_version: str
    arguments: Mapping[str, JSONValue]
    workspace: str
    deadline_seconds: float
    max_output_bytes: int
    max_artifact_bytes: int
    granted_capabilities: tuple[str, ...]

    def __post_init__(self) -> None:
        object.__setattr__(
            self, "protocol_version", _bounded_version(self.protocol_version, "protocol_version")
        )
        if not isinstance(self.kind, ToolEnvelopeKind):
            raise ToolEnvelopeError("kind must be a ToolEnvelopeKind")
        if self.kind is not ToolEnvelopeKind.REQUEST:
            raise ToolEnvelopeError("kind must be a request envelope")
        object.__setattr__(self, "call_id", _bounded_identifier(self.call_id, "call_id"))
        object.__setattr__(self, "tool_id", _bounded_identifier(self.tool_id, "tool_id"))
        object.__setattr__(
            self, "tool_version", _bounded_version(self.tool_version, "tool_version")
        )
        object.__setattr__(self, "arguments", _frozen_object(self.arguments, "arguments"))
        canonical_json_dumps(self.arguments, max_bytes=MAX_ARGUMENT_BYTES)
        workspace = _bounded_text(self.workspace, "workspace", maximum=MAX_PATH_LENGTH)
        if not workspace.startswith("/"):
            raise ToolEnvelopeError("workspace must be an absolute path")
        object.__setattr__(self, "workspace", workspace)
        object.__setattr__(
            self, "deadline_seconds", _positive_number(self.deadline_seconds, "deadline_seconds")
        )
        object.__setattr__(
            self, "max_output_bytes", _positive_integer(self.max_output_bytes, "max_output_bytes")
        )
        object.__setattr__(
            self,
            "max_artifact_bytes",
            _positive_integer(self.max_artifact_bytes, "max_artifact_bytes"),
        )
        object.__setattr__(
            self,
            "granted_capabilities",
            _capability_tuple(self.granted_capabilities, "granted_capabilities"),
        )

    @property
    def arguments_json(self) -> str:
        return canonical_json_dumps(self.arguments, max_bytes=MAX_ARGUMENT_BYTES)

    @property
    def arguments_digest(self) -> str:
        return sha256_hex(self.arguments_json)

    @property
    def envelope_digest(self) -> str:
        """Bind every dispatch-relevant field of this exact envelope."""
        return canonical_json_digest(self.to_payload())

    def to_payload(self) -> JsonObject:
        return {
            "arguments": cast(JsonObject, dict(self.arguments)),
            "call_id": self.call_id,
            "deadline_seconds": self.deadline_seconds,
            "granted_capabilities": list(self.granted_capabilities),
            "kind": self.kind.value,
            "max_artifact_bytes": self.max_artifact_bytes,
            "max_output_bytes": self.max_output_bytes,
            "protocol_version": self.protocol_version,
            "tool_id": self.tool_id,
            "tool_version": self.tool_version,
            "workspace": self.workspace,
        }

    def to_json(self) -> str:
        return canonical_json_dumps(self.to_payload())

    @classmethod
    def from_payload(cls, payload: Mapping[str, JSONValue]) -> ToolRequestEnvelope:
        return cls(
            protocol_version=cast(str, payload.get("protocol_version", PROTOCOL_VERSION)),
            kind=ToolEnvelopeKind.REQUEST,
            call_id=cast(str, payload.get("call_id", "")),
            tool_id=cast(str, payload.get("tool_id", "")),
            tool_version=cast(str, payload.get("tool_version", "")),
            arguments=cast(Mapping[str, JSONValue], payload.get("arguments", {})),
            workspace=cast(str, payload.get("workspace", "")),
            deadline_seconds=cast(float, payload.get("deadline_seconds", 0.0)),
            max_output_bytes=cast(int, payload.get("max_output_bytes", 0)),
            max_artifact_bytes=cast(int, payload.get("max_artifact_bytes", 0)),
            granted_capabilities=tuple(
                cast(Sequence[str], payload.get("granted_capabilities", ()))
            ),
        )

    @classmethod
    def from_json(cls, payload: str | bytes) -> ToolRequestEnvelope:
        decoded = strict_json_loads(payload)
        if not isinstance(decoded, Mapping):
            raise ToolEnvelopeError("request envelope must be a JSON object")
        return cls.from_payload(cast(Mapping[str, JSONValue], decoded))

    @classmethod
    def from_call(
        cls,
        call: ToolCall,
        *,
        workspace: Path,
        deadline_seconds: float,
        max_output_bytes: int,
        max_artifact_bytes: int,
        granted_capabilities: Iterable[str],
    ) -> ToolRequestEnvelope:
        if not isinstance(call, ToolCall):
            raise TypeError("call must be a ToolCall")
        return cls(
            protocol_version=PROTOCOL_VERSION,
            kind=ToolEnvelopeKind.REQUEST,
            call_id=call.call_id,
            tool_id=call.tool_id,
            tool_version=call.tool_version,
            arguments=cast(Mapping[str, JSONValue], strict_json_loads(call.arguments_json)),
            workspace=workspace.resolve(strict=False).as_posix(),
            deadline_seconds=deadline_seconds,
            max_output_bytes=max_output_bytes,
            max_artifact_bytes=max_artifact_bytes,
            granted_capabilities=tuple(granted_capabilities),
        )


@dataclass(frozen=True, slots=True)
class ToolResponseEnvelope:
    protocol_version: str
    kind: ToolEnvelopeKind
    call_id: str
    tool_id: str
    tool_version: str
    status: ToolRunStatus
    result: Mapping[str, JSONValue] | None
    error_code: str | None
    error_message: str | None
    artifacts: tuple[ArtifactRef, ...]
    duration_ms: float

    def __post_init__(self) -> None:
        object.__setattr__(
            self, "protocol_version", _bounded_version(self.protocol_version, "protocol_version")
        )
        if not isinstance(self.kind, ToolEnvelopeKind):
            raise ToolEnvelopeError("kind must be a ToolEnvelopeKind")
        if self.kind is not ToolEnvelopeKind.RESPONSE:
            raise ToolEnvelopeError("kind must be a response envelope")
        object.__setattr__(self, "call_id", _bounded_identifier(self.call_id, "call_id"))
        object.__setattr__(self, "tool_id", _bounded_identifier(self.tool_id, "tool_id"))
        object.__setattr__(
            self, "tool_version", _bounded_version(self.tool_version, "tool_version")
        )
        object.__setattr__(self, "status", _run_status(self.status, "status"))
        if self.result is None:
            object.__setattr__(self, "result", None)
        else:
            object.__setattr__(self, "result", _frozen_object(self.result, "result"))
            canonical_json_dumps(self.result)
        if self.error_code is None:
            if self.status in _FAILURE_STATUSES:
                raise ToolEnvelopeError("failed tool runs require an error code")
        else:
            object.__setattr__(
                self,
                "error_code",
                _bounded_identifier(
                    str(self.error_code.value)
                    if isinstance(self.error_code, ToolErrorCode)
                    else self.error_code,
                    "error_code",
                ),
            )
        if self.error_message is None:
            object.__setattr__(self, "error_message", None)
        else:
            object.__setattr__(
                self,
                "error_message",
                _bounded_text(self.error_message, "error_message", maximum=2_048),
            )
        artifacts = self.artifacts
        if not isinstance(artifacts, (tuple, list)) or any(
            not isinstance(item, ArtifactRef) for item in artifacts
        ):
            raise ToolEnvelopeError("artifacts must be ArtifactRef values")
        object.__setattr__(self, "artifacts", tuple(artifacts))
        if isinstance(self.duration_ms, bool) or not isinstance(self.duration_ms, (int, float)):
            raise ToolEnvelopeError("duration_ms must be a number")
        duration = float(self.duration_ms)
        if not math.isfinite(duration) or duration < 0.0:
            raise ToolEnvelopeError("duration_ms must be finite and nonnegative")
        object.__setattr__(self, "duration_ms", duration)
        if self.status in _FAILURE_STATUSES and self.result is not None:
            raise ToolEnvelopeError("failed tool runs cannot carry a result payload")
        if self.status is ToolRunStatus.SUCCEEDED and self.result is None:
            raise ToolEnvelopeError("successful tool runs require a result payload")

    @property
    def succeeded(self) -> bool:
        return self.status is ToolRunStatus.SUCCEEDED

    @property
    def result_json(self) -> str | None:
        if self.result is None:
            return None
        return canonical_json_dumps(self.result)

    @property
    def result_digest(self) -> str:
        return sha256_hex(self.result_json or "{}")

    def to_payload(self) -> JsonObject:
        return {
            "artifacts": [artifact.model_dump(mode="json") for artifact in self.artifacts],
            "call_id": self.call_id,
            "duration_ms": self.duration_ms,
            "error_code": None if self.error_code is None else str(self.error_code),
            "error_message": self.error_message,
            "kind": self.kind.value,
            "protocol_version": self.protocol_version,
            "result": None if self.result is None else cast(JsonObject, dict(self.result)),
            "status": self.status.value,
            "tool_id": self.tool_id,
            "tool_version": self.tool_version,
        }

    def to_json(self) -> str:
        return canonical_json_dumps(self.to_payload())

    @classmethod
    def succeeded_envelope(
        cls,
        *,
        call_id: str,
        tool_id: str,
        tool_version: str,
        result: Mapping[str, JSONValue],
        artifacts: Sequence[ArtifactRef] = (),
        duration_ms: float = 0.0,
    ) -> ToolResponseEnvelope:
        return cls(
            protocol_version=PROTOCOL_VERSION,
            kind=ToolEnvelopeKind.RESPONSE,
            call_id=call_id,
            tool_id=tool_id,
            tool_version=tool_version,
            status=ToolRunStatus.SUCCEEDED,
            result=result,
            error_code=None,
            error_message=None,
            artifacts=tuple(artifacts),
            duration_ms=duration_ms,
        )

    @classmethod
    def failed_envelope(
        cls,
        *,
        call_id: str,
        tool_id: str,
        tool_version: str,
        status: ToolRunStatus = ToolRunStatus.FAILED,
        error_code: ToolErrorCode | str | None,
        error_message: str | None = None,
        duration_ms: float = 0.0,
    ) -> ToolResponseEnvelope:
        return cls(
            protocol_version=PROTOCOL_VERSION,
            kind=ToolEnvelopeKind.RESPONSE,
            call_id=call_id,
            tool_id=tool_id,
            tool_version=tool_version,
            status=status,
            result=None,
            error_code=None if error_code is None else str(error_code),
            error_message=error_message,
            artifacts=(),
            duration_ms=duration_ms,
        )

    @classmethod
    def from_payload(cls, payload: Mapping[str, JSONValue]) -> ToolResponseEnvelope:
        raw_artifacts = payload.get("artifacts", [])
        if not isinstance(raw_artifacts, list):
            raise ToolEnvelopeError("artifacts must be an array")
        artifacts: list[ArtifactRef] = []
        for item in raw_artifacts:
            if not isinstance(item, Mapping):
                raise ToolEnvelopeError("artifacts must be ArtifactRef objects")
            try:
                artifacts.append(ArtifactRef.model_validate(dict(item)))
            except Exception as error:
                raise ToolEnvelopeError("artifact reference is invalid") from error
        status_value = payload.get("status")
        status = _run_status(status_value, "status")
        result = payload.get("result")
        if result is not None and not isinstance(result, Mapping):
            raise ToolEnvelopeError("result must be a JSON object")
        error_code = payload.get("error_code")
        error_message = payload.get("error_message")
        return cls(
            protocol_version=cast(str, payload.get("protocol_version", PROTOCOL_VERSION)),
            kind=ToolEnvelopeKind.RESPONSE,
            call_id=cast(str, payload.get("call_id", "")),
            tool_id=cast(str, payload.get("tool_id", "")),
            tool_version=cast(str, payload.get("tool_version", "")),
            status=status,
            result=cast(Mapping[str, JSONValue] | None, result),
            error_code=None if error_code is None else cast(str, error_code),
            error_message=None if error_message is None else cast(str, error_message),
            artifacts=tuple(artifacts),
            duration_ms=cast(float, payload.get("duration_ms", 0.0)),
        )

    @classmethod
    def from_json(cls, payload: str | bytes) -> ToolResponseEnvelope:
        decoded = strict_json_loads(payload)
        if not isinstance(decoded, Mapping):
            raise ToolEnvelopeError("response envelope must be a JSON object")
        return cls.from_payload(cast(Mapping[str, JSONValue], decoded))

    def to_result(self, *, started_at: datetime, finished_at: datetime) -> ToolResult:
        if started_at.tzinfo is None or started_at.utcoffset() is None:
            raise ToolEnvelopeError("started_at must include a timezone")
        if finished_at.tzinfo is None or finished_at.utcoffset() is None:
            raise ToolEnvelopeError("finished_at must include a timezone")
        if finished_at < started_at:
            raise ToolEnvelopeError("finished_at cannot precede started_at")
        return ToolResult(
            call_id=self.call_id,
            tool_id=self.tool_id,
            status=self.status,
            result_json=self.result_json,
            error_code=None if self.error_code is None else str(self.error_code),
            error_message=self.error_message,
            artifacts=self.artifacts,
            started_at=started_at,
            finished_at=finished_at,
            duration_ms=self.duration_ms,
        )


@runtime_checkable
class ToolHandler(Protocol):
    @property
    def descriptor(self) -> ToolDescriptor: ...

    def invoke(self, request: ToolRequestEnvelope) -> ToolResponseEnvelope: ...


@dataclass(frozen=True, slots=True)
class SubprocessToolHandler:
    descriptor: ToolDescriptor
    entry_point: str
    interpreter_args: tuple[str, ...] = ("-B", "-s")

    def __post_init__(self) -> None:
        if not isinstance(self.descriptor, ToolDescriptor):
            raise ToolEnvelopeError("descriptor must be a ToolDescriptor")
        entry_point = self.entry_point
        if not isinstance(entry_point, str):
            raise ToolEnvelopeError("entry_point must be a string")
        module_name, separator, attribute = entry_point.partition(":")
        if not separator or not module_name or not attribute:
            raise ToolEnvelopeError("entry_point must be formatted as module:attribute")
        for part in (*module_name.split("."), attribute):
            if not part.isidentifier():
                raise ToolEnvelopeError("entry_point components must be identifiers")
        object.__setattr__(self, "entry_point", entry_point)
        arguments = self.interpreter_args
        if not isinstance(arguments, (tuple, list)) or any(
            not isinstance(item, str) for item in arguments
        ):
            raise ToolEnvelopeError("interpreter_args must be a sequence of strings")
        normalized = tuple(arguments)
        for item in normalized:
            if not item.startswith("-") or item in {"-c", "-E", "-I", "-m"}:
                raise ToolEnvelopeError("interpreter_args cannot replace the worker bootstrap")
        object.__setattr__(self, "interpreter_args", normalized)

    def invoke(self, request: ToolRequestEnvelope) -> ToolResponseEnvelope:
        raise ToolProtocolError(
            "subprocess handlers are dispatched by the executor and cannot run in process"
        )


WORKER_BOOTSTRAP: Final[str] = "\n".join(
    (
        "import importlib",
        "import sys",
        "from kala_agent.tools.protocol import (",
        "    ToolErrorCode,",
        "    ToolResponseEnvelope,",
        "    ToolRunStatus,",
        "    read_request_stream,",
        "    write_response_stream,",
        ")",
        "entry_point = sys.argv[1]",
        "module_name, _, attribute = entry_point.partition(':')",
        "module = importlib.import_module(module_name)",
        "target = getattr(module, attribute)",
        "request = read_request_stream(sys.stdin.buffer)",
        "channel = sys.stdout",
        "sys.stdout = sys.stderr",
        "try:",
        "    runner = target if callable(target) else getattr(target, 'invoke')",
        "    envelope = runner(request)",
        "except BaseException as error:",
        "    envelope = ToolResponseEnvelope.failed_envelope(",
        "        call_id=request.call_id,",
        "        tool_id=request.tool_id,",
        "        tool_version=request.tool_version,",
        "        status=ToolRunStatus.FAILED,",
        "        error_code=ToolErrorCode.INTERNAL_ERROR,",
        "        error_message=type(error).__name__,",
        "    )",
        "sys.stdout = channel",
        "write_response_stream(envelope, channel)",
    )
)


def build_worker_command(
    handler: SubprocessToolHandler,
    *,
    python_executable: str | None = None,
) -> tuple[str, ...]:
    if not isinstance(handler, SubprocessToolHandler):
        raise TypeError("handler must be a SubprocessToolHandler")
    interpreter = python_executable if python_executable is not None else sys.executable
    if not interpreter:
        raise ToolProtocolError("no python interpreter is available for the tool worker")
    return (
        interpreter,
        *handler.interpreter_args,
        "-c",
        WORKER_BOOTSTRAP,
        handler.entry_point,
    )


PROTECTED_CHILD_ENVIRONMENT_KEYS: Final[frozenset[str]] = frozenset(
    {
        "HOME",
        "KALA_TOOL_PROTOCOL",
        "LANG",
        "PATH",
        "PYTHONDONTWRITEBYTECODE",
        "PYTHONHASHSEED",
        "PYTHONIOENCODING",
        "PYTHONPATH",
        "PYTHONNOUSERSITE",
        "PYTHONSAFEPATH",
        "TMP",
        "TMPDIR",
        "TEMP",
    }
)


def build_child_environment(
    *,
    workspace: Path,
    import_roots: Sequence[Path] = (),
    extra: Mapping[str, str] | None = None,
) -> dict[str, str]:
    """Build the child's environment, refusing any override of a fixed key.

    The executor owns every key that bounds the child: the working home, the
    import roots, the loader path, and the temporary directory. A profile may
    not replace any of them, so a tool can never be pointed at another home
    directory, a host import path, or a writable temporary location.
    """
    resolved_workspace = workspace.resolve(strict=False).as_posix()
    roots = [str(Path(root).resolve(strict=False)) for root in import_roots]
    environment = {
        "HOME": resolved_workspace,
        "KALA_TOOL_PROTOCOL": PROTOCOL_VERSION,
        "LANG": "C.UTF-8",
        "PATH": "/usr/local/bin:/usr/bin:/bin",
        "PYTHONDONTWRITEBYTECODE": "1",
        "PYTHONHASHSEED": "0",
        "PYTHONIOENCODING": "utf-8",
        "PYTHONNOUSERSITE": "1",
        "PYTHONPATH": os.pathsep.join(roots),
        "PYTHONSAFEPATH": "1",
        "TMPDIR": resolved_workspace,
    }
    if extra is not None:
        for key, value in extra.items():
            if not isinstance(key, str) or _ENVIRONMENT_KEY_PATTERN.fullmatch(key) is None:
                raise ToolEnvelopeError("environment keys must be simple names")
            if key in PROTECTED_CHILD_ENVIRONMENT_KEYS:
                raise ToolEnvelopeError(f"environment key {key} is protected by the executor")
            if not isinstance(value, str) or "\x00" in value:
                raise ToolEnvelopeError("environment values must be null-free strings")
            environment[key] = value
    return environment


def read_request_stream(stream: IO[bytes]) -> ToolRequestEnvelope:
    data = stream.read(MAX_JSON_BYTES + 1)
    if len(data) > MAX_JSON_BYTES:
        raise ToolEnvelopeError("request envelope exceeds the bounded size")
    return ToolRequestEnvelope.from_json(data)


def write_response_stream(
    envelope: ToolResponseEnvelope,
    stream: TextIO | IO[bytes],
) -> None:
    payload = envelope.to_json() + "\n"
    if isinstance(stream, TextIOBase):
        stream.write(payload)
    else:
        cast(IO[bytes], stream).write(payload.encode("utf-8"))
    stream.flush()


def summarize_error(error: BaseException, *, replacement: str | None = None) -> str:
    text = f"{type(error).__name__}"
    detail = str(error).strip()
    if detail:
        if replacement is not None:
            detail = detail.replace(replacement, "<workspace>")
        detail = re.sub(r"\s+", " ", detail)[:512]
        return f"{text}: {detail}"
    return text


def sanitize_child_stderr(payload: bytes, *, replacement: str | None = None) -> str:
    """Turn a child's stderr into a short, sanitized diagnostic.

    An import failure or a sandbox setup refusal is only actionable if its
    message survives, so the first and the last line are kept: for a Python
    traceback the last line carries the exception. A child is untrusted, so the
    sample is bounded, stripped of control characters, and has host paths
    replaced.
    """
    if not payload:
        return ""
    text = payload[: MAX_SANITIZED_DETAIL_LENGTH * 2].decode("utf-8", errors="replace")
    text = "".join(character for character in text if character == "\n" or character >= " ")
    if replacement is not None and replacement:
        text = text.replace(replacement, "<workspace>")
    lines = [re.sub(r"\s+", " ", line).strip() for line in text.splitlines()]
    selected = [line for line in lines if line]
    if len(selected) > 2:
        selected = [selected[0], "...", selected[-1]]
    return " | ".join(selected)[:MAX_SANITIZED_DETAIL_LENGTH]


__all__ = (
    "MAX_ARGUMENT_BYTES",
    "MAX_COLLECTION_ITEMS",
    "MAX_JSON_BYTES",
    "MAX_PATTERN_INPUT_LENGTH",
    "MAX_PATTERN_LENGTH",
    "MAX_STRING_LENGTH",
    "PROTECTED_CHILD_ENVIRONMENT_KEYS",
    "PROTOCOL_VERSION",
    "WORKER_BOOTSTRAP",
    "JSONValue",
    "JsonEnvelopeError",
    "JsonObject",
    "SchemaValidationError",
    "SubprocessToolHandler",
    "ToolEnvelopeError",
    "ToolEnvelopeKind",
    "ToolErrorCode",
    "ToolHandler",
    "ToolProtocolError",
    "ToolRequestEnvelope",
    "ToolResponseEnvelope",
    "ToolWorkspaceError",
    "build_child_environment",
    "build_worker_command",
    "canonical_json_digest",
    "canonical_json_dumps",
    "check_object_schema",
    "read_request_stream",
    "resolve_workspace_path",
    "sanitize_child_stderr",
    "sha256_hex",
    "strict_json_loads",
    "summarize_error",
    "validate_against_schema",
    "write_response_stream",
)
